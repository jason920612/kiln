//! Bees (`Bee`, an `Animal` and a `NeutralMob`): small flyers that go from flower to flower
//! (`BeePollinateGoal`) until they carry nectar, take it home to a beehive or bee nest
//! (`BeeGoToHiveGoal`, `BeeEnterHiveGoal`: the hive keeps them for a while and fills up with honey), help
//! crops grow on the way (`BeeGrowCropGoal`) and sting what hurt them or their hive — once: a bee
//! that stung loses its stinger and dies within a minute or so.
//!
//! Flight is a `FlyingMoveControl` (max turn 20, hovering) with a `FlyingPathNavigation` that
//! floats not and needs a path of 48; falls hurt nothing.

use super::anger::{self, Anger, AngryAtPlayerGoal};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{BeehiveView, DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::control;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::fly;
use crate::mob::goals::{self, Goal, Living, MOVE, MeleeKind};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::path::{self, PathType};
use crate::mob::random_pos;
use crate::mob::{DamageSource, MobData};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};
use std::collections::HashMap;

pub struct Bee;

pub static KIND: Bee = Bee;

static INFO: Info = Info {
    sounds: Some("bee"),
    ..Info::animal("minecraft:bee", &[(MaxHealth, 10.0), (FlyingSpeed, 0.6000000238418579), (MovementSpeed, 0.30000001192092896), (AttackDamage, 2.0)])
};

/// `DATA_FLAGS_ID` bits: rolling (angry and close), stung, nectar.
const FLAG_ROLL: u8 = 2;
const FLAG_STUNG: u8 = 4;
const FLAG_NECTAR: u8 = 8;

#[derive(Clone, Debug)]
pub struct State {
    /// `NeutralMob`: `DATA_ANGER_END_TIME` and the persistent anger target.
    pub anger: Anger,
    pub nectar: bool,
    pub stung: bool,
    pub rolling: bool,
    pub roll_amount: f32,
    pub roll_amount_o: f32,
    pub time_since_sting: i32,
    /// `ticksWithoutNectarSinceExitingHive` (saved as `TicksSincePollination`).
    pub ticks_without_nectar: i32,
    /// `stayOutOfHiveCountdown` (`CannotEnterHiveTicks`).
    pub stay_out_of_hive: i32,
    pub crops_grown: i32,
    pub cooldown_hive: i32,
    pub cooldown_flower: i32,
    pub flower_pos: Option<BlockPos>,
    pub hive_pos: Option<BlockPos>,
    pub under_water: i32,
    /// `BeePollinateGoal.pollinating`.
    pub pollinating: bool,
    /// `BeeGoToHiveGoal.blacklistedTargets` (at most 3).
    pub blacklist: Vec<BlockPos>,
    /// The cooldowns of the validate goals (`Mth.nextInt(random, 20, 40)` each, drawn when the
    /// goals are made).
    validate_hive: i32,
    validate_flower: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("bee state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("bee state")
}

/// `Mth.nextInt(random, min, max)`.
fn next_int(r: &mut dyn RandomSource, min: i32, max: i32) -> i32 {
    if min >= max { min } else { r.next_int_bounded(max - min + 1) + min }
}

/// `NeutralMob.isAngry`.
pub fn is_angry(m: &MobData, level: &dyn EntityLevel) -> bool {
    anger::is_angry(m, level)
}

/// `Bee.hasStung`.
pub fn has_stung(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.stung)
}

/// `BlockPos.closerThan(Vec3i, d)` against the bee's block.
fn closer_than(e: &Entity, p: BlockPos, d: i32) -> bool {
    let b = e.block_position();
    let (dx, dy, dz) = ((p.x - b.x) as f64, (p.y - b.y) as f64, (p.z - b.z) as f64);
    dx * dx + dy * dy + dz * dz < (d * d) as f64
}

/// `Bee.isTooFarAway`.
fn too_far(e: &Entity, p: BlockPos) -> bool {
    !closer_than(e, p, 48)
}

/// `Mob.hasHome`.
fn has_home(m: &MobData) -> bool {
    m.home.is_some_and(|(_, r)| r != -1)
}

/// `Bee.attractsBees`: a flower that is not under water (the lower half of a sunflower is not).
pub fn attracts_bees(state: u16) -> bool {
    if !super::wolf::block_in_tag(state, "minecraft:bee_attractive") {
        return false;
    }
    let info = kiln_data::blocks_types::block_of(state);
    if info.property(state, "waterlogged") == Some("true") {
        return false;
    }
    if info.name == "minecraft:sunflower" {
        return info.property(state, "half") == Some("upper");
    }
    true
}

