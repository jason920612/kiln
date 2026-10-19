//! Mobs in the simulation (kiln-entity has their behaviour): the world state they read (time
//! of day, game rules), what clients see of them (entity data, equipment), their loot, and
//! creating them (`/summon`, spawn eggs, natural spawning in [`crate::spawner`]).

use crate::entities::{Body, Spawn};
use kiln_data::entities::data;
use kiln_entity::mob::{self, MobData, MobKind, Species};
use kiln_proto::packets::entity::{DataValue, EntityData};

/// World state and game rules mobs depend on (part of the block environment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MobRules {
    /// Time of day of the overworld clock.
    pub day_time: i64,
    /// `Level.skyDarken` (0 at noon, 11 at midnight).
    pub sky_darken: i32,
    /// The `minecraft:monsters_burn` timeline value.
    pub monsters_burn: bool,
    /// The `minecraft:gameplay/creaking_active` value (the overworld's night).
    pub creaking_active: bool,
    pub griefing: bool,
    pub drops: bool,
    /// `minecraft:entity_drops` (vehicles and other non-mob entities drop their items).
    pub entity_drops: bool,
    pub spawn_mobs: bool,
    pub spawn_monsters: bool,
    /// `minecraft:spawn_wardens`.
    pub spawn_wardens: bool,
    /// `minecraft:spawn_phantoms`.
    pub spawn_phantoms: bool,
    /// `minecraft:universal_anger` and `minecraft:forgive_dead_players`.
    pub universal_anger: bool,
    pub forgive_dead_players: bool,
    /// `minecraft:ender_pearls_vanish_on_death`.
    pub ender_pearls_vanish: bool,
    /// `block_explosion_drop_decay`, `mob_explosion_drop_decay` and `tnt_explosion_drop_decay`.
    pub explosion_decay: [bool; 3],
    /// `minecraft:global_sound_events`.
    pub global_sound_events: bool,
    /// `minecraft:projectiles_can_break_blocks`.
    pub projectiles_break_blocks: bool,
    /// `minecraft:spawner_blocks_work`.
    pub spawner_blocks: bool,
    pub cramming: i32,
    pub difficulty: u8,
    /// The world spawn (no natural spawns within 24 blocks).
    pub spawn_point: [i32; 3],
    /// `minecraft:spectators_generate_chunks`: whether a spectator counts for the spawn range (`ChunkMap.skipPlayer`).
    pub spectators_generate_chunks: bool,
}

impl Default for MobRules {
    fn default() -> Self {
        MobRules {
            day_time: 1000,
            sky_darken: 0,
            monsters_burn: true,
            creaking_active: false,
            griefing: true,
            drops: true,
            entity_drops: true,
            spawn_mobs: true,
            spawn_monsters: true,
            spawn_wardens: true,
            spawn_phantoms: true,
            universal_anger: false,
            forgive_dead_players: true,
            ender_pearls_vanish: true,
            explosion_decay: [true, true, false],
            global_sound_events: true,
            projectiles_break_blocks: true,
            spawner_blocks: true,
            cramming: 24,
            difficulty: 2,
            spawn_point: [0, 64, 0],
            spectators_generate_chunks: true,
        }
    }
}

/// The `minecraft:gameplay/monsters_burn` timeline: off from 12542 to 23460.
pub(crate) fn monsters_burn(day_time: i64) -> bool {
    let t = day_time.rem_euclid(24000);
    !(12542..23460).contains(&t)
}

/// The `minecraft:gameplay/creaking_active` timeline: on from 12600 to 23401.
pub(crate) fn creaking_active(day_time: i64) -> bool {
    (12600..23401).contains(&day_time.rem_euclid(24000))
}

