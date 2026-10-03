//! Results of block scans an entity can reuse while nothing they read changed.
//!
//! A mob that stands still repeats the same scans every tick: its box, movement and the blocks
//! around it are the same as the tick before, so `Entity.collide`, `findSupportingBlock`,
//! `isInWall` and the inside-block walk give the same answers. Each result is kept with every
//! input it was computed from (the box, movement and collision context bit for bit, and
//! [`BlocksEpoch`], which changes whenever a block of the chunks the scan reads changes), and is
//! used again only when all of them are equal, so a hit is exactly what the scan would return.
//! A level that cannot give an epoch (`None`) never hits.

use crate::collision::CollisionContext;
use crate::level::{BlocksEpoch, EntityLevel};
use crate::math::{Aabb, BlockPos, Vec3, floor};

/// `KILN_MEMO_CHECK=1`: every reuse is checked against a fresh scan (tests and parity runs).
pub(crate) fn checking() -> bool {
    static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECK.get_or_init(|| std::env::var_os("KILN_MEMO_CHECK").is_some_and(|v| v != "0"))
}

/// `KILN_MEMO=0` turns the reuse off (to compare against a fresh scan every time).
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KILN_MEMO").is_none_or(|v| v != "0"))
}

/// Panics when a reused result differs from the fresh one.
pub(crate) fn verify<T: PartialEq + std::fmt::Debug>(what: &str, reused: &T, fresh: impl FnOnce() -> T) {
    if checking() {
        let fresh = fresh();
        assert_eq!(*reused, fresh, "stale {what} memo");
    }
}

pub(crate) type BoxBits = [u64; 6];
pub(crate) type VecBits = [u64; 3];

pub(crate) fn box_bits(b: &Aabb) -> BoxBits {
    [b.min_x.to_bits(), b.min_y.to_bits(), b.min_z.to_bits(), b.max_x.to_bits(), b.max_y.to_bits(), b.max_z.to_bits()]
}

pub(crate) fn vec_bits(v: Vec3) -> VecBits {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

/// Every field the collision shapes of a block can depend on.
pub(crate) fn ctx_bits(c: &CollisionContext) -> [u64; 3] {
    let flags = c.descending as u64
        | (c.placement as u64) << 1
        | (c.always_collide_with_fluid as u64) << 2
        | (c.has_entity as u64) << 3
        | (c.falling_block as u64) << 4
        | (c.walks_on_powder_snow as u64) << 5
        | (c.stands_on_lava as u64) << 6;
    [flags, c.entity_bottom.to_bits(), c.fall_distance.to_bits()]
}

/// The epoch of the chunks a scan of `area` reads: its blocks, one block of margin for the
/// shape tests and one more for safety.
pub(crate) fn area_epoch(level: &dyn EntityLevel, area: &Aabb) -> Option<BlocksEpoch> {
    if !enabled() {
        return None;
    }
    let lo = BlockPos::new(floor(area.min_x) - 2, floor(area.min_y) - 2, floor(area.min_z) - 2);
    let hi = BlockPos::new(floor(area.max_x) + 2, floor(area.max_y) + 2, floor(area.max_z) + 2);
    level.blocks_epoch(lo, hi)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollideKey {
    pub epoch: BlocksEpoch,
    pub bb: BoxBits,
    pub movement: VecBits,
    pub ctx: [u64; 3],
    pub max_up_step: u32,
    pub on_ground: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SupportKey {
    pub epoch: BlocksEpoch,
    pub bx: BoxBits,
    pub position: VecBits,
    pub ctx: [u64; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoxKey {
    pub epoch: BlocksEpoch,
    pub bx: BoxBits,
}

/// What the entity remembers between ticks; absent until its first scan.
#[derive(Clone, Debug, Default)]
pub(crate) struct BlockMemo {
    pub collide: Option<(CollideKey, Vec3)>,
    pub support: Option<(SupportKey, Option<BlockPos>)>,
    /// The blocks of the box (as the inside-block walk enumerates them) were all air.
    pub air: Option<BoxKey>,
    /// `isInWall` for the eye box: the answer.
    pub wall: Option<(BoxKey, bool)>,
}

#[cfg(test)]
mod tests {
    use crate::physics;

    /// `Section` counts fluids with `has_fluid`, and the fluid scan skips boxes by that count: the
    /// two must agree with the fluid states entities see, for every block state.
    #[test]
    fn air_by_state_ids_matches_the_physics_flag() {
        for s in 0..kiln_data::blocks::STATE_COUNT as u16 {
            assert_eq!(physics::is_air(s), crate::physics::entry_is_air(s), "state {s} ({})", crate::blocks::block_name(s));
        }
    }

    #[test]
    fn section_fluid_flag_matches_fluid_states() {
        for s in 0..kiln_data::blocks::STATE_COUNT as u16 {
            assert_eq!(
                kiln_data::blocks_types::has_fluid(s),
                !physics::fluid_state(s).is_empty(),
                "state {s} ({})",
                crate::blocks::block_name(s)
            );
        }
    }
}
