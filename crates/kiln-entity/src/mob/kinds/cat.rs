//! Cat: shy of players until tamed with raw cod or salmon (a `TemptGoal` that can be scared
//! off), then sits, follows, lies on beds and sits on chests, furnaces and beds; variants
//! (the all-black cat under a full moon), collars, feeding, breeding.

use super::tame::{self, FollowOwnerGoal, NonTameRandomTargetGoal, SitWhenOrderedToGoal, Tame, TamableAnimalPanicGoal};
use super::wolf::{block_in_tag, synced_name};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, PlayerView};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::{Attr::*, Op};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{GroupData, MobData, SpawnContext, item_tag, path, random_pos};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Cat;

pub static KIND: Cat = Cat;

static INFO: Info = Info::animal("minecraft:cat", &[(MaxHealth, 10.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 3.0)]);

const DEFAULT_COLLAR: u8 = 14;

#[derive(Clone, Debug)]
pub struct State {
    pub tame: Tame,
    pub lying: bool,
    pub relax_one: bool,
    pub collar: u8,
    /// `Pose.CROUCHING` (tempted, sneaking up) and sprinting, from the move speed.
    pub crouching: bool,
    pub sprinting: bool,
    /// The tempt goal is running (`temptGoal.isRunning()`).
    pub tempted: bool,
    /// `tickCount` (for `removeWhenFarAway`).
    pub age_ticks: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("cat state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("cat state")
}

fn is_cat_food(item: i32) -> bool {
    item_tag(item, "minecraft:cat_food")
}

/// The `CatSoundSet` sound `what`.
fn sound(m: &MobData, what: &str) -> &'static str {
    let v = synced_name("minecraft:cat_sound_variant", m.sound_variant).unwrap_or("minecraft:classic");
    let set = if v == "minecraft:classic" { "cat".to_owned() } else { format!("cat_{}", &v[10..]) };
    crate::mob::sound_event(&format!("minecraft:entity.{set}.{what}"))
}

fn synced_len(registry: &str) -> i32 {
    kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry).map_or(1, |(_, e)| e.len() as i32)
}

impl Kind for Cat {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.variant = kiln_data::synced_id("minecraft:cat_variant", "minecraft:black").unwrap_or(0);
        m.sound_variant = kiln_data::synced_id("minecraft:cat_sound_variant", "minecraft:classic").unwrap_or(0);
        Some(Box::new(State {
            tame: Tame::default(),
            lying: false,
            relax_one: false,
            collar: DEFAULT_COLLAR,
            crouching: false,
            sprinting: false,
            tempted: false,
            age_ticks: 0,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(1, Goal::Custom(Box::new(TamableAnimalPanicGoal::new(1.5, "minecraft:panic_causes"))));
        g.add(2, Goal::Custom(Box::new(SitWhenOrderedToGoal)));
        g.add(3, Goal::Custom(Box::new(CatRelaxOnOwnerGoal)));
        g.add(4, Goal::Custom(Box::new(CatTemptGoal { speed: 0.6, player: None, calm_down: 0, px: 0.0, py: 0.0, pz: 0.0, rot_x: 0.0, rot_y: 0.0, selected: None })));
        g.add(5, Goal::Custom(Box::new(MoveToBlock::new(BlockGoal::Lie, 1.1, 8, 6, -2))));
        g.add(6, Goal::Custom(Box::new(FollowOwnerGoal::new(1.0, 10.0, 5.0))));
        g.add(7, Goal::Custom(Box::new(MoveToBlock::new(BlockGoal::Sit, 0.8, 8, 1, 0))));
        g.add(8, Goal::LeapAtTarget { yd: 0.3, target: None });
        g.add(9, Goal::Custom(Box::new(OcelotAttackGoal { target: None, attack_time: 0 })));
        g.add(10, Goal::Breed { speed: 0.8, partner: None, love_time: 0 });
        g.add(11, Goal::RandomStroll { speed: 0.8, interval: 120, check_no_action: true, water_avoiding: Some(1.0000001e-5), wanted: Vec3::ZERO, force: false });
        g.add(12, Goal::LookAtPlayer { dist: 10.0, probability: 0.02, look_at: None, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::Custom(Box::new(NonTameRandomTargetGoal::new(&["minecraft:rabbit"], false))));
        t.add(1, Goal::Custom(Box::new(NonTameRandomTargetGoal::new(&[], false))));
        // `reassessTameGoals` in the constructor: the untamed cat avoids players (the goal checks
        // the tame flag itself, as vanilla removes it on taming).
        m.goals.add(4, Goal::Custom(Box::new(AvoidPlayerGoal { name: "CatAvoidEntityGoal", max_dist: 16.0, walk: 0.8, sprint: 1.33, to_avoid: None, path: None })));
    }

    fn custom_server_ai_step(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let (crouch, sprint) = if m.mov.has_wanted() {
            let s = m.mov.speed_modifier;
            (s == 0.6, s == 1.33)
        } else {
            (false, false)
        };
        let s = st_mut(m);
        s.crouching = crouch;
        if s.sprinting != sprint {
            s.sprinting = sprint;
            // `LivingEntity.setSprinting`: the sprint speed bonus.
            if sprint {
                m.attrs.set_modifier(MovementSpeed, "minecraft:sprinting", 0.30000001192092896, Op::AddMultipliedTotal);
            } else {
                m.attrs.remove_modifier(MovementSpeed, "minecraft:sprinting");
            }
        }
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `Entity.baseTick`: a sprinting cat kicks up block particles (two draws). Approximation:
        // the water check sees the fluid state of the previous tick.
        let s = st(m);
        if s.sprinting && !s.crouching && !e.is_in_water() && !e.is_in_lava() && crate::mob::is_alive(e, m) {
            let below = level.block(e.on_pos_legacy(level));
            let name = crate::blocks::block_name(below);
            let invisible = kiln_data::blocks_types::is_air(below) || matches!(name, "minecraft:barrier" | "minecraft:light" | "minecraft:structure_void" | "minecraft:moving_piston");
            if !invisible {
                e.random.next_double();
                e.random.next_double();
            }
        }
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        s.age_ticks = e.tick_count;
        let (tempted, lying) = (s.tempted, s.lying || s.relax_one);
        if tempted && !tame::is_tame(m) && e.tick_count % 100 == 0 && !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: sound(m, "beg_for_food"), source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        // `handleLieDown`.
        if lying && e.tick_count % 5 == 0 {
            let volume = 0.6 + 0.4 * (e.random.next_float() - e.random.next_float());
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: sound(m, "purr"), source: "neutral", volume, pitch: 1.0 });
            }
        }
    }

