//! Goat: an `Animal` on a brain (`GoatAi`): it strolls, is tempted by wheat, breeds, and now and
//! then (every 10 seconds to 5 minutes; every 5 to 15 seconds for a screaming goat) picks a fight:
//! `PrepareRamNearestTarget` finds a start block 4 to 7 blocks from the nearest target in a straight
//! line, waits there a second and charges (`RamTarget`), knocking the target back and dropping a
//! horn when it runs into a stone-like block. Between fights it long-jumps (`LongJumpToRandomPos`,
//! `LongJumpMidJump`). Goats take 10 points less fall damage, scream 2% of the time, can be milked
//! with a bucket and have two horns (a `1 in 10` chance of only one).

use crate::behavior_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::sensors::{self};
use crate::mob::brain::util::{self, Targeting, uniform};
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Control, Cx, Gate, Mem, Sensor, Status, Timed};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Living;
use crate::mob::interact::{self, HeldChange, Interactor, Outcome};
use crate::mob::path::{self, PathType};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext};
use crate::level::DamageKind;
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{Registered, ValueAbsent, ValuePresent};

pub struct Goat;

pub static KIND: Goat = Goat;

static INFO: Info = Info {
    head: (15, 40, 10),
    ..Info::animal("minecraft:goat", &[(MaxHealth, 10.0), (MovementSpeed, 0.20000000298023224), (AttackDamage, 2.0)])
};

/// `GoatAi.TIME_BETWEEN_LONG_JUMPS`, `TIME_BETWEEN_RAMS`, `TIME_BETWEEN_RAMS_SCREAMER`.
const LONG_JUMPS: (i32, i32) = (600, 1200);
const RAMS: (i32, i32) = (600, 6000);
const RAMS_SCREAMER: (i32, i32) = (100, 300);
const MAX_LONG_JUMP: i32 = 5;
const MAX_JUMP_VELOCITY_MULTIPLIER: f32 = 3.5714288;

#[derive(Clone, Debug)]
pub struct State {
    pub screaming: bool,
    pub left_horn: bool,
    pub right_horn: bool,
    /// `Pose.LONG_JUMPING`.
    pub long_jumping: bool,
    /// `setDiscardFriction`.
    pub discard_friction: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("goat state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("goat state")
}

pub fn is_screaming(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.screaming)
}

fn rams(m: &MobData) -> (i32, i32) {
    if is_screaming(m) { RAMS_SCREAMER } else { RAMS }
}

/// `GoatAi.initMemories`: the first long jump and ram cooldowns.
fn init_memories(m: &mut MobData, r: &mut dyn RandomSource) {
    let long_jump = uniform(r, LONG_JUMPS.0, LONG_JUMPS.1);
    let ram = uniform(r, RAMS.0, RAMS.1);
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.set(Mem::LongJumpCooldownTicks, brain::Val::Int(long_jump));
        b.st.mem.set(Mem::RamCooldownTicks, brain::Val::Int(ram));
    }
}

/// The sound `name` of the goat: `entity.goat.<name>` or `entity.goat.screaming.<name>`.
fn voice(m: &MobData, name: &str) -> &'static str {
    let full = if is_screaming(m) { format!("minecraft:entity.goat.screaming.{name}") } else { format!("minecraft:entity.goat.{name}") };
    mob::sound_event(&full)
}

/// `EntityDimensions` of the goat now: `BABY_DIMENSIONS`, and 0.7 of them while long jumping.
fn goat_dimensions(baby: bool, long_jumping: bool, base: (f32, f32, f32)) -> (f32, f32, f32) {
    let (w, h, eye) = if baby { (0.45, 0.65, 0.59375) } else { base };
    if long_jumping { (w * 0.7, h * 0.7, eye * 0.7) } else { (w, h, eye) }
}

fn broadcast(cx: &mut Cx, event: u8) {
    let id = cx.e.id;
    cx.level.emit(Event::EntityEvent { entity: id, event });
}

fn play(cx: &mut Cx, sound: &'static str, volume: f32, pitch: f32) {
    let pos = cx.e.position();
    cx.level.emit(Event::Sound { pos, sound, source: "neutral", volume, pitch });
}

// ---------------------------------------------------------------------- targeting

