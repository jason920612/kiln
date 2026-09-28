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

/// An entity as advancement criteria see it, taken when the event that fires them happened
/// (the entity may be gone, or not yet added, by the time the simulation looks).
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub id: i32,
    pub type_name: &'static str,
    pub pos: Vec3,
    pub on_ground: bool,
    pub on_fire: bool,
    pub baby: bool,
    /// Entity data components predicates can match exactly (the variant).
    pub components: Vec<kiln_item::Component>,
    /// A lightning bolt's `blocksSetOnFire`.
    pub lightning_fires: Option<i32>,
}

impl Seen {
    pub fn of(e: &Entity) -> Seen {
        let m = crate::mob::data(e);
        Seen {
            id: e.id,
            type_name: e.type_name,
            pos: e.position(),
            on_ground: e.on_ground,
            on_fire: e.is_on_fire(),
            baby: m.is_some_and(|m| m.baby()),
            components: m.map(crate::mob::variant_components).unwrap_or_default(),
            lightning_fires: None,
        }
    }

    /// A mob whose data is taken out for its tick.
    pub fn of_mob(e: &Entity, m: &crate::mob::MobData) -> Seen {
        Seen { baby: m.baby(), components: crate::mob::variant_components(m), ..Seen::of(e) }
    }
}

/// Criteria triggers about entities this crate simulates, for player `player` of
/// [`Event::Criterion`].
#[derive(Clone, Debug, PartialEq)]
pub enum Criterion {
    /// `BredAnimalsTrigger`: the parents and the baby.
    BredAnimals { parent: Seen, partner: Seen, child: Option<Seen> },
    /// `TameAnimalTrigger`.
    TameAnimal { animal: Seen },
    /// `SummonedEntityTrigger` (golems built, the dragon respawned).
    SummonedEntity { entity: Seen },
    /// `CuredZombieVillagerTrigger`.
    CuredZombieVillager { zombie: Seen, villager: Seen },
    /// `LightningStrikeTrigger`: the bolt and the entities it struck (bystanders).
    LightningStrike { lightning: Seen, victims: Vec<Seen>, blocks_set_on_fire: i32 },
    /// `ChanneledLightningTrigger`: the entities a channeling trident's bolt struck.
    ChanneledLightning { victims: Vec<Seen> },
    /// `KilledByArrowTrigger` (a crossbow's piercing arrow): the victims and the weapon.
    KilledByArrow { victims: Vec<Seen>, weapon: Option<kiln_item::ItemStack> },
    /// `TargetBlockTrigger`: a projectile hit a target block at `pos` for `signal`.
    TargetHit { projectile: Seen, pos: BlockPos, signal: i32 },
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
    /// The player's `tickCount`, the clock of its hurt timestamps (the simulation uses the
    /// game time for both).
    pub tick_count: i32,
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
    /// The amplifier of the player's Hero of the Village effect.
    pub hero_of_the_village: Option<i32>,
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
            tick_count: 0,
            last_hurt_by_mob_time: 0,
            last_hurt_mob: None,
            last_hurt_mob_time: 0,
            hurt_recently: false,
            vehicle: None,
            hero_of_the_village: None,
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
    /// `minecraft:lightning_bolt`.
    LightningBolt,
    // Slice 3 work packages add damage types below their own marker.
    // -- slice 3: mob effects
    /// `minecraft:wither` (the wither effect).
    Wither,

    // -- slice 3: raids
    /// `minecraft:starve` (a vex's limited life runs out).
    Starve,

    // -- slice 3: the end

    // -- slice 3: wither and guardians

    // -- slice 3: warden

    // -- slice 3: common mobs A

