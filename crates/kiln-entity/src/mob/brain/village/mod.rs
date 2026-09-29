//! What the villagers' brain (`Villager.BRAIN_PROVIDER`, `VillagerGoalPackages`) is made of: the
//! sensors that look at beds, hostiles, babies and golems, the behaviours that claim and use
//! points of interest (`AcquirePoi`, `WorkAtPoi`, `SleepInBed` ...), the strolls around them, the
//! social ones (`InteractWith`, `TradeWithVillager`, `VillagerMakeLove`), the raid ones and the
//! panic ones.
//!
//! The pieces that read a villager's own data (profession, trades, inventory) reach it through
//! [`crate::mob::kinds::villager`].

pub mod poi;
pub mod raid;
pub mod sensors;
pub mod social;
pub mod stroll;
pub mod work;

use super::memory::{GlobalPos, Val};
use super::{Cx, Mem};
use crate::math::{BlockPos, Vec3};

/// The dimension a point of interest belongs to (Kiln's memories name the overworld; a level does
/// not tell its own name, so every memory counts as this level's).
pub const DIM: &str = super::persist::OVERWORLD;

/// `Vec3i.closerToCenterThan(pos, distance)`.
pub fn closer_to_center_than(p: BlockPos, pos: Vec3, distance: f64) -> bool {
    let (dx, dy, dz) = (p.x as f64 + 0.5 - pos.x, p.y as f64 + 0.5 - pos.y, p.z as f64 + 0.5 - pos.z);
    dx * dx + dy * dy + dz * dz < distance * distance
}

/// `GlobalPos.of(level.dimension(), pos)`.
pub fn gpos(pos: BlockPos) -> Val {
    Val::Pos(GlobalPos::new(DIM, pos))
}

/// `BlockPos.distSqr(other)`.
pub fn dist_sqr(a: BlockPos, b: BlockPos) -> f64 {
    super::util::dist_sqr_pos(a, b)
}

/// The position a memory holds (`GlobalPos.pos()`).
pub fn mem_pos(cx: &Cx, m: Mem) -> Option<BlockPos> {
    cx.b.mem.global_pos(m).map(|g| g.pos)
}
