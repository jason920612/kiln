//! `LookControl`, `MoveControl`, `JumpControl` and `BodyRotationControl`.

use super::MobData;
use super::attributes::Attr;
use super::mth;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{Axis, BlockPos};

#[derive(Clone, Debug, Default)]
pub struct LookControl {
    pub wanted: [f64; 3],
    pub y_max_rot_speed: f32,
    pub x_max_rot_angle: f32,
    pub cooldown: i32,
}

impl LookControl {
    pub fn set_look_at(&mut self, x: f64, y: f64, z: f64, speed: f32, max_x: f32) {
        self.wanted = [x, y, z];
        self.y_max_rot_speed = speed;
        self.x_max_rot_angle = max_x;
        self.cooldown = 2;
    }

    pub fn is_looking_at_target(&self) -> bool {
        self.cooldown > 0
    }
}

/// `setLookAt(x, y, z)` with the mob's head speed and pitch limit.
pub fn look_at(m: &mut MobData, x: f64, y: f64, z: f64) {
    let (speed, max_x) = (m.kind.head_rot_speed() as f32, m.max_head_x_rot() as f32);
    m.look.set_look_at(x, y, z, speed, max_x);
}

/// `LookControl.tick`.
pub fn tick_look(e: &mut Entity, m: &mut MobData) {
    e.x_rot = 0.0;
    if m.look.cooldown > 0 {
        m.look.cooldown -= 1;
        let [wx, wy, wz] = m.look.wanted;
        let (dx, dz) = (wx - e.x(), wz - e.z());
        if dz.abs() > 9.999999747378752e-6 || dx.abs() > 9.999999747378752e-6 {
            let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
            m.y_head_rot = mth::rotate_towards(m.y_head_rot, yaw, m.look.y_max_rot_speed);
        }
        let dy = wy - e.eye_y();
        let h = (dx * dx + dz * dz).sqrt();
        if dy.abs() > 9.999999747378752e-6 || h.abs() > 9.999999747378752e-6 {
            let pitch = (-(mth::atan2(dy, h) * 57.2957763671875)) as f32;
            e.x_rot = mth::rotate_towards(e.x_rot, pitch, m.look.x_max_rot_angle);
        }
    } else {
        m.y_head_rot = mth::rotate_towards(m.y_head_rot, m.y_body_rot, 10.0);
    }
    if !m.nav.is_done() {
        m.y_head_rot = mth::rotate_if_necessary(m.y_head_rot, m.y_body_rot, m.kind.max_head_y_rot() as f32);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operation {
    #[default]
    Wait,
    MoveTo,
    Strafe,
    Jumping,
}

#[derive(Clone, Debug, Default)]
pub struct MoveControl {
    pub wanted: [f64; 3],
    pub speed_modifier: f64,
    pub strafe_forwards: f32,
    pub strafe_right: f32,
    pub operation: Operation,
    /// `setWantedPosition` calls so far, and those with a positive speed and the last such
    /// speed (types that override `setWantedPosition` catch up on them: rabbits).
    pub sets: u32,
    pub positive_sets: u32,
    pub last_positive_speed: f64,
}

impl MoveControl {
    pub fn has_wanted(&self) -> bool {
        self.operation == Operation::MoveTo
    }

    pub fn set_wanted_position(&mut self, x: f64, y: f64, z: f64, speed: f64) {
        self.wanted = [x, y, z];
        self.speed_modifier = speed;
        self.sets = self.sets.wrapping_add(1);
        if speed > 0.0 {
            self.positive_sets = self.positive_sets.wrapping_add(1);
            self.last_positive_speed = speed;
        }
        if self.operation != Operation::Jumping {
            self.operation = Operation::MoveTo;
        }
    }

    pub fn strafe(&mut self, forwards: f32, right: f32) {
        self.operation = Operation::Strafe;
        self.strafe_forwards = forwards;
        self.strafe_right = right;
        self.speed_modifier = 0.25;
    }
}

/// `MoveControl.rotlerp`.
pub fn rotlerp(from: f32, to: f32, max: f32) -> f32 {
    let mut d = mth::wrap_degrees(to - from);
    if d > max {
        d = max;
    }
    if d < -max {
        d = -max;
    }
    let mut r = from + d;
    if r < 0.0 {
        r += 360.0;
    } else if r > 360.0 {
        r -= 360.0;
    }
    r
}

/// `Mob.setSpeed`: the speed and the forward input.
pub fn set_speed(m: &mut MobData, speed: f32) {
    m.speed = speed;
    m.zza = speed;
}

/// `MoveControl.tick`.
pub fn tick_move(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let speed_attr = m.attrs.value(Attr::MovementSpeed);
    match m.mov.operation {
        Operation::Strafe => {
            let f = speed_attr as f32;
            let g = m.mov.speed_modifier as f32 * f;
            let (mut h, mut i) = (m.mov.strafe_forwards, m.mov.strafe_right);
            let mut j = mth::sqrt_f(h * h + i * i);
            if j < 1.0 {
                j = 1.0;
            }
            j = g / j;
            h *= j;
            i *= j;
            let k = mth::sin((e.y_rot * 0.017453292) as f64);
            let l = mth::cos((e.y_rot * 0.017453292) as f64);
            let n = h * l - i * k;
            let o = i * l + h * k;
            if !is_walkable(e, m, level, n, o) {
                m.mov.strafe_forwards = 1.0;
                m.mov.strafe_right = 0.0;
            }
            set_speed(m, g);
            m.zza = m.mov.strafe_forwards;
            m.xxa = m.mov.strafe_right;
            m.mov.operation = Operation::Wait;
        }
        Operation::MoveTo => {
            m.mov.operation = Operation::Wait;
            let [wx, wy, wz] = m.mov.wanted;
            let (d, e2, f) = (wx - e.x(), wz - e.z(), wy - e.y());
            let g = d * d + f * f + e2 * e2;
            if g < 2.500000277905201e-7 {
                m.zza = 0.0;
                return;
            }
            let h = (mth::atan2(e2, d) * 57.2957763671875) as f32 - 90.0;
            e.y_rot = rotlerp(e.y_rot, h, 90.0);
            set_speed(m, (m.mov.speed_modifier * speed_attr) as f32);
            let pos = e.block_position();
            let state = level.block(pos);
            let (shape, _) = crate::collision::collision_shape(state, pos, &crate::collision::CollisionContext::EMPTY);
            let up = e.max_up_step as f64;
            let jump = (f > up && d * d + e2 * e2 < (1f32.max(e.width)) as f64)
                || (!shape.is_empty()
                    && e.y() < shape.max(Axis::Y, 0.0) + pos.y as f64
                    && !crate::blocks::has_tag(state, crate::blocks::Tag::Doors)
                    && !crate::blocks::has_tag(state, crate::blocks::Tag::Fences));
            if jump {
                m.jump.jump = true;
                m.mov.operation = Operation::Jumping;
            }
        }
        Operation::Jumping => {
            set_speed(m, (m.mov.speed_modifier * speed_attr) as f32);
            if e.on_ground || (e.is_in_water() || e.is_in_lava()) {
                m.mov.operation = Operation::Wait;
            }
        }
        Operation::Wait => m.zza = 0.0,
    }
}

/// `MoveControl.isWalkable`.
fn is_walkable(e: &Entity, _m: &MobData, level: &dyn EntityLevel, dx: f32, dz: f32) -> bool {
    let p = BlockPos::containing(e.x() + dx as f64, e.block_position().y as f64, e.z() + dz as f64);
    // `getPathType(mob, pos)` with a fresh context: the static type.
    super::path::path_type_static(level, p.x, p.y, p.z) == super::path::PathType::Walkable
}

#[derive(Clone, Debug, Default)]
pub struct JumpControl {
    pub jump: bool,
}

/// `JumpControl.tick`.
pub fn tick_jump(m: &mut MobData) {
    m.jumping = m.jump.jump;
    m.jump.jump = false;
}

#[derive(Clone, Debug, Default)]
pub struct BodyRotationControl {
    head_stable_time: i32,
    last_stable_y_head_rot: f32,
}

/// `BodyRotationControl.clientTick` (run by `Mob.tickHeadTurn` on the server too).
pub fn tick_body(e: &Entity, m: &mut MobData) {
    let max = m.kind.max_head_y_rot() as f32;
    let (dx, dz) = (e.x() - e.old_pos.x, e.z() - e.old_pos.z);
    if dx * dx + dz * dz > 2.500000277905201e-7 {
        m.y_body_rot = e.y_rot;
        m.y_head_rot = mth::rotate_if_necessary(m.y_head_rot, m.y_body_rot, max);
        m.body.last_stable_y_head_rot = m.y_head_rot;
        m.body.head_stable_time = 0;
        return;
    }
    if (m.y_head_rot - m.body.last_stable_y_head_rot).abs() > 15.0 {
        m.body.head_stable_time = 0;
        m.body.last_stable_y_head_rot = m.y_head_rot;
        m.y_body_rot = mth::rotate_if_necessary(m.y_body_rot, m.y_head_rot, max);
    } else {
        m.body.head_stable_time += 1;
        if m.body.head_stable_time > 10 {
            let i = m.body.head_stable_time - 10;
            let f = mth::clamp(i as f32 / 10.0, 0.0, 1.0);
            let g = max * (1.0 - f);
            m.y_body_rot = mth::rotate_if_necessary(m.y_body_rot, m.y_head_rot, g);
        }
    }
}