/// `GoatAi.RAM_TARGET_CONDITIONS`: `TargetingConditions.forCombat()` that leaves goats (and, with
/// mob griefing off, armor stands) alone.
fn ram_ok(cx: &mut Cx, t: &Living) -> bool {
    if t.type_name == "minecraft:goat" {
        return false;
    }
    if !cx.level.mob_griefing() && t.type_name == "minecraft:armor_stand" {
        return false;
    }
    Targeting::combat().test(cx, t)
}

fn walkable_block(cx: &Cx, pos: BlockPos) -> bool {
    path::stable_destination(cx.m, &*cx.level, pos) && path::malus(cx.m, path::path_type_static(&*cx.level, pos.x, pos.y, pos.z)) == 0.0
}

// ---------------------------------------------------------------------- ramming

/// `PrepareRamNearestTarget`: walks to a start block in line with the nearest target, faces it for
/// 20 ticks, then sets `RAM_TARGET`.
#[derive(Clone, Debug)]
struct PrepareRam {
    /// `reachedRamPositionTimestamp`.
    reached: Option<i64>,
    candidate: Option<RamCandidate>,
}

#[derive(Clone, Copy, Debug)]
struct RamCandidate {
    start: BlockPos,
    target_pos: BlockPos,
    target: i32,
}

impl PrepareRam {
    fn choose(&mut self, cx: &mut Cx, target: i32) {
        self.reached = None;
        self.candidate = None;
        let Some(t) = util::living(cx, target) else { return };
        let tb = BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
        if let Some(start) = self.ramming_start(cx, tb) {
            self.candidate = Some(RamCandidate { start, target_pos: tb, target });
        }
    }

    /// `calculateRammingStartPosition`: the nearest reachable end of a line of walkable blocks (4 to
    /// 7 long) away from the target's block.
    fn ramming_start(&self, cx: &mut Cx, target: BlockPos) -> Option<BlockPos> {
        if !walkable_block(cx, target) {
            return None;
        }
        let mut list: Vec<BlockPos> = Vec::new();
        // `Direction.Plane.HORIZONTAL`: north, east, south, west.
        for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let mut p = target;
            for _ in 0..7 {
                let next = p.offset(dx, 0, dz);
                if walkable_block(cx, next) {
                    p = next;
                } else {
                    break;
                }
            }
            if util::dist_manhattan(p, target) >= 4 {
                list.push(p);
            }
        }
        let me = cx.e.block_position();
        // A stable sort by the distance to the goat, then the first that has a path.
        list.sort_by(|a, b| util::dist_sqr_pos(me, *a).total_cmp(&util::dist_sqr_pos(me, *b)));
        for p in list {
            let reach = path::create_path(cx.e, cx.m, &*cx.level, p, 0);
            if reach.is_some_and(|r| r.reached) {
                return Some(p);
            }
        }
        None
    }
}

/// `Mth.sign(double)`.
fn sign(v: i32) -> f64 {
    if v > 0 { 1.0 } else if v < 0 { -1.0 } else { 0.0 }
}

impl Behavior for PrepareRam {
    fn name(&self) -> &'static str {
        "PrepareRamNearestTarget"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Registered), (Mem::RamCooldownTicks, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent), (Mem::RamTarget, ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (160, 160)
    }
    fn start(&mut self, cx: &mut Cx) {
        let found = util::find_closest_visible(cx, |cx, id| util::living(cx, id).is_some_and(|l| ram_ok(cx, &l)));
        if let Some(t) = found {
            self.choose(cx, t);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        if !cx.b.mem.has(Mem::RamTarget) {
            broadcast(cx, 59);
            // `getCooldownOnFail`: the shortest cooldown there is.
            let n = rams(cx.m).0;
            cx.b.mem.set(Mem::RamCooldownTicks, brain::Val::Int(n));
        }
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        self.candidate.is_some_and(|c| util::living(cx, c.target).is_some_and(|l| l.alive))
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(c) = self.candidate else { return };
        cx.b.mem.set(Mem::WalkTarget, brain::Val::Walk(brain::WalkTarget::block(c.start, 1.25, 0)));
        cx.b.mem.set(Mem::LookTarget, brain::Val::Look(brain::Tracker::entity(c.target, true)));
        let moved = util::living(cx, c.target).is_none_or(|l| BlockPos::containing(l.pos.x, l.pos.y, l.pos.z) != c.target_pos);
        if moved {
            broadcast(cx, 59);
            cx.m.nav.stop();
            self.choose(cx, c.target);
            return;
        }
        let here = cx.e.block_position();
        if here != c.start {
            return;
        }
        broadcast(cx, 58);
        let reached = *self.reached.get_or_insert(cx.time);
        if cx.time - reached >= 20 {
            let edge = Vec3::new(c.target_pos.x as f64 + 0.5 + 0.5 * sign(c.target_pos.x - here.x), c.target_pos.y as f64, c.target_pos.z as f64 + 0.5 + 0.5 * sign(c.target_pos.z - here.z));
            cx.b.mem.set(Mem::RamTarget, brain::Val::Vec3(edge));
            let sound = voice(cx.m, "prepare_ram");
            // `getVoicePitch`.
            let pitch = (cx.e.random.next_float() - cx.e.random.next_float()) * 0.2 + if cx.m.baby() { 1.5 } else { 1.0 };
            play(cx, sound, 1.0, pitch);
            self.candidate = None;
        }
    }
    behavior_boilerplate!();
}