    fn ambient_sound(&self, e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if tame::is_tame(m) {
            if m.in_love > 0 {
                return Some(Some(sound(m, "purr")));
            }
            if e.random.next_int_bounded(4) == 0 {
                return Some(Some(sound(m, "purreow")));
            }
            return Some(Some(sound(m, "ambient")));
        }
        Some(Some(sound(m, "stray_ambient")))
    }

    fn can_attack(&self, m: &MobData, level: &dyn EntityLevel, t: &goals::Living) -> bool {
        !(t.player && tame::get(m).and_then(|x| x.owner).is_some_and(|u| level.player(t.id).is_some_and(|p| p.uuid == u)))
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        tame::is_tame(m) && tame::is_tame(partner)
    }

    fn is_food(&self, item: i32) -> bool {
        is_cat_food(item)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.3, 0.35, 0.34375) } else { base }
    }

    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        Some(!tame::is_tame(m) && st(m).age_ticks > 2400)
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
        // `CatVariants`: the ten common cats, and the all-black one under a (nearly) full moon.
        // Approximation: the witch hut rule (`#cats_spawn_as_black` structures) is not checked.
        let full_moon = ctx.moon_brightness >= 0.9;
        let candidates: Vec<&str> = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:cat_variant")
            .map_or(&[][..], |(_, e)| *e)
            .iter()
            .copied()
            .filter(|v| *v != "minecraft:all_black" || full_moon)
            .collect();
        if !candidates.is_empty() {
            let pick = candidates[r.next_int_bounded(candidates.len() as i32) as usize];
            m.variant = kiln_data::synced_id("minecraft:cat_variant", pick).unwrap_or(0);
        }
        m.sound_variant = r.next_int_bounded(synced_len("minecraft:cat_sound_variant"));
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let food = !stack.is_empty() && is_cat_food(stack.item());
        if tame::is_tame(m) {
            if tame::owned_by(m, level, who.id) {
                if !stack.is_empty() && item_tag(stack.item(), "minecraft:cat_collar_dyes") {
                    if let Some(dye) = crate::mob::interact::dye_color(stack)
                        && dye != st(m).collar
                    {
                        st_mut(m).collar = dye;
                        m.persistence_required = true;
                        return Some(Outcome::success(HeldChange::Consume(1)));
                    }
                } else if food && m.health < m.max_health() {
                    // `feed(player, hand, stack, 1, 1)`.
                    let n = stack.get(kiln_item::keys::FOOD).map_or(1.0, |f| f.nutrition as f32);
                    if m.health > 0.0 {
                        let h = m.health + n;
                        m.set_health(h);
                    }
                    play_eating_sound(e, m, level);
                    return Some(Outcome::success(HeldChange::Consume(1)));
                }
                let out = crate::mob::interact::animal_interact(e, m, level, who, stack);
                if !out.success {
                    let sit = !tame::ordered_to_sit(m);
                    tame::set_ordered_to_sit(m, sit);
                    return Some(Outcome::success(HeldChange::None));
                }
                return Some(out);
            }
        } else if food {
            // `tryToTame`.
            if e.random.next_int_bounded(3) == 0 {
                let uuid = level.player(who.id).map_or(0, |p| p.uuid);
                tame::tame(m, uuid);
                tame::set_ordered_to_sit(m, true);
                level.emit(Event::EntityEvent { entity: e.id, event: 7 });
            } else {
                level.emit(Event::EntityEvent { entity: e.id, event: 6 });
            }
            m.persistence_required = true;
            play_eating_sound(e, m, level);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        let out = crate::mob::interact::animal_interact(e, m, level, who, stack);
        if out.success {
            m.persistence_required = true;
        }
        Some(out)
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        child.variant = if e.random.next_bool() { m.variant } else { partner.variant };
        if tame::is_tame(m) {
            let owner = tame::get(m).and_then(|t| t.owner);
            if let Some(t) = tame::get_mut(child) {
                t.owner = owner;
                t.tame = true;
            }
            let (a, b) = (st(m).collar, ext::state::<State>(partner).map_or(DEFAULT_COLLAR, |s| s.collar));
            let mixed = crate::mob::breed::mixed_dye(a, b).unwrap_or_else(|| if level.random().next_bool() { a } else { b });
            st_mut(child).collar = mixed;
        }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        tame::load(&mut st_mut(m).tame, r);
        if let Some(v) = r.get("variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:cat_variant", v)) {
            m.variant = v;
        }
        if let Some(v) = r.get("sound_variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:cat_sound_variant", v)) {
            m.sound_variant = v;
        }
        st_mut(m).collar = r.byte_or("CollarColor", DEFAULT_COLLAR as i8) as u8 & 15;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        tame::save(&s.tame, o);
        if let Some(v) = synced_name("minecraft:cat_variant", m.variant) {
            o.put("variant", Tag::String(v.to_owned()));
        }
        if let Some(v) = synced_name("minecraft:cat_sound_variant", m.sound_variant) {
            o.put("sound_variant", Tag::String(v.to_owned()));
        }
        o.put("CollarColor", Tag::Byte(s.collar as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        tame::entity_data(&s.tame, d);
        if s.crouching {
            d.set(data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::CROUCHING));
        }
        if s.sprinting {
            d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(0x08));
        }
        d.set(data::cat::VARIANT, &DataValue::Holder(m.variant));
        d.set(data::cat::IS_LYING, &DataValue::Boolean(s.lying));
        d.set(data::cat::RELAX_STATE_ONE, &DataValue::Boolean(s.relax_one));
        d.set(data::cat::COLLAR_COLOR, &DataValue::Int(s.collar as i32));
        d.set(data::cat::SOUND_VARIANT, &DataValue::Holder(m.sound_variant));
    }
}