/// Entity data of a mob for its viewers.
pub(crate) fn metadata(e: &kiln_entity::Entity, m: &MobData) -> EntityData {
    let mut d = EntityData::new();
    // Shared flags: on fire, invisible (5) and glowing (6) from the effects.
    let effects = kiln_entity::mob::effects::invisible(m) as i8;
    let flags = (e.is_on_fire() as i8) | (effects << 5) | ((kiln_entity::mob::effects::glowing(m) as i8) << 6);
    d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(flags));
    if e.air_supply != 300 {
        d.set(data::entity::AIR_SUPPLY, &DataValue::Int(e.air_supply));
    }
    // `DATA_CUSTOM_NAME`, `DATA_CUSTOM_NAME_VISIBLE`, `DATA_SILENT`, `DATA_NO_GRAVITY` (name tags, `/summon`).
    if let Some((_, name)) = e.extra.iter().find(|(k, _)| k == "CustomName") {
        d.set(data::entity::CUSTOM_NAME, &DataValue::OptionalComponent(Some(name.clone())));
    }
    if e.extra.iter().any(|(k, v)| k == "CustomNameVisible" && v.as_i64().is_some_and(|b| b != 0)) {
        d.set(data::entity::CUSTOM_NAME_VISIBLE, &DataValue::Boolean(true));
    }
    if e.silent {
        d.set(data::entity::SILENT, &DataValue::Boolean(true));
    }
    if e.no_gravity {
        d.set(data::entity::NO_GRAVITY, &DataValue::Boolean(true));
    }
    if m.is_dead_or_dying() {
        d.set(data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::DYING));
    }
    // `DATA_EFFECT_PARTICLES` (always set: viewers get the empty list when the effects end)
    // and `DATA_EFFECT_AMBIENCE_ID`.
    let (particles, ambient) = kiln_entity::mob::effects::particles(m);
    d.set(data::living_entity::EFFECT_PARTICLES, &DataValue::Particles(particles));
    if ambient {
        d.set(data::living_entity::EFFECT_AMBIENCE, &DataValue::Boolean(true));
    }
    d.set(data::living_entity::HEALTH, &DataValue::Float(m.health));
    // `DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID`: the viewers get the count while it is above 0 (and 0 when it falls back).
    if m.arrow_count > 0 {
        d.set(data::living_entity::ARROW_COUNT, &DataValue::Int(m.arrow_count));
    }
    if m.stinger_count > 0 {
        d.set(data::living_entity::STINGER_COUNT, &DataValue::Int(m.stinger_count));
    }
    // `Mob.DATA_MOB_FLAGS_ID` (15): a mannequin is no `Mob`, and its field 15 is the `Avatar`'s main hand.
    if m.kind.is_mob() {
        let mob_flags = (m.no_ai as i8) | ((m.left_handed as i8) << 1) | ((m.aggressive as i8) << 2);
        d.set(data::mob::MOB_FLAGS, &DataValue::Byte(mob_flags));
    }
    if kiln_entity::mob::breed::is_ageable(m.kind) && m.age < 0 {
        d.set(data::ageable_mob::BABY, &DataValue::Boolean(true));
    }
    // `AgeableMob.AGE_LOCKED` (a tadpole has its own, see its entity data).
    if m.age_locked && m.kind != MobKind::Tadpole && kiln_entity::mob::breed::is_ageable(m.kind) {
        d.set(data::ageable_mob::AGE_LOCKED, &DataValue::Boolean(true));
    }
    match &m.species {
        Species::Cow | Species::Chicken { .. } => {
            let (variant, sound) = match m.kind {
                MobKind::Cow => (data::cow::VARIANT, data::cow::SOUND_VARIANT),
                _ => (data::chicken::VARIANT, data::chicken::SOUND_VARIANT),
            };
            d.set(variant, &DataValue::Holder(m.variant));
            d.set(sound, &DataValue::Holder(m.sound_variant));
        }
        Species::Sheep { color, sheared } => {
            d.set(data::sheep::WOOL, &DataValue::Byte(((*color & 15) | if *sheared { 16 } else { 0 }) as i8));
        }
        Species::Zombie { .. } => {
            if m.zombie_baby {
                d.set(data::zombie::BABY, &DataValue::Boolean(true));
            }
        }
        Species::Creeper { swell_dir, powered, ignited, .. } => {
            d.set(data::creeper::SWELL_DIR, &DataValue::Int(*swell_dir));
            d.set(data::creeper::IS_POWERED, &DataValue::Boolean(*powered));
            d.set(data::creeper::IS_IGNITED, &DataValue::Boolean(*ignited));
        }
        Species::Spider { climbing } => {
            d.set(data::spider::FLAGS, &DataValue::Byte(*climbing as i8));
        }
        Species::Skeleton { .. } | Species::Plain | Species::Ext(_) => {}
    }
    if let Some(k) = m.kind.ext() {
        k.entity_data(e, m, &mut d);
    }
    d
}

/// Equipment worth showing (main hand, off hand, armor), as (slot ordinal, stack).
pub(crate) fn shown_equipment(m: &MobData) -> Vec<(u8, kiln_item::ItemStack)> {
    let mut v: Vec<(u8, kiln_item::ItemStack)> = m.equipment.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| (i as u8, s.clone())).collect();
    if let Some(k) = m.kind.ext() {
        v.extend(k.extra_equipment(m));
    }
    v
}

/// `MobBucketItem.checkExtraContent`: the fish of a mob bucket (`cod_bucket`, ...) emptied at
/// block `pos` (`EntitySpawnReason.BUCKET`: `finalizeSpawn`, then the bucket's components and
/// `bucket_entity_data`, `FromBucket`). `seed` seeds the spawn's random draws. `None` for other
/// items.
#[allow(dead_code)]
pub(crate) fn bucket_release(bucket: &kiln_item::ItemStack, pos: [i32; 3], difficulty: u8, game_time: i64, seed: i64) -> Option<Spawn> {
    let kind = kiln_entity::mob::kinds::fish::bucket_mob(kiln_entity::mob::item_name(bucket))?;
    let mut e = kiln_entity::mob::new(kind, 0, 0, seed);
    let at = kiln_entity::math::Vec3::new(pos[0] as f64 + 0.5, pos[1] as f64, pos[2] as f64 + 0.5);
    e.set_pos(at);
    e.set_old_pos_and_rot();
    let ctx = difficulty_instance(difficulty, game_time, 0, 1.0);
    let mut r = kiln_javamath::random::LegacyRandom::new(seed);
    mob::finalize_spawn(&mut e, &mut r, &ctx, &mut mob::GroupData::default(), false);
    if kind == MobKind::Tadpole {
        kiln_entity::mob::kinds::tadpole::apply_bucket(&mut e, bucket);
    } else if kind == MobKind::SulfurCube {
        kiln_entity::mob::kinds::sulfur_cube::apply_bucket(&mut e, bucket);
    } else {
        kiln_entity::mob::kinds::fish::apply_bucket(&mut e, bucket);
    }
    let t = kiln_data::entities::by_name(kind.type_name())?;
    Some(Spawn { kind: t, pos: [at.x, at.y, at.z], vel: [0.0; 3], body: Body::Ready(Box::new(e)) })
}