/// `RamTarget`: charges at `RAM_TARGET` at three times the walking speed; whoever is in the way is
/// hurt and thrown back, a hard block breaks a horn; done when it arrives.
#[derive(Clone, Debug)]
struct RamTargetBehavior {
    direction: Vec3,
}

impl RamTargetBehavior {
    fn finish(&mut self, cx: &mut Cx) {
        broadcast(cx, 59);
        let (lo, hi) = rams(cx.m);
        let n = uniform(cx.rng(), lo, hi);
        cx.b.mem.set(Mem::RamCooldownTicks, brain::Val::Int(n));
        cx.b.mem.erase(Mem::RamTarget);
    }

    /// `hasRammedHornBreakingBlock`: the block ahead (or the one above it) snaps horns.
    fn rammed_horn_breaking_block(&self, cx: &Cx) -> bool {
        let dir = cx.e.delta.multiply(1.0, 0.0, 1.0).normalize();
        let p = cx.e.position() + dir;
        let pos = BlockPos::containing(p.x, p.y, p.z);
        let snaps = |p: BlockPos| super::wolf::block_in_tag(cx.level.block(p), "minecraft:snaps_goat_horn");
        snaps(pos) || snaps(pos.above())
    }
}

impl Behavior for RamTargetBehavior {
    fn name(&self) -> &'static str {
        "RamTarget"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::RamCooldownTicks, ValueAbsent), (Mem::RamTarget, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (200, 200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::RamTarget)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::RamTarget)
    }
    fn start(&mut self, cx: &mut Cx) {
        let here = cx.e.block_position();
        let Some(target) = cx.b.mem.vec3(Mem::RamTarget) else { return };
        self.direction = Vec3::new(here.x as f64 - target.x, 0.0, here.z as f64 - target.z).normalize();
        cx.b.mem.set(Mem::WalkTarget, brain::Val::Walk(brain::WalkTarget::vec(target, 3.0, 0)));
    }
    fn tick(&mut self, cx: &mut Cx) {
        // `level.getNearbyEntities(LivingEntity.class, RAM_TARGET_CONDITIONS, goat, goat.getBoundingBox())`.
        let area: Aabb = cx.e.bounding_box();
        let me = cx.e.id;
        let mut ids: Vec<i32> = cx.level.entities_in(&area, EntityFilter::Living, me);
        ids.extend(cx.level.players_in(&area).iter().map(|p| p.id));
        let mut hit: Option<Living> = None;
        for id in ids {
            if let Some(l) = util::living(cx, id)
                && l.bb.intersects(&area)
                && ram_ok(cx, &l)
            {
                hit = Some(l);
                break;
            }
        }
        if let Some(t) = hit {
            let source = DamageSource { kind: DamageKind::NoAggroMobAttack, attacker: Some(me), direct: Some(me), pos: Some(cx.e.position()), attacker_is_player: false };
            let damage = cx.m.attrs.value(AttackDamage) as f32;
            mob::hurt_living(cx.level, &t, source, damage);
            let amp = |id: i32| mob::effects::amplifier(cx.m, id).map_or(0, |a| a + 1);
            let (speed_amp, slow_amp) = (amp(crate::effect::ids::speed()), amp(crate::effect::ids::slowness()));
            let extra = 0.25 * (speed_amp - slow_amp) as f32;
            let strength = mob::mth::clamp(cx.m.speed * 1.65, 0.2, 3.0) + extra;
            // `applyItemBlocking`: shields are not modelled (a blocked ram halves the strength).
            let force = if cx.m.baby() { 1.0 } else { 2.5 };
            super::raider::knockback_other(cx.level, t.id, ((1.0f32 * strength) as f64) * force, self.direction.x, self.direction.z);
            self.finish(cx);
            let sound = voice(cx.m, "ram_impact");
            play(cx, sound, 1.0, 1.0);
        } else if self.rammed_horn_breaking_block(cx) {
            let sound = voice(cx.m, "ram_impact");
            play(cx, sound, 1.0, 1.0);
            if drop_horn(cx.e, cx.m, cx.level) {
                play(cx, mob::sound_event("minecraft:entity.goat.horn_break"), 1.0, 1.0);
            }
            self.finish(cx);
        } else {
            let walk = cx.b.mem.walk_target();
            let ram = cx.b.mem.vec3(Mem::RamTarget);
            let done = match (walk, ram) {
                (Some(w), Some(r)) => util::tracker_pos(cx, &w.target).is_none_or(|p| p.distance_to_sqr(r) < 0.25 * 0.25),
                _ => true,
            };
            if done {
                self.finish(cx);
            }
        }
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------- horns

/// `Goat.createHorn`: a goat horn with an instrument picked from the regular (or screaming) horns
/// by a random seeded with the goat's UUID hash.
fn create_horn(e: &Entity, m: &MobData) -> Option<ItemStack> {
    use kiln_item::component::InstrumentComponent;
    // `UUID.hashCode`.
    let uuid = e.uuid;
    let hilo = ((uuid >> 64) as u64) ^ (uuid as u64);
    let hash = ((hilo >> 32) as u32 ^ hilo as u32) as i32;
    let mut r = LegacyRandom::new(hash as i64);
    let tag = if is_screaming(m) { "minecraft:screaming_goat_horns" } else { "minecraft:regular_goat_horns" };
    let list = kiln_data::registries::TAGS
        .iter()
        .find(|(reg, _)| *reg == "minecraft:instrument")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .map(|(_, ids)| *ids)?;
    let mut stack = ItemStack::of("minecraft:goat_horn", 1)?;
    if !list.is_empty() {
        let id = list[r.next_int_bounded(list.len() as i32) as usize];
        stack.insert(kiln_item::keys::INSTRUMENT, InstrumentComponent(kiln_item::Holder::Reference(id)));
    }
    Some(stack)
}

/// `Goat.dropHorn`: an adult with a horn loses one (both: a random one) and drops a horn item.
pub fn drop_horn(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    if m.baby() {
        return false;
    }
    let (left, right) = (st(m).left_horn, st(m).right_horn);
    if !left && !right {
        return false;
    }
    let lose_left = if !left {
        false
    } else if !right {
        true
    } else {
        e.random.next_bool()
    };
    if lose_left {
        st_mut(m).left_horn = false;
    } else {
        st_mut(m).right_horn = false;
    }
    let pos = e.position();
    let Some(horn) = create_horn(e, m) else { return true };
    let mut between = |lo: f32, hi: f32| (e.random.next_float() * (hi - lo) + lo) as f64;
    let (dx, dy, dz) = (between(-0.2, 0.2), between(0.3, 0.7), between(-0.2, 0.2));
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, horn, seed);
    item.set_pos(pos);
    item.delta = Vec3::new(dx, dy, dz);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
    true
}