/// `Bee.getBeehiveBlockEntity`.
fn beehive(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<BeehiveView> {
    let p = st(m).hive_pos?;
    if too_far(e, p) {
        return None;
    }
    level.beehive_at(p)
}

fn is_hive_valid(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    beehive(e, m, level).is_some()
}

/// `Bee.wantsToEnterHive`.
fn wants_to_enter_hive(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    let s = st(m);
    if s.stay_out_of_hive <= 0 && !s.pollinating && !s.stung && m.target.is_none() {
        let flag = s.nectar || s.ticks_without_nectar > 3600 || level.bees_stay_in_hive();
        flag && !beehive(e, m, level).is_some_and(|b| b.fire_nearby)
    } else {
        false
    }
}

fn drop_hive(m: &mut MobData) {
    let s = st_mut(m);
    s.hive_pos = None;
    s.cooldown_hive = 200;
}

fn drop_flower(e: &mut Entity, m: &mut MobData) {
    let s = st_mut(m);
    s.flower_pos = None;
    s.cooldown_flower = next_int(&mut e.random, 20, 60);
}

/// `Bee.pathfindRandomlyTowards`.
fn pathfind_randomly_towards(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, pos: BlockPos) {
    let target = Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5);
    let mut i = 0;
    let here = e.block_position();
    let j = (target.y as i32) - here.y;
    if j > 2 {
        i = 4;
    } else if j < -2 {
        i = -4;
    }
    let (mut k, mut l) = (6, 8);
    let manhattan = (here.x - pos.x).abs() + (here.y - pos.y).abs() + (here.z - pos.z).abs();
    if manhattan < 15 {
        k = manhattan / 2;
        l = manhattan / 2;
    }
    // `AirRandomPos.getPosTowards`.
    let Some(p) = random_pos::air_pos_towards(e, m, level, k, l, i, target, 0.3141592741012573) else { return };
    m.nav.max_visited_nodes_multiplier = 0.5;
    path::move_to(e, m, level, p.x, p.y, p.z, 1.0);
}

// ---------------------------------------------------------------------- the type

