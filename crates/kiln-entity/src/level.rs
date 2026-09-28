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
    /// The player's UUID (tamed animals remember their owner by it).
    pub uuid: u128,
    pub pos: Vec3,
    pub eye_height: f32,
    pub spectator: bool,
    pub creative: bool,
    pub sneaking: bool,
    /// Alive (not dead and waiting to respawn).
    pub alive: bool,
    pub invisible: bool,
    /// `getArmorCoverPercentage`.
    pub armor_cover: f32,
    /// Item ids in the hands (`minecraft:item` protocol ids, 0 for none).
    pub main_hand: i32,
    pub off_hand: i32,
    /// Wears a piece of `#minecraft:piglin_safe_armor` (piglins leave the player alone).
    pub piglin_safe_armor: bool,
    /// `isInWater` when known (`None`: from the blocks around the player, as its own tick
    /// would find).
    pub in_water: Option<bool>,
    /// The item id on the head (`EquipmentSlot.HEAD`; 0 for none).
    pub head: i32,
    /// Rotations: `yHeadRot` (a player's head turns with its body) and `xRot`, in degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    /// Active effects: bit `id` for `minecraft:mob_effect` network id `id` (below 64).
    pub effects: u64,
    /// `getLastHurtByMob` and `getLastHurtByMobTimestamp` (tamed animals defend their owner).
    pub last_hurt_by_mob: Option<i32>,
    pub last_hurt_by_mob_time: i32,
    /// `getLastHurtMob` and `getLastHurtMobTimestamp` (tamed animals join their owner's fight).
    pub last_hurt_mob: Option<i32>,
    pub last_hurt_mob_time: i32,
    /// `getLastDamageSource(100)` is set and not in `no_wolf_retaliation`.
    pub hurt_recently: bool,
    /// The entity the player rides.
    pub vehicle: Option<i32>,
}

impl PlayerView {
    /// A standing survival player with empty hands.
    pub fn new(id: i32, pos: Vec3) -> PlayerView {
        PlayerView {
            id,
            uuid: 0,
            pos,
            eye_height: 1.62,
            spectator: false,
            creative: false,
            sneaking: false,
            alive: true,
            invisible: false,
            armor_cover: 0.0,
            main_hand: 0,
            off_hand: 0,
            piglin_safe_armor: false,
            in_water: None,
            head: 0,
            yaw: 0.0,
            pitch: 0.0,
            health: 20.0,
            effects: 0,
            last_hurt_by_mob: None,
            last_hurt_by_mob_time: 0,
            last_hurt_mob: None,
            last_hurt_mob_time: 0,
            hurt_recently: false,
            vehicle: None,
        }
    }

