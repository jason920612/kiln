//! A survival bot's body: vanilla player movement (`LivingEntity.travel`, `Entity.move`) on
//! the blocks the bot knows. The server trusts the client's positions within its movement
//! checks, so this only has to be plausible: gravity, jumping, stepping up 0.6, swimming, and
//! never entering a collision box.

use crate::world::{self, World};
use kiln_data::block_props;

const WIDTH: f64 = 0.6;
const HEIGHT: f64 = 1.8;
const STEP: f64 = 0.6;
pub const EYE: f64 = 1.62;
const WALK_SPEED: f64 = 0.1;
const SPRINT_SPEED: f64 = 0.13;

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub forward: bool,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

#[derive(Debug, Clone, Copy)]
struct Aabb {
    min: [f64; 3],
    max: [f64; 3],
}

impl Aabb {
    fn at(pos: [f64; 3]) -> Self {
        let r = WIDTH / 2.0;
        Aabb { min: [pos[0] - r, pos[1], pos[2] - r], max: [pos[0] + r, pos[1] + HEIGHT, pos[2] + r] }
    }

    fn moved(&self, d: [f64; 3]) -> Self {
        Aabb { min: [0, 1, 2].map(|i| self.min[i] + d[i]), max: [0, 1, 2].map(|i| self.max[i] + d[i]) }
    }

    fn overlaps_other_axes(&self, o: &Aabb, axis: usize) -> bool {
        (0..3).filter(|&i| i != axis).all(|i| self.min[i] < o.max[i] && self.max[i] > o.min[i])
    }
}

#[derive(Debug, Clone)]
pub struct Body {
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub in_water: bool,
    pub in_lava: bool,
    /// Blocks fallen since the last landing.
    pub fall_distance: f64,
    scratch: Vec<Aabb>,
}

impl Body {
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> Self {
        Self {
            pos,
            vel: [0.0; 3],
            yaw,
            pitch,
            on_ground: false,
            horizontal_collision: false,
            in_water: false,
            in_lava: false,
            fall_distance: 0.0,
            scratch: Vec::new(),
        }
    }

    pub fn teleport(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) {
        self.pos = pos;
        self.yaw = yaw;
        self.pitch = pitch;
        self.vel = [0.0; 3];
        self.fall_distance = 0.0;
    }

    pub fn eye(&self) -> [f64; 3] {
        [self.pos[0], self.pos[1] + EYE, self.pos[2]]
    }

    pub fn block_pos(&self) -> [i32; 3] {
        [self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32]
    }

    /// One client tick.
    pub fn tick(&mut self, w: &World, input: Input) {
        let [bx, by, bz] = self.block_pos();
        let feet = w.block(bx, by, bz);
        let chest = w.block(bx, (self.pos[1] + 0.9).floor() as i32, bz);
        self.in_water = world::is_water(feet) || world::is_water(chest);
        self.in_lava = world::is_lava(feet) || world::is_lava(chest);
        let (sin, cos) = (self.yaw as f64).to_radians().sin_cos();
        let dir = if input.forward { [-sin * 0.98, cos * 0.98] } else { [0.0, 0.0] };

        if self.in_water || self.in_lava {
            if input.jump {
                self.vel[1] += 0.04;
            }
            let accel = 0.02;
            self.vel[0] += dir[0] * accel;
            self.vel[2] += dir[1] * accel;
            let v = self.vel;
            self.step(w, v);
            // Climbing out onto a ledge.
            if self.horizontal_collision && self.free(w, [self.vel[0], self.vel[1] + 0.6, self.vel[2]]) {
                self.vel[1] = 0.3;
            }
            let drag = if self.in_lava { 0.5 } else { 0.8 };
            self.vel[0] *= drag;
            self.vel[2] *= drag;
            self.vel[1] = self.vel[1] * drag - if self.in_lava { 0.02 / 4.0 } else { 0.02 / 4.0 };
            self.fall_distance = 0.0;
            return;
        }

        if input.jump && self.on_ground {
            self.vel[1] = 0.42;
            if input.sprint {
                self.vel[0] -= sin * 0.2;
                self.vel[2] += cos * 0.2;
            }
        }
        let accel = if self.on_ground {
            if input.sprint { SPRINT_SPEED } else { WALK_SPEED }
        } else if input.sprint {
            0.026
        } else {
            0.02
        };
        self.vel[0] += dir[0] * accel;
        self.vel[2] += dir[1] * accel;
        let was_ground = self.on_ground;
        let y0 = self.pos[1];
        let v = self.vel;
        self.step(w, v);
        if self.pos[1] < y0 && !self.on_ground {
            self.fall_distance += y0 - self.pos[1];
        } else if self.on_ground {
            self.fall_distance = 0.0;
        }
        let _ = was_ground;
        let friction = if self.on_ground { 0.6 * 0.91 } else { 0.91 };
        self.vel[0] *= friction;
        self.vel[2] *= friction;
        self.vel[1] = (self.vel[1] - 0.08) * 0.98;
    }