// ---------------------------------------------------------------------- long jumps

/// `LongJumpUtil.calculateJumpVectorForAngle` (`checkCollisions`: true).
fn jump_vector_for_angle(cx: &Cx, target: Vec3, max_speed: f32, angle: i32) -> Option<Vec3> {
    let pos = cx.e.position();
    let toward = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z).normalize().scale(0.5);
    let aim = target - toward;
    let d = aim - pos;
    let a = ((angle as f32) * 3.1415927f32) / 180.0f32;
    let heading = d.z.atan2(d.x);
    let horizontal_sqr = Vec3::new(d.x, d.y - d.y, d.z).length_sqr();
    let horizontal = horizontal_sqr.sqrt();
    let dy = d.y;
    let gravity = cx.m.attrs.value(Gravity);
    let sin2a = ((2.0f32 * a) as f64).sin();
    let cos_sq = {
        let c = (a as f64).cos();
        c * c
    };
    let sin_a = (a as f64).sin();
    let cos_a = (a as f64).cos();
    let (sin_h, cos_h) = (heading.sin(), heading.cos());
    let v_sqr = (horizontal_sqr * gravity) / ((horizontal * sin2a) - ((2.0 * dy) * cos_sq));
    if !(v_sqr >= 0.0) {
        return None;
    }
    let v = v_sqr.sqrt();
    if v > max_speed as f64 {
        return None;
    }
    let vh = v * cos_a;
    let vv = v * sin_a;
    // The path must be clear (`isClearTransition` along the arc).
    let steps = mob::mth::ceil(horizontal / vh) * 2;
    let mut travelled = 0.0;
    let mut previous: Option<Vec3> = None;
    let dims = goat_dimensions(cx.m.baby(), true, {
        let t = kiln_data::entities::by_name("minecraft:goat")?;
        (t.width, t.height, t.eye_height)
    });
    for _ in 0..(steps - 1).max(0) {
        travelled += horizontal / steps as f64;
        let y = ((sin_a / cos_a) * travelled) - (((travelled * travelled) * gravity) / ((2.0 * v_sqr) * (cos_a * cos_a)));
        let (x, z) = (travelled * cos_h, travelled * sin_h);
        let p = Vec3::new(pos.x + x, pos.y + y, pos.z + z);
        if let Some(prev) = previous
            && !clear_transition(cx, dims, prev, p)
        {
            return None;
        }
        previous = Some(p);
    }
    Some(Vec3::new(vh * cos_h, vv, vh * sin_h).scale(0.949999988079071))
}