/// `MobBucketItem.checkExtraContent` for an axolotl bucket emptied at `pos`: the axolotl comes out
/// as the bucket kept it (`EntitySpawnReason.BUCKET`: no `finalizeSpawn`; the variant, health, age
/// and hunting cooldown from the bucket, `FromBucket` set).
pub(crate) fn bucket_axolotl(bucket: &kiln_item::ItemStack, pos: [f64; 3]) -> Option<Spawn> {
    let seed = (pos[0].to_bits() ^ pos[2].to_bits().rotate_left(21) ^ pos[1].to_bits().rotate_left(42)) as i64;
    let mut e = kiln_entity::mob::new(MobKind::Axolotl, 0, 0, seed);
    e.set_pos(kiln_entity::math::Vec3::new(pos[0], pos[1], pos[2]));
    e.set_old_pos_and_rot();
    kiln_entity::mob::kinds::axolotl::apply_bucket(&mut e, bucket);
    let t = kiln_data::entities::by_name(MobKind::Axolotl.type_name())?;
    Some(Spawn { kind: t, pos, vel: [0.0; 3], body: Body::Ready(Box::new(e)) })
}

/// `EntityType.getYOffset` for a mob made in the block `pos` (a dispenser's egg or bucket, with its
/// feet aligned to the floor): the mob stands 1 above the block's floor and drops by the free space under
/// its feet inside the block, that is by up to the block's collision top under the mob's footprint.
pub(crate) fn align_offset(level: &impl kiln_blocks::Level, pos: kiln_blocks::BlockPos, width: f32) -> f64 {
    let half = width as f64 / 2.0;
    let (lo, hi) = (0.5 - half, 0.5 + half);
    let mut top = 0.0f64;
    for b in kiln_data::block_props::collision(level.block(pos)) {
        let (min_x, min_z, max_x, max_z, max_y) = (b[0] as f64, b[2] as f64, b[3] as f64, b[5] as f64, b[4] as f64);
        if min_x < hi && max_x > lo && min_z < hi && max_z > lo && max_y <= 1.0 {
            top = top.max(max_y);
        }
    }
    top
}

/// A new mob of `kind` at `pos`, facing `yaw` (the entity's own random decides nothing
/// here; `finalize` runs `finalizeSpawn` with the given context when set).
pub(crate) fn spawn(kind: MobKind, pos: [f64; 3], yaw: Option<f32>, finalize: Option<Finalize>) -> Spawn {
    let t = kiln_data::entities::by_name(kind.type_name()).expect("mob type");
    Spawn { kind: t, pos, vel: [0.0; 3], body: Body::Mob { kind, yaw, finalize } }
}

