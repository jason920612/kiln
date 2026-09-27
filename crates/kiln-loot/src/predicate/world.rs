//! The advancement-style predicates loot hands to the world: `EntityPredicate`,
//! `DamageSourcePredicate` and `LocationPredicate`. They are decoded (and validated) here and
//! evaluated by the [`crate::LootContext`] implementation, which has the entities and the
//! level. Each keeps its source JSON so a context can also key on it.

use super::block::BlockPredicate;
use super::item_predicate;
use crate::json::Json;
use crate::parse::{IdSet, NameSet, PResult, Parser, boolean, fail, ident, list, obj, opt, value};
use kiln_item::component::{DataComponentMatchers, DoubleBounds, IntBounds, ItemPredicate};
use kiln_item::registry;
use kiln_item::{Component, Identifier};

/// `EntityFlagsPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityFlags {
    pub is_on_ground: Option<bool>,
    pub is_on_fire: Option<bool>,
    pub is_sneaking: Option<bool>,
    pub is_sprinting: Option<bool>,
    pub is_swimming: Option<bool>,
    pub is_flying: Option<bool>,
    pub is_baby: Option<bool>,
    pub is_in_water: Option<bool>,
    pub is_fall_flying: Option<bool>,
}

/// One entry of an `EntityPredicate` (`minecraft:entity_sub_predicate_type`).
#[derive(Debug, Clone, PartialEq)]
pub enum EntitySubPredicate {
    /// `entity_type`: `minecraft:entity_type` entries or tag.
    EntityType(IdSet),
    Flags(EntityFlags),
    /// `components`: exact entity data components (variants, colors...).
    Components(Vec<Component>),
    /// `predicates`: partial component predicates.
    Predicates(DataComponentMatchers),
    /// `equipment`: slot name → item predicate.
    Equipment(Vec<(String, ItemPredicate)>),
    Vehicle(Box<EntityPredicate>),
    Passenger(Box<EntityPredicate>),
    TargetedEntity(Box<EntityPredicate>),
    Location(Box<LocationPredicate>),
    SteppingOn(Box<LocationPredicate>),
    MovementAffectedBy(Box<LocationPredicate>),
    /// `type_specific/sheep`.
    Sheep { sheared: Option<bool> },
    /// `type_specific/cube_mob`.
    CubeMob { size: IntBounds },
    /// `type_specific/raider`.
    Raider { has_raid: Option<bool>, is_captain: Option<bool> },
    /// `type_specific/fishing_hook`.
    FishingHook { in_open_water: Option<bool> },
    /// Any other sub-predicate (distance, movement, effects, nbt, periodic_tick, team, slots,
    /// entity_tags, type_specific/lightning, type_specific/player), kept as written.
    Other(Identifier, Json),
}

/// `EntityPredicate`: all parts must match.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityPredicate {
    pub parts: Vec<EntitySubPredicate>,
    pub json: Json,
}

const SUB_PREDICATES: &[&str] = &[
    "entity_type",
    "location",
    "stepping_on",
    "movement_affected_by",
    "distance",
    "movement",
    "effects",
    "nbt",
    "flags",
    "equipment",
    "periodic_tick",
    "vehicle",
    "passenger",
    "targeted_entity",
    "team",
    "slots",
    "components",
    "predicates",
    "entity_tags",
    "type_specific/lightning",
    "type_specific/fishing_hook",
    "type_specific/player",
    "type_specific/cube_mob",
    "type_specific/raider",
    "type_specific/sheep",
];

fn opt_bool(j: &Json, key: &str) -> PResult<Option<bool>> {
    opt(j, key, boolean)
}

fn int_bounds(j: &Json) -> PResult<IntBounds> {
    value(j, IntBounds::from_value)
}