impl Kind for Bee {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructor: the goals' cooldown draws (`registerGoals` runs in the superclass
    /// constructor), the flower search cooldown, a `FlyingMoveControl(this, 20, true)` and a
    /// `FlyingPathNavigation` that does not float, opens no doors, needs a path of 48.
    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.fly = true;
        m.nav.can_float = false;
        m.nav.can_open_doors = false;
        m.nav.required_path_length = 48.0;
        m.maluses.retain(|(t, _)| !matches!(t, PathType::Fire | PathType::Water | PathType::WaterBorder | PathType::Cocoa | PathType::Fence));
        m.maluses.push((PathType::Fire, -1.0));
        m.maluses.push((PathType::Water, -1.0));
        m.maluses.push((PathType::WaterBorder, 16.0));
        m.maluses.push((PathType::Cocoa, -1.0));
        m.maluses.push((PathType::Fence, -1.0));
        let validate_hive = next_int(random, 20, 40);
        let validate_flower = next_int(random, 20, 40);
        let cooldown_flower = next_int(random, 20, 60);
        Some(Box::new(State {
            anger: Anger::default(),
            nectar: false,
            stung: false,
            rolling: false,
            roll_amount: 0.0,
            roll_amount_o: 0.0,
            time_since_sting: 0,
            ticks_without_nectar: 0,
            stay_out_of_hive: 0,
            crops_grown: 0,
            cooldown_hive: 0,
            cooldown_flower,
            flower_pos: None,
            hive_pos: None,
            under_water: 0,
            pollinating: false,
            blacklist: Vec::new(),
            validate_hive,
            validate_flower,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let (vh, vf) = (st(m).validate_hive, st(m).validate_flower);
        let g = &mut m.goals;
        g.add(0, Goal::Melee { kind: MeleeKind::Bee, speed: 1.399999976158142, follow_unseen: true, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(1, Goal::Custom(Box::new(EnterHiveGoal)));
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(3, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
        g.add(3, Goal::Custom(Box::new(ValidateHiveGoal { cooldown: vh, last_validate: -1 })));
        g.add(3, Goal::Custom(Box::new(ValidateFlowerGoal { cooldown: vf, last_validate: -1 })));
        g.add(4, Goal::Custom(Box::new(PollinateGoal::default())));
        g.add(5, Goal::FollowParent { speed: 1.25, parent: None, recalc: 0 });
        g.add(5, Goal::Custom(Box::new(LocateHiveGoal)));
        g.add(5, Goal::Custom(Box::new(GoToHiveGoal::default())));
        g.add(6, Goal::Custom(Box::new(GoToKnownFlowerGoal::default())));
        g.add(7, Goal::Custom(Box::new(GrowCropGoal)));
        g.add(8, Goal::Custom(Box::new(BeeWanderGoal)));
        g.add(9, Goal::Float);
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(AngryAtPlayerGoal::default())));
        // `ResetUniversalAngerTargetGoal`: the `universal_anger` game rule is off.
        t.add(3, Goal::Never);
    }

    /// `Bee.getWalkTargetValue`: open air is good.
    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        Some(if kiln_data::blocks_types::is_air(level.block(p)) { 10.0 } else { 0.0 })
    }

    /// `Bee.tick` after the base: nectar drips, the roll turns.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = level;
        let s = st(m);
        if s.nectar && s.crops_grown < 10 && e.random.next_float() < 0.05 {
            // (The particles are the client's; the draws are the server's too.)
            let mut i = 0;
            while i < e.random.next_int_bounded(2) + 1 {
                i += 1;
            }
        }
        let s = st_mut(m);
        s.roll_amount_o = s.roll_amount;
        s.roll_amount = if s.rolling { (s.roll_amount + 0.2).min(1.0) } else { (s.roll_amount - 0.24).max(0.0) };
    }

    /// `Bee.aiStep` after the base.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        {
            let s = st_mut(m);
            if s.stay_out_of_hive > 0 {
                s.stay_out_of_hive -= 1;
            }
            if s.cooldown_hive > 0 {
                s.cooldown_hive -= 1;
            }
            if s.cooldown_flower > 0 {
                s.cooldown_flower -= 1;
            }
        }
        let rolling = is_angry(m, level)
            && !has_stung(m)
            && m.target.and_then(|id| goals::living(level, id)).is_some_and(|t| t.pos.distance_to_sqr(e.position()) < 4.0);
        st_mut(m).rolling = rolling;
        if e.tick_count % 20 == 0 && !is_hive_valid(e, m, level) {
            st_mut(m).hive_pos = None;
        }
    }

    /// `Bee.customServerAiStep`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let stung = st(m).stung;
        let in_water = e.is_in_water();
        {
            let s = st_mut(m);
            if in_water {
                s.under_water += 1;
            } else {
                s.under_water = 0;
            }
        }
        if st(m).under_water > 20 {
            let source = DamageSource { kind: DamageKind::Drown, attacker: None, direct: None, pos: None, attacker_is_player: false };
            crate::mob::hurt(e, m, level, source, 1.0);
        }
        if stung {
            st_mut(m).time_since_sting += 1;
            let t = st(m).time_since_sting;
            if t % 5 == 0 && e.random.next_int_bounded((1200 - t).clamp(1, 1200)) == 0 {
                let source = DamageSource { kind: DamageKind::Generic, attacker: None, direct: None, pos: None, attacker_is_player: false };
                let health = m.health;
                crate::mob::hurt(e, m, level, source, health);
            }
        }
        if !st(m).nectar {
            st_mut(m).ticks_without_nectar += 1;
        }
        anger::update_persistent_anger(e, m, level);
    }

    /// `Bee.hurtServer`: a bee that is hurt stops pollinating.
    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        st_mut(m).pollinating = false;
        None
    }

    /// `Bee.doHurtTarget`: the sting, poison on normal and hard, and the bee is spent.
    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        let damage = m.attrs.value(AttackDamage) as i32 as f32;
        let source = DamageSource { kind: DamageKind::Named("minecraft:sting"), attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        let hurt = crate::mob::hurt_living(level, t, source, damage);
        if hurt {
            let seconds = match level.difficulty() {
                2 => 10,
                3 => 18,
                _ => 0,
            };
            if seconds > 0 {
                level.add_effect(t.id, "minecraft:poison", seconds * 20, 0, Some(e.id));
            }
            st_mut(m).stung = true;
            anger::stop_being_angry(m);
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.bee.sting", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
        }
        Some(hurt)
    }

    /// `Bee.mobInteract`: a flower in hand is eaten (it breeds, and the flower's own effect).
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Option<Outcome> {
        let _ = who;
        let item = if stack.is_empty() { 0 } else { stack.item() };
        if item > 0 && crate::mob::item_tag(item, "minecraft:bee_food") {
            // `FlowerBlock.getBeeInteractionEffect`: only the eyeblossoms (poison) and the wither rose.
            let name = crate::mob::item_name(stack);
            let fx = match name {
                "minecraft:open_eyeblossom" | "minecraft:closed_eyeblossom" => Some(("minecraft:poison", 25)),
                "minecraft:wither_rose" => Some(("minecraft:wither", 40)),
                _ => None,
            };
            if let Some(effect) = fx.and_then(|(n, d)| crate::effect::Effect::named(n, d, 0)) {
                crate::mob::effects::add(e, m, level, effect, None);
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
        }
        None
    }

    fn is_food(&self, item: i32) -> bool {
        crate::mob::item_tag(item, "minecraft:bee_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        crate::mob::item_tag(item, "minecraft:bee_food")
    }

    /// `FlyingMoveControl(this, 20, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        fly::tick_move(e, m, 20.0, true);
        true
    }

    fn omnidirectional_air_mover(&self) -> bool {
        true
    }

    /// `BeeLookControl`: an angry bee does not turn its head; the pitch is kept while pollinating.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if is_angry(m, level) {
            return true;
        }
        let reset = !st(m).pollinating;
        control::tick_look_with(e, m, reset);
        true
    }

    /// `Bee$1.isStableDestination`: anything with a block under it.
    fn stable_destination_for(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<bool> {
        Some(!kiln_data::blocks_types::is_air(level.block(p.below())))
    }

    /// `Bee$1.tick`: no navigation while pollinating.
    fn ticks_navigation(&self, m: &MobData) -> bool {
        !st(m).pollinating
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    fn sound_volume(&self, _m: &MobData) -> f32 {
        0.4
    }

    /// `Bee.getAmbientSound`: none (the loop is the client's).
    fn ambient_sound(&self, _e: &mut Entity, _m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(None)
    }

    /// `Bee.jumpInLiquid`: a little lift.
    fn jump_in_liquid(&self, e: &mut Entity, _m: &mut MobData, _lava: bool) -> bool {
        e.delta = e.delta.add(0.0, 0.01, 0.0);
        true
    }

    /// No placement rule (`SpawnPlacements` has none for bees): anywhere the biome lists them.
    fn placement(&self) -> ext::Placement {
        ext::Placement::NoRestrictions
    }

    fn check_spawn_rules(&self, _view: &dyn SpawnView, _pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(true)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let pos_of = |t: Option<&Tag>| match t {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        };
        let hive = pos_of(r.get("hive_pos"));
        let flower = pos_of(r.get("flower_pos"));
        let nectar = r.bool_or("HasNectar", false);
        let stung = r.bool_or("HasStung", false);
        let ticks = r.int_or("TicksSincePollination", 0);
        let out = r.int_or("CannotEnterHiveTicks", 0);
        let crops = r.int_or("CropsGrownSincePollination", 0);
        let end = match r.get("anger_end_time") {
            Some(Tag::Long(t)) => *t,
            _ => -1,
        };
        let s = st_mut(m);
        s.hive_pos = hive;
        s.flower_pos = flower;
        s.nectar = nectar;
        s.stung = stung;
        s.ticks_without_nectar = ticks;
        s.stay_out_of_hive = out;
        s.crops_grown = crops;
        s.anger.end = end;
        if nectar {
            s.ticks_without_nectar = 0;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        let pos = |p: BlockPos| Tag::IntArray(vec![p.x, p.y, p.z]);
        if let Some(p) = s.hive_pos {
            o.put("hive_pos", pos(p));
        }
        if let Some(p) = s.flower_pos {
            o.put("flower_pos", pos(p));
        }
        o.put("HasNectar", Tag::Byte(s.nectar as i8));
        o.put("HasStung", Tag::Byte(s.stung as i8));
        o.put("TicksSincePollination", Tag::Int(s.ticks_without_nectar));
        o.put("CannotEnterHiveTicks", Tag::Int(s.stay_out_of_hive));
        o.put("CropsGrownSincePollination", Tag::Int(s.crops_grown));
        o.put("anger_end_time", Tag::Long(s.anger.end));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        let flags = if s.rolling { FLAG_ROLL } else { 0 } | if s.stung { FLAG_STUNG } else { 0 } | if s.nectar { FLAG_NECTAR } else { 0 };
        d.set(data::bee::FLAGS, &DataValue::Byte(flags as i8));
        d.set(data::bee::ANGER_END_TIME, &DataValue::Long(s.anger.end));
    }
}

// ---------------------------------------------------------------------- goals

/// `BaseBeeGoal.canUse` / `canContinueToUse`: not while angry.
fn calm(m: &MobData, level: &dyn EntityLevel) -> bool {
    !is_angry(m, level)
}

/// `BeeEnterHiveGoal`: at its hive, wanting in, the bee goes inside.
#[derive(Clone, Debug)]
struct EnterHiveGoal;

impl CustomGoal for EnterHiveGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeEnterHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        let Some(hive) = st(m).hive_pos else { return false };
        let near = Vec3::new(hive.x as f64 + 0.5, hive.y as f64 + 0.5, hive.z as f64 + 0.5).distance_to_sqr(e.position()) < 4.0;
        if wants_to_enter_hive(e, m, level) && near && let Some(b) = beehive(e, m, level) {
            if b.full {
                st_mut(m).hive_pos = None;
            } else {
                return true;
            }
        }
        false
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(hive) = st(m).hive_pos
            && beehive(e, m, level).is_some()
        {
            level.emit(Event::BeeEntersHive { bee: e.id, hive });
        }
    }
}

