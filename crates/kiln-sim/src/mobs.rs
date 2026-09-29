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
    pub griefing: bool,
    pub drops: bool,
    pub spawn_mobs: bool,
    pub spawn_monsters: bool,
    /// `minecraft:spawn_wardens`.
    pub spawn_wardens: bool,
    pub cramming: i32,
    pub difficulty: u8,
    /// The world spawn (no natural spawns within 24 blocks).
    pub spawn_point: [i32; 3],
}

impl Default for MobRules {
    fn default() -> Self {
        MobRules {
            day_time: 1000,
            sky_darken: 0,
            monsters_burn: true,
            griefing: true,
            drops: true,
            spawn_mobs: true,
            spawn_monsters: true,
            spawn_wardens: true,
            cramming: 24,
            difficulty: 2,
            spawn_point: [0, 64, 0],
        }
    }
}

/// The `minecraft:gameplay/monsters_burn` timeline: off from 12542 to 23460.
pub(crate) fn monsters_burn(day_time: i64) -> bool {
    let t = day_time.rem_euclid(24000);
    !(12542..23460).contains(&t)
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
    let mob_flags = (m.no_ai as i8) | ((m.left_handed as i8) << 1) | ((m.aggressive as i8) << 2);
    d.set(data::mob::MOB_FLAGS, &DataValue::Byte(mob_flags));
    if kiln_entity::mob::breed::is_ageable(m.kind) && m.age < 0 {
        d.set(data::ageable_mob::BABY, &DataValue::Boolean(true));
    }
    match &m.species {
        Species::Pig | Species::Cow | Species::Chicken { .. } => {
            let (variant, sound) = match m.kind {
                MobKind::Pig => (data::pig::VARIANT, data::pig::SOUND_VARIANT),
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
    } else {
        kiln_entity::mob::kinds::fish::apply_bucket(&mut e, bucket);
    }
    let t = kiln_data::entities::by_name(kind.type_name())?;
    Some(Spawn { kind: t, pos: [at.x, at.y, at.z], vel: [0.0; 3], body: Body::Ready(Box::new(e)) })
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
        let mut e = kiln_entity::persist::load(&Tag::Compound(c), 0, seed).ok()?;
        e.set_pos(kiln_entity::math::Vec3::new(pos[0], pos[1], pos[2]));
        spawns.push(Spawn::loaded(e)?);
        let path = entity.strip_prefix("minecraft:").unwrap_or(entity);
        return Some(Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.{path}")))]));
    };
    let name = Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.{}", kind.short_name())))]);
    match nbt {
        Some(Tag::Compound(fields)) => {
            let mut c: Vec<(String, Tag)> = fields.iter().filter(|(k, _)| k != "id" && k != "Pos").cloned().collect();
            c.push(("id".into(), Tag::String(kind.type_name().into())));
            c.push(("Pos".into(), Tag::List(pos.iter().map(|&v| Tag::Double(v)).collect())));
            let e = kiln_entity::persist::load(&Tag::Compound(c), 0, seed).ok()?;
            let mut e = e;
            e.set_pos(kiln_entity::math::Vec3::new(pos[0], pos[1], pos[2]));
            spawns.push(Spawn::loaded(e)?);
        }
        _ => {
            let finalize = initialize.then(|| Finalize {
                ctx: difficulty_instance(difficulty, game_time, 0, 1.0),
                seed,
                persistent: false,
            });
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
}

impl kiln_loot::LootContext for DeathContext {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        match target {
            kiln_loot::EntityTarget::This => true,
            kiln_loot::EntityTarget::Attacker | kiln_loot::EntityTarget::DirectAttacker | kiln_loot::EntityTarget::AttackingPlayer => {
                self.killed_by_player
            }
            _ => false,
        }
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn has_damage_source(&self) -> bool {
        true
    }
    fn damage_source_matches(&self, p: &kiln_loot::predicate::world::DamageSourcePredicate) -> bool {
        // Only the damage type tags are known here; entity sub-predicates fail.
        let id = kiln_data::synced_id("minecraft:damage_type", self.damage_type).unwrap_or(0);
        p.direct_entity.is_none()
            && p.source_entity.is_none()
            && p.tags.iter().all(|t| crate::health::damage_type_tag(id, t.tag.as_str()) == t.expected)
    }
    fn entity_matches(&self, target: kiln_loot::EntityTarget, predicate: &kiln_loot::predicate::EntityPredicate) -> bool {
        if target != kiln_loot::EntityTarget::This {
            return false;
        }
        let type_id = kiln_item::registry::ENTITY_TYPE.id(self.type_name).unwrap_or(-1);
        let view = crate::enchant::EntityView {
            type_id,
            pos: self.origin,
            on_ground: true,
            on_fire: self.on_fire,
            sneaking: false,
            sprinting: false,
            flying: false,
        };
        use kiln_loot::predicate::world::EntitySubPredicate as P;
        predicate.parts.iter().all(|part| match part {
            P::Raider { has_raid, is_captain } => {
                let (raid, captain) = self.raider.unwrap_or((false, false));
                self.raider.is_some() && has_raid.is_none_or(|h| h == raid) && is_captain.is_none_or(|c| c == captain)
            }
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

    #[test]
    fn difficulty_of_a_new_world() {
        let d = difficulty_instance(2, 0, 0, 1.0);
        assert_eq!(d.effective_difficulty, 1.5);
        assert_eq!(d.special_multiplier, 0.0);
    }
}