/// `Cat.playEatingSound`.
fn play_eating_sound(e: &Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: sound(m, "eat"), source: "neutral", volume: 1.0, pitch: 1.0 });
    }
}

// ---------------------------------------------------------------------- goals

/// `Cat.CatRelaxOnOwnerGoal`: lies next to its sleeping owner. Players do not sleep in Kiln's
/// world yet, so it never starts.
#[derive(Clone, Debug)]
struct CatRelaxOnOwnerGoal;

impl CustomGoal for CatRelaxOnOwnerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CatRelaxOnOwnerGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
}

/// `Cat.CatTemptGoal`: a `TemptGoal` (speed 0.6, cat food, can be scared) for untamed cats.
#[derive(Clone, Debug)]
struct CatTemptGoal {
    speed: f64,
    player: Option<i32>,
    calm_down: i32,
    px: f64,
    py: f64,
    pz: f64,
    rot_x: f64,
    rot_y: f64,
    /// `selectedPlayer`: a player the cat trusts not to scare it.
    selected: Option<i32>,
}

impl CatTemptGoal {
    fn find(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let range = m.attrs.value(TemptRange);
        self.player = goals::nearest_player(e, m, level, false, range, false, |p| is_cat_food(p.main_hand) || is_cat_food(p.off_hand)).map(|p| p.id);
        self.player.is_some()
    }
}

