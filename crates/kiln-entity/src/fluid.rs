//! Fluid heights and currents (`FluidState`, `FlowingFluid.getFlow`) and vanilla 26.3's
//! `EntityFluidInteraction`: per-fluid submersion trackers and the current accumulators that
//! push entities.

use crate::blocks::{Kind, Tag, has_tag, kind};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{BlockPos, Direction, Vec3, ceil, floor, jmax};
use crate::physics::{self, FluidKind, FluidState};

pub fn fluid_at(level: &dyn EntityLevel, pos: BlockPos) -> FluidState {
    physics::fluid_state(level.block(pos))
}

/// `FluidState.getHeight`: 1 under the same fluid, else `amount / 9`.
pub fn height(level: &dyn EntityLevel, pos: BlockPos, f: &FluidState) -> f32 {
    if f.kind.is_same(fluid_at(level, pos.above()).kind) { 1.0 } else { f.own_height() }
}

/// `FluidState.getHeightForCamera`.
fn height_for_camera(level: &dyn EntityLevel, pos: BlockPos, f: &FluidState) -> f32 {
    if f.source && physics::is_face_sturdy(level.block(pos.above()), Direction::Down) {
        return 1.0;
    }
    height(level, pos, f)
}

/// `FlowingFluid.getFlow`: the current of the fluid at `pos`.
pub fn flow(level: &dyn EntityLevel, pos: BlockPos, f: &FluidState) -> Vec3 {
    if f.is_empty() {
        return Vec3::ZERO;
    }
    let affects = |o: &FluidState| o.is_empty() || o.kind.is_same(f.kind);
    let (mut dx, mut dz) = (0.0f64, 0.0f64);
    for dir in Direction::HORIZONTAL {
        let n = pos.relative(dir);
        let other = fluid_at(level, n);
        if !affects(&other) {
            continue;
        }
        let mut h = other.own_height();
        let mut diff = 0.0f32;
        if h == 0.0 {
            if !has_tag(level.block(n), Tag::BlocksFluidFlow) {
                let below = fluid_at(level, n.below());
                if affects(&below) {
                    h = below.own_height();
                    if h > 0.0 {
                        diff = f.own_height() - (h - 0.8888889);
                    }
                }
            }
        } else if h > 0.0 {
            diff = f.own_height() - h;
        }
        if diff != 0.0 {
            let (sx, _, sz) = dir.step();
            dx += (sx as f32 * diff) as f64;
            dz += (sz as f32 * diff) as f64;
        }
    }
    let mut v = Vec3::new(dx, 0.0, dz);
    if f.falling {
        for dir in Direction::HORIZONTAL {
            let n = pos.relative(dir);
            if is_solid_face(level, n, dir, f.kind) || is_solid_face(level, n.above(), dir, f.kind) {
                v = v.normalize().add(0.0, -6.0, 0.0);
                break;
            }
        }
    }
    v.normalize()
}

/// `FlowingFluid.isSolidFace`.
fn is_solid_face(level: &dyn EntityLevel, pos: BlockPos, dir: Direction, fluid: FluidKind) -> bool {
    let state = level.block(pos);
    if physics::fluid_state(state).kind.is_same(fluid) {
        return false;
    }
    if dir == Direction::Up {
        return true;
    }
    if kind(state) == Kind::Ice {
        return false;
    }
    physics::is_face_sturdy(state, dir)
}

#[derive(Clone, Debug)]
struct Tracker {
    fluid: FluidKind,
    height: f64,
    eyes_inside: bool,
}

#[derive(Clone, Debug, Default)]
struct Accumulator {
    height: f64,
    current: Vec3,
    count: i32,
}

/// `EntityFluidInteraction`.
#[derive(Clone, Debug, Default)]
pub struct FluidInteraction {
    trackers: Vec<Tracker>,
    water: Accumulator,
    lava: Accumulator,
}

impl FluidInteraction {
    /// `getFluidHeight(WATER / LAVA)`.
    pub fn height(&self, water: bool) -> f64 {
        let mut best = 0.0;
        for t in &self.trackers {
            if t.height > best && t.fluid.is_water() == water && t.fluid != FluidKind::Empty {
                best = t.height;
            }
        }
        best
    }

    pub fn is_in_water(&self) -> bool {
        self.height(true) > 0.0
    }

    pub fn is_in_lava(&self) -> bool {
        self.height(false) > 0.0
    }

    pub fn is_eye_in_water(&self) -> bool {
        self.trackers.iter().any(|t| t.eyes_inside && t.fluid.is_water())
    }

