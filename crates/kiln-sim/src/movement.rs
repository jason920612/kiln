//! Player movement checks from vanilla `ServerGamePacketListenerImpl.handleMovePlayer`.
//!
//! Implemented: invalid values, the "moved too quickly" distance check and the "colliding
//! with anything new" block check. The "moved wrongly" check needs the player's collision
//! physics (`Entity.move`) and arrives with entity physics.

use kiln_world::World;

/// Ticks a client has to report that its world finished loading before movement counts anyway.
pub(crate) const CLIENT_LOADED_TIMEOUT: u32 = 60;
/// Unacknowledged teleports are resent after this many ticks.
pub(crate) const TELEPORT_RESEND_TICKS: i64 = 20;

const PLAYER_WIDTH: f64 = 0.6;
const STANDING_HEIGHT: f64 = 1.8;
const CROUCHING_HEIGHT: f64 = 1.5;
const EPSILON: f64 = 1.0e-5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    /// The player's box standing (or crouching) at `pos` (bottom centre).
    pub fn player(pos: [f64; 3], crouching: bool) -> Self {
        let h = if crouching { CROUCHING_HEIGHT } else { STANDING_HEIGHT };
        let r = PLAYER_WIDTH / 2.0;
        Self { min: [pos[0] - r, pos[1], pos[2] - r], max: [pos[0] + r, pos[1] + h, pos[2] + r] }
    }

    fn deflate(self, d: f64) -> Self {
        Self { min: self.min.map(|v| v + d), max: self.max.map(|v| v - d) }
    }

    fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|i| self.min[i] < o.max[i] && self.max[i] > o.min[i])
    }
}

/// `containsInvalidValues`: NaN coordinates or non-finite angles disconnect the player.
pub(crate) fn invalid(pos: Option<[f64; 3]>, rot: Option<[f32; 2]>) -> bool {
    pos.is_some_and(|p| p.iter().any(|c| c.is_nan())) || rot.is_some_and(|r| r.iter().any(|a| !a.is_finite()))
}

/// Vanilla's coordinate limits for accepted positions.
pub(crate) fn clamp_position(p: [f64; 3]) -> [f64; 3] {
    [p[0].clamp(-3.0e7, 3.0e7), p[1].clamp(-2.0e7, 2.0e7), p[2].clamp(-3.0e7, 3.0e7)]
}

/// Yaw wrapped to [-180, 180), pitch clamped to [-90, 90] (`Entity.absSnapTo`).
pub(crate) fn normalize_rotation(rot: [f32; 2]) -> [f32; 2] {
    [wrap_degrees(rot[0]) % 360.0, wrap_degrees(rot[1]).clamp(-90.0, 90.0) % 360.0]
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(a: f32) -> f32 {
    let mut w = a % 360.0;
    if w >= 180.0 {
        w -= 360.0;
    }
    if w < -180.0 {
        w += 360.0;
    }
    w
}

/// The "moved too quickly" check: squared distance from where the player started the tick,
/// minus its squared velocity, against 100 (300 when gliding) blocks² per move packet this tick.
pub(crate) fn too_fast(first_good: [f64; 3], to: [f64; 3], velocity_sqr: f64, packets: u32, gliding: bool) -> bool {
    // More than 5 packets in a tick counts as one (vanilla logs it and carries on).
    let packets = if packets > 5 { 1 } else { packets };
    let limit = if gliding { 300.0f32 } else { 100.0f32 };
    let d: f64 = (0..3).map(|i| (to[i] - first_good[i]).powi(2)).sum();
    d - velocity_sqr > (limit * packets as f32) as f64
}

/// `isEntityCollidingWithAnythingNew`: whether the box at the new position overlaps a block
/// collision shape that the old box did not already overlap.
pub(crate) fn collides_with_anything_new(world: &World, old: Aabb, new: Aabb) -> bool {
    let new = new.deflate(EPSILON);
    let old = old.deflate(EPSILON);
    // One block of margin: shapes such as fences and walls reach outside their block.
    let lo = new.min.map(|v| (v - 1.0e-7).floor() as i32 - 1);
    let hi = new.max.map(|v| (v + 1.0e-7).floor() as i32 + 1);
    for x in lo[0]..=hi[0] {
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                let Some(state) = world.get_block(x, y, z) else { continue };
                let boxes = kiln_data::block_props::collision(state);
                if boxes.is_empty() {
                    continue;
                }
                let placed = |b: &[f32; 6]| Aabb {
                    min: [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64],
                    max: [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64],
                };
                let hits_new = boxes.iter().any(|b| placed(b).intersects(&new));
                if hits_new && !boxes.iter().any(|b| placed(b).intersects(&old)) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as block;
    use kiln_world::OVERWORLD;

    fn world() -> World {
        World::flat(OVERWORLD, 0, 67)
    }

    #[test]
    fn speed_limit_is_ten_blocks_per_packet() {
        let o = [0.0, 0.0, 0.0];
        assert!(!too_fast(o, [9.9, 0.0, 0.0], 0.0, 1, false));
        assert!(too_fast(o, [10.1, 0.0, 0.0], 0.0, 1, false));
        assert!(!too_fast(o, [14.0, 0.0, 0.0], 0.0, 2, false));
        assert!(!too_fast(o, [17.0, 0.0, 0.0], 0.0, 1, true));
        // A burst of more than five packets gets the one-packet allowance.
        assert!(too_fast(o, [11.0, 0.0, 0.0], 0.0, 6, false));
    }

    #[test]
    fn walking_on_flat_ground_is_fine_but_entering_a_wall_is_not() {
        let mut w = world();
        let y = w.flat_surface_y();
        let from = [0.5, y, 0.5];
        let to = [0.8, y, 0.5];
        assert!(!collides_with_anything_new(&w, Aabb::player(from, false), Aabb::player(to, false)));
        w.set_block(1, y as i32, 0, block::STONE);
        assert!(collides_with_anything_new(&w, Aabb::player(from, false), Aabb::player(to, false)));
        // Standing against the wall already, moving along it is not new.
        let from = [0.7, y, 0.5];
        assert!(!collides_with_anything_new(&w, Aabb::player(from, false), Aabb::player([0.7, y, 0.4], false)));
    }

    #[test]
    fn fences_reach_above_their_block() {
        let mut w = world();
        let y = w.flat_surface_y();
        w.set_block(0, y as i32, 0, block::OAK_FENCE);
        // Standing on top of the block space but inside the fence's 1.5-high post.
        let from = [0.5, y + 3.0, 0.5];
        assert!(collides_with_anything_new(&w, Aabb::player(from, false), Aabb::player([0.5, y + 1.2, 0.5], false)));
        assert!(!collides_with_anything_new(&w, Aabb::player(from, false), Aabb::player([0.5, y + 1.5, 0.5], false)));
    }

    #[test]
    fn rotation_is_normalized_like_vanilla() {
        assert_eq!(normalize_rotation([190.0, 100.0]), [-170.0, 90.0]);
        assert_eq!(normalize_rotation([-540.0, -10.0]), [-180.0, -10.0]);
    }
}
