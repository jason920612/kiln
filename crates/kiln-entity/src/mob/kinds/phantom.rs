//! Phantom: circles an anchor point and swoops at sleepless players (`PhantomAttackStrategyGoal`
//! switching between `PhantomCircleAroundAnchorGoal` and `PhantomSweepAttackGoal`), with its
//! own move control (banking toward the move target), a look control that does nothing and a
//! body control that follows the yaw. Burns in daylight (the shared `Mob.aiStep` rule).

use super::ghast::travel_flying;
use crate::entity::Entity;
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, state, state_mut};
use crate::mob::goals::{self, Goal, Living, MOVE};
use crate::mob::{self, GroupData, MobData, SpawnContext, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Phantom;

pub static KIND: Phantom = Phantom;

static INFO: Info = Info { burns_in_daylight: true, breathes_under_water: true, extends_monster: false, ..Info::monster("minecraft:phantom", &[]) };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackPhase {
    Circle,
    Swoop,
}

#[derive(Clone, Debug)]
pub struct PhantomState {
    pub size: i32,
    pub move_target: Vec3,
    pub anchor: Option<BlockPos>,
    pub phase: AttackPhase,
    /// `PhantomMoveControl.speed`.
    pub speed: f32,
}

fn st(m: &MobData) -> &PhantomState {
    state::<PhantomState>(m).expect("phantom state")
}

fn st_mut(m: &mut MobData) -> &mut PhantomState {
    state_mut::<PhantomState>(m).expect("phantom state")
}

/// `setPhantomSize` + `updatePhantomSizeInfo`.
/// (The update runs only when the size changes, as the synched data's does: a size 0 phantom
/// keeps the monster attack damage of 2.)
pub fn set_size(e: &mut Entity, m: &mut MobData, size: i32) {
    let size = size.clamp(0, 64);
    if st(m).size == size {
        return;
    }
    st_mut(m).size = size;
    mob::refresh_dimensions(e, m);
    if let Some(i) = m.attrs.get_mut(Attr::AttackDamage) {
        i.base = (6 + size) as f64;
    }
}

/// `TargetingConditions.test` for the phantom: combat, line of sight (the sensing cache) and an
/// optional range.
fn can_attack(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, t: &Living, range: Option<f64>) -> bool {
    if t.id == e.id || t.spectator || !t.alive {
        return false;
    }
    if (t.player && level.difficulty() == 0) || t.invulnerable || t.creative {
        return false;
    }
    if let Some(range) = range {
        let mut vis = 1.0;
        if t.sneaking {
            vis *= 0.8;
        }
        if t.invisible {
            vis *= 0.7 * t.armor_cover.max(0.1) as f64;
        }
        let d = (range * mth::clamp_d(vis, 0.0, 10.0)).max(2.0);
        if e.position().distance_to_sqr(t.pos) > d * d {
            return false;
        }
    }
    mob::has_line_of_sight_cached(e, m, level, t)
}

/// `level.getHeightmapPos(MOTION_BLOCKING, pos)`: above the highest block that blocks motion
/// or holds a fluid.
fn motion_blocking_top(level: &dyn EntityLevel, p: BlockPos) -> BlockPos {
    let mut y = level.max_y();
    while y >= level.min_y() {
        let s = level.block(BlockPos::new(p.x, y, p.z));
        if !kiln_data::blocks_types::is_air(s) && (crate::physics::is_solid(s) || !crate::physics::fluid_state(s).is_empty()) {
            return BlockPos::new(p.x, y + 1, p.z);
        }
        y -= 1;
    }
    BlockPos::new(p.x, level.min_y(), p.z)
}

/// `PhantomMoveControl.tick`.
fn tick_move(e: &mut Entity, m: &mut MobData) {
    let s = st(m);
    let (target, mut speed) = (s.move_target, s.speed);
    if e.horizontal_collision {
        e.y_rot += 180.0;
        speed = 0.1;
    }
    let (mut dx, dy, mut dz) = (target.x - e.x(), target.y - e.y(), target.z - e.z());
    let mut horiz = (dx * dx + dz * dz).sqrt();
    if horiz.abs() > 9.999999747378752e-6 {
        let k = 1.0 - (dy * 0.699999988079071).abs() / horiz;
        dx *= k;
        dz *= k;
        horiz = (dx * dx + dz * dz).sqrt();
        let dist = (dx * dx + dz * dz + dy * dy).sqrt();
        let old = e.y_rot;
        let angle = mth::atan2(dz, dx) as f32;
        let cur = mth::wrap_degrees(e.y_rot + 90.0);
        let tgt = mth::wrap_degrees(angle * 57.295776);
        e.y_rot = approach_degrees(cur, tgt, 4.0) - 90.0;
        m.y_body_rot = e.y_rot;
        if mth::degrees_difference(old, e.y_rot).abs() < 3.0 {
            speed = approach(speed, 1.8, 0.005 * (1.8 / speed));
        } else {
            speed = approach(speed, 0.2, 0.025);
        }
        let x_rot = (-(mth::atan2(-dy, horiz) * 57.2957763671875)) as f32;
        e.x_rot = x_rot;
        let yaw = e.y_rot + 90.0;
        let vx = (speed * mth::cos((yaw * 0.017453292) as f64)) as f64 * (dx / dist).abs();
        let vz = (speed * mth::sin((yaw * 0.017453292) as f64)) as f64 * (dz / dist).abs();
        let vy = (speed * mth::sin((x_rot * 0.017453292) as f64)) as f64 * (dy / dist).abs();
        let v = e.delta;
        e.delta = v + (Vec3::new(vx, vy, vz) - v).scale(0.2);
    }
    st_mut(m).speed = speed;
}

/// `Mth.approach`.
fn approach(v: f32, target: f32, max: f32) -> f32 {
    let max = max.abs();
    if v < target { mth::clamp(v + max, v, target) } else { mth::clamp(v - max, target, v) }
}

/// `Mth.approachDegrees`.
fn approach_degrees(cur: f32, target: f32, max: f32) -> f32 {
    let f = mth::degrees_difference(cur, target);
    approach(cur, cur + f, max)
}

impl Kind for Phantom {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(PhantomState { size: 0, move_target: Vec3::ZERO, anchor: None, phase: AttackPhase::Circle, speed: 0.1 }))
    }
    fn register_goals(&self, m: &mut MobData) {
        m.goals.add(1, Goal::Custom(Box::new(AttackStrategy { next_sweep: 0 })));
        m.goals.add(2, Goal::Custom(Box::new(SweepAttack { scared_of_cat: false, cat_search_tick: 0 })));
        m.goals.add(3, Goal::Custom(Box::new(CircleAroundAnchor { angle: 0.0, distance: 0.0, height: 0.0, clockwise: 0.0 })));
        m.targets.add(1, Goal::Custom(Box::new(AttackPlayerTarget { next_scan: mth::reduced_tick_delay(20) })));
    }
    fn travel(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        travel_flying(e, level, input, 0.02, 0.02, 0.2);
        true
    }
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        tick_move(e, m);
        true
    }
    /// `PhantomLookControl`: nothing.
    fn tick_look(&self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        true
    }
    /// `PhantomBodyRotationControl`.
    fn tick_body(&self, e: &mut Entity, m: &mut MobData) -> bool {
        m.y_head_rot = m.y_body_rot;
        m.y_body_rot = e.y_rot;
        true
    }
    fn checks_fall_damage(&self) -> bool {
        false
    }
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        let f = 1.0 + 0.15 * state::<PhantomState>(m).map_or(0, |s| s.size) as f32;
        (base.0 * f, base.1 * f, base.2 * f)
    }
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        st_mut(m).anchor = Some(e.block_position().above().above().above().above().above());
        set_size(e, m, 0);
        ext::mob_finalize(m, r);
    }
    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let anchor = match r.get("anchor_pos") {
            Some(Tag::IntArray(a)) if a.len() == 3 => Some(BlockPos::new(a[0], a[1], a[2])),
            _ => None,
        };
        st_mut(m).anchor = anchor;
        let size = r.int_or("size", 0);
        set_size(e, m, size);
    }
    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        if let Some(a) = s.anchor {
            o.put("anchor_pos", Tag::IntArray(vec![a.x, a.y, a.z]));
        }
        o.put("size", Tag::Int(s.size));
    }
    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::phantom::ID_SIZE, &DataValue::Int(st(m).size));
    }
}