/// `/summon`: a mob at `pos` (with `nbt` merged into its saved form, else initialized by
/// `finalizeSpawn`), queued for the next id assignment. Returns its display name.
pub(crate) fn summon(
    spawns: &mut Vec<Spawn>,
    entity: &str,
    pos: [f64; 3],
    nbt: Option<&kiln_proto::nbt::Tag>,
    initialize: bool,
    difficulty: u8,
    game_time: i64,
    inhabited: i64,
    seed: i64,
) -> Option<kiln_proto::nbt::Tag> {
    use kiln_proto::nbt::Tag;
    if entity == "minecraft:lightning_bolt" {
        let kind = &kiln_data::entities::types::LIGHTNING_BOLT;
        spawns.push(Spawn { kind, pos, vel: [0.0; 3], body: crate::entities::Body::Lightning { visual_only: false } });
        return Some(Tag::Compound(vec![("translate".into(), Tag::String("entity.minecraft.lightning_bolt".into()))]));
    }
    let Some(kind) = MobKind::by_name(entity) else {
        // Other entities kiln-entity can load (orbs, items, TNT, arrows, ...): from the data.
        let mut c: Vec<(String, Tag)> = match nbt {
            Some(Tag::Compound(fields)) => fields.iter().filter(|(k, _)| k != "id" && k != "Pos").cloned().collect(),
            _ => Vec::new(),
        };
        c.push(("id".into(), Tag::String(entity.into())));
        c.push(("Pos".into(), Tag::List(pos.iter().map(|&v| Tag::Double(v)).collect())));
        spawns.push(Spawn::from_saved(&Tag::Compound(c), seed, false).ok()?);
        let path = entity.strip_prefix("minecraft:").unwrap_or(entity);
        return Some(Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.{path}")))]));
    };
    let name = Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.{}", kind.short_name())))]);
    match nbt {
        Some(Tag::Compound(fields)) => {
            let mut c: Vec<(String, Tag)> = fields.iter().filter(|(k, _)| k != "id" && k != "Pos").cloned().collect();
            c.push(("id".into(), Tag::String(kind.type_name().into())));
            c.push(("Pos".into(), Tag::List(pos.iter().map(|&v| Tag::Double(v)).collect())));
            spawns.push(Spawn::from_saved(&Tag::Compound(c), seed, false).ok()?);
        }
        _ => {
            let finalize = initialize.then(|| Finalize::command(difficulty_instance(difficulty, game_time, inhabited, 1.0), seed, false, difficulty == 0));
            spawns.push(spawn(kind, pos, None, finalize));
        }
    }
    Some(name)
}

/// How a new mob's `finalizeSpawn` runs.
#[derive(Clone, Debug)]
pub(crate) struct Finalize {
    pub ctx: mob::SpawnContext,
    /// Seeds the level random stand-in the spawn decisions draw from.
    pub seed: i64,
    /// Natural spawns may despawn; summoned and egg-spawned ones are persistent only when
    /// asked (vanilla: `/summon` mobs despawn too).
    pub persistent: bool,
    /// `EntitySpawnReason.NATURAL`: the natural spawner's mobs (not eggs or `/summon`).
    pub natural: bool,
    /// `ServerLevel.isSpawningMonsters` is false (a peaceful world or `spawn_monsters` off).
    pub monsters_disabled: bool,
    /// The box a camel husk jockey would stand in is free (`Husk.finalizeSpawn`'s `noCollision`;
    /// only the natural spawner looks).
    pub camel_space: bool,
    /// The mob appears inside a piece of a `#cats_spawn_as_black` structure (a swamp hut).
    pub black_cat: bool,
}

impl Finalize {
    /// A summoned or egg-spawned mob (not `NATURAL`).
    pub(crate) fn command(ctx: mob::SpawnContext, seed: i64, persistent: bool, monsters_disabled: bool) -> Finalize {
        Finalize { ctx, seed, persistent, natural: false, monsters_disabled, camel_space: false, black_cat: false }
    }
}

/// `DifficultyInstance` for a position: the effective difficulty and its special multiplier.
pub(crate) fn difficulty_instance(difficulty: u8, game_time: i64, inhabited: i64, moon_brightness: f32) -> mob::SpawnContext {
    let base = difficulty as f32;
    let effective = if difficulty == 0 {
        0.0
    } else {
        let hard = difficulty == 3;
        let mut f = 0.75f32;
        let g = ((game_time as f32 + -72000.0) / 1440000.0).clamp(0.0, 1.0) * 0.25;
        f += g;
        let mut h = 0.0f32;
        h += (inhabited as f32 / 3600000.0).clamp(0.0, 1.0) * if hard { 1.0 } else { 0.75 };
        h += (moon_brightness * 0.25).clamp(0.0, g);
        if difficulty == 1 {
            h *= 0.5;
        }
        f += h;
        base * f
    };
    let special = if effective < 2.0 {
        0.0
    } else if effective > 4.0 {
        1.0
    } else {
        (effective - 2.0) / 2.0
    };
    mob::SpawnContext {
        special_multiplier: special,
        effective_difficulty: effective,
        hard: difficulty == 3,
        halloween: false,
        biome: None,
        moon_brightness: moon_brightness,
    }
}

/// The loot context of a dying mob (`LootContextParamSets.ENTITY`).
pub(crate) struct DeathContext {
    pub type_name: &'static str,
    pub origin: [f64; 3],
    pub on_fire: bool,
    pub baby: bool,
    pub killed_by_player: bool,
    pub damage_type: &'static str,
    /// The killer's main hand item (looting), if a player.
    pub weapon: Option<kiln_item::ItemStack>,
    /// A raider's `type_specific/raider` facts: (has a raid, is a captain).
    pub raider: Option<(bool, bool)>,
    /// Whoever dealt the damage (`damage_source_properties`' `source_entity`): its type and the
    /// components entity predicates match (a frog's variant).
    pub attacker: Option<AttackerView>,
    /// The dying mob's own components `entity_properties` of `this` match (a chicken's variant
    /// decides the colour of its egg).
    pub components: Vec<kiln_item::Component>,
    /// The `type_specific` facts of the dying mob that loot tables ask for: a sheep's `sheared` and a slime's or cube's `size`.
    pub specific: TypeSpecific,
    /// What an `attacker`, `direct_attacker` or `vehicle` predicate needs beyond that.
    pub extra: DeathExtra,
}

/// The loot data (its tags answer item and enchantment tag predicates of the killer's weapon) and the type of the mob the
/// dying one rode (a husk on a camel husk, a zombie on a zombie horse have more to drop).
#[derive(Clone, Default)]
pub(crate) struct DeathExtra {
    pub loot: Option<std::sync::Arc<kiln_loot::LootData>>,
    pub vehicle_type: Option<&'static str>,
    /// The recipes (`furnace_smelt` cooks the meat of a mob that burned or was killed with fire aspect).
    pub rules: Option<std::sync::Arc<kiln_inventory::Rules>>,
    /// The `tool` parameter, for the tables that have one (shearing: the shears).
    pub tool: Option<kiln_item::ItemStack>,
    /// The context has no `damage_source` (gifts, shearing, equipment: not a death).
    pub no_damage_source: bool,
}

/// `EntitySubPredicate`s that depend on the entity's class (`type_specific/sheep`, `type_specific/cube_mob`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TypeSpecific {
    pub sheared: Option<bool>,
    pub cube_size: Option<i32>,
}