    fn free(&mut self, w: &World, d: [f64; 3]) -> bool {
        let b = Aabb::at(self.pos).moved(d);
        self.boxes(w, &b);
        !self.scratch.iter().any(|o| (0..3).all(|i| b.min[i] < o.max[i] && b.max[i] > o.min[i]))
    }

    /// Fills `scratch` with the collision boxes of blocks overlapping `region`.
    fn boxes(&mut self, w: &World, region: &Aabb) {
        self.scratch.clear();
        let lo = region.min.map(|v| (v - 1e-7).floor() as i32);
        let hi = region.max.map(|v| (v + 1e-7).floor() as i32);
        for x in lo[0]..=hi[0] {
            for z in lo[2]..=hi[2] {
                for y in (lo[1] - 1)..=hi[1] {
                    let s = w.block(x, y, z);
                    for b in block_props::collision(s) {
                        self.scratch.push(Aabb {
                            min: [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64],
                            max: [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64],
                        });
                    }
                }
            }
        }
    }

    fn clip(&self, b: &Aabb, axis: usize, mut d: f64) -> f64 {
        if d == 0.0 {
            return 0.0;
        }
        for o in &self.scratch {
            if !b.overlaps_other_axes(o, axis) {
                continue;
            }
            if d > 0.0 && o.min[axis] >= b.max[axis] - 1e-9 {
                d = d.min(o.min[axis] - b.max[axis]);
            } else if d < 0.0 && o.max[axis] <= b.min[axis] + 1e-9 {
                d = d.max(o.max[axis] - b.min[axis]);
            }
        }
        d
    }

    /// `Entity.collideBoundingBox`: Y first, then the longer horizontal axis.
    fn collide_box(&self, b: &Aabb, d: [f64; 3]) -> [f64; 3] {
        let mut b = *b;
        let dy = self.clip(&b, 1, d[1]);
        b = b.moved([0.0, dy, 0.0]);
        let x_first = d[0].abs() >= d[2].abs();
        let (dx, dz);
        if x_first {
            dx = self.clip(&b, 0, d[0]);
            b = b.moved([dx, 0.0, 0.0]);
            dz = self.clip(&b, 2, d[2]);
        } else {
            dz = self.clip(&b, 2, d[2]);
            b = b.moved([0.0, 0.0, dz]);
            dx = self.clip(&b, 0, d[0]);
        }
        [dx, dy, dz]
    }

