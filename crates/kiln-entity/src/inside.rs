//! `applyEffectsFromBlocks`: the blocks and fluids an entity's path this tick passed through
//! (`checkInsideBlocks`, `BlockGetter.forEachBlockIntersectedBetween`) and their effects, ordered
//! by vanilla's `InsideBlockEffectApplier.StepBasedCollector`.

use crate::blocks::{Kind, kind};
use crate::entity::{Entity, EntityKind, Movement};
use crate::fluid;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, Axis, BlockPos, Vec3, floor};
use crate::physics::{self, FluidKind};
use kiln_javamath::random::RandomSource;

/// `InsideBlockEffectType`, in apply order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectType {
    Freeze = 0,
    ClearFreeze = 1,
    FireIgnite = 2,
    LavaIgnite = 3,
    Extinguish = 4,
}

const APPLY_ORDER: [EffectType; 5] =
    [EffectType::Freeze, EffectType::ClearFreeze, EffectType::FireIgnite, EffectType::LavaIgnite, EffectType::Extinguish];

/// The consumers blocks attach with `runBefore` / `runAfter`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// `BaseFireBlock`: hurt by fire.
    FireHurt(f32),
    /// `LavaFluid`: `Entity.lavaHurt`.
    LavaHurt,
    /// `PowderSnowBlock`: a burning entity melts the powder snow.
    MeltPowderSnow(BlockPos),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Effect {
    Type(EffectType),
    Action(Action),
}

/// `InsideBlockEffectApplier.StepBasedCollector`.
#[derive(Clone, Debug)]
pub struct InsideCollector {
    in_step: u8,
    before: [Vec<Action>; 5],
    after: [Vec<Action>; 5],
    effects: Vec<Effect>,
    last_step: i32,
}

impl Default for InsideCollector {
    fn default() -> Self {
        InsideCollector { in_step: 0, before: Default::default(), after: Default::default(), effects: Vec::new(), last_step: -1 }
    }
}

impl InsideCollector {
    pub fn apply(&mut self, t: EffectType) {
        self.in_step |= 1 << t as u8;
    }

    pub fn run_before(&mut self, t: EffectType, a: Action) {
        self.before[t as usize].push(a);
    }

    pub fn run_after(&mut self, t: EffectType, a: Action) {
        self.after[t as usize].push(a);
    }

    fn advance_step(&mut self, step: i32) {
        if self.last_step != step {
            self.last_step = step;
            self.flush_step();
        }
    }

    fn flush_step(&mut self) {
        for t in APPLY_ORDER {
            let i = t as usize;
            self.effects.extend(self.before[i].drain(..).map(Effect::Action));
            if self.in_step & (1 << i) != 0 {
                self.in_step &= !(1 << i);
                self.effects.push(Effect::Type(t));
            }
            self.effects.extend(self.after[i].drain(..).map(Effect::Action));
        }
    }
}

impl Entity {
    /// `applyEffectsFromBlocks()`: effects along this tick's recorded movements.
    pub fn apply_effects_from_blocks(&mut self, level: &mut dyn EntityLevel) {
        self.final_movements_this_tick.clear();
        self.final_movements_this_tick.extend(self.movement_this_tick.drain(..));
        let position = self.position();
        match self.final_movements_this_tick.last() {
            None => self.final_movements_this_tick.push(Movement { from: self.old_pos, to: position, axis_dependent_original: None }),
            Some(last) if last.to.distance_to_sqr(position) > 9.999999439624929e-11 => {
                let from = last.to;
                self.final_movements_this_tick.push(Movement { from, to: position, axis_dependent_original: None });
            }
            _ => {}
        }
        let movements = std::mem::take(&mut self.final_movements_this_tick);
        self.apply_effects_from_movements(level, &movements);
        self.final_movements_this_tick = movements;
    }

    /// `applyEffectsFromBlocksForLastMovements`.
    pub fn apply_effects_from_last_movements(&mut self, level: &mut dyn EntityLevel) {
        let movements = std::mem::take(&mut self.final_movements_this_tick);
        self.apply_effects_from_movements(level, &movements);
        self.final_movements_this_tick = movements;
    }

    /// `applyEffectsFromBlocks(from, to)`.
    pub fn apply_effects_between(&mut self, level: &mut dyn EntityLevel, from: Vec3, to: Vec3) {
        self.apply_effects_from_movements(level, &[Movement { from, to, axis_dependent_original: None }]);
    }