impl TypeSpecific {
    /// The facts of `m`, when it is a sheep or a cube mob (a slime, magma cube, sulfur cube).
    pub(crate) fn of(m: Option<&MobData>) -> TypeSpecific {
        let Some(m) = m else { return TypeSpecific::default() };
        let sheared = match &m.species {
            Species::Sheep { sheared, .. } => Some(*sheared),
            _ => None,
        };
        let cube_size = matches!(m.kind, MobKind::Slime | MobKind::MagmaCube | MobKind::SulfurCube).then(|| kiln_entity::mob::kinds::slime::size(m));
        TypeSpecific { sheared, cube_size }
    }
}

/// The killer as a `source_entity` predicate sees it.
#[derive(Clone, Debug)]
pub(crate) struct AttackerView {
    pub type_name: &'static str,
    pub components: Vec<kiln_item::Component>,
}

impl AttackerView {
    /// `EntityPredicate.matches` for the parts Kiln can answer about a killer: its type and
    /// components.
    fn matches(&self, p: &kiln_loot::predicate::world::EntityPredicate) -> bool {
        use kiln_loot::predicate::world::EntitySubPredicate as P;
        let type_id = kiln_item::registry::ENTITY_TYPE.id(self.type_name).unwrap_or(-1);
        p.parts.iter().all(|part| match part {
            P::EntityType(set) => set.contains(type_id),
            P::Components(cs) => cs.iter().all(|c| self.components.contains(c)),
            _ => false,
        })
    }
}

impl kiln_loot::LootContext for DeathContext {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        match target {
            kiln_loot::EntityTarget::This => true,
            kiln_loot::EntityTarget::Attacker | kiln_loot::EntityTarget::DirectAttacker => self.killed_by_player || self.attacker.is_some(),
            kiln_loot::EntityTarget::AttackingPlayer => self.killed_by_player,
            _ => false,
        }
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn has_damage_source(&self) -> bool {
        !self.extra.no_damage_source
    }
    fn tool(&self) -> Option<&kiln_item::ItemStack> {
        self.extra.tool.as_ref()
    }
    /// `furnace_smelt`: the first smelting recipe that takes the item (`RecipeManager.getRecipeFor(SMELTING)` and `assemble`).
    fn smelt(&self, input: &kiln_item::ItemStack) -> Option<kiln_item::ItemStack> {
        let rules = self.extra.rules.as_ref()?;
        let i = rules.recipes.find_cooking(kiln_inventory::recipe::CookingKind::Smelting, input, None)?;
        Some(rules.recipes.assemble_single(i))
    }
    fn damage_source_matches(&self, p: &kiln_loot::predicate::world::DamageSourcePredicate) -> bool {
        // Only the damage type tags are known here; entity sub-predicates fail.
        let id = kiln_data::synced_id("minecraft:damage_type", self.damage_type).unwrap_or(0);
        let entity_ok = |pred: &Option<kiln_loot::predicate::world::EntityPredicate>| match pred {
            None => true,
            Some(pr) => self.attacker.as_ref().is_some_and(|a| a.matches(pr)),
        };
        entity_ok(&p.direct_entity)
            && entity_ok(&p.source_entity)
            && p.tags.iter().all(|t| crate::health::damage_type_tag(id, t.tag.as_str()) == t.expected)
    }
    fn entity_matches(&self, target: kiln_loot::EntityTarget, predicate: &kiln_loot::predicate::EntityPredicate) -> bool {
        use kiln_loot::predicate::world::EntitySubPredicate as Q;
        if matches!(target, kiln_loot::EntityTarget::Attacker | kiln_loot::EntityTarget::DirectAttacker) {
            // The killer: its type and variant (a skeleton's creeper disc), and a player's main hand (fire aspect cooks the meat).
            let Some(a) = self.attacker.as_ref() else { return false };
            return predicate.parts.iter().all(|part| match part {
                Q::Equipment(slots) => {
                    let (Some(loot), true) = (self.extra.loot.as_ref(), a.type_name == "minecraft:player") else { return false };
                    slots.iter().all(|(slot, ip)| match (slot.as_str(), self.weapon.as_ref()) {
                        ("mainhand", Some(w)) => kiln_loot::predicate::item_matches(&loot.tags, ip, w),
                        _ => false,
                    })
                }
                other => a.matches(&kiln_loot::predicate::world::EntityPredicate { parts: vec![other.clone()], json: predicate.json.clone() }),
            });
        }
        if target != kiln_loot::EntityTarget::This {
            return false;
        }
        let type_id = kiln_item::registry::ENTITY_TYPE.id(self.type_name).unwrap_or(-1);
        let view = crate::enchant::EntityView {
            type_id,
            pos: self.origin,
            on_ground: true,
            on_fire: self.on_fire,
            ..Default::default()
        };
        use kiln_loot::predicate::world::EntitySubPredicate as P;
        predicate.parts.iter().all(|part| match part {
            P::Raider { has_raid, is_captain } => {
                let (raid, captain) = self.raider.unwrap_or((false, false));
                self.raider.is_some() && has_raid.is_none_or(|h| h == raid) && is_captain.is_none_or(|c| c == captain)
            }
            P::Components(cs) => cs.iter().all(|c| self.components.contains(c)),
            // `vehicle`: what the dying mob rode.
            P::Vehicle(v) => self.extra.vehicle_type.is_some_and(|t| {
                let id = kiln_item::registry::ENTITY_TYPE.id(t).unwrap_or(-1);
                v.parts.iter().all(|p| matches!(p, P::EntityType(set) if set.contains(id)))
            }),
            // `SheepPredicate` / `CubeMobPredicate`: only for a mob of the class.
            P::Sheep { sheared } => self.specific.sheared.is_some_and(|s| sheared.is_none_or(|w| w == s)),
            P::CubeMob { size } => self.specific.cube_size.is_some_and(|c| size.min.is_none_or(|m| c >= m) && size.max.is_none_or(|m| c <= m)),
            P::Flags(f) if f.is_baby.is_some() => f.is_baby == Some(self.baby) && {
                let mut f2 = f.clone();
                f2.is_baby = None;
                view.matches(&kiln_loot::predicate::world::EntityPredicate { parts: vec![P::Flags(f2)], json: predicate.json.clone() })
            },
            _ => view.matches(&kiln_loot::predicate::world::EntityPredicate { parts: vec![part.clone()], json: predicate.json.clone() }),
        })
    }
    fn entity_enchantment_level(&self, target: kiln_loot::EntityTarget, enchantment: i32, _slots: &[kiln_item::component::EquipmentSlotGroup]) -> i32 {
        if target == kiln_loot::EntityTarget::This {
            return 0;
        }
        self.weapon.as_ref().and_then(kiln_loot::predicate::item::enchantments).map_or(0, |e| e.level(enchantment))
    }
}