/// `Bee$ValidateHiveGoal`: now and then the known hive is looked at again.
#[derive(Clone, Debug)]
struct ValidateHiveGoal {
    cooldown: i32,
    last_validate: i64,
}

impl CustomGoal for ValidateHiveGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ValidateHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        calm(m, level) && level.game_time() > self.last_validate + st(m).validate_hive as i64
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = st(m).hive_pos
            && level.is_loaded(p)
            && !is_hive_valid(e, m, level)
        {
            drop_hive(m);
        }
        self.last_validate = level.game_time();
    }
}

/// `Bee$ValidateFlowerGoal`.
#[derive(Clone, Debug)]
struct ValidateFlowerGoal {
    cooldown: i32,
    last_validate: i64,
}

impl CustomGoal for ValidateFlowerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ValidateFlowerGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        calm(m, level) && level.game_time() > self.last_validate + st(m).validate_flower as i64
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = st(m).flower_pos
            && level.is_loaded(p)
            && !attracts_bees(level.block(p))
        {
            drop_flower(e, m);
        }
        self.last_validate = level.game_time();
    }
}

/// `BeePollinateGoal`: finds a flower near, flies to it and hovers over it for 400 ticks or more.
#[derive(Clone, Debug, Default)]
struct PollinateGoal {
    successful_ticks: i32,
    last_sound: i32,
    hover_pos: Option<Vec3>,
    pollinating_ticks: i32,
    unreachable: HashMap<i64, i64>,
}