// ---------------------------------------------------------------------- goals

/// `PhantomAttackPlayerTargetGoal` (target selector, no flags).
#[derive(Clone, Debug)]
struct AttackPlayerTarget {
    next_scan: i32,
}

impl CustomGoal for AttackPlayerTarget {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PhantomAttackPlayerTargetGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.next_scan > 0 {
            self.next_scan -= 1;
            return false;
        }
        self.next_scan = mth::reduced_tick_delay(60);
        let area = e.bounding_box().inflate(16.0, 64.0, 16.0);
        let mut players: Vec<Living> = level
            .players_in(&area)
            .iter()
            .map(goals::living_player)
            .filter(|t| t.bb.intersects(&area))
            .collect();
        players.retain(|t| can_attack(e, m, level, t, Some(64.0)));
        // Highest first (a stable sort, as vanilla's).
        players.sort_by(|a, b| b.pos.y.total_cmp(&a.pos.y));
        for t in players {
            if can_attack(e, m, level, &t, None) {
                m.target = Some(t.id);
                return true;
            }
        }
        false
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match goals::target(m, level) {
            Some(t) => can_attack(e, m, level, &t, None),
            None => false,
        }
    }
}

/// `PhantomAttackStrategyGoal`.
#[derive(Clone, Debug)]
struct AttackStrategy {
    next_sweep: i32,
}