impl CustomGoal for CatTemptGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CatTemptGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.find(e, m, level) && !tame::is_tame(m)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let can_scare = !(self.selected.is_some() && self.selected == self.player);
        if can_scare && let Some(p) = self.player.and_then(|id| level.player(id)) {
            if e.position().distance_to_sqr(p.pos) < 36.0 {
                if p.pos.distance_to_sqr(Vec3::new(self.px, self.py, self.pz)) > 0.010000000000000002 {
                    return false;
                }
                if (p.rot[1] as f64 - self.rot_x).abs() > 5.0 || (p.rot[0] as f64 - self.rot_y).abs() > 5.0 {
                    return false;
                }
            } else {
                (self.px, self.py, self.pz) = (p.pos.x, p.pos.y, p.pos.z);
            }
            self.rot_x = p.rot[1] as f64;
            self.rot_y = p.rot[0] as f64;
        }
        self.can_use(e, m, level)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = self.player.and_then(|id| level.player(id)) {
            (self.px, self.py, self.pz) = (p.pos.x, p.pos.y, p.pos.z);
        }
        st_mut(m).tempted = true;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.player = None;
        m.nav.stop();
        self.calm_down = reduced_tick_delay(100);
        st_mut(m).tempted = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = self.player.and_then(|id| goals::living(level, id)) {
            let (hs, hx) = ((m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
            m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, hs, hx);
            if e.position().distance_to_sqr(p.pos) < 2.5 * 2.5 {
                m.nav.stop();
            } else {
                let b = BlockPos::containing(p.pos.x, p.pos.y, p.pos.z);
                path::move_to_entity(e, m, level, b, self.speed);
            }
        }
        if self.selected.is_none() && e.random.next_int_bounded(reduced_tick_delay(600)) == 0 {
            self.selected = self.player;
        } else if e.random.next_int_bounded(reduced_tick_delay(500)) == 0 {
            self.selected = None;
        }
    }
}

/// `AvoidEntityGoal<Player>` (untamed cats: `CatAvoidEntityGoal`): runs from the nearest
/// survival player within `max_dist`.
#[derive(Clone, Debug)]
pub struct AvoidPlayerGoal {
    pub name: &'static str,
    pub max_dist: f32,
    pub walk: f64,
    pub sprint: f64,
    to_avoid: Option<i32>,
    path: Option<path::Path>,
}

impl CustomGoal for AvoidPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if tame::is_tame(m) {
            return false;
        }
        let d = self.max_dist as f64;
        let area = e.bounding_box().inflate(d, 3.0, d);
        let mut best: Option<(f64, PlayerView)> = None;
        for p in level.players() {
            let h = if p.sneaking { 1.5 } else { 1.8 };
            let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
            if !pb.intersects(&area) || p.creative || p.spectator {
                continue;
            }
            let Some(t) = goals::living(level, p.id) else { continue };
            if !goals::targeting_ok(e, m, level, &t, true, d, true) {
                continue;
            }
            let dist = e.position().distance_to_sqr(p.pos);
            if best.as_ref().is_none_or(|(b, _)| dist < *b) {
                best = Some((dist, p));
            }
        }
        let Some((_, p)) = best else { return false };
        self.to_avoid = Some(p.id);
        let Some(away) = random_pos::default_pos_away(e, m, level, 16, 7, p.pos) else { return false };
        if p.pos.distance_to_sqr(away) < p.pos.distance_to_sqr(e.position()) {
            return false;
        }
        self.path = path::create_path(e, m, level, BlockPos::containing(away.x, away.y, away.z), 0);
        self.path.is_some()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !tame::is_tame(m) && !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let p = self.path.take();
        path::move_to_path(e, m, level, p, self.walk);
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.to_avoid = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.to_avoid.and_then(|id| level.player(id)) else { return };
        m.nav.speed_modifier = if e.position().distance_to_sqr(p.pos) < 49.0 { self.sprint } else { self.walk };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockGoal {
    /// `CatLieOnBlockGoal` (beds).
    Lie,
    /// `CatSitOnBlockGoal` (chests, lit furnaces, beds).
    Sit,
}

/// `MoveToBlockGoal` as the cat's `CatLieOnBlockGoal` and `CatSitOnBlockGoal`.
#[derive(Clone, Debug)]
struct MoveToBlock {
    which: BlockGoal,
    speed: f64,
    range: i32,
    vrange: i32,
    vstart: i32,
    next_start: i32,
    try_ticks: i32,
    max_stay: i32,
    block: BlockPos,
    reached: bool,
}