impl PollinateGoal {
    fn pollinated_long_enough(&self) -> bool {
        self.successful_ticks > 400
    }

    /// `findNearbyFlower`.
    fn find_nearby_flower(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> Option<BlockPos> {
        let mut cache: HashMap<i64, i64> = HashMap::new();
        let here = e.block_position();
        let now = level.game_time();
        let mut found = None;
        for p in super::turtle::within_manhattan(here, 5, 5, 5) {
            if !level.is_loaded(p) || !attracts_bees(level.block(p)) {
                continue;
            }
            let key = pack(p);
            let v = self.unreachable.get(&key).copied().unwrap_or(i64::MIN);
            if now < v {
                cache.insert(key, v);
                continue;
            }
            let reachable = path::create_path(e, m, level, p, 1).is_some_and(|path| path.reached);
            if reachable {
                found = Some(p);
                break;
            }
            cache.insert(key, now + 600);
        }
        self.unreachable = cache;
        found
    }

    fn set_wanted_pos(&self, m: &mut MobData) {
        if let Some(h) = self.hover_pos {
            m.mov.set_wanted_position(h.x, h.y, h.z, 0.3499999940395355);
        }
    }

    fn offset(e: &mut Entity) -> f32 {
        (e.random.next_float() * 2.0 - 1.0) * 0.33333334
    }
}

/// `BlockPos.asLong`.
fn pack(p: BlockPos) -> i64 {
    (((p.x as i64) & 0x3FF_FFFF) << 38) | (((p.z as i64) & 0x3FF_FFFF) << 12) | ((p.y as i64) & 0xFFF)
}

impl CustomGoal for PollinateGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeePollinateGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        if st(m).cooldown_flower > 0 {
            return false;
        }
        if st(m).nectar {
            return false;
        }
        if level.is_raining() {
            return false;
        }
        match self.find_nearby_flower(e, m, level) {
            Some(p) => {
                st_mut(m).flower_pos = Some(p);
                path::move_to(e, m, level, p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5, 1.2000000476837158);
                true
            }
            None => {
                st_mut(m).cooldown_flower = next_int(&mut e.random, 20, 60);
                false
            }
        }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        if !st(m).pollinating {
            return false;
        }
        if st(m).flower_pos.is_none() {
            return false;
        }
        if level.is_raining() {
            return false;
        }
        if self.pollinated_long_enough() {
            return e.random.next_float() < 0.2;
        }
        true
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.successful_ticks = 0;
        self.pollinating_ticks = 0;
        self.last_sound = 0;
        let s = st_mut(m);
        s.pollinating = true;
        s.ticks_without_nectar = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if self.pollinated_long_enough() {
            let s = st_mut(m);
            s.nectar = true;
            s.ticks_without_nectar = 0;
        }
        let s = st_mut(m);
        s.pollinating = false;
        m.nav.stop();
        st_mut(m).cooldown_flower = 200;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(flower) = st(m).flower_pos else { return };
        self.pollinating_ticks += 1;
        if self.pollinating_ticks > 600 {
            drop_flower(e, m);
            st_mut(m).pollinating = false;
            st_mut(m).cooldown_flower = 200;
            return;
        }
        let target = Vec3::new(flower.x as f64 + 0.5, flower.y as f64, flower.z as f64 + 0.5).add(0.0, 0.6000000238418579, 0.0);
        if target.distance_to_sqr(e.position()).sqrt() > 1.0 {
            self.hover_pos = Some(target);
            self.set_wanted_pos(m);
            return;
        }
        if self.hover_pos.is_none() {
            self.hover_pos = Some(target);
        }
        let hover = self.hover_pos.unwrap_or(target);
        let reached = e.position().distance_to_sqr(hover).sqrt() <= 0.1;
        let mut flag = true;
        if !reached && self.pollinating_ticks > 600 {
            drop_flower(e, m);
            return;
        }
        if reached {
            let change = e.random.next_int_bounded(25) == 0;
            if change {
                let ox = Self::offset(e) as f64;
                let oz = Self::offset(e) as f64;
                self.hover_pos = Some(Vec3::new(target.x + ox, target.y, target.z + oz));
                m.nav.stop();
            } else {
                flag = false;
            }
            control::look_at(m, target.x, target.y, target.z);
        }
        if flag {
            self.set_wanted_pos(m);
        }
        self.successful_ticks += 1;
        if e.random.next_float() < 0.05 && self.successful_ticks > self.last_sound + 60 {
            self.last_sound = self.successful_ticks;
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.bee.pollinate", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
        }
    }
}