    /// `hasEffect` for a `minecraft:` effect id.
    pub fn has_effect(&self, effect: &str) -> bool {
        kiln_data::builtin_id("minecraft:mob_effect", effect).is_some_and(|id| (0..64).contains(&id) && self.effects & (1 << id) != 0)
    }
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
    MobAttack,
    PlayerAttack,
    Drown,
    InWall,
    OutOfWorld,
    Fall,
    Kill,
    Cramming,
    PlayerExplosion,
    /// A ghast's or blaze's fireball (`DamageSources.fireball`).
    Fireball,
    /// `minecraft:trident` (a thrown trident).
    Trident,
    /// `mobProjectile` (shulker bullets, llama spit).
    MobProjectile,
    Magic,
    IndirectMagic,
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
    /// A block an explosion destroyed: the simulation drops its loot (`decay`: the
    /// `explosion_radius` loot parameter applies) before this crate sets it to air.
    BlockExploded { pos: BlockPos, state: u16, decay: bool, source: Option<i32> },
    /// Vanilla block side effects of an entity inside a block that this crate does not
    /// simulate (hoppers, pressure plates, tripwires, portals, detector rails).
    EntityInsideBlock { pos: BlockPos, state: u16, entity: i32 },
    /// A projectile hit a block (`Block.onProjectileHit`) or an entity: damage, egg hatching,
    /// pearl teleports and potion splashes are the simulation's.
    ProjectileHit { projectile: i32, projectile_type: &'static str, owner: Option<i32>, hit: crate::projectile::Hit },
    /// An explosion at `pos`; `blocks` were destroyed (for the explode packet).
    Explosion { pos: Vec3, power: f32, blocks: Vec<BlockPos>, source: Option<i32> },
    /// A mob took a full hit (`broadcastDamageEvent`: the hurt animation for viewers).
    MobHurt { entity: i32, kind: DamageKind, attacker: Option<i32>, direct: Option<i32> },
    /// A mob died: the simulation drops its loot table (`LivingEntity.dropFromLootTable`).
    /// `killer`: the player credited with the kill (`last_damage_player`).
    DeathLoot {
        entity: i32,
        table: String,
        pos: Vec3,
        killer: Option<i32>,
        attacker: Option<i32>,
        direct: Option<i32>,
        kind: DamageKind,
        on_fire: bool,
    },
    /// `dropFromGiftLootTable` (a chicken's egg).
    GiftLoot { entity: i32, table: &'static str, pos: Vec3 },
    /// A splash potion (`minecraft:` potion id) reached player `target` at `scale` of its full
    /// strength (`ThrownSplashPotion.onHitAsPotion`); `owner` threw it.
    PotionSplash { target: i32, potion: &'static str, scale: f64, owner: Option<i32> },
    /// `dropFromShearingLootTable` (a sheep's wool).
    ShearLoot { entity: i32, table: String, pos: Vec3 },
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

    /// `Level.getSeaLevel` (63 in the overworld, 32 in the nether, -63 in a superflat world).
    fn sea_level(&self) -> i32 {
        63
    }

    /// Highest block y of the dimension.
    fn max_y(&self) -> i32 {
        self.min_y() + 383
    }

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

    /// A seed for a new entity's own random source (vanilla seeds each from a global
    /// uniquifier and the clock).
    fn fresh_seed(&mut self) -> i64;

    /// Players (for experience orbs); empty by default.
    fn players(&self) -> Vec<PlayerView> {
        Vec::new()
    }

    /// Player `id`, if it is one.
    fn player(&self, id: i32) -> Option<PlayerView> {
        self.players().into_iter().find(|p| p.id == id)
    }

    fn emit(&mut self, event: Event);

    /// `getRawBrightness(pos, skyDarken)`: the larger of the sky light less `sky_darken` and
    /// the block light.
    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        let _ = pos;
        15 - sky_darken
    }

    /// The sky light at `pos`.
    fn sky_light(&self, pos: BlockPos) -> i32 {
        let _ = pos;
        15
    }

    /// `Level.getSkyDarken`.
    fn sky_darken(&self) -> i32 {
        0
    }

    /// `DimensionType.ambientLight` (0 in the overworld).
    fn ambient_light(&self) -> f32 {
        0.0
    }

    /// `canSeeSky`: full sky light.
    fn can_see_sky(&self, pos: BlockPos) -> bool {
        self.sky_light(pos) >= 15
    }

    /// `Level.isBrightOutside`.
    fn is_bright_outside(&self) -> bool {
        self.sky_darken() < 4
    }

    /// 0 peaceful to 3 hard.
    fn difficulty(&self) -> u8 {
        2
    }

    /// `DifficultyInstance.getEffectiveDifficulty` at `pos`.
    fn effective_difficulty(&self, pos: BlockPos) -> f32 {
        let _ = pos;
        1.5
    }

    /// The `minecraft:monsters_burn` environment attribute.
    fn monsters_burn(&self) -> bool {
        true
    }

    /// `minecraft:mob_drops`.
    fn mob_drops(&self) -> bool {
        true
    }

    /// A mob hits player `id` (`Player.hurtServer`); returns whether the hit landed.
    fn hurt_player(&mut self, id: i32, source: crate::mob::DamageSource, amount: f32) -> bool {
        self.emit(Event::Hurt { target: id, amount, kind: source.kind, attacker: source.attacker });
        true
    }

    /// `LivingEntity.addEffect` on player or entity `id` (`effect`: a `minecraft:mob_effect`
    /// name; `source`: the entity responsible). Returns whether it took (mobs have no effects
    /// yet).
    fn add_effect(&mut self, id: i32, effect: &'static str, duration: i32, amplifier: i32, source: Option<i32>) -> bool {
        let _ = (id, effect, duration, amplifier, source);
        false
    }

    /// Sets entity or player `id` on fire for `seconds`.
    fn ignite(&mut self, id: i32, seconds: f32) {
        if let Some(e) = self.entity_mut(id) {
            e.ignite_for_seconds(seconds);
        }
    }

    /// The `minecraft:gameplay/piglins_zombify` environment attribute (false in the nether).
    fn piglins_zombify(&self) -> bool {
        !self.fast_lava()
    }

    /// `AbstractVillager.addOffersFromTradeSet`: the offers the datapack trade set `set` (a
    /// `minecraft:trade_set` id) rolls for `merchant`; none without trade data.
    fn trade_offers(&mut self, set: &str, merchant: &TradeMerchant) -> Vec<kiln_item::trading::MerchantOffer> {
        let _ = (set, merchant);
        Vec::new()
    }
}

/// The merchant a trade set is rolled for (the loot context's `this` entity and origin).
#[derive(Clone, Copy, Debug)]
pub struct TradeMerchant {
    pub entity: i32,
    pub pos: Vec3,
    /// The villager's `minecraft:villager_type` (for type-restricted trades).
    pub villager_type: &'static str,
}