    fn apply_effects_from_movements(&mut self, level: &mut dyn EntityLevel, movements: &[Movement]) {
        if !self.is_affected_by_blocks() {
            return;
        }
        if self.on_ground {
            let pos = self.on_pos_legacy(level);
            let state = level.block(pos);
            self.step_on(level, pos, state);
        }
        let was_on_fire = self.is_on_fire();
        let was_freezing = self.ticks_frozen > 0;
        let fire_before = self.remaining_fire_ticks;
        self.check_inside_blocks(level, movements);
        self.apply_and_clear_inside(level);
        if self.is_in_rain(level) {
            self.clear_fire();
        }
        if (was_on_fire && !self.is_on_fire()) || (was_freezing && self.ticks_frozen <= 0) {
            let pitch = 1.6 + (self.random.next_float() - self.random.next_float()) * 0.4;
            level.emit(Event::Sound {
                pos: self.position(),
                sound: "minecraft:entity.generic.extinguish_fire",
                source: "neutral",
                volume: 0.7,
                pitch,
            });
        }
        let fire_increased = self.remaining_fire_ticks > fire_before;
        if !self.is_on_fire() && !fire_increased {
            self.remaining_fire_ticks = 0;
        }
    }

    fn is_in_rain(&self, level: &dyn EntityLevel) -> bool {
        let pos = self.block_position();
        level.is_raining_at(pos)
            || level.is_raining_at(BlockPos::containing(pos.x as f64, self.bounding_box().max_y, pos.z as f64))
    }

    fn apply_and_clear_inside(&mut self, level: &mut dyn EntityLevel) {
        self.inside.flush_step();
        let effects = std::mem::take(&mut self.inside.effects);
        for e in &effects {
            if !self.is_alive() {
                break;
            }
            match *e {
                Effect::Type(t) => self.apply_effect_type(level, t),
                Effect::Action(a) => self.apply_action(level, a),
            }
        }
        self.inside.effects = effects;
        self.inside.effects.clear();
        self.inside.last_step = -1;
    }

    fn apply_effect_type(&mut self, level: &mut dyn EntityLevel, t: EffectType) {
        match t {
            EffectType::Freeze => {
                self.is_in_powder_snow = true;
                if self.can_freeze() {
                    self.ticks_frozen = 140.min(self.ticks_frozen + 1);
                }
            }
            EffectType::ClearFreeze => self.ticks_frozen = 0,
            EffectType::FireIgnite => {
                // BaseFireBlock.fireIgnite (the player-only random bump is not needed here).
                if !self.fire_immune() {
                    if self.remaining_fire_ticks < 0 {
                        self.remaining_fire_ticks += 1;
                    }
                    if self.remaining_fire_ticks >= 0 {
                        self.ignite_for_seconds(8.0);
                    }
                }
            }
            EffectType::LavaIgnite => {
                if !self.fire_immune() {
                    self.ignite_for_seconds(15.0);
                }
            }
            EffectType::Extinguish => self.clear_fire(),
        }
        let _ = level;
    }

    fn apply_action(&mut self, level: &mut dyn EntityLevel, a: Action) {
        match a {
            Action::FireHurt(damage) => {
                self.hurt(level, DamageKind::InFire, damage, None);
            }
            Action::LavaHurt => self.lava_hurt(level),
            Action::MeltPowderSnow(pos) => {
                if self.is_on_fire() && level.mob_griefing() {
                    level.destroy_block(pos, false);
                }
            }
        }
    }

    /// `Entity.lavaHurt`.
    pub fn lava_hurt(&mut self, level: &mut dyn EntityLevel) {
        if self.fire_immune() {
            return;
        }
        if self.hurt(level, DamageKind::Lava, 4.0, None) && self.should_play_lava_hurt_sound() && !self.silent {
            let pitch = 2.0 + self.random.next_float() * 0.4;
            level.emit(Event::Sound {
                pos: self.position(),
                sound: "minecraft:entity.generic.burn",
                source: "neutral",
                volume: 0.4,
                pitch,
            });
        }
    }

    fn should_play_lava_hurt_sound(&self) -> bool {
        match &self.kind {
            EntityKind::Item(item) => item.health <= 0 || self.tick_count % 10 == 0,
            _ => true,
        }
    }