/// `BeeLocateHiveGoal`: a bee without a hive looks for one within 20 blocks.
#[derive(Clone, Debug)]
struct LocateHiveGoal;

impl CustomGoal for LocateHiveGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeLocateHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        calm(m, level) && st(m).cooldown_hive == 0 && st(m).hive_pos.is_none() && wants_to_enter_hive(e, m, level)
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        st_mut(m).cooldown_hive = 200;
        let here = e.block_position();
        let mut hives: Vec<BlockPos> = level
            .poi_in_range(&["#minecraft:bee_home"], here, 20, crate::level::PoiOccupancy::Any)
            .into_iter()
            .filter(|&p| level.beehive_at(p).is_some_and(|b| !b.full))
            .collect();
        let d2 = |p: &BlockPos| {
            let (dx, dy, dz) = ((p.x - here.x) as f64, (p.y - here.y) as f64, (p.z - here.z) as f64);
            dx * dx + dy * dy + dz * dz
        };
        hives.sort_by(|a, b| d2(a).total_cmp(&d2(b)));
        if hives.is_empty() {
            return;
        }
        for p in &hives {
            if !st(m).blacklist.contains(p) {
                st_mut(m).hive_pos = Some(*p);
                return;
            }
        }
        st_mut(m).blacklist.clear();
        st_mut(m).hive_pos = Some(hives[0]);
    }
}

/// `BeeGoToHiveGoal`: flies to the hive, a few blocks at a time while it is far.
#[derive(Clone, Debug, Default)]
struct GoToHiveGoal {
    travelling_ticks: i32,
    last_path: Option<path::Path>,
    ticks_stuck: i32,
}

impl GoToHiveGoal {
    fn drop_and_blacklist(&mut self, m: &mut MobData) {
        if let Some(p) = st(m).hive_pos {
            let s = st_mut(m);
            s.blacklist.push(p);
            while s.blacklist.len() > 3 {
                s.blacklist.remove(0);
            }
        }
        drop_hive(m);
    }

    /// `hasReachedTarget`.
    fn reached(e: &Entity, m: &MobData, p: BlockPos) -> bool {
        if closer_than(e, p, 2) {
            return true;
        }
        m.nav.path.as_ref().is_some_and(|path| path.target == p && path.reached && path.is_done())
    }

    /// `pathfindDirectlyTowards`.
    fn pathfind_directly(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, p: BlockPos) -> bool {
        let _ = if closer_than(e, p, 3) { 1 } else { 2 };
        m.nav.max_visited_nodes_multiplier = 10.0;
        path::move_to(e, m, level, p.x as f64, p.y as f64, p.z as f64, 1.0);
        m.nav.path.as_ref().is_some_and(|path| path.reached)
    }
}