    /// `update(entity, ignoreCurrent)`: measures submersion and accumulates currents.
    fn update(&mut self, level: &dyn EntityLevel, e: &Entity, ignore_current: bool) -> bool {
        self.trackers.retain_mut(|t| {
            if t.height == 0.0 && !t.eyes_inside {
                return false;
            }
            t.height = 0.0;
            t.eyes_inside = false;
            true
        });
        self.water = Accumulator::default();
        self.lava = Accumulator::default();
        let bb = e.bounding_box().deflate_all(0.001);
        let (x0, y0, z0) = (floor(bb.min_x), floor(bb.min_y), floor(bb.min_z));
        let (x1, y1, z1) = (ceil(bb.max_x) - 1, ceil(bb.max_y) - 1, ceil(bb.max_z) - 1);
        let entity_min_y = e.bounding_box().min_y;
        let block = e.block_position();
        let eye_y = e.eye_y();
        let mut last: Option<FluidKind> = None;
        let mut tracker = 0usize;
        let mut acc: Option<bool> = None;
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let pos = BlockPos::new(x, y, z);
                    let f = fluid_at(level, pos);
                    if f.is_empty() {
                        continue;
                    }
                    let block_y = y as f64;
                    let top = block_y + height(level, pos, &f) as f64;
                    if top < bb.min_y {
                        continue;
                    }
                    if last != Some(f.kind) {
                        last = Some(f.kind);
                        tracker = match self.trackers.iter().position(|t| t.fluid == f.kind) {
                            Some(i) => i,
                            None => {
                                self.trackers.push(Tracker { fluid: f.kind, height: 0.0, eyes_inside: false });
                                self.trackers.len() - 1
                            }
                        };
                        if !ignore_current {
                            acc = Some(f.kind.is_water());
                        }
                    }
                    if x == block.x && z == block.z && eye_y >= block_y {
                        let camera_top = block_y + height_for_camera(level, pos, &f) as f64;
                        if eye_y <= camera_top {
                            self.trackers[tracker].eyes_inside = true;
                        }
                    }
                    let t = &mut self.trackers[tracker];
                    t.height = jmax(top - entity_min_y, t.height);
                    let t_height = t.height;
                    if let Some(water) = acc {
                        let mut current = flow(level, pos, &f);
                        let a = if water { &mut self.water } else { &mut self.lava };
                        a.height = jmax(t_height, a.height);
                        if a.height < 0.4 {
                            current = current.scale(a.height);
                        }
                        a.current = a.current + current;
                        a.count += 1;
                    }
                }
            }
        }
        last.is_some()
    }

    /// `CurrentAccumulator.applyTo`.
    fn apply_current(&self, water: bool, e: &mut Entity, scale: f64, player: bool) {
        let a = if water { &self.water } else { &self.lava };
        if a.count == 0 || a.current.length_sqr() < 9.999999747378752e-6 {
            return;
        }
        let mut v = if player { a.current.scale(1.0 / a.count as f64) } else { a.current.normalize() };
        let d = e.delta;
        v = v.scale(scale);
        if d.x.abs() < 0.003 && d.z.abs() < 0.003 && v.length() < 0.0045000000000000005 {
            v = v.normalize().scale(0.0045000000000000005);
        }
        e.add_delta_movement(v);
    }
}

impl Entity {
    /// `Entity.updateFluidInteraction`: submersion, fall reset in water, and fluid currents.
    pub fn update_fluid_interaction(&mut self, level: &mut dyn EntityLevel) -> bool {
        let mut fluid = std::mem::take(&mut self.fluid);
        let result = fluid.update(level, self, !self.is_pushed_by_fluid());
        let in_water = fluid.is_in_water();
        let in_lava = fluid.is_in_lava();
        self.fluid = fluid;
        if in_water {
            self.fall_distance = 0.0;
            if !self.was_touching_water && !self.first_tick {
                self.do_water_splash_effect(level);
            }
        }
        self.was_touching_water = in_water;
        if self.is_pushed_by_fluid() {
            let fluid = std::mem::take(&mut self.fluid);
            if in_water {
                fluid.apply_current(true, self, 0.014, false);
            }
            if in_lava {
                let scale = if level.fast_lava() { 0.007 } else { 0.0023333333333333335 };
                fluid.apply_current(false, self, scale, false);
            }
            self.fluid = fluid;
        }
        result
    }

    /// `doWaterSplashEffect`: a splash sound and particles; only the random draws matter here.
    fn do_water_splash_effect(&mut self, level: &mut dyn EntityLevel) {
        // ExperienceOrb overrides it with nothing.
        if matches!(self.kind, crate::entity::EntityKind::ExperienceOrb(_)) {
            return;
        }
        let d = self.delta;
        let volume = (1.0f32).min(((d.x * d.x * 0.20000000298023224 + d.y * d.y + d.z * d.z * 0.20000000298023224).sqrt() as f32) * 0.2);
        let sound = if volume < 0.25 { "minecraft:entity.generic.splash" } else { "minecraft:entity.generic.swim" };
        let pitch = 1.0 + (self.random_next_float() - self.random_next_float()) * 0.4;
        self.play_sound(level, sound, volume, pitch);
        let n = 1.0 + self.width * 20.0;
        let mut i = 0;
        while (i as f32) < n {
            self.random_next_double();
            self.random_next_double();
            self.random_next_double();
            i += 1;
        }
        let mut i = 0;
        while (i as f32) < n {
            self.random_next_double();
            self.random_next_double();
            i += 1;
        }
        level.emit(crate::level::Event::GameEvent { event: "minecraft:splash", pos: self.position(), entity: Some(self.id) });
    }

    fn random_next_float(&mut self) -> f32 {
        kiln_javamath::random::RandomSource::next_float(&mut self.random)
    }

    fn random_next_double(&mut self) -> f64 {
        kiln_javamath::random::RandomSource::next_double(&mut self.random)
    }
}