impl EntityPredicate {
    pub fn parse(p: &Parser, j: &Json) -> PResult<EntityPredicate> {
        let mut parts = Vec::new();
        for (key, v) in obj(j)? {
            let id = Identifier::parse(key).ok_or_else(|| crate::parse::ParseError::new(format!("invalid key {key:?}")))?;
            let name = id.as_str().strip_prefix("minecraft:").unwrap_or("");
            if !SUB_PREDICATES.contains(&name) {
                return fail(format!("unknown entity sub-predicate type {id}"));
            }
            let part = (|| -> PResult<EntitySubPredicate> {
                Ok(match name {
                    "entity_type" => EntitySubPredicate::EntityType(p.id_set(v, registry::ENTITY_TYPE)?),
                    "flags" => EntitySubPredicate::Flags(EntityFlags {
                        is_on_ground: opt_bool(v, "is_on_ground")?,
                        is_on_fire: opt_bool(v, "is_on_fire")?,
                        is_sneaking: opt_bool(v, "is_sneaking")?,
                        is_sprinting: opt_bool(v, "is_sprinting")?,
                        is_swimming: opt_bool(v, "is_swimming")?,
                        is_flying: opt_bool(v, "is_flying")?,
                        is_baby: opt_bool(v, "is_baby")?,
                        is_in_water: opt_bool(v, "is_in_water")?,
                        is_fall_flying: opt_bool(v, "is_fall_flying")?,
                    }),
                    "components" => {
                        let mut out = Vec::new();
                        for (k, cv) in obj(v)? {
                            let cid = kiln_item::component::by_name(k)
                                .ok_or_else(|| crate::parse::ParseError::new(format!("unknown component type {k}")))?;
                            out.push(value(cv, |x| Component::from_value(cid, x)).map_err(|e| e.at(k))?);
                        }
                        EntitySubPredicate::Components(out)
                    }
                    "predicates" => {
                        let wrapped = Json::Obj(vec![("predicates".into(), v.clone())]);
                        let map = wrapped.to_value();
                        EntitySubPredicate::Predicates(
                            DataComponentMatchers::from_fields(map.as_map().map_err(|e| crate::parse::ParseError::new(e.0))?)
                                .map_err(|e| crate::parse::ParseError::new(e.0))?,
                        )
                    }
                    "equipment" => EntitySubPredicate::Equipment(
                        obj(v)?
                            .iter()
                            .map(|(slot, ip)| Ok((slot.clone(), item_predicate(ip).map_err(|e| e.at(slot))?)))
                            .collect::<PResult<_>>()?,
                    ),
                    "vehicle" => EntitySubPredicate::Vehicle(Box::new(EntityPredicate::parse(p, v)?)),
                    "passenger" => EntitySubPredicate::Passenger(Box::new(EntityPredicate::parse(p, v)?)),
                    "targeted_entity" => EntitySubPredicate::TargetedEntity(Box::new(EntityPredicate::parse(p, v)?)),
                    "location" => EntitySubPredicate::Location(Box::new(LocationPredicate::parse(p, v)?)),
                    "stepping_on" => EntitySubPredicate::SteppingOn(Box::new(LocationPredicate::parse(p, v)?)),
                    "movement_affected_by" => {
                        EntitySubPredicate::MovementAffectedBy(Box::new(LocationPredicate::parse(p, v)?))
                    }
                    "type_specific/sheep" => EntitySubPredicate::Sheep { sheared: opt_bool(v, "sheared")? },
                    "type_specific/cube_mob" => {
                        EntitySubPredicate::CubeMob { size: opt(v, "size", int_bounds)?.unwrap_or_default() }
                    }
                    "type_specific/raider" => EntitySubPredicate::Raider {
                        has_raid: opt_bool(v, "has_raid")?,
                        is_captain: opt_bool(v, "is_captain")?,
                    },
                    "type_specific/fishing_hook" => {
                        EntitySubPredicate::FishingHook { in_open_water: opt_bool(v, "in_open_water")? }
                    }
                    _ => EntitySubPredicate::Other(id.clone(), v.clone()),
                })
            })()
            .map_err(|e| e.at(key))?;
            parts.push(part);
        }
        Ok(EntityPredicate { parts, json: j.clone() })
    }
}

/// `TagPredicate<DamageType>`.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageTagPredicate {
    pub tag: Identifier,
    pub expected: bool,
}

/// `DamageSourcePredicate`.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageSourcePredicate {
    pub tags: Vec<DamageTagPredicate>,
    pub direct_entity: Option<EntityPredicate>,
    pub source_entity: Option<EntityPredicate>,
    pub is_direct: Option<bool>,
    pub json: Json,
}

impl DamageSourcePredicate {
    pub fn parse(p: &Parser, j: &Json) -> PResult<DamageSourcePredicate> {
        obj(j)?;
        Ok(DamageSourcePredicate {
            tags: opt(j, "tags", |v| {
                list(v, |t| {
                    let id = crate::parse::req(t, "id", crate::parse::string)?;
                    let tag = id
                        .strip_prefix('#')
                        .and_then(Identifier::parse)
                        .ok_or_else(|| crate::parse::ParseError::new(format!("invalid tag {id:?}")))?;
                    Ok(DamageTagPredicate { tag, expected: crate::parse::req(t, "expected", boolean)? })
                })
            })?
            .unwrap_or_default(),
            direct_entity: opt(j, "direct_entity", |v| EntityPredicate::parse(p, v))?,
            source_entity: opt(j, "source_entity", |v| EntityPredicate::parse(p, v))?,
            is_direct: opt(j, "is_direct", boolean)?,
            json: j.clone(),
        })
    }
}

/// `LocationPredicate.PositionPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PositionPredicate {
    pub x: DoubleBounds,
    pub y: DoubleBounds,
    pub z: DoubleBounds,
}

/// `LocationPredicate`.
#[derive(Debug, Clone, PartialEq)]
pub struct LocationPredicate {
    pub position: Option<PositionPredicate>,
    /// `minecraft:worldgen/biome` entries or tag.
    pub biomes: Option<NameSet>,
    /// `minecraft:worldgen/structure` entries or tag.
    pub structures: Option<NameSet>,
    pub dimension: Option<Identifier>,
    pub smokey: Option<bool>,
    /// `light.light`.
    pub light: Option<IntBounds>,
    pub block: Option<BlockPredicate>,
    /// `fluid`, kept as written (`FluidPredicate`: fluids, state).
    pub fluid: Option<Json>,
    pub can_see_sky: Option<bool>,
    pub json: Json,
}

impl LocationPredicate {
    pub fn parse(p: &Parser, j: &Json) -> PResult<LocationPredicate> {
        obj(j)?;
        let bounds = |v: &Json, key: &str| opt(v, key, |b| value(b, DoubleBounds::from_value)).map(Option::unwrap_or_default);
        Ok(LocationPredicate {
            position: opt(j, "position", |v| {
                obj(v)?;
                Ok(PositionPredicate { x: bounds(v, "x")?, y: bounds(v, "y")?, z: bounds(v, "z")? })
            })?,
            biomes: opt(j, "biomes", |v| p.name_set(v, "minecraft:worldgen/biome"))?,
            structures: opt(j, "structures", |v| p.name_set(v, "minecraft:worldgen/structure"))?,
            dimension: opt(j, "dimension", ident)?,
            smokey: opt(j, "smokey", boolean)?,
            light: opt(j, "light", |v| opt(v, "light", int_bounds).map(Option::unwrap_or_default))?,
            block: opt(j, "block", |v| BlockPredicate::parse(p, v))?,
            fluid: opt(j, "fluid", |v| obj(v).map(|_| v.clone()))?,
            can_see_sky: opt(j, "can_see_sky", boolean)?,
            json: j.clone(),
        })
    }
}