/// `LongJumpUtil.isClearTransition`: the goat's long-jumping box fits at each step from `from` to `to`.
fn clear_transition(cx: &Cx, dims: (f32, f32, f32), from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    let side = dims.0.min(dims.1) as f64;
    let n = mob::mth::ceil(d.length() / side);
    let dir = d.normalize();
    let mut p = from;
    let ctx = cx.e.collision_context();
    for i in 0..n {
        p = if i == n - 1 { to } else { p + dir.scale(side * 0.8999999761581421) };
        let half = (dims.0 / 2.0) as f64;
        let b = Aabb::new(p.x - half, p.y, p.z - half, p.x + half, p.y + dims.1 as f64, p.z + half);
        if !crate::collision::no_collision(&*cx.level, &ctx, cx.e.id, &b) {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug)]
struct LongJumpToRandomPos {
    running: bool,
    end: i64,
    /// `jumpCandidates`: (position, weight).
    candidates: Vec<(BlockPos, i32)>,
    initial_position: Option<Vec3>,
    chosen: Option<Vec3>,
    tries: i32,
    prepare_start: i64,
    /// `Collections.shuffle`'s random (vanilla: an unseeded one; the parity harness pins it).
    shuffle: LegacyRandom,
}

impl LongJumpToRandomPos {
    fn new() -> Box<dyn Control> {
        Box::new(LongJumpToRandomPos {
            running: false,
            end: 0,
            candidates: Vec::new(),
            initial_position: None,
            chosen: None,
            tries: 0,
            prepare_start: 0,
            shuffle: LegacyRandom::new(0),
        })
    }

    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let honey = crate::blocks::block_name(cx.level.block(cx.e.block_position())) == "minecraft:honey_block";
        let flag = cx.e.on_ground && !cx.e.is_in_water() && !cx.e.is_in_lava() && !honey;
        if !flag {
            let n = uniform(cx.rng(), LONG_JUMPS.0, LONG_JUMPS.1) / 2;
            cx.b.mem.set(Mem::LongJumpCooldownTicks, brain::Val::Int(n));
        }
        flag
    }

    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let flag = self.initial_position.is_some_and(|p| p == cx.e.position())
            && self.tries > 0
            && !cx.e.is_in_water()
            && (self.chosen.is_some() || !self.candidates.is_empty());
        if !flag && !cx.b.mem.has(Mem::LongJumpMidJump) {
            let n = uniform(cx.rng(), LONG_JUMPS.0, LONG_JUMPS.1) / 2;
            cx.b.mem.set(Mem::LongJumpCooldownTicks, brain::Val::Int(n));
            cx.b.mem.erase(Mem::LookTarget);
        }
        flag
    }

    fn start(&mut self, cx: &mut Cx) {
        self.chosen = None;
        self.tries = 20;
        self.initial_position = Some(cx.e.position());
        let c = cx.e.block_position();
        self.candidates.clear();
        // `BlockPos.betweenClosedStream`: x fastest, then y, then z.
        for z in c.z - MAX_LONG_JUMP..=c.z + MAX_LONG_JUMP {
            for y in c.y - MAX_LONG_JUMP..=c.y + MAX_LONG_JUMP {
                for x in c.x - MAX_LONG_JUMP..=c.x + MAX_LONG_JUMP {
                    let p = BlockPos::new(x, y, z);
                    if p != c {
                        self.candidates.push((p, mob::mth::ceil(util::dist_sqr_pos(c, p))));
                    }
                }
            }
        }
    }

    fn tick(&mut self, cx: &mut Cx) {
        match self.chosen {
            Some(jump) => {
                if cx.time - self.prepare_start >= 40 {
                    cx.e.y_rot = cx.m.y_body_rot;
                    st_mut(cx.m).discard_friction = true;
                    let d = jump.length();
                    let e = d + mob::effects::jump_boost_power(cx.m) as f64;
                    cx.e.delta = jump.scale(e / d);
                    cx.b.mem.set(Mem::LongJumpMidJump, brain::Val::Bool(true));
                    let sound = voice(cx.m, "long_jump");
                    play(cx, sound, 1.0, 1.0);
                }
            }
            None => {
                self.tries -= 1;
                self.pick_candidate(cx);
            }
        }
    }

    /// `getJumpCandidate`: `WeightedRandom.getRandomItem` from the level's random, removing it.
    fn candidate(&mut self, cx: &mut Cx) -> Option<BlockPos> {
        let total: i64 = self.candidates.iter().map(|c| c.1 as i64).sum();
        if total == 0 {
            return None;
        }
        let mut n = cx.rng().next_int_bounded(total as i32);
        for i in 0..self.candidates.len() {
            n -= self.candidates[i].1;
            if n < 0 {
                return Some(self.candidates.remove(i).0);
            }
        }
        None
    }

    fn pick_candidate(&mut self, cx: &mut Cx) {
        while !self.candidates.is_empty() {
            let Some(target) = self.candidate(cx) else { continue };
            // `isAcceptableLandingPosition`.
            let here = cx.e.block_position();
            if here.x == target.x && here.z == target.z {
                continue;
            }
            let ok = kiln_data::block_props::solid_render(cx.level.block(target.below()))
                && path::malus(cx.m, path::path_type_static(&*cx.level, target.x, target.y, target.z)) == 0.0;
            if !ok {
                continue;
            }
            let center = target.center();
            let Some(jump) = self.optimal_vector(cx, center) else { continue };
            cx.b.mem.set(Mem::LookTarget, brain::Val::Look(brain::Tracker::block(target)));
            let reach = path::create_path_len(cx.e, cx.m, &*cx.level, target, 0, 8.0);
            if reach.is_none_or(|p| !p.reached) {
                self.chosen = Some(jump);
                self.prepare_start = cx.time;
                return;
            }
        }
    }

    /// `calculateOptimalJumpVector`: the angles 65, 70, 75, 80 in a shuffled order, the first that
    /// works.
    fn optimal_vector(&mut self, cx: &mut Cx, target: Vec3) -> Option<Vec3> {
        let mut angles = [65, 70, 75, 80];
        // `Collections.shuffle`.
        for i in (2..=angles.len()).rev() {
            let j = self.shuffle.next_int_bounded(i as i32) as usize;
            angles.swap(i - 1, j);
        }
        let max_speed = (cx.m.attrs.value(JumpStrength) * MAX_JUMP_VELOCITY_MULTIPLIER as f64) as f32;
        angles.iter().find_map(|&a| jump_vector_for_angle(cx, target, max_speed, a))
    }
}