/// `setAnchorAboveTarget`.
fn set_anchor_above_target(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    if st(m).anchor.is_none() {
        return;
    }
    let Some(t) = goals::target(m, level) else { return };
    let tp = BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
    let mut a = BlockPos::new(tp.x, tp.y + 20 + e.random.next_int_bounded(20), tp.z);
    if a.y < level.sea_level() {
        a = BlockPos::new(a.x, level.sea_level() + 1, a.z);
    }
    st_mut(m).anchor = Some(a);
}

impl CustomGoal for AttackStrategy {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PhantomAttackStrategyGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match goals::target(m, level) {
            Some(t) => can_attack(e, m, level, &t, None),
            None => false,
        }
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.next_sweep = mth::reduced_tick_delay(10);
        st_mut(m).phase = AttackPhase::Circle;
        set_anchor_above_target(e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(a) = st(m).anchor {
            let top = motion_blocking_top(level, a);
            let up = 10 + e.random.next_int_bounded(20);
            st_mut(m).anchor = Some(BlockPos::new(top.x, top.y + up, top.z));
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).phase != AttackPhase::Circle {
            return;
        }
        self.next_sweep -= 1;
        if self.next_sweep <= 0 {
            st_mut(m).phase = AttackPhase::Swoop;
            set_anchor_above_target(e, m, level);
            self.next_sweep = mth::reduced_tick_delay((8 + e.random.next_int_bounded(4)) * 20);
            let pitch = 0.95 + e.random.next_float() * 0.1;
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.phantom.swoop", source: "hostile", volume: 10.0, pitch });
            }
        }
    }
}

/// `PhantomMoveTargetGoal.touchingTarget`.
fn touching_target(e: &Entity, m: &MobData) -> bool {
    st(m).move_target.distance_to_sqr(e.position()) < 4.0
}