/// Seed for a loot roll: the world seed, the tick and the entity, so it does not depend on how
/// regions split the world (vanilla draws from a server-wide random sequence).
pub(crate) fn loot_seed(world_seed: i64, game_time: i64, entity: i32, salt: u64) -> i64 {
    let mut h = (world_seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ game_time as u64;
    for v in [entity as u64, salt] {
        h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    (h | 1) as i64
}

/// Items of `table` for `ctx`, with `seed`.
pub(crate) fn roll(loot: &kiln_loot::LootData, table: &str, ctx: &dyn kiln_loot::LootContext, seed: i64) -> Vec<kiln_item::ItemStack> {
    let Some(id) = kiln_item::Identifier::parse(table) else { return Vec::new() };
    let Some(t) = loot.table(&id) else { return Vec::new() };
    let (mut sequences, mut level) = (kiln_loot::RandomSequences::new(0), kiln_javamath::random::LegacyRandom::new(seed));
    let mut rng = t.random(seed, &mut sequences, &mut level);
    loot.random_items(&id, ctx, rng.source()).into_iter().filter(|s| !s.is_empty()).collect()
}

/// An item dropped at `pos` the way `Entity.spawnAtLocation` throws it (a small random throw
/// from `h`).
pub(crate) fn drop_item(stack: kiln_item::ItemStack, pos: [f64; 3], h: u64) -> Spawn {
    let unit = |shift: u32| ((h >> shift) & 0xFFFF) as f64 / 65536.0;
    Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos,
        vel: [unit(0) * 0.2 - 0.1, 0.2, unit(16) * 0.2 - 0.1],
        body: Body::Item { stack, pickup_delay: 10, thrower: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camel (and its husk) is tame by `isTamed()` alone: a vanilla server does not list the horse flags (field 18) for it
    /// unless it is grazing, and Kiln sent the tame bit.
    #[test]
    fn a_camel_does_not_send_the_tame_flag() {
        for name in ["minecraft:camel", "minecraft:camel_husk"] {
            let tag = kiln_proto::nbt::Tag::Compound(vec![
                ("id".into(), kiln_proto::nbt::Tag::String(name.into())),
                ("UUID".into(), kiln_proto::nbt::Tag::IntArray(vec![1, 2, 3, 4])),
                ("Pos".into(), kiln_proto::nbt::Tag::List(vec![kiln_proto::nbt::Tag::Double(0.5), kiln_proto::nbt::Tag::Double(64.0), kiln_proto::nbt::Tag::Double(0.5)])),
            ]);
            let spawn = Spawn::from_saved(&tag, 7, true).unwrap_or_else(|_| panic!("{name} loads"));
            let crate::entities::Body::Loaded(e) = &spawn.body else { panic!("a loaded mob") };
            let d = metadata(e, mob::data(e).unwrap());
            assert!(!d.fields().iter().any(|f| f.0 == 18), "{name}: the horse flags are not sent: {:?}", d.fields());
        }
    }

    /// A client decodes an entity data entry by the serializer registered for its index on that entity type and
    /// disconnects on any other ("Invalid entity data item type for field 15 on entity ClientMannequin": a mannequin,
    /// which is no `Mob`, was sent the mob flags byte where its `Avatar` main hand belongs). Every mob type's entity
    /// data must be fields of that type, with the serializer the type gives them.
    #[test]
    fn every_mob_sends_only_fields_of_its_entity_type() {
        for &kind in mob::ALL_KINDS {
            let name = kind.type_name();
            let t = kiln_data::entities::by_name(name).expect("entity type");
            let allowed: Vec<(u8, i32)> = t.fields().iter().map(|(_, f)| (f.index, f.serializer)).collect();
            for (label, extra) in [("adult", None), ("baby", Some(("Age", kiln_proto::nbt::Tag::Int(-24000))))] {
                let mut fields = vec![
                    ("id".to_owned(), kiln_proto::nbt::Tag::String(name.into())),
                    ("UUID".into(), kiln_proto::nbt::Tag::IntArray(vec![1, 2, 3, 4])),
                    ("Pos".into(), kiln_proto::nbt::Tag::List(vec![kiln_proto::nbt::Tag::Double(0.5), kiln_proto::nbt::Tag::Double(64.0), kiln_proto::nbt::Tag::Double(0.5)])),
                ];
                if let Some((k, v)) = extra {
                    fields.push((k.into(), v));
                }
                let spawn = Spawn::from_saved(&kiln_proto::nbt::Tag::Compound(fields), 7, true).unwrap_or_else(|_| panic!("{name} loads"));
                let e = match &spawn.body {
                    crate::entities::Body::Loaded(e) => e,
                    crate::entities::Body::LoadedStack(_, riders) => &riders[0].entity,
                    _ => panic!("{name}: a loaded mob"),
                };
                let m = mob::data(e).unwrap_or_else(|| panic!("{name}: mob data"));
                let d = metadata(e, m);
                for f in d.fields() {
                    assert!(allowed.contains(f), "{name} ({label}) sends field {} with serializer {}, which its type does not define: {allowed:?}", f.0, f.1);
                }
            }
        }
    }

    #[test]
    fn difficulty_of_a_new_world() {
        let d = difficulty_instance(2, 0, 0, 1.0);
        assert_eq!(d.effective_difficulty, 1.5);
        assert_eq!(d.special_multiplier, 0.0);
    }

    /// The mobs of structures (`finalizeSpawn` with `STRUCTURE`): a drowned of an ocean ruin holds a trident or a
    /// fishing rod one time in ten and is a baby one time in twenty; a bastion's piglin brute holds its golden
    /// axe and its piglins wear gold pieces (and are neither babies nor given a spawn weapon).
    #[test]
    fn structure_mobs_are_finalized() {
        use kiln_proto::nbt::Tag;
        let make = |id: &str, n: i32| -> Spawn {
            let tag = Tag::Compound(vec![
                ("id".into(), Tag::String(id.into())),
                ("UUID".into(), Tag::IntArray(vec![n, n ^ 0x5555, 7, n.wrapping_mul(31)])),
                ("Pos".into(), Tag::List(vec![Tag::Double(0.5), Tag::Double(64.0), Tag::Double(0.5)])),
            ]);
            let spawn = Spawn::from_saved(&tag, i64::from(n), true).expect("loads");
            finalize_structure_mob(spawn, 2, 0)
        };
        let (mut tools, mut babies) = (0, 0);
        for n in 0..4000 {
            let s = make("minecraft:drowned", n);
            let e = match &s.body {
                crate::entities::Body::Loaded(e) => e,
                crate::entities::Body::LoadedStack(_, riders) => &riders[0].entity,
                _ => panic!("a loaded mob"),
            };
            let m = mob::data(e).unwrap();
            let held = m.equipment[mob::MAINHAND].item_name();
            if held == "minecraft:trident" || held == "minecraft:fishing_rod" {
                tools += 1;
            }
            if m.baby() {
                babies += 1;
            }
        }
        assert!((300..=500).contains(&tools), "about 10% of 4000 drowned hold a trident or a rod: {tools}");
        assert!((120..=280).contains(&babies), "about 5% of 4000 drowned are babies: {babies}");
        for n in 0..50 {
            let brute = make("minecraft:piglin_brute", n);
            let crate::entities::Body::Loaded(e) = &brute.body else { panic!("a loaded mob") };
            assert_eq!(mob::data(e).unwrap().equipment[mob::MAINHAND].item_name(), "minecraft:golden_axe");
        }
        let (mut armored, mut armed, mut baby_piglins) = (0, 0, 0);
        for n in 0..2000 {
            let piglin = make("minecraft:piglin", n);
            let crate::entities::Body::Loaded(e) = &piglin.body else { panic!("a loaded mob") };
            let m = mob::data(e).unwrap();
            armored += [mob::HEAD, mob::CHEST, mob::LEGS, mob::FEET].iter().filter(|&&s| !m.equipment[s].is_empty()).count();
            armed += !m.equipment[mob::MAINHAND].is_empty() as i32;
            baby_piglins += m.baby() as i32;
        }
        assert!((600..=1000).contains(&armored), "gold pieces: each of 4 slots at 10% of 2000 piglins: {armored}");
        assert_eq!((armed, baby_piglins), (0, 0), "a structure's piglins keep what their template gave them and are adults");
    }
}

/// The pairing packet: a mob paired in the tick it was made (`fresh`) is paired before its first step, so without the instances that step makes.
pub(crate) fn attributes_pairing_packet(entity_id: i32, m: &MobData, fresh: bool) -> Option<bytes::Bytes> {
    attributes_packet_of(entity_id, m, None, fresh)
}

/// [`attributes_packet`] with only the attributes in `only` (`AttributeMap.getAttributesToSync`).
pub(crate) fn attributes_packet_of(entity_id: i32, m: &MobData, only: Option<&[mob::attributes::Attr]>, skip_made_by_step: bool) -> Option<bytes::Bytes> {
    use kiln_proto::packets::entity::{AttributeModifier, AttributeSnapshot, ModifierOperation};
    let list: Vec<(i32, &mob::attributes::Instance)> = m
        .attrs
        .list
        .iter()
        .filter(|i| i.created && !(skip_made_by_step && i.by_step) && i.attr.is_client_syncable() && only.is_none_or(|o| o.contains(&i.attr)))
        .filter_map(|i| Some((kiln_data::builtin_id("minecraft:attribute", i.attr.name())?, i)))
        .collect();
    if list.is_empty() {
        return None;
    }
    let op = |o: mob::attributes::Op| match o {
        mob::attributes::Op::AddValue => ModifierOperation::AddValue,
        mob::attributes::Op::AddMultipliedBase => ModifierOperation::AddMultipliedBase,
        mob::attributes::Op::AddMultipliedTotal => ModifierOperation::AddMultipliedTotal,
    };
    let mods: Vec<Vec<AttributeModifier>> =
        list.iter().map(|(_, i)| i.modifiers.iter().map(|x| AttributeModifier { id: &x.id, amount: x.amount, operation: op(x.op) }).collect()).collect();
    let snapshots: Vec<AttributeSnapshot> =
        list.iter().zip(&mods).map(|((id, i), modifiers)| AttributeSnapshot { attribute: *id, base: i.base, modifiers }).collect();
    Some(kiln_proto::packets::entity::update_attributes(entity_id, &snapshots))
}

/// A hash of what [`attributes_packet`] would say (never 0), to tell when some viewers' copy is stale at all.
pub(crate) fn attributes_hash(m: &MobData) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    for i in m.attrs.list.iter().filter(|i| i.created && i.attr.is_client_syncable()) {
        mix(i.attr as u64);
        mix(i.base.to_bits());
        for x in &i.modifiers {
            mix(x.amount.to_bits());
            mix(x.op as u64);
            for b in x.id.bytes() {
                mix(b as u64);
            }
        }
        mix(u64::MAX);
    }
    h | 1
}

/// A hash per existing client-syncable attribute of what [`attributes_packet`] would say about it (never 0), to tell which of them the
/// viewers' copy has stale.
pub(crate) fn attribute_hashes(m: &MobData) -> Vec<(mob::attributes::Attr, u64)> {
    m.attrs
        .list
        .iter()
        .filter(|i| i.created && i.attr.is_client_syncable())
        .map(|i| {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
            mix(i.base.to_bits());
            for x in &i.modifiers {
                mix(x.amount.to_bits());
                mix(x.op as u64);
                for b in x.id.bytes() {
                    mix(b as u64);
                }
            }
            (i.attr, h | 1)
        })
        .collect()
}

/// `StructureTemplate.placeEntities` / the pieces' data markers: `finalizeSpawn` with
/// `EntitySpawnReason.STRUCTURE` on a mob a chunk's generation placed (random equipment and
/// enchantments, a baby's chance, a zombie nautilus under a trident drowned), with the entity's own
/// random. Entities that are not mobs pass through.
pub(crate) fn finalize_structure_mob(mut spawn: Spawn, difficulty: u8, game_time: i64) -> Spawn {
    use crate::entities::Body;
    let Body::Loaded(e) = &mut spawn.body else { return spawn };
    if mob::data(e).is_none() {
        return spawn;
    }
    let ctx = difficulty_instance(difficulty, game_time, 0, 1.0);
    let mut random = kiln_javamath::random::LegacyRandom::new(crate::entities::seed_for_uuid(e.uuid) ^ 0x5374_7275_6374);
    let mut group = mob::GroupData { structure: true, ..Default::default() };
    mob::finalize_spawn(e, &mut random, &ctx, &mut group, false);
    // A trident drowned on its nautilus: the nautilus is the root of the stack, the drowned rides it.
    if let Some(i) = group.companions.iter().position(|c| c.seat == mob::Seat::UnderMob) {
        let vehicle = group.companions.swap_remove(i).entity;
        let Body::Loaded(rider) = std::mem::replace(&mut spawn.body, Body::Lightning { visual_only: false }) else { return spawn };
        let kind = kiln_data::entities::by_name(vehicle.type_name).unwrap_or(spawn.kind);
        let pos = [vehicle.position().x, vehicle.position().y, vehicle.position().z];
        return Spawn { kind, pos, vel: [0.0; 3], body: Body::LoadedStack(Box::new(vehicle), vec![kiln_entity::persist::Rider { entity: *rider, vehicle: 0 }]) };
    }
    spawn
}