impl MoveToBlock {
    fn new(which: BlockGoal, speed: f64, range: i32, vrange: i32, vstart: i32) -> MoveToBlock {
        MoveToBlock { which, speed, range, vrange, vstart, next_start: 0, try_ticks: 0, max_stay: 0, block: BlockPos::default(), reached: false }
    }

    fn valid(&self, level: &dyn EntityLevel, p: BlockPos) -> bool {
        if !kiln_data::blocks_types::is_air(level.block(p.above())) {
            return false;
        }
        let s = level.block(p);
        match self.which {
            BlockGoal::Lie => block_in_tag(s, "minecraft:cats_can_lie_on"),
            BlockGoal::Sit => {
                if !block_in_tag(s, "minecraft:cats_can_sit_on") {
                    return false;
                }
                let info = kiln_data::blocks_types::block_of(s);
                match info.name {
                    // Chests nobody has open (open counts are not tracked here).
                    "minecraft:chest" => true,
                    "minecraft:furnace" => info.property(s, "lit") == Some("true"),
                    _ if block_in_tag(s, "minecraft:beds") => info.property(s, "part") != Some("head"),
                    _ => true,
                }
            }
        }
    }

    fn find(&mut self, e: &Entity, level: &dyn EntityLevel) -> bool {
        let o = e.block_position();
        let mut dy = self.vstart;
        while dy <= self.vrange {
            for r in 0..self.range {
                let mut dx = 0;
                while dx <= r {
                    let mut dz = if dx < r && dx > -r { r } else { 0 };
                    while dz <= r {
                        let p = o.offset(dx, dy - 1, dz);
                        if self.valid(level, p) {
                            self.block = p;
                            return true;
                        }
                        dz = if dz > 0 { -dz } else { 1 - dz };
                    }
                    dx = if dx > 0 { -dx } else { 1 - dx };
                }
            }
            dy = if dy > 0 { -dy } else { 1 - dy };
        }
        false
    }
}

impl CustomGoal for MoveToBlock {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        match self.which {
            BlockGoal::Lie => "CatLieOnBlockGoal",
            BlockGoal::Sit => "CatSitOnBlockGoal",
        }
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !tame::is_tame(m) || tame::ordered_to_sit(m) || (self.which == BlockGoal::Lie && st(m).lying) {
            return false;
        }
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = match self.which {
            BlockGoal::Lie => 40,
            BlockGoal::Sit => reduced_tick_delay(200 + e.random.next_int_bounded(200)),
        };
        self.find(e, level)
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.try_ticks >= -self.max_stay && self.try_ticks <= 1200 && self.valid(level, self.block)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let b = self.block;
        path::move_to(e, m, level, b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, self.speed);
        self.try_ticks = 0;
        let inner = e.random.next_int_bounded(1200);
        self.max_stay = e.random.next_int_bounded(inner + 1200) + 1200;
        tame::set_sitting(m, false);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        match self.which {
            BlockGoal::Lie => st_mut(m).lying = false,
            BlockGoal::Sit => tame::set_sitting(m, false),
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let t = self.block.above();
        let c = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= 1.0 {
            self.reached = false;
            self.try_ticks += 1;
            if self.try_ticks % 40 == 0 {
                path::move_to(e, m, level, t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5, self.speed);
            }
        } else {
            self.reached = true;
            self.try_ticks -= 1;
        }
        match self.which {
            BlockGoal::Lie => {
                tame::set_sitting(m, false);
                st_mut(m).lying = self.reached;
            }
            BlockGoal::Sit => tame::set_sitting(m, self.reached),
        }
    }
}

/// `OcelotAttackGoal`: runs at the target and swipes at it once a second.
#[derive(Clone, Debug)]
struct OcelotAttackGoal {
    target: Option<i32>,
    attack_time: i32,
}

impl CustomGoal for OcelotAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "OcelotAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.target = goals::target(m, level).map(|t| t.id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return false };
        if !t.alive || e.position().distance_to_sqr(t.pos) > 225.0 {
            return false;
        }
        !m.nav.is_done() || self.can_use(e, m, level)
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.target = None;
        m.nav.stop();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return };
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        let reach = (e.width * 2.0 * e.width * 2.0) as f64;
        let d = e.position().distance_to_sqr(t.pos);
        let speed = if d > reach && d < 16.0 {
            1.33
        } else if d < 225.0 {
            0.6
        } else {
            0.8
        };
        path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), speed);
        self.attack_time = (self.attack_time - 1).max(0);
        if d > reach || self.attack_time > 0 {
            return;
        }
        self.attack_time = 20;
        crate::mob::do_hurt_target(e, m, level, &t);
    }
}