/// `PhantomSweepAttackGoal`.
#[derive(Clone, Debug)]
struct SweepAttack {
    scared_of_cat: bool,
    cat_search_tick: i32,
}

impl CustomGoal for SweepAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PhantomSweepAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some() && st(m).phase == AttackPhase::Swoop
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = goals::target(m, level) else { return false };
        if !t.alive || (t.player && (t.spectator || t.creative)) || st(m).phase != AttackPhase::Swoop {
            return false;
        }
        if e.tick_count > self.cat_search_tick {
            self.cat_search_tick = e.tick_count + 20;
            let area = e.bounding_box().inflate_all(16.0);
            let cats = level.entities_in(&area, EntityFilter::Living, e.id).into_iter().filter(|&id| level.entity(id).is_some_and(|c| c.type_name == "minecraft:cat")).count();
            self.scared_of_cat = cats > 0;
        }
        !self.scared_of_cat
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
        st_mut(m).phase = AttackPhase::Circle;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        st_mut(m).move_target = Vec3::new(t.pos.x, t.pos.y + (t.bb.max_y - t.bb.min_y) as f32 as f64 * 0.5, t.pos.z);
        if e.bounding_box().inflate_all(0.20000000298023224).intersects(&t.bb) {
            mob::do_hurt_target(e, m, level, &t);
            st_mut(m).phase = AttackPhase::Circle;
            if !e.silent {
                level.emit(Event::LevelEvent { event: 1039, pos: e.block_position(), data: 0 });
            }
        } else if e.horizontal_collision || m.hurt_time > 0 {
            st_mut(m).phase = AttackPhase::Circle;
        }
    }
}

/// `PhantomCircleAroundAnchorGoal`.
#[derive(Clone, Debug)]
struct CircleAroundAnchor {
    angle: f32,
    distance: f32,
    height: f32,
    clockwise: f32,
}

impl CircleAroundAnchor {
    fn select_next(&mut self, e: &Entity, m: &mut MobData) {
        let bp = e.block_position();
        let s = st_mut(m);
        let a = *s.anchor.get_or_insert(bp);
        self.angle += self.clockwise * 15.0 * 0.017453292;
        s.move_target = Vec3::new(a.x as f64, a.y as f64, a.z as f64).add(
            (self.distance * mth::cos(self.angle as f64)) as f64,
            (-4.0 + self.height) as f64,
            (self.distance * mth::sin(self.angle as f64)) as f64,
        );
    }
}

impl CustomGoal for CircleAroundAnchor {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PhantomCircleAroundAnchorGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_none() || st(m).phase == AttackPhase::Circle
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.distance = 5.0 + e.random.next_float() * 10.0;
        self.height = -4.0 + e.random.next_float() * 9.0;
        self.clockwise = if e.random.next_bool() { 1.0 } else { -1.0 };
        self.select_next(e, m);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if e.random.next_int_bounded(mth::reduced_tick_delay(350)) == 0 {
            self.height = -4.0 + e.random.next_float() * 9.0;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(250)) == 0 {
            self.distance += 1.0;
            if self.distance > 15.0 {
                self.distance = 5.0;
                self.clockwise = -self.clockwise;
            }
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(450)) == 0 {
            self.angle = e.random.next_float() * 2.0 * 3.1415927;
            self.select_next(e, m);
        }
        if touching_target(e, m) {
            self.select_next(e, m);
        }
        let bp = e.block_position();
        let air = |p: BlockPos| kiln_data::blocks_types::is_air(level.block(p));
        if st(m).move_target.y < e.y() && !air(bp.below()) {
            self.height = self.height.max(1.0);
            self.select_next(e, m);
        }
        if st(m).move_target.y > e.y() && !air(bp.above()) {
            self.height = self.height.min(-1.0);
            self.select_next(e, m);
        }
    }
}