impl Control for LongJumpToRandomPos {
    fn name(&self) -> &'static str {
        "LongJumpToRandomPos"
    }
    fn running(&self) -> bool {
        self.running
    }
    fn required(&self, out: &mut Vec<Mem>) {
        out.extend([Mem::LookTarget, Mem::LongJumpCooldownTicks, Mem::LongJumpMidJump]);
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        let entry_ok = cx.b.mem.check(Mem::LookTarget, Registered)
            && cx.b.mem.check(Mem::LongJumpCooldownTicks, ValueAbsent)
            && cx.b.mem.check(Mem::LongJumpMidJump, ValueAbsent);
        if entry_ok && self.check_extra_start(cx) {
            self.running = true;
            self.end = cx.time + 200;
            // `Behavior.tryStart`: the duration draw (min == max: still one draw).
            cx.rng().next_int_bounded(1);
            self.start(cx);
            true
        } else {
            false
        }
    }
    fn tick_or_stop(&mut self, cx: &mut Cx) {
        if !(cx.time > self.end) && self.can_still_use(cx) {
            self.tick(cx);
        } else {
            self.do_stop(cx);
        }
    }
    fn do_stop(&mut self, _cx: &mut Cx) {
        self.running = false;
    }
    /// The shuffle's random: its own stream per goat, seeded from the brain's.
    fn seed_gates(&mut self, base: i64, _k: &mut i64) {
        self.shuffle = LegacyRandom::new(base ^ SHUFFLE_SEED_XOR);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// What the goat's `Collections.shuffle` stream is seeded with (`MobVectors.pinBrain` does the
/// same on its side).
pub const SHUFFLE_SEED_XOR: i64 = 0x5DEE_CE66_D;

/// `LongJumpMidJump`: the flight itself; ends on landing, with a landing sound and a new cooldown.
#[derive(Clone, Debug)]
struct LongJumpMidJump;

impl Behavior for LongJumpMidJump {
    fn name(&self) -> &'static str {
        "LongJumpMidJump"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Registered), (Mem::LongJumpMidJump, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (100, 100)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        !cx.e.on_ground
    }
    fn start(&mut self, cx: &mut Cx) {
        st_mut(cx.m).discard_friction = true;
        st_mut(cx.m).long_jumping = true;
        mob::refresh_dimensions_in(cx.e, cx.m, &*cx.level);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if cx.e.on_ground {
            cx.e.delta = cx.e.delta.multiply(0.10000000149011612, 1.0, 0.10000000149011612);
            play(cx, mob::sound_event("minecraft:entity.goat.step"), 2.0, 1.0);
        }
        st_mut(cx.m).discard_friction = false;
        st_mut(cx.m).long_jumping = false;
        mob::refresh_dimensions_in(cx.e, cx.m, &*cx.level);
        cx.b.mem.erase(Mem::LongJumpMidJump);
        let n = uniform(cx.rng(), LONG_JUMPS.0, LONG_JUMPS.1);
        cx.b.mem.set(Mem::LongJumpCooldownTicks, brain::Val::Int(n));
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------- sensors

/// `NearestItemSensor`: goats want no item, so it only ever clears its memory.
#[derive(Clone, Debug)]
struct NearestItems;

impl Sensor for NearestItems {
    fn name(&self) -> &'static str {
        "NearestItemSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleWantedItem]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::NearestVisibleWantedItem);
    }
    sensor_boilerplate!();
}

// ---------------------------------------------------------------------- the brain

fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(NearestItems),
        Box::new(sensors::Adult { any_type: false }),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Tempting::for_animal()),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            Swim::new(0.8),
            AnimalPanic::new(2.0),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
            CountDownCooldownTicks::new(Mem::LongJumpCooldownTicks),
            CountDownCooldownTicks::new(Mem::RamCooldownTicks),
        ],
    );
    let idle = ActivityData::with_conditions(
        Activity::Idle,
        vec![
            (0, SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (30, 60))),
            (0, AnimalMakeLove::new("minecraft:goat", 1.0, 2)),
            (1, FollowTemptation::new(|_| 1.25)),
            (2, baby_follow_adult((5, 16), |_| 1.25, Mem::NearestVisibleAdult, false)),
            (
                3,
                Gate::run_one(vec![
                    (stroll(1.0, StrollKind::Land { avoid_water: false }), 2),
                    (set_walk_target_from_look_target(1.0, 3), 2),
                    (DoNothing::new(30, 60), 1),
                ]),
            ),
        ],
        &[(Mem::RamCooldownTicks, ValuePresent), (Mem::LongJumpCooldownTicks, ValuePresent)],
    );
    let long_jump = ActivityData::with_conditions(
        Activity::LongJump,
        vec![(0, Timed::new(LongJumpMidJump)), (1, LongJumpToRandomPos::new())],
        &[(Mem::TemptingPlayer, ValueAbsent), (Mem::BreedTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LongJumpCooldownTicks, ValueAbsent)],
    );
    let ram = ActivityData::with_conditions(
        Activity::Ram,
        vec![
            (0, Timed::new(RamTargetBehavior { direction: Vec3::ZERO })),
            (1, Timed::new(PrepareRam { reached: None, candidate: None })),
        ],
        &[(Mem::TemptingPlayer, ValueAbsent), (Mem::BreedTarget, ValueAbsent), (Mem::RamCooldownTicks, ValueAbsent)],
    );
    Brain::new(&[], sensors, vec![core, idle, long_jump, ram], random)
}

