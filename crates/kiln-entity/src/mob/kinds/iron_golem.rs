//! Iron golem: strolls about, offers poppies by day, fights monsters it meets (and bumps into),
//! gets angry at players who hurt it (`NeutralMob`), hits for 7.5 to 21.5 and throws its
//! victims up; repaired with iron ingots. Villages are not simulated: `MoveBackToVillageGoal`
//! finds no village, `DefendVillageTargetGoal` has no reputations to act on, and the village
//! stroll goes anywhere (as vanilla's does away from villages).

use super::anger::{self, Anger, AngryAtPlayerGoal};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, MeleeKind, Wanted, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{DamageSource, MobData, item_name, path, random_pos};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct IronGolem;

/// `Crackiness.GOLEM.byFraction(health / maxHealth)` as a level (0 none to 3 high).
fn crackiness(m: &MobData) -> u8 {
    let fraction = m.health / m.attrs.value(Attr::MaxHealth) as f32;
    if fraction < 0.25 {
        3
    } else if fraction < 0.5 {
        2
    } else if fraction < 0.75 {
        1
    } else {
        0
    }
}

pub static KIND: IronGolem = IronGolem;

static INFO: Info = Info {
    ambient_interval: 120,
    breathes_under_water: true,
    ..Info::misc("minecraft:iron_golem", &[(MaxHealth, 100.0), (MovementSpeed, 0.25), (KnockbackResistance, 1.0), (AttackDamage, 15.0), (StepHeight, 1.0)])
};

/// The monsters a golem goes after (`Enemy`, not creepers).
const ENEMIES: &[&str] = &[
    "minecraft:zombie",
    "minecraft:skeleton",
    "minecraft:spider",
    "minecraft:husk",
    "minecraft:stray",
    "minecraft:drowned",
    "minecraft:zombie_villager",
    "minecraft:zombified_piglin",
    "minecraft:wither_skeleton",
    "minecraft:enderman",
    "minecraft:endermite",
    "minecraft:shulker",
    "minecraft:witch",
    "minecraft:slime",
    "minecraft:magma_cube",
    "minecraft:phantom",
    "minecraft:ghast",
    "minecraft:blaze",
    "minecraft:piglin",
    "minecraft:hoglin",
];

#[derive(Clone, Debug, Default)]
pub struct State {
    pub anger: Anger,
    pub player_created: bool,
    attack_animation: i32,
    offer_flower: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("iron golem state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("iron golem state")
}

impl Kind for IronGolem {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(
            1,
            Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: true, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 },
        );
        g.add(2, Goal::Custom(Box::new(MoveTowardsTargetGoal { speed: 0.9, within: 32.0, target: None, wanted: Vec3::ZERO })));
        g.add(2, Goal::Custom(Box::new(MoveBackToVillageGoal)));
        g.add(4, Goal::Custom(Box::new(GolemRandomStrollInVillageGoal { speed: 0.6, wanted: Vec3::ZERO })));
        g.add(5, Goal::Custom(Box::new(OfferFlowerGoal::new())));
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        // `DefendVillageTargetGoal`: no villager reputations to act on.
        t.add(1, Goal::Never);
        t.add(2, Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(3, Goal::Custom(Box::new(AngryAtPlayerGoal::default())));
        t.add(3, Goal::NearestAttackable { wanted: Wanted::Types(ENEMIES), interval: reduced_tick_delay(5), must_see: false, target: None, unseen: 0, spider: false });
        // `ResetUniversalAngerTargetGoal`: the `universal_anger` game rule is off.
        t.add(4, Goal::Never);
    }

    /// `IronGolem.hurtServer`: a hit that cracks it further makes the damage sound.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let before = crackiness(m);
        let hurt = crate::mob::hurt_base(e, m, level, *source, amount);
        if hurt && crackiness(m) != before {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.iron_golem.damage", source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        Some(hurt)
    }

    fn pre_tick(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        // `canSpawnSprintParticle` in `Entity.baseTick`: while moving, one time in five, the
        // block particles (two draws).
        if e.delta.horizontal_distance_sqr() > 2.500000277905201e-7 && e.random.next_int_bounded(5) == 0 {
            let below = level.block(e.on_pos_legacy(level));
            let name = crate::blocks::block_name(below);
            let invisible = kiln_data::blocks_types::is_air(below) || matches!(name, "minecraft:barrier" | "minecraft:light" | "minecraft:structure_void" | "minecraft:moving_piston");
            if !invisible {
                e.random.next_double();
                e.random.next_double();
            }
        }
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        if s.attack_animation > 0 {
            s.attack_animation -= 1;
        }
        if s.offer_flower > 0 {
            s.offer_flower -= 1;
        }
        anger::update_persistent_anger(e, m, level);
    }

    fn can_attack(&self, m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        !(st(m).player_created && t.player) && t.type_name != "minecraft:creeper"
    }

