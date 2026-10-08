//! Flying mobs: `FlyingMoveControl`, `LivingEntity.travelFlying`, and a stand-in for
//! `FlyingPathNavigation` that flies straight at the target (no path search around
//! obstacles: [`DirectFlight`], replaceable by a real fly-node navigation).

use super::attributes::Attr;
use super::control::{self, Operation};
use super::{MobData, mth};
use crate::entity::{Entity, MoverType};
use crate::level::EntityLevel;
use crate::math::Vec3;

/// `FlyingMoveControl.tick` (`max_turn`: the pitch turn limit; `hovers`: keeps no gravity when
/// idle).
pub fn tick_move(e: &mut Entity, m: &mut MobData, max_turn: f32, hovers: bool) {
    if m.mov.operation != Operation::MoveTo {
        if !hovers {
            e.no_gravity = false;
        }
        m.yya = 0.0;
        m.zza = 0.0;
        return;
    }
    m.mov.operation = Operation::Wait;
    e.no_gravity = true;
    let [wx, wy, wz] = m.mov.wanted;
    let (xd, yd, zd) = (wx - e.x(), wy - e.y(), wz - e.z());
    let dd = xd * xd + yd * yd + zd * zd;
    if dd < 2.5000003e-7f32 as f64 {
        m.yya = 0.0;
        m.zza = 0.0;
        return;
    }
    let yaw = (mth::atan2(zd, xd) * 57.2957763671875) as f32 - 90.0;
    e.y_rot = control::rotlerp(e.y_rot, yaw, 90.0);
    let attr = if e.on_ground { Attr::MovementSpeed } else { Attr::FlyingSpeed };
    let speed = (m.mov.speed_modifier * m.attrs.value(attr)) as f32;
    control::set_speed(m, speed);
    let sd = (xd * xd + zd * zd).sqrt();
    if yd.abs() > 1.0e-5f32 as f64 || sd.abs() > 1.0e-5f32 as f64 {
        let pitch = (-(mth::atan2(yd, sd) * 57.2957763671875)) as f32;
        e.set_x_rot(control::rotlerp(e.x_rot, pitch, max_turn));
        m.yya = if yd > 0.0 { speed } else { -speed };
    }
}

/// `LivingEntity.travelFlying(input, speed)`: 0.02 in water and lava, `speed` in the air.
pub fn travel(e: &mut Entity, level: &mut dyn EntityLevel, input: Vec3, speed: f32) {
    let (s, drag) = if e.is_in_water() {
        (0.02, 0.800000011920929)
    } else if e.is_in_lava() {
        (0.02, 0.5)
    } else {
        (speed, 0.9100000262260437)
    };
    super::move_relative(e, s, input);
    let d = e.delta;
    e.do_move(level, MoverType::SelfMove, d);
    e.delta = e.delta.scale(drag);
}

/// A flight target the move control is steered at every tick until it is reached (within a
/// block) or dropped: the stand-in for `FlyingPathNavigation.moveTo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirectFlight {
    pub target: Option<Vec3>,
    pub speed: f64,
}

impl DirectFlight {
    pub fn fly_to(&mut self, to: Vec3, speed: f64) {
        self.target = Some(to);
        self.speed = speed;
    }

    pub fn stop(&mut self) {
        self.target = None;
    }

    pub fn is_done(&self) -> bool {
        self.target.is_none()
    }

    /// Before the move control: the wanted position, or done once within a block.
    pub fn tick(&mut self, e: &Entity, m: &mut MobData) {
        let Some(t) = self.target else { return };
        if t.distance_to_sqr(e.position()) < 1.0 {
            self.target = None;
            return;
        }
        m.mov.set_wanted_position(t.x, t.y, t.z, self.speed);
    }
}