impl CustomGoal for GoToHiveGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeGoToHiveGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        let Some(hive) = st(m).hive_pos else { return false };
        !too_far(e, hive)
            && !has_home(m)
            && wants_to_enter_hive(e, m, level)
            && !Self::reached(e, m, hive)
            && super::wolf::block_in_tag(level.block(hive), "minecraft:beehives")
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.travelling_ticks = 0;
        self.ticks_stuck = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.travelling_ticks = 0;
        self.ticks_stuck = 0;
        m.nav.stop();
        m.nav.max_visited_nodes_multiplier = 1.0;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(hive) = st(m).hive_pos else { return };
        self.travelling_ticks += 1;
        if self.travelling_ticks > reduced_tick_delay(2400) {
            self.drop_and_blacklist(m);
            return;
        }
        if !m.nav.is_done() {
            return;
        }
        if closer_than(e, hive, 16) {
            if !Self::pathfind_directly(e, m, level, hive) {
                self.drop_and_blacklist(m);
            } else if self.last_path.is_some() && m.nav.path.as_ref().is_some_and(|p| p.same_as(self.last_path.as_ref())) {
                self.ticks_stuck += 1;
                if self.ticks_stuck > 60 {
                    drop_hive(m);
                    self.ticks_stuck = 0;
                }
            } else {
                self.last_path = m.nav.path.clone();
            }
        } else {
            if too_far(e, hive) {
                drop_hive(m);
                return;
            }
            pathfind_randomly_towards(e, m, level, hive);
        }
    }
}

/// `BeeGoToKnownFlowerGoal`: back to a flower it knows, when it has been without nectar long.
#[derive(Clone, Debug, Default)]
struct GoToKnownFlowerGoal {
    travelling_ticks: i32,
}

impl CustomGoal for GoToKnownFlowerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeGoToKnownFlowerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        match st(m).flower_pos {
            Some(f) => !has_home(m) && st(m).ticks_without_nectar > 600 && !closer_than(e, f, 2),
            None => false,
        }
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.travelling_ticks = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.travelling_ticks = 0;
        m.nav.stop();
        m.nav.max_visited_nodes_multiplier = 1.0;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(flower) = st(m).flower_pos else { return };
        self.travelling_ticks += 1;
        if self.travelling_ticks > reduced_tick_delay(2400) {
            drop_flower(e, m);
            return;
        }
        if !m.nav.is_done() {
            return;
        }
        if too_far(e, flower) {
            drop_flower(e, m);
            return;
        }
        pathfind_randomly_towards(e, m, level, flower);
    }
}

/// `BeeGrowCropGoal`: with nectar on the way to a hive, the crop below may grow.
#[derive(Clone, Debug)]
struct GrowCropGoal;

impl CustomGoal for GrowCropGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeGrowCropGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !calm(m, level) {
            return false;
        }
        if st(m).crops_grown >= 10 {
            return false;
        }
        if e.random.next_float() < 0.3 {
            return false;
        }
        st(m).nectar && is_hive_valid(e, m, level)
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if e.random.next_int_bounded(reduced_tick_delay(30)) != 0 {
            return;
        }
        for i in 1..=2 {
            let pos = e.block_position().offset(0, -i, 0);
            let state = level.block(pos);
            let mut new_state = None;
            if super::wolf::block_in_tag(state, "minecraft:bee_growables") {
                let info = kiln_data::blocks_types::block_of(state);
                let age: Option<i32> = info.property(state, "age").and_then(|a| a.parse().ok());
                match info.name {
                    "minecraft:sweet_berry_bush" => {
                        if let Some(a) = age.filter(|&a| a < 3) {
                            new_state = info.with_property(state, "age", &(a + 1).to_string());
                        }
                    }
                    "minecraft:cave_vines" | "minecraft:cave_vines_plant" => {
                        // `BonemealSource.MOB`: berries only on a vine that has none.
                        if info.property(state, "berries") == Some("false") {
                            level.set_block(pos, info.with_property(state, "berries", "true").unwrap_or(state), 2);
                            new_state = Some(level.block(pos));
                        }
                    }
                    _ if kiln_data::block_logic::is_instance(state, kiln_data::block_logic::BlockClass::StemBlock) => {
                        if let Some(a) = age.filter(|&a| a < 7) {
                            new_state = info.with_property(state, "age", &(a + 1).to_string());
                        }
                    }
                    _ if kiln_data::block_logic::is_instance(state, kiln_data::block_logic::BlockClass::CropBlock) => {
                        let max = info.properties.iter().find(|p| p.name == "age").map_or(0, |p| p.values.len() as i32 - 1);
                        if let Some(a) = age.filter(|&a| a < max) {
                            new_state = info.with_property(state, "age", &(a + 1).to_string());
                        }
                    }
                    _ => {}
                }
            }
            if let Some(s) = new_state {
                level.emit(Event::LevelEvent { event: 2011, pos, data: 15 });
                level.set_block(pos, s, 3);
                st_mut(m).crops_grown += 1;
            }
        }
    }
}