    /// `Block.stepOn` for the blocks that react to non-player entities.
    fn step_on(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16) {
        match kind(state) {
            Kind::Slime => {
                let d = self.delta.y.abs();
                if d < 0.1 && !self.shift_key_down {
                    let e = 0.4 + d * 0.2;
                    self.delta = self.delta.multiply(e, 1.0, e);
                }
            }
            _ => {
                let name = crate::blocks::block_name(state);
                if (name == "minecraft:redstone_ore" || name == "minecraft:deepslate_redstone_ore") && !self.shift_key_down {
                    let info = kiln_data::blocks_types::block_of(state);
                    if let (Some("false"), Some(lit)) = (info.property(state, "lit"), info.with_property(state, "lit", "true")) {
                        level.set_block(pos, lit, 3);
                    }
                }
            }
        }
    }

    /// `checkInsideBlocks(movements, collector)`.
    fn check_inside_blocks(&mut self, level: &mut dyn EntityLevel, movements: &[Movement]) {
        if !self.is_affected_by_blocks() {
            return;
        }
        let mut visited = SmallSet::default();
        for m in movements {
            let mut from = m.from;
            let d = m.to - m.from;
            let mut max_steps = 16;
            match m.axis_dependent_original {
                Some(original) if d.length_sqr() > 0.0 => {
                    for axis in Axis::step_order(original) {
                        let v = d.get(axis);
                        if v != 0.0 {
                            let to = from.relative(axis.positive(), v);
                            max_steps -= self.check_inside_segment(level, from, to, &mut visited, max_steps);
                            from = to;
                        }
                    }
                }
                _ => max_steps -= self.check_inside_segment(level, m.from, m.to, &mut visited, 16),
            }
            if max_steps <= 0 {
                self.check_inside_segment(level, m.to, m.to, &mut visited, 1);
            }
        }
    }

    /// `checkInsideBlocks(from, to, collector, visited, maxSteps)`: returns the steps used.
    fn check_inside_segment(
        &mut self,
        level: &mut dyn EntityLevel,
        from: Vec3,
        to: Vec3,
        visited: &mut SmallSet,
        max_steps: i32,
    ) -> i32 {
        let bb = self.make_bounding_box(to).deflate_all(9.999999747378752e-6);
        let too_far = from.distance_to_sqr(to) > 0.9999900000002526 * 0.9999900000002526;
        let mut counter = 0;
        let mut blocks = Vec::new();
        for_each_block_intersected_between(from, to, &bb, |pos, step| {
            if step >= max_steps {
                return false;
            }
            blocks.push((pos, step));
            true
        });
        for (pos, step) in blocks {
            if !self.is_alive() {
                break;
            }
            counter = step;
            let state = level.block(pos);
            if physics::is_air(state) {
                continue;
            }
            let collided = match self.inside_shape(state) {
                None => true,
                Some(shape) => {
                    let boxes: Vec<Aabb> = shape.boxes().iter().map(|b| b.offset(pos.x as f64, pos.y as f64, pos.z as f64)).collect();
                    self.make_bounding_box(from).collided_along_vector(to - from, &boxes)
                }
            };
            let f = physics::fluid_state(state);
            let fluid_collided = !f.is_empty() && {
                let h = fluid::height(level, pos, &f);
                let fluid_box = Aabb::new(
                    pos.x as f64,
                    pos.y as f64,
                    pos.z as f64,
                    pos.x as f64 + 1.0,
                    (pos.y as f32 + h) as f64,
                    pos.z as f64 + 1.0,
                );
                self.make_bounding_box(from).collided_along_vector(to - from, &[fluid_box])
            };
            if (!collided && !fluid_collided) || !visited.insert(pos.as_long()) {
                continue;
            }
            if collided {
                let intersects = too_far || bb.intersects_block(pos);
                self.inside.advance_step(step);
                self.entity_inside(level, pos, state, intersects);
            }
            if fluid_collided {
                self.inside.advance_step(step);
                match f.kind {
                    FluidKind::Water | FluidKind::FlowingWater => self.inside.apply(EffectType::Extinguish),
                    FluidKind::Lava | FluidKind::FlowingLava => {
                        self.inside.apply(EffectType::ClearFreeze);
                        self.inside.apply(EffectType::LavaIgnite);
                        self.inside.run_after(EffectType::LavaIgnite, Action::LavaHurt);
                    }
                    FluidKind::Empty => {}
                }
            }
        }
        counter + 1
    }

