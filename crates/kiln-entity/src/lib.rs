//! Vanilla 26.3 entity behaviour, independent of the simulation: movement physics
//! (`Entity.move` with vanilla's voxel-shape collision, step-up, fluids, block effects) and the
//! non-mob entities (items, experience orbs, falling blocks, primed TNT).
//!
//! The simulation implements [`EntityLevel`] over its world and ticks entities with
//! [`Entity::common_tick`] then [`Entity::tick`].

pub mod arrow;
pub mod blocks;
pub mod clip;
pub mod effect;
pub mod collision;
pub mod entity;
pub mod explosion;
pub mod ext_entity;
pub mod fall;
pub mod falling_block;
pub mod fluid;
pub mod inside;
pub mod item;
pub mod leash;
pub mod level;
pub mod math;
mod memo;
pub mod memory;
mod memory_poi;
pub mod mob;
pub mod persist;
pub mod physics;
pub mod player;
pub mod prof;
pub mod projectile;
pub mod ride;
pub mod shape;
pub mod tnt;
pub mod vibration;
pub mod xp_orb;

pub use entity::{Entity, EntityKind, MoverType, RemovalReason};
pub use level::{EntityFilter, EntityLevel, Event};