    /// `Entity.move`: collides, steps up, and updates the ground and collision flags.
    fn step(&mut self, w: &World, d: [f64; 3]) {
        let b = Aabb::at(self.pos);
        let region = Aabb {
            min: [0, 1, 2].map(|i| b.min[i] + d[i].min(0.0) - 0.01),
            max: [0, 1, 2].map(|i| b.max[i] + d[i].max(0.0) + 0.01 + if i == 1 { STEP } else { 0.0 }),
        };
        self.boxes(w, &region);
        let mut m = self.collide_box(&b, d);
        let (cx, cz, cy) = (d[0] != m[0], d[2] != m[2], d[1] != m[1]);
        if (self.on_ground || (cy && d[1] < 0.0)) && (cx || cz) {
            let mut up = self.collide_box(&b, [d[0], STEP, d[2]]);
            let lift = self.collide_box(&b.moved([0.0; 3]), [0.0, STEP, 0.0]);
            if lift[1] < STEP {
                let mut alt = self.collide_box(&b.moved(lift), [d[0], 0.0, d[2]]);
                alt[1] += lift[1];
                if alt[0] * alt[0] + alt[2] * alt[2] > up[0] * up[0] + up[2] * up[2] {
                    up = alt;
                }
            }
            if up[0] * up[0] + up[2] * up[2] > m[0] * m[0] + m[2] * m[2] {
                let down = self.collide_box(&b.moved(up), [0.0, -up[1] + d[1], 0.0]);
                m = [up[0] + down[0], up[1] + down[1], up[2] + down[2]];
            }
        }
        for i in 0..3 {
            self.pos[i] += m[i];
        }
        self.horizontal_collision = cx || cz;
        self.on_ground = cy && d[1] < 0.0;
        if cx {
            self.vel[0] = 0.0;
        }
        if cz {
            self.vel[2] = 0.0;
        }
        if cy {
            self.vel[1] = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    fn flat() -> World {
        crate::world::test_world()
    }

    fn run(b: &mut Body, w: &World, ticks: usize, input: Input) {
        for _ in 0..ticks {
            b.tick(w, input);
        }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        let w = flat();
        let mut b = Body::new([0.5, 10.0, 0.5], 0.0, 0.0);
        run(&mut b, &w, 40, Input::default());
        assert!(b.on_ground);
        assert!((b.pos[1] - 0.0).abs() < 1e-6, "{}", b.pos[1]);
    }

    #[test]
    fn walking_and_sprinting_match_vanilla_speeds() {
        let w = flat();
        for (sprint, expect) in [(false, 4.317), (true, 5.612)] {
            let mut b = Body::new([0.5, 0.0, 0.5], 0.0, 0.0);
            run(&mut b, &w, 10, Input::default());
            run(&mut b, &w, 10, Input { forward: true, sprint, ..Input::default() });
            let z0 = b.pos[2];
            run(&mut b, &w, 20, Input { forward: true, sprint, ..Input::default() });
            let speed = b.pos[2] - z0;
            assert!((speed - expect).abs() < 0.05, "sprint {sprint}: {speed}");
        }
    }

    #[test]
    fn steps_up_slabs_and_jumps_one_block() {
        let mut w = flat();
        // yaw 0 walks toward +z. A full block at z=3 needs a jump.
        w.set(0, 0, 3, d::STONE);
        w.set(-1, 0, 3, d::STONE);
        let mut b = Body::new([0.5, 0.0, 0.5], 0.0, 0.0);
        run(&mut b, &w, 5, Input::default());
        run(&mut b, &w, 30, Input { forward: true, ..Input::default() });
        assert!(b.pos[2] < 3.0, "walked into the block: {}", b.pos[2]);
        assert!(b.horizontal_collision);
        run(&mut b, &w, 30, Input { forward: true, jump: true, ..Input::default() });
        assert!(b.pos[2] > 3.5 && b.pos[1] >= 1.0 - 1e-6, "{:?}", b.pos);
    }

    #[test]
    fn swims_up_in_water() {
        let mut w = flat();
        for y in -4..3 {
            for x in -3..3 {
                for z in -3..3 {
                    w.set(x, y, z, d::WATER);
                }
            }
        }
        let mut b = Body::new([0.5, -3.0, 0.5], 0.0, 0.0);
        run(&mut b, &w, 3, Input::default());
        assert!(b.in_water);
        run(&mut b, &w, 60, Input { jump: true, ..Input::default() });
        assert!(b.pos[1] > 0.0, "{}", b.pos[1]);
    }
}