    /// `getEntityInsideCollisionShape`; `None` for the full block.
    fn inside_shape(&self, state: u16) -> Option<&'static crate::shape::Shape> {
        if kind(state) == Kind::PowderSnow {
            let (shape, _) = crate::collision::collision_shape(state, self.block_position(), &self.collision_context());
            return match shape {
                std::borrow::Cow::Borrowed(s) if !s.is_empty() => Some(s),
                _ => None,
            };
        }
        physics::inside_shape(state)
    }

    /// `BlockState.entityInside` for the blocks with entity effects.
    fn entity_inside(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16, intersects: bool) {
        match kind(state) {
            Kind::Fire | Kind::SoulFire => {
                let damage = if kind(state) == Kind::SoulFire { 2.0 } else { 1.0 };
                self.inside.apply(EffectType::ClearFreeze);
                self.inside.apply(EffectType::FireIgnite);
                self.inside.run_after(EffectType::FireIgnite, Action::FireHurt(damage));
            }
            Kind::Cobweb => self.make_stuck_in_block(Vec3::new(0.25, 0.05000000074505806, 0.25)),
            Kind::SweetBerryBush => {}
            Kind::PowderSnow => {
                self.make_stuck_in_block(Vec3::new(0.8999999761581421, 1.5, 0.8999999761581421));
                let known = self.delta;
                if known.x != 0.0 || known.z != 0.0 {
                    level.random().next_bool();
                }
                self.inside.run_before(EffectType::Extinguish, Action::MeltPowderSnow(pos));
                self.inside.apply(EffectType::Freeze);
                self.inside.apply(EffectType::Extinguish);
            }
            Kind::BubbleColumn => {
                if intersects {
                    let above = level.block(pos.above());
                    let drag_down =
                        kiln_data::blocks_types::block_of(state).property(state, "drag") == Some("true");
                    if physics::collision_shape(above).is_empty() && physics::fluid_state(above).is_empty() {
                        self.on_above_bubble_column(drag_down);
                    } else {
                        self.on_inside_bubble_column(drag_down);
                    }
                }
            }
            Kind::HoneyBlock => {
                if self.is_sliding_down_honey(pos) {
                    self.do_honey_slide_movement();
                    if matches!(self.kind, EntityKind::Tnt(_)) {
                        let r = level.random();
                        if r.next_int_bounded(5) == 0 {
                            self.play_sound(level, "minecraft:block.honey_block.slide", 1.0, 1.0);
                        }
                        if level.random().next_int_bounded(5) == 0 {
                            level.emit(Event::EntityEvent { entity: self.id, event: 53 });
                        }
                    }
                }
            }
            Kind::Cactus => {
                self.hurt(level, DamageKind::Cactus, 1.0, None);
            }
            Kind::LavaCauldron => {
                self.inside.apply(EffectType::LavaIgnite);
                self.inside.run_after(EffectType::LavaIgnite, Action::LavaHurt);
            }
            _ => {}
        }
    }

    /// `handleOnAboveBubbleColumn`.
    fn on_above_bubble_column(&mut self, drag_down: bool) {
        let v = self.delta;
        let y = if drag_down { crate::math::jmax(-0.9, v.y - 0.03) } else { crate::math::jmin(1.8, v.y + 0.1) };
        self.delta = Vec3::new(v.x, y, v.z);
    }

    /// `handleOnInsideBubbleColumn`.
    fn on_inside_bubble_column(&mut self, drag_down: bool) {
        let v = self.delta;
        let y = if drag_down { crate::math::jmax(-0.3, v.y - 0.03) } else { crate::math::jmin(0.7, v.y + 0.06) };
        self.delta = Vec3::new(v.x, y, v.z);
        self.fall_distance = 0.0;
    }

    /// `HoneyBlock.isSlidingDown`.
    fn is_sliding_down_honey(&self, pos: BlockPos) -> bool {
        if self.on_ground {
            return false;
        }
        if self.y() > pos.y as f64 + 0.9375 - 1.0e-7 {
            return false;
        }
        if self.delta.y / 0.9800000190734863 + 0.08 >= -0.08 {
            return false;
        }
        let dx = (pos.x as f64 + 0.5 - self.x()).abs();
        let dz = (pos.z as f64 + 0.5 - self.z()).abs();
        let reach = 0.4375 + (self.width / 2.0) as f64;
        dx + 1.0e-7 > reach || dz + 1.0e-7 > reach
    }

    /// `HoneyBlock.doSlideMovement`.
    fn do_honey_slide_movement(&mut self) {
        let v = self.delta;
        let new_y = (-0.05 - 0.08) * 0.9800000190734863;
        let old_y = v.y / 0.9800000190734863 + 0.08;
        if old_y < -0.13 {
            let f = -0.05 / old_y;
            self.delta = Vec3::new(v.x * f, new_y, v.z * f);
        } else {
            self.delta = Vec3::new(v.x, new_y, v.z);
        }
        self.fall_distance = 0.0;
    }
}