    fn do_push(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, other: i32) {
        let enemy = level.entity(other).is_some_and(|o| ENEMIES.contains(&o.type_name));
        if enemy && e.random.next_int_bounded(20) == 0 {
            m.target = Some(other);
        }
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        st_mut(m).attack_animation = 10;
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        let base = m.attrs.value(AttackDamage) as f32;
        let damage = if base as i32 > 0 { base / 2.0 + e.random.next_int_bounded(base as i32) as f32 } else { base };
        let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        let hurt = crate::mob::hurt_living(level, t, source, damage);
        if hurt && let Some(o) = level.entity_mut(t.id) {
            let res = crate::mob::data(o).map_or(0.0, |om| om.attrs.value(Attr::KnockbackResistance));
            let f = (1.0 - res).max(0.0);
            o.delta = o.delta.add(0.0, 0.4000000059604645 * f, 0.0);
            o.needs_sync = true;
        }
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.iron_golem.attack", source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        Some(hurt)
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if stack.is_empty() || item_name(stack) != "minecraft:iron_ingot" {
            return Some(Outcome::PASS);
        }
        let before = m.health;
        if m.health > 0.0 {
            let h = m.health + 25.0;
            m.set_health(h);
        }
        if m.health == before {
            return Some(Outcome::PASS);
        }
        let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.iron_golem.repair", source: "neutral", volume: 1.0, pitch });
        }
        Some(Outcome::success(HeldChange::Consume(1)))
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let created = r.bool_or("PlayerCreated", false);
        let end = match r.get("anger_end_time") {
            Some(Tag::Long(t)) => *t,
            _ => -1,
        };
        let s = st_mut(m);
        s.player_created = created;
        s.anger.end = end;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("PlayerCreated", Tag::Byte(s.player_created as i8));
        o.put("anger_end_time", Tag::Long(s.anger.end));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        if st(m).player_created {
            d.set(data::iron_golem::FLAGS, &DataValue::Byte(1));
        }
    }
}

/// `MoveTowardsTargetGoal`: walks toward a target within `within` blocks.
#[derive(Clone, Debug)]
struct MoveTowardsTargetGoal {
    speed: f64,
    within: f32,
    target: Option<i32>,
    wanted: Vec3,
}

impl CustomGoal for MoveTowardsTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveTowardsTargetGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = goals::target(m, level) else {
            self.target = None;
            return false;
        };
        self.target = Some(t.id);
        if t.pos.distance_to_sqr(e.position()) > (self.within * self.within) as f64 {
            return false;
        }
        match random_pos::default_pos_towards(e, m, level, 16, 7, t.pos, 1.5707963705062866) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return false };
        !m.nav.is_done() && t.alive && t.pos.distance_to_sqr(e.position()) < (self.within * self.within) as f64
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.wanted.x, self.wanted.y, self.wanted.z, self.speed);
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.target = None;
    }
}

/// `MoveBackToVillageGoal` (a `RandomStrollGoal`, interval 10): away from any village it rolls
/// its interval and finds no village section to walk to.
#[derive(Clone, Debug)]
struct MoveBackToVillageGoal;

impl CustomGoal for MoveBackToVillageGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveBackToVillageGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let _ = e.random.next_int_bounded(reduced_tick_delay(10));
        false
    }
}

/// `GolemRandomStrollInVillageGoal` (interval 240): with no villages about, the level's random
/// picks the kind of spot and the golem strolls anywhere (`LandRandomPos`).
#[derive(Clone, Debug)]
struct GolemRandomStrollInVillageGoal {
    speed: f64,
    wanted: Vec3,
}

impl CustomGoal for GolemRandomStrollInVillageGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "GolemRandomStrollInVillageGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_int_bounded(reduced_tick_delay(240)) != 0 {
            return false;
        }
        // `getPosition`: towards anywhere (30%), else a villager or a village section first,
        // neither of which exists here.
        let r = level.random().next_float();
        if r >= 0.3 {
            let _ = level.random().next_float();
        }
        match random_pos::land_pos(e, m, level, 10, 7) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.wanted.x, self.wanted.y, self.wanted.z, self.speed);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// `OfferFlowerGoal`: by day, rarely, holds a poppy out to the nearest villager or copper golem
/// within reach; after 20 seconds a copper golem without an antenna takes it.
#[derive(Clone, Debug)]
struct OfferFlowerGoal {
    tick: i32,
    entity: Option<i32>,
}

impl OfferFlowerGoal {
    fn new() -> OfferFlowerGoal {
        OfferFlowerGoal { tick: 0, entity: None }
    }

    /// `getGolemBoundingBox`.
    fn golem_box(e: &Entity) -> crate::math::Aabb {
        e.bounding_box().inflate(6.0, 2.0, 6.0)
    }
}

/// `IronGolem.offerFlower`.
fn offer_flower(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, on: bool) {
    st_mut(m).offer_flower = if on { 400 } else { 0 };
    level.emit(Event::EntityEvent { entity: e.id, event: if on { 11 } else { 34 } });
}

impl CustomGoal for OfferFlowerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "OfferFlowerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !level.is_bright_outside() {
            return false;
        }
        if e.random.next_int_bounded(8000) != 0 {
            return false;
        }
        // `getNearestEntity(CANDIDATE_FOR_IRON_GOLEM_GIFT, forNonCombat().range(6), golem, x, y, z, box)`.
        let area = Self::golem_box(e);
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
            let Some(t) = goals::living(&*level, id) else { continue };
            if !matches!(t.type_name, "minecraft:villager" | "minecraft:copper_golem") || !goals::targeting_ok(e, m, &*level, &t, false, 6.0, true) {
                continue;
            }
            let d = t.dist_sqr(e.x(), e.y(), e.z());
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.entity = best.map(|(_, id)| id);
        self.entity.is_some()
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        self.tick > 0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.tick = reduced_tick_delay(400);
        offer_flower(e, m, level, true);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        offer_flower(e, m, level, false);
        if self.tick == 0
            && let Some(id) = self.entity
        {
            let area = Self::golem_box(e);
            if let Some(other) = level.entity_mut(id)
                && other.type_name == "minecraft:copper_golem"
                && area.intersects(&other.bounding_box())
                && let Some(om) = crate::mob::data_mut(other)
            {
                let s = super::copper_golem::st_mut(om);
                if s.antenna.is_empty() {
                    s.antenna = ItemStack::of("minecraft:poppy", 1).unwrap_or_else(ItemStack::empty);
                    s.antenna_drop = 2.0;
                }
            }
        }
        self.entity = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(t) = self.entity.and_then(|id| goals::living(&*level, id)) {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        }
        self.tick -= 1;
    }
}