// ---------------------------------------------------------------------- the kind

impl Kind for Goat {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        super::tame::set_malus(m, PathType::PowderSnow, -1.0);
        super::tame::set_malus(m, PathType::OnTopOfPowderSnow, -1.0);
        Some(Box::new(State { screaming: false, left_horn: true, right_horn: true, long_jumping: false, discard_friction: false }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:goat_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    /// The brain, then `GoatAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Ram, Activity::LongJump, Activity::Idle]);
        }
    }

    fn discard_friction(&self, m: &MobData) -> bool {
        st(m).discard_friction
    }

    fn fall_damage_reduction(&self) -> i32 {
        10
    }

    /// `Goat.setYHeadRot`: the head stays within `getMaxHeadYRot` (15) of the body.
    fn set_head_rot(&self, body: f32, head: f32) -> f32 {
        let max = INFO.head.0 as f32;
        body + mob::mth::clamp(mob::mth::degrees_difference(body, head), -max, max)
    }

    /// `ageBoundaryReached`: babies hit for 1, adults for 2.
    fn age_boundary_reached(&self, m: &mut MobData) {
        let damage = if m.baby() { 1.0 } else { 2.0 };
        if let Some(i) = m.attrs.get_mut(AttackDamage) {
            i.base = damage;
        }
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(voice(m, "ambient")))
    }

    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(voice(m, "hurt"))
    }

    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(voice(m, "death"))
    }

    /// `mobInteract`: a bucket is milked; food (love, growing up) makes it chew.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if !stack.is_empty() && mob::item_name(stack) == "minecraft:bucket" && !m.baby() {
            let mut out = Outcome::success(HeldChange::Fill(ItemStack::of("minecraft:milk_bucket", 1)?));
            out.player_sound = Some(voice(m, "milk"));
            return Some(out);
        }
        let out = interact::animal_interact(e, m, level, who, stack);
        if !out.success {
            return None;
        }
        // `playEatingSound`: the pitch from the level's random.
        let pitch = {
            let r = match level.shared_ai_random() {
                Some(r) => r.next_float(),
                None => m.brain_random.next_float(),
            };
            r * (1.2 - 0.8) + 0.8
        };
        level.emit(Event::Sound { pos: e.position(), sound: voice(m, "eat"), source: "neutral", volume: 1.0, pitch });
        Some(out)
    }

    /// `Goat.getBreedOffspring`: new memories, screaming from a screaming parent or by chance.
    fn breed_offspring(&self, _e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        init_memories(child, level.random());
        let parent_screams = if level.random().next_bool() { is_screaming(m) } else { is_screaming(partner) };
        let screaming = parent_screams || level.random().next_double() < 0.02;
        st_mut(child).screaming = screaming;
    }

    /// `checkGoatSpawnRules`: on stone, snow and the like, in the light.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::wolf::block_in_tag(view.block(pos.below()), "minecraft:goats_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    /// `Goat.finalizeSpawn`: memories, a 2% scream, a 10% chance of one horn.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        init_memories(m, r);
        st_mut(m).screaming = r.next_double() < 0.02;
        self.age_boundary_reached(m);
        if !m.baby() && (r.next_float() as f64) < 0.10000000149011612 {
            if r.next_bool() {
                st_mut(m).left_horn = false;
            } else {
                st_mut(m).right_horn = false;
            }
        }
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        goat_dimensions(m.baby(), st(m).long_jumping, base)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let (screaming, left, right) = (r.bool_or("IsScreamingGoat", false), r.bool_or("HasLeftHorn", true), r.bool_or("HasRightHorn", true));
        let s = st_mut(m);
        s.screaming = screaming;
        s.left_horn = left;
        s.right_horn = right;
        self.age_boundary_reached(m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("IsScreamingGoat", Tag::Byte(s.screaming as i8));
        o.put("HasLeftHorn", Tag::Byte(s.left_horn as i8));
        o.put("HasRightHorn", Tag::Byte(s.right_horn as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        use kiln_data::entities::data::goat;
        d.set(goat::IS_SCREAMING_GOAT, &DataValue::Boolean(s.screaming));
        d.set(goat::HAS_LEFT_HORN, &DataValue::Boolean(s.left_horn));
        d.set(goat::HAS_RIGHT_HORN, &DataValue::Boolean(s.right_horn));
        if s.long_jumping {
            d.set(kiln_data::entities::data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::LONG_JUMPING));
        }
    }
}