/// A set of packed block positions; entity paths touch few blocks, so a vector is fastest.
#[derive(Default)]
pub(crate) struct SmallSet(Vec<i64>);

impl SmallSet {
    /// `LongSet.add`: true if newly added.
    fn insert(&mut self, v: i64) -> bool {
        if self.0.contains(&v) {
            false
        } else {
            self.0.push(v);
            true
        }
    }
}

/// `BlockGetter.forEachBlockIntersectedBetween`: every block the box `bb` (placed at `to`)
/// touches on its way from `from`, in vanilla's visiting order, with the step index.
pub fn for_each_block_intersected_between(from: Vec3, to: Vec3, bb: &Aabb, mut visit: impl FnMut(BlockPos, i32) -> bool) -> bool {
    let d = to - from;
    if d.length_sqr() < (1.0e-5f32 * 1.0e-5f32) as f64 {
        let lo = BlockPos::containing(bb.min_x, bb.min_y, bb.min_z);
        let hi = BlockPos::containing(bb.max_x, bb.max_y, bb.max_z);
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    if !visit(BlockPos::new(x, y, z), 0) {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    let mut seen = SmallSet::default();
    for pos in corners_in_direction(&bb.offset_vec(d.scale(-1.0)), d) {
        if !visit(pos, 0) {
            return false;
        }
        seen.insert(pos.as_long());
    }
    let steps = add_collisions_along_travel(&mut seen, d, bb, &mut visit);
    if steps < 0 {
        return false;
    }
    for pos in corners_in_direction(bb, d) {
        if seen.insert(pos.as_long()) && !visit(pos, steps + 1) {
            return false;
        }
    }
    true
}

/// `BlockPos.betweenCornersInDirection(AABB, Vec3)`.
fn corners_in_direction(bb: &Aabb, d: Vec3) -> Vec<BlockPos> {
    corners_between(
        floor(bb.min_x),
        floor(bb.min_y),
        floor(bb.min_z),
        floor(bb.max_x),
        floor(bb.max_y),
        floor(bb.max_z),
        d,
    )
}

/// `BlockPos.betweenCornersInDirection(ints, Vec3)`: the box's blocks starting from the corner
/// facing away from `d`, the first step-order axis outermost.
fn corners_between(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, d: Vec3) -> Vec<BlockPos> {
    let (min_x, min_y, min_z) = (x0.min(x1), y0.min(y1), z0.min(z1));
    let (max_x, max_y, max_z) = (x0.max(x1), y0.max(y1), z0.max(z1));
    let sizes = [max_x - min_x, max_y - min_y, max_z - min_z];
    let start = [
        if d.x >= 0.0 { min_x } else { max_x },
        if d.y >= 0.0 { min_y } else { max_y },
        if d.z >= 0.0 { min_z } else { max_z },
    ];
    let order = Axis::step_order(d);
    let dir = |a: Axis| -> [i32; 3] {
        let s = if d.get(a) >= 0.0 { 1 } else { -1 };
        let mut v = [0; 3];
        v[a as usize] = s;
        v
    };
    let (d1, d2, d3) = (dir(order[0]), dir(order[1]), dir(order[2]));
    let (m1, m2, m3) = (sizes[order[0] as usize], sizes[order[1] as usize], sizes[order[2] as usize]);
    let mut out = Vec::with_capacity(((m1 + 1) * (m2 + 1) * (m3 + 1)).max(0) as usize);
    for i in 0..=m1 {
        for j in 0..=m2 {
            for k in 0..=m3 {
                out.push(BlockPos::new(
                    start[0] + d1[0] * i + d2[0] * j + d3[0] * k,
                    start[1] + d1[1] * i + d2[1] * j + d3[1] * k,
                    start[2] + d1[2] * i + d2[2] * j + d3[2] * k,
                ));
            }
        }
    }
    out
}

/// `BlockGetter.getFurthestCorner`.
fn furthest_corner(d: Vec3) -> [i32; 3] {
    let ax = (d.x * 1.0 + d.y * 0.0 + d.z * 0.0).abs();
    let ay = (d.x * 0.0 + d.y * 1.0 + d.z * 0.0).abs();
    let az = (d.x * 0.0 + d.y * 0.0 + d.z * 1.0).abs();
    let sx = if d.x >= 0.0 { 1 } else { -1 };
    let sy = if d.y >= 0.0 { 1 } else { -1 };
    let sz = if d.z >= 0.0 { 1 } else { -1 };
    if ax <= ay && ax <= az {
        [-sx, -sz, sy]
    } else if ay <= az {
        [sz, -sy, -sx]
    } else {
        [-sy, sx, -sz]
    }
}

fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

fn frac(v: f64) -> f64 {
    v - v.floor() as i64 as f64
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo { lo } else { crate::math::jmin(v, hi) }
}

/// `BlockGetter.addCollisionsAlongTravel`: walks the grid along the travel of the box's leading
/// corner and visits the box-sized slabs it sweeps; returns the step count or -1 on abort.
fn add_collisions_along_travel(
    seen: &mut SmallSet,
    d: Vec3,
    bb: &Aabb,
    visit: &mut impl FnMut(BlockPos, i32) -> bool,
) -> i32 {
    let (xs, ys, zs) = (bb.x_size(), bb.y_size(), bb.z_size());
    let corner = furthest_corner(d);
    let c = bb.center();
    let end = Vec3::new(c.x + xs * 0.5 * corner[0] as f64, c.y + ys * 0.5 * corner[1] as f64, c.z + zs * 0.5 * corner[2] as f64);
    let start = end - d;
    let (mut x, mut y, mut z) = (floor(start.x), floor(start.y), floor(start.z));
    let (sx, sy, sz) = (sign(d.x), sign(d.y), sign(d.z));
    let tx = if sx == 0 { f64::MAX } else { sx as f64 / d.x };
    let ty = if sy == 0 { f64::MAX } else { sy as f64 / d.y };
    let tz = if sz == 0 { f64::MAX } else { sz as f64 / d.z };
    let mut nx = tx * if sx > 0 { 1.0 - frac(start.x) } else { frac(start.x) };
    let mut ny = ty * if sy > 0 { 1.0 - frac(start.y) } else { frac(start.y) };
    let mut nz = tz * if sz > 0 { 1.0 - frac(start.z) } else { frac(start.z) };
    let mut steps = 0;
    while nx <= 1.0 || ny <= 1.0 || nz <= 1.0 {
        if nx < ny {
            if nx < nz {
                x += sx;
                nx += tx;
            } else {
                z += sz;
                nz += tz;
            }
        } else if ny < nz {
            y += sy;
            ny += ty;
        } else {
            z += sz;
            nz += tz;
        }
        let cell = Aabb {
            min_x: x as f64,
            min_y: y as f64,
            min_z: z as f64,
            max_x: (x + 1) as f64,
            max_y: (y + 1) as f64,
            max_z: (z + 1) as f64,
        };
        let Some(hit) = cell.clip(start, end) else { continue };
        steps += 1;
        let hx = clamp(hit.x, x as f64 + 9.999999747378752e-6, x as f64 + 1.0 - 9.999999747378752e-6);
        let hy = clamp(hit.y, y as f64 + 9.999999747378752e-6, y as f64 + 1.0 - 9.999999747378752e-6);
        let hz = clamp(hit.z, z as f64 + 9.999999747378752e-6, z as f64 + 1.0 - 9.999999747378752e-6);
        let ox = floor(hx - xs * corner[0] as f64);
        let oy = floor(hy - ys * corner[1] as f64);
        let oz = floor(hz - zs * corner[2] as f64);
        let step = steps;
        for pos in corners_between(x, y, z, ox, oy, oz, d) {
            if seen.insert(pos.as_long()) && !visit(pos, step) {
                return -1;
            }
        }
    }
    steps
}
