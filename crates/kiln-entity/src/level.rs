//! What entity behaviour needs from the world: the simulation implements [`EntityLevel`] over
//! a region, the tests over a small in-memory world.

use crate::entity::Entity;
use crate::math::{Aabb, BlockPos, Vec3};
use kiln_javamath::random::LegacyRandom;

/// Which entities a query wants (vanilla's `getEntitiesOfClass` class argument).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityFilter {
    Any,
    Item,
    ExperienceOrb,
    /// Entities that are alive and can be hurt by a falling block or pushed by explosions.
    Living,
}

/// A player as the experience orb sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub id: i32,
    pub pos: Vec3,
    pub eye_height: f32,
    pub spectator: bool,
}

/// Why an entity took damage (the vanilla damage type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind {
    OnFire,
    InFire,
    Lava,
    FallingBlock,
    FallingAnvil,
    FallingStalactite,
    Explosion,
    Cactus,
    SweetBerryBush,
    HotFloor,
    Freeze,
    Arrow,
    Thrown,
    Generic,
}

/// Side effects the simulation carries out or broadcasts.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A sound at a position (`minecraft:` sound event id, source category name).
    Sound { pos: Vec3, sound: &'static str, source: &'static str, volume: f32, pitch: f32 },
    /// `Level.levelEvent` (block break particles 2001, fizz 1501, ...).
    LevelEvent { event: i32, pos: BlockPos, data: i32 },
    /// A game event for vibrations (`minecraft:hit_ground`, `minecraft:entity_place`, ...).
    GameEvent { event: &'static str, pos: Vec3, entity: Option<i32> },
    /// Damage to an entity this crate does not simulate (mobs, players).
    Hurt { target: i32, amount: f32, kind: DamageKind, attacker: Option<i32> },
    /// `Level.broadcastEntityEvent`.
    EntityEvent { entity: i32, event: u8 },
    /// An explosion at `pos`; `blocks` were destroyed (for the explode packet).
    Explosion { pos: Vec3, power: f32, blocks: Vec<BlockPos>, source: Option<i32> },
}

/// World access for entity ticks.
///
/// The entity being ticked is not reachable through `entity_mut` (the caller holds it); every
/// other entity is.
pub trait EntityLevel {
    /// Block state id at `pos`; air outside loaded chunks.
    fn block(&self, pos: BlockPos) -> u16;

    /// Whether the chunk holding `pos` is loaded (collisions skip unloaded chunks).
    fn is_loaded(&self, pos: BlockPos) -> bool {
        let _ = pos;
        true
    }

    /// Sets a block with vanilla update `flags` (`Block.UPDATE_*`); false if nothing changed.
    fn set_block(&mut self, pos: BlockPos, state: u16, flags: u32) -> bool;

    /// `Level.destroyBlock(pos, drop)`: breaks the block with particles and drops.
    fn destroy_block(&mut self, pos: BlockPos, drop: bool) -> bool {
        let _ = drop;
        self.set_block(pos, 0, 3)
    }

    /// The level's shared random source (`Level.random`).
    fn random(&mut self) -> &mut LegacyRandom;

    fn game_time(&self) -> i64;

    /// Lowest block y of the dimension.
    fn min_y(&self) -> i32;

    fn is_raining_at(&self, pos: BlockPos) -> bool {
        let _ = pos;
        false
    }

    /// The `minecraft:fast_lava` environment attribute (true in the nether).
    fn fast_lava(&self) -> bool {
        false
    }

    /// Whether `minecraft:mob_griefing` is on.
    fn mob_griefing(&self) -> bool {
        true
    }

    /// Bounding boxes of entities `entity` collides with (boats, shulkers, ...), in `area`
    /// (vanilla's `getEntityCollisions` without the size check and inflation, done here).
    fn entity_collision_boxes(&self, entity: i32, area: &Aabb) -> Vec<Aabb> {
        let _ = (entity, area);
        Vec::new()
    }

    /// Ids of entities whose bounding box intersects `area`, excluding `exclude`, in vanilla's
    /// iteration order (entity sections in order, then insertion order within a section).
    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32>;

    fn entity_mut(&mut self, id: i32) -> Option<&mut Entity>;

    fn entity(&self, id: i32) -> Option<&Entity>;

    /// Adds a new entity (vanilla `addFreshEntity`); it ticks from the next tick on.
    fn add_entity(&mut self, entity: Entity);

    /// A fresh network id for a new entity.
    fn next_entity_id(&mut self) -> i32;

    /// Players (for experience orbs); empty by default.
    fn players(&self) -> Vec<PlayerView> {
        Vec::new()
    }

    fn emit(&mut self, event: Event);
}