    // -- slice 3: common mobs B

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
    /// A mob died (`LivingEntity.die`): `credit` is the player the kill counts for
    /// (`getKillCredit` when it is a player: statistics, kill criteria, advancements).
    /// `equipment`: what it wore when it died (by slot name), before any of it dropped.
    Killed {
        entity: i32,
        entity_type: &'static str,
        credit: Option<i32>,
        kind: DamageKind,
        attacker: Option<i32>,
        direct: Option<i32>,
        equipment: Vec<(&'static str, kiln_item::ItemStack)>,
    },
    /// `dropFromGiftLootTable` (a chicken's egg).
    GiftLoot { entity: i32, table: &'static str, pos: Vec3 },
    /// A splash potion (`minecraft:` potion id) reached player `target` at `scale` of its full
    /// strength (`ThrownSplashPotion.onHitAsPotion`); `owner` threw it.
    PotionSplash { target: i32, potion: &'static str, scale: f64, owner: Option<i32> },
    /// `dropFromShearingLootTable` (a sheep's wool).
    ShearLoot { entity: i32, table: String, pos: Vec3 },
    /// A criteria trigger for player `player` (entity id).
    Criterion { player: i32, criterion: Criterion },
    /// A raider's news for its raid.
    Raid(RaidEvent),
    /// What the ender dragon and end crystals tell the level's dragon fight.
    DragonFight(DragonFightEvent),
}

/// The level's `EnderDragonFight` as its dragon and crystals see it
/// ([`EntityLevel::dragon_fight`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragonFightView {
    /// `dragonUUID`: the fight's dragon.
    pub dragon: Option<u128>,
    /// `aliveCrystals` (counted every 100 ticks).
    pub alive_crystals: i32,
    /// `hasPreviouslyKilledDragon`.
    pub previously_killed: bool,
    /// The fight's origin (`BlockPos.ZERO` in the End).
    pub origin: BlockPos,
}

/// Calls from the dragon and the crystals into `EnderDragonFight`.
#[derive(Clone, Debug, PartialEq)]
pub enum DragonFightEvent {
    /// `updateDragon`: the fight's dragon is alive with this health (the boss bar).
    Update { dragon: i32, uuid: u128, pos: Vec3, health: f32, max_health: f32 },
    /// `setDragonKilled`: the dragon finished dying (or was killed by `/kill`).
    Killed { dragon: i32, uuid: u128 },
    /// `globalLevelEvent(1028)`: the dragon's death roar for every player.
    DeathRoar { pos: BlockPos },
    /// `onCrystalDestroyed`: end crystal `crystal` at `pos` was destroyed by `kind` from
    /// `attacker`.
    CrystalDestroyed { crystal: i32, uuid: u128, pos: Vec3, kind: DamageKind, attacker: Option<i32> },
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

    /// `getHeightmapPos(MOTION_BLOCKING_NO_LEAVES or MOTION_BLOCKING, (x, z))`: the y above the
    /// highest motion blocking block of the column (`min_y` for an empty one).
    fn heightmap(&self, x: i32, z: i32, no_leaves: bool) -> i32 {
        let mut y = self.max_y();
        while y >= self.min_y() {
            let s = self.block(BlockPos::new(x, y, z));
            let blocks = if no_leaves { kiln_data::block_props::motion_blocking_no_leaves(s) } else { kiln_data::block_props::motion_blocking(s) };
            if blocks {
                return y + 1;
            }
            y -= 1;
        }
        self.min_y()
    }

    /// `ServerLevel.getDragonFight`: the level's ender dragon fight (the End's), if any.
    fn dragon_fight(&self) -> Option<DragonFightView> {
        None
    }

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

    /// `LightningBolt.spawnFire` at one position: fire (or soul fire) where the block is air
    /// and fire survives. Returns whether fire was placed.
    /// `ServerLevel.canSpreadFireAround`.
    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        let _ = pos;
        true
    }

    fn place_lightning_fire(&mut self, pos: BlockPos) -> bool {
        let _ = pos;
        false
    }

    /// `LightningBolt.powerLightningRod`: the block the bolt struck.
    fn lightning_strike_block(&mut self, pos: BlockPos) {
        let _ = pos;
    }

    /// `Entity.thunderHit` of player `id` (fire and 5 lightning damage).
    fn thunder_hit_player(&mut self, id: i32) {
        let _ = id;
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

    /// Players (for experience orbs); none by default. Borrowed: mobs ask for them several
    /// times a tick each, and a crowd server has a thousand.
    fn players(&self) -> &[PlayerView] {
        &[]
    }

    /// Player `id`, if it is one.
    fn player(&self, id: i32) -> Option<PlayerView> {
        self.players().iter().find(|p| p.id == id).copied()
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
    /// name; `source`: the entity responsible). Returns whether it took. The default reaches
    /// the level's mobs; implementations handle players first.
    fn add_effect(&mut self, id: i32, effect: &'static str, duration: i32, amplifier: i32, source: Option<i32>) -> bool {
        match crate::effect::Effect::named(effect, duration, amplifier) {
            Some(fx) => self.add_effect_instance(id, fx, source),
            None => false,
        }
    }

    /// `LivingEntity.addEffect` with a full instance (flags, hidden effects) on player or entity
    /// `id`. Implementations reach mobs through [`crate::mob::effects::add_to_entity`].
    fn add_effect_instance(&mut self, id: i32, effect: crate::effect::Effect, source: Option<i32>) -> bool {
        let _ = (id, effect, source);
        false
    }

    /// `MobEffect.applyInstantaneousEffect` on player or entity `id` (a splash potion's or a
    /// cloud's instant health or harm at `scale`; `source`: the potion or cloud and its
    /// position, `owner`: who threw it). Implementations reach mobs through
    /// [`crate::mob::effects::apply_instantaneous_to_entity`].
    fn apply_instantaneous_effect(&mut self, id: i32, effect: &crate::effect::Effect, source: Option<(i32, Vec3)>, owner: Option<i32>, scale: f64) {
        let _ = (id, effect, source, owner, scale);
    }

    /// `minecraft:max_entity_cramming`.
    fn max_entity_cramming(&self) -> i32 {
        24
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

    /// Raid `id` of the level (`Raids.get`), as the simulation last updated it; `None` once
    /// it stopped and was removed.
    fn raid(&self, id: i32) -> Option<&RaidView> {
        let _ = id;
        None
    }

    /// The `MOTION_BLOCKING_NO_LEAVES` heightmap at (`x`, `z`): the y above the column's topmost
    /// block that blocks motion or holds a fluid, leaves not counted.
    fn motion_blocking_no_leaves_height(&self, x: i32, z: i32) -> i32 {
        for y in (self.min_y()..=self.max_y()).rev() {
            let s = self.block(BlockPos::new(x, y, z));
            if kiln_data::block_props::motion_blocking(s) && !crate::blocks::block_name(s).ends_with("_leaves") {
                return y + 1;
            }
        }
        self.min_y()
    }

    /// `ServerLevel.getRaidAt`: the nearest active raid whose center is closer than 96 blocks.
    fn raid_at(&self, pos: BlockPos) -> Option<&RaidView> {
        let _ = pos;
        None
    }

    /// `ServerLevel.sectionsToVillage` (`PoiManager`'s distance tracker: sections from the
    /// nearest section with an occupied village point of interest, 7 when farther than 6).
    fn sections_to_village(&self, pos: BlockPos) -> i32 {
        let _ = pos;
        7
    }

    /// `ServerLevel.isVillage`.
    fn is_village(&self, pos: BlockPos) -> bool {
        self.sections_to_village(pos) <= 1
    }

    /// `PoiManager.getInRange(...).map(getPos)`: points of interest of the `minecraft:point_of_interest_type`
    /// entries in `types` within `radius` of `center` passing `occupancy`, in storage order.
    fn poi_in_range(&self, types: &[&str], center: BlockPos, radius: i32, occupancy: PoiOccupancy) -> Vec<BlockPos> {
        let _ = (types, center, radius, occupancy);
        Vec::new()
    }

    /// `PoiManager.take`: claims a ticket of the first point of interest of `types` with space
    /// within `radius` of `center` that `accept` takes.
    fn poi_take(&mut self, types: &[&str], center: BlockPos, radius: i32, accept: &dyn Fn(&str, BlockPos) -> bool) -> Option<BlockPos> {
        let _ = (types, center, radius, accept);
        None
    }

    /// `PoiManager.release`: gives a ticket back.
    fn poi_release(&mut self, pos: BlockPos) {
        let _ = pos;
    }

    /// `PoiManager.getType`: the point of interest type at `pos`.
    fn poi_type(&self, pos: BlockPos) -> Option<&'static str> {
        let _ = pos;
        None
    }
}

/// `PoiManager.Occupancy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoiOccupancy {
    HasSpace,
    IsOccupied,
    Any,
}

/// A raid (`Raid`) as entity behaviour sees it; the simulation refreshes these every tick.
#[derive(Clone, Debug, PartialEq)]
pub struct RaidView {
    pub id: i32,
    pub center: BlockPos,
    pub active: bool,
    /// `isOver` (victory or loss), `isLoss`, `isStarted`.
    pub over: bool,
    pub loss: bool,
    pub started: bool,
    pub groups_spawned: i32,
    pub omen_level: i32,
    /// `groupToLeaderMap`: (wave, entity id).
    pub leaders: Vec<(i32, i32)>,
}

impl RaidView {
    /// `getLeader(wave)`.
    pub fn leader(&self, wave: i32) -> Option<i32> {
        self.leaders.iter().find(|(w, _)| *w == wave).map(|(_, id)| *id)
    }
}

/// What raiders tell their raid (the simulation's `Raid` bookkeeping).
#[derive(Clone, Debug, PartialEq)]
pub enum RaidEvent {
    /// `Raid.joinRaid(level, wave, raider, null, true)`: an existing raider joined.
    Joined { raid: i32, entity: i32, wave: i32 },
    /// `Raider.die`: the raid loses the raider (and its leader for the wave); a player
    /// killer becomes a hero of the village.
    Died { raid: i32, entity: i32, wave: i32, leader: bool, hero: Option<i32> },
    /// `Raid.setLeader`: the raider picked up the ominous banner and leads its wave.
    Leader { raid: i32, wave: i32, entity: i32 },
}

/// The merchant a trade set is rolled for (the loot context's `this` entity and origin).
#[derive(Clone, Copy, Debug)]
pub struct TradeMerchant {
    pub entity: i32,
    pub pos: Vec3,
    /// The villager's `minecraft:villager_type` (for type-restricted trades).
    pub villager_type: &'static str,
}