/// `BeeWanderGoal`: now and then a spot to fly to, near the hive when it is far.
#[derive(Clone, Debug)]
struct BeeWanderGoal;

impl BeeWanderGoal {
    fn wander_threshold(m: &MobData) -> i32 {
        let s = st(m);
        let i = if s.hive_pos.is_some() || s.flower_pos.is_some() { 24 } else { 16 };
        48 - i
    }

    fn find_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Vec3> {
        let view = match st(m).hive_pos {
            Some(h) if is_hive_valid(e, m, level) && !closer_than(e, h, Self::wander_threshold(m)) => {
                (Vec3::new(h.x as f64 + 0.5, h.y as f64 + 0.5, h.z as f64 + 0.5) - e.position()).normalize()
            }
            _ => crate::ext_entity::fireball::view_vector(e.x_rot_o, m.y_head_rot_o),
        };
        let angle = std::f32::consts::FRAC_PI_2;
        random_pos::hover_pos(e, m, level, 8, 7, view.x, view.z, angle, 3, 1)
            .or_else(|| random_pos::air_and_water_pos(e, m, level, 8, 4, -2, view.x, view.z, 1.5707963705062866))
    }
}

impl CustomGoal for BeeWanderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BeeWanderGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        m.nav.is_done() && e.random.next_int_bounded(10) == 0
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = Self::find_pos(e, m, level) {
            let at = BlockPos::containing(p.x, p.y, p.z);
            let path = path::create_path(e, m, level, at, 1);
            path::move_to_path(e, m, level, path, 1.0);
        }
    }
}

// ---------------------------------------------------------------------- the hive's side

/// `Bee.getSavedFlowerPos`.
pub fn saved_flower_pos(e: &Entity) -> Option<BlockPos> {
    crate::mob::data(e).and_then(ext::state::<State>).and_then(|s| s.flower_pos)
}

/// `Occupant.createEntity` for a bee: `setHivePos` and `setBeeReleaseData` (the bee grows up, or
/// its love cools, by the ticks it spent in the hive).
pub fn release_setup(e: &mut Entity, hive: BlockPos, ticks_in_hive: i32) {
    e.no_gravity = true;
    let Some(m) = crate::mob::data_mut(e) else { return };
    if let Some(s) = ext::state_mut::<State>(m) {
        s.hive_pos = Some(hive);
    }
    if !m.age_locked {
        let age = m.age;
        if age < 0 {
            m.age = (age + ticks_in_hive).min(0);
        } else if age > 0 {
            m.age = (age - ticks_in_hive).max(0);
        }
    }
    m.in_love = (m.in_love - ticks_in_hive).max(0);
}

/// `Bee.setSavedFlowerPos`.
pub fn set_saved_flower_pos(e: &mut Entity, p: BlockPos) {
    if let Some(s) = crate::mob::data_mut(e).and_then(ext::state_mut::<State>) {
        s.flower_pos = Some(p);
    }
}

/// `Bee.dropOffNectar`.
pub fn drop_off_nectar(e: &mut Entity) {
    if let Some(s) = crate::mob::data_mut(e).and_then(ext::state_mut::<State>) {
        s.nectar = false;
        s.crops_grown = 0;
    }
}

/// `Bee.setStayOutOfHiveCountdown`.
pub fn set_stay_out_of_hive(e: &mut Entity, ticks: i32) {
    if let Some(s) = crate::mob::data_mut(e).and_then(ext::state_mut::<State>) {
        s.stay_out_of_hive = ticks;
    }
}

/// `Bee.setTarget(player)` (a plain `Mob.setTarget`).
pub fn set_target(e: &mut Entity, target: i32) {
    let mut m = crate::mob::take(e);
    crate::mob::set_target(e, &mut m, Some(target));
    crate::mob::put(e, m);
}

/// Whether the bee has a target.
pub fn has_target(e: &Entity) -> bool {
    crate::mob::data(e).is_some_and(|m| m.target.is_some())
}

/// For the parity replay: the constructor's draws as the recording had them (vanilla draws them from
/// the mob's unseeded random): the flower-search cooldown and the validate goals' cooldowns.
pub fn pin_constructor_draws(e: &mut Entity, flower: i32, validate_hive: i32, validate_flower: i32) {
    if let Some(s) = crate::mob::data_mut(e).and_then(ext::state_mut::<State>) {
        s.cooldown_flower = flower;
        s.validate_hive = validate_hive;
        s.validate_flower = validate_flower;
    }
}
