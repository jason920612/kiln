//! Item modifiers (`LootItemFunction`, datapack registry `item_modifier/`).

use crate::condition::Condition;
use crate::context::{EntityTarget, ExplorationMap, Source};
use crate::data::Kind;
use crate::entry::Entry;
use crate::eval::Eval;
use crate::json::Json;
use crate::number::{FloatProvider, IntProvider, entity_target, parse_nbt_path};
use crate::parse::{
    IdSet, NameSet, PResult, ParseError, Parser, Ref, boolean, fail, ident, int, list, long, obj, opt, opt_or, req, string, value,
};
use crate::predicate::{self, item_predicate};
use crate::random::RngExt;
use crate::stack;
use kiln_command::nbt_path::NbtPath;
use kiln_item::component::{
    self as comp, AttributeDisplay, AttributeModifier, AttributeModifiers, AttributeOperation, BannerPatternLayers,
    BlockItemStateProperties, CustomModelData, DyedColor, EquipmentSlotGroup,
    Filterable, FireworkExplosion, FireworkShape, Fireworks, InstrumentComponent, ItemPredicate, Lore,
    OminousBottleAmplifier, PotionContents, SeededContainerLoot, StewEffect, SuspiciousStewEffects, TooltipDisplay,
    WritableBookContent, WrittenBookContent, ids, keys,
};
use kiln_item::registry;
use kiln_item::{ComponentId, DataComponentPatch, Holder, Identifier, ItemStack, Text};
use kiln_proto::nbt::Tag;

/// `ListOperation`: how new list elements combine with existing ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListOperation {
    ReplaceAll,
    ReplaceSection { offset: usize, size: Option<usize> },
    Insert { offset: usize },
    Append,
}

impl ListOperation {
    /// `ListOperation.codec(max)`: a `mode` field and its parameters, inlined.
    fn parse(j: &Json, max: usize) -> PResult<ListOperation> {
        let mode = req(j, "mode", string)?;
        let non_negative = |v: &Json| -> PResult<usize> {
            let n = int(v)?;
            usize::try_from(n).map_err(|_| ParseError::new(format!("value must be non-negative: {n}")))
        };
        Ok(match mode.as_str() {
            "replace_all" => ListOperation::ReplaceAll,
            "replace_section" => {
                let op = ListOperation::ReplaceSection {
                    offset: opt_or(j, "offset", 0, non_negative)?,
                    size: opt(j, "size", non_negative)?,
                };
                if let ListOperation::ReplaceSection { size: Some(s), .. } = op
                    && s > max
                {
                    return fail(format!("size value too large: {s}, max size is {max}"));
                }
                op
            }
            "insert" => ListOperation::Insert { offset: opt_or(j, "offset", 0, non_negative)? },
            "append" => ListOperation::Append,
            other => return fail(format!("unknown list operation mode {other}")),
        })
    }

    /// `ListOperation.apply(original, new, max)`; an operation that does not fit keeps the
    /// original (vanilla logs an error).
    pub fn apply<T: Clone>(&self, original: &[T], new: &[T], max: usize) -> Vec<T> {
        match self {
            ListOperation::ReplaceAll => new.to_vec(),
            ListOperation::ReplaceSection { offset, size } => {
                if *offset > original.len() {
                    return original.to_vec();
                }
                let mut out: Vec<T> = original[..*offset].to_vec();
                out.extend_from_slice(new);
                let resume = offset + size.unwrap_or(new.len());
                if resume < original.len() {
                    out.extend_from_slice(&original[resume..]);
                }
                if out.len() > max { original.to_vec() } else { out }
            }
            ListOperation::Insert { offset } => {
                if *offset > original.len() || original.len() + new.len() > max {
                    return original.to_vec();
                }
                let mut out: Vec<T> = original[..*offset].to_vec();
                out.extend_from_slice(new);
                out.extend_from_slice(&original[*offset..]);
                out
            }
            ListOperation::Append => {
                if original.len() + new.len() > max {
                    return original.to_vec();
                }
                let mut out = original.to_vec();
                out.extend_from_slice(new);
                out
            }
        }
    }
}

/// `ApplyBonusCount.Formula`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BonusFormula {
    OreDrops,
    UniformBonusCount { bonus_multiplier: i32 },
    BinomialWithBonusCount { extra: i32, probability: f32 },
}

/// `ContainerComponentManipulators`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Container,
    BundleContents,
    ChargedProjectiles,
}

/// `NbtProvider`.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtSource {
    Storage(Identifier),
    Context(Source),
}

/// `CopyCustomDataFunction.MergeStrategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    Replace,
    Append,
    Merge,
}

#[derive(Debug, Clone)]
pub struct CopyOperation {
    pub source: NbtPath,
    pub target: NbtPath,
    pub op: MergeStrategy,
}

/// One modifier of `set_attributes`.
#[derive(Debug, Clone)]
pub struct AttributeModifierSpec {
    pub id: Identifier,
    pub attribute: i32,
    pub operation: AttributeOperation,
    pub amount: Ref<FloatProvider>,
    pub slots: Vec<EquipmentSlotGroup>,
}

#[derive(Debug, Clone)]
pub enum FunctionKind {
    SetCount { count: Ref<IntProvider>, add: bool },
    SetItem(i32),
    EnchantWithLevels { levels: Ref<IntProvider>, options: Option<IdSet>, include_additional_cost_component: bool },
    EnchantRandomly { options: Option<IdSet>, only_compatible: bool, include_additional_cost_component: bool },
    SetEnchantments { enchantments: Vec<(i32, Ref<IntProvider>)>, add: bool },
    SetCustomData(Tag),
    SetComponents(DataComponentPatch),
    FurnaceSmelt { use_input_count: bool },
    EnchantedCountIncrease { enchantment: i32, count: Ref<FloatProvider>, limit: i32 },
    SetDamage { damage: Ref<FloatProvider>, add: bool },
    SetAttributes { modifiers: Vec<AttributeModifierSpec>, replace: bool },
    SetName { name: Option<Text>, entity: Option<EntityTarget>, item_name: bool },
    ExplorationMap { destination: NameSet, decoration: i32, zoom: i8, search_radius: i32, skip_existing_chunks: bool },
    SetStewEffect(Vec<(i32, Ref<IntProvider>)>),
    CopyName(Source),
    SetContents { component: ContainerKind, entries: Vec<Entry> },
    ModifyContents { component: ContainerKind, modifier: Ref<Function> },
    Filtered { filter: ItemPredicate, on_pass: Option<Ref<Function>>, on_fail: Option<Ref<Function>> },
    LimitCount { min: Option<Ref<IntProvider>>, max: Option<Ref<IntProvider>> },
    ApplyBonus { enchantment: i32, formula: BonusFormula },
    SetLootTable { table: Identifier, seed: i64 },
    ExplosionDecay,
    SetLore { lore: Vec<Text>, mode: ListOperation, entity: Option<EntityTarget> },
    FillPlayerHead(EntityTarget),
    CopyCustomData { source: NbtSource, ops: Vec<CopyOperation> },
    CopyState { properties: Vec<String> },
    SetBannerPattern { patterns: BannerPatternLayers, append: bool },
    SetPotion(i32),
    SetRandomDyes(Ref<IntProvider>),
    SetRandomPotion(Option<IdSet>),
    SetInstrument(IdSet),
    Sequence(Vec<Ref<Function>>),
    CopyComponents { source: Source, include: Option<Vec<ComponentId>>, exclude: Option<Vec<ComponentId>> },
    SetFireworks { explosions: Option<(Vec<FireworkExplosion>, ListOperation)>, flight_duration: Option<i32> },
    SetFireworkExplosion {
        shape: Option<FireworkShape>,
        colors: Option<Vec<i32>>,
        fade_colors: Option<Vec<i32>>,
        trail: Option<bool>,
        twinkle: Option<bool>,
    },
    SetBookCover { title: Option<Filterable<String>>, author: Option<String>, generation: Option<i32> },
    SetWrittenBookPages { pages: Vec<Filterable<Text>>, mode: ListOperation },
    SetWritableBookPages { pages: Vec<Filterable<String>>, mode: ListOperation },
    ToggleTooltips(Vec<(ComponentId, bool)>),
    SetOminousBottleAmplifier(Ref<IntProvider>),
    SetCustomModelData {
        floats: Option<(Vec<Ref<FloatProvider>>, ListOperation)>,
        flags: Option<(Vec<bool>, ListOperation)>,
        strings: Option<(Vec<String>, ListOperation)>,
        colors: Option<(Vec<Ref<IntProvider>>, ListOperation)>,
    },
    Discard,
}

/// A loot function with its optional condition (`LootItemConditionalFunction`).
#[derive(Debug, Clone)]
pub struct Function {
    pub condition: Option<Ref<Condition>>,
    pub kind: FunctionKind,
}

fn component_type(j: &Json) -> PResult<ComponentId> {
    let name = string(j)?;
    comp::by_name(&name).ok_or_else(|| ParseError::new(format!("unknown data component type {name}")))
}

fn container_kind(j: &Json) -> PResult<ContainerKind> {
    Ok(match ident(j)?.as_str() {
        "minecraft:container" => ContainerKind::Container,
        "minecraft:bundle_contents" => ContainerKind::BundleContents,
        "minecraft:charged_projectiles" => ContainerKind::ChargedProjectiles,
        other => return fail(format!("unknown container component {other}")),
    })
}

fn text(j: &Json) -> PResult<Text> {
    crate::text::parse(j)
}

/// `ListOperation.StandAlone`: `{values, mode...}`.
fn stand_alone<T>(j: &Json, max: usize, mut f: impl FnMut(&Json) -> PResult<T>) -> PResult<(Vec<T>, ListOperation)> {
    let values = req(j, "values", |v| list(v, &mut f))?;
    if values.len() > max {
        return fail(format!("too many values: {} > {max}", values.len()));
    }
    Ok((values, ListOperation::parse(j, max)?))
}

fn filterable<T>(j: &Json, f: impl Fn(&Json) -> PResult<T>) -> PResult<Filterable<T>> {
    // `Filterable.codec`: `{raw, filtered?}` or the raw value alone.
    if j.as_object().is_some() && j.get("raw").is_some() {
        Ok(Filterable { raw: req(j, "raw", &f)?, filtered: opt(j, "filtered", &f)? })
    } else {
        Ok(Filterable::pass_through(f(j)?))
    }
}

impl Function {
    /// `LootItemFunctions.CODEC`: a reference to `item_modifier/`, a typed object, or a list
    /// of either (an inline sequence).
    pub fn parse_ref(p: &Parser, j: &Json) -> PResult<Ref<Function>> {
        match j {
            Json::Str(_) => p.holder(j, Kind::Modifier, Function::parse),
            _ => Function::parse(p, j).map(Ref::direct),
        }
    }

    /// `LootItemFunctions.DIRECT_CODEC`: a typed object, or a list (`SequenceFunction`).
    pub fn parse(p: &Parser, j: &Json) -> PResult<Function> {
        if j.as_array().is_some() {
            return Ok(Function { condition: None, kind: FunctionKind::Sequence(Function::parse_list(p, j)?) });
        }
        Function::parse_typed(p, j)
    }

    /// `LootItemFunctions.LIST_CODEC`: references or typed objects.
    fn parse_list(p: &Parser, j: &Json) -> PResult<Vec<Ref<Function>>> {
        p.holder_list(j, Kind::Modifier, Function::parse_typed)
    }

    fn parse_typed(p: &Parser, j: &Json) -> PResult<Function> {
        let ty = req(j, "type", ident)?;
        let condition = opt(j, "condition", |v| Condition::parse_ref(p, v))?;
        let int_ref = |key: &str| req(j, key, |v| IntProvider::parse_ref(p, v));
        let float_ref = |key: &str| req(j, key, |v| FloatProvider::parse_ref(p, v));
        let fn_ref = |key: &str| opt(j, key, |v| Function::parse_ref(p, v));
        let ench = |key: &str| req(j, key, |v| p.id(v, registry::ENCHANTMENT));
        let kind = match ty.as_str() {
            "minecraft:set_count" => FunctionKind::SetCount { count: int_ref("count")?, add: opt_or(j, "add", false, boolean)? },
            "minecraft:set_item" => FunctionKind::SetItem(req(j, "item", |v| p.id(v, registry::ITEM))?),
            "minecraft:enchant_with_levels" => FunctionKind::EnchantWithLevels {
                levels: int_ref("levels")?,
                options: opt(j, "options", |v| p.id_set(v, registry::ENCHANTMENT))?,
                include_additional_cost_component: opt_or(j, "include_additional_cost_component", false, boolean)?,
            },
            "minecraft:enchant_randomly" => FunctionKind::EnchantRandomly {
                options: opt(j, "options", |v| p.id_set(v, registry::ENCHANTMENT))?,
                only_compatible: opt_or(j, "only_compatible", true, boolean)?,
                include_additional_cost_component: opt_or(j, "include_additional_cost_component", false, boolean)?,
            },
            "minecraft:set_enchantments" => FunctionKind::SetEnchantments {
                enchantments: opt_or(j, "enchantments", Vec::new(), |v| {
                    obj(v)?
                        .iter()
                        .map(|(k, n)| {
                            let id = crate::parse::registry_id(
                                registry::ENCHANTMENT,
                                &Identifier::parse(k).ok_or_else(|| ParseError::new(format!("invalid identifier {k:?}")))?,
                            )?;
                            Ok((id, IntProvider::parse_ref(p, n).map_err(|e| e.at(k))?))
                        })
                        .collect()
                })?,
                add: opt_or(j, "add", false, boolean)?,
            },
            "minecraft:set_custom_data" => FunctionKind::SetCustomData(req(j, "tag", |v| match v {
                Json::Str(s) => value(&Json::Str(s.clone()), |x| comp::NbtPredicate::from_value(x).map(|n| n.0)),
                Json::Obj(_) => Ok(v.to_value().to_nbt()),
                _ => fail("expected a compound or SNBT"),
            })?),
            "minecraft:set_components" => {
                FunctionKind::SetComponents(req(j, "components", |v| value(v, DataComponentPatch::from_value_strict))?)
            }
            "minecraft:furnace_smelt" => {
                FunctionKind::FurnaceSmelt { use_input_count: opt_or(j, "use_input_count", true, boolean)? }
            }
            "minecraft:enchanted_count_increase" => FunctionKind::EnchantedCountIncrease {
                enchantment: ench("enchantment")?,
                count: float_ref("count")?,
                limit: opt_or(j, "limit", 0, int)?,
            },
            "minecraft:set_damage" => FunctionKind::SetDamage { damage: float_ref("damage")?, add: opt_or(j, "add", false, boolean)? },
            "minecraft:set_attributes" => FunctionKind::SetAttributes {
                modifiers: req(j, "modifiers", |v| {
                    list(v, |m| {
                        Ok(AttributeModifierSpec {
                            id: req(m, "id", ident)?,
                            attribute: req(m, "attribute", |a| p.id(a, registry::ATTRIBUTE))?,
                            operation: req(m, "operation", |o| value(o, AttributeOperation::from_value))?,
                            amount: req(m, "amount", |a| FloatProvider::parse_ref(p, a))?,
                            slots: req(m, "slot", |s| match s {
                                Json::Arr(_) => {
                                    let v = list(s, |e| value(e, EquipmentSlotGroup::from_value))?;
                                    if v.is_empty() {
                                        return fail("empty slot list");
                                    }
                                    Ok(v)
                                }
                                _ => Ok(vec![value(s, EquipmentSlotGroup::from_value)?]),
                            })?,
                        })
                    })
                })?,
                replace: opt_or(j, "replace", true, boolean)?,
            },
            "minecraft:set_name" => FunctionKind::SetName {
                name: opt(j, "name", text)?,
                entity: opt(j, "entity", entity_target)?,
                item_name: match opt(j, "target", string)?.as_deref() {
                    None | Some("custom_name") => false,
                    Some("item_name") => true,
                    Some(other) => return fail(format!("unknown name target {other}")).map_err(|e: ParseError| e.at("target")),
                },
            },
            "minecraft:exploration_map" => FunctionKind::ExplorationMap {
                destination: req(j, "destination", |v| p.name_set(v, "minecraft:worldgen/structure"))?,
                decoration: opt(j, "decoration", |v| p.id(v, registry::MAP_DECORATION_TYPE))?
                    .map_or_else(|| crate::parse::registry_id(registry::MAP_DECORATION_TYPE, &Identifier::new_unchecked("minecraft:mansion")), Ok)?,
                zoom: opt_or(j, "zoom", 2, |v| int(v).map(|z| z as i8))?,
                search_radius: opt_or(j, "search_radius", 50, int)?,
                skip_existing_chunks: opt_or(j, "skip_existing_chunks", true, boolean)?,
            },
            "minecraft:set_stew_effect" => FunctionKind::SetStewEffect(opt_or(j, "effects", Vec::new(), |v| {
                let effects = list(v, |e| {
                    Ok((req(e, "type", |t| p.id(t, registry::MOB_EFFECT))?, req(e, "duration", |d| IntProvider::parse_ref(p, d))?))
                })?;
                for (i, (e, _)) in effects.iter().enumerate() {
                    if effects[..i].iter().any(|(x, _)| x == e) {
                        return fail(format!("duplicate effect {}", registry::MOB_EFFECT.name(*e).unwrap_or("?")));
                    }
                }
                Ok(effects)
            })?),
            "minecraft:copy_name" => FunctionKind::CopyName(req(j, "source", |v| {
                let s = string(v)?;
                match Source::by_name(&s) {
                    Some(src @ (Source::Entity(_) | Source::BlockEntity)) => Ok(src),
                    _ => fail(format!("unknown name source {s:?}")),
                }
            })?),
            "minecraft:set_contents" => FunctionKind::SetContents {
                component: req(j, "component", container_kind)?,
                entries: req(j, "entries", |v| list(v, |e| Entry::parse(p, e)))?,
            },
            "minecraft:modify_contents" => FunctionKind::ModifyContents {
                component: req(j, "component", container_kind)?,
                modifier: req(j, "modifier", |v| Function::parse_ref(p, v))?,
            },
            "minecraft:filtered" => FunctionKind::Filtered {
                filter: req(j, "item_filter", item_predicate)?,
                on_pass: fn_ref("on_pass")?,
                on_fail: fn_ref("on_fail")?,
            },
            "minecraft:limit_count" => {
                let (min, max) = req(j, "limit", |v| {
                    obj(v)?;
                    Ok((opt(v, "min", |x| IntProvider::parse_ref(p, x))?, opt(v, "max", |x| IntProvider::parse_ref(p, x))?))
                })?;
                FunctionKind::LimitCount { min, max }
            }
            "minecraft:apply_bonus" => {
                let enchantment = ench("enchantment")?;
                let formula = req(j, "formula", ident)?;
                let formula = match formula.as_str() {
                    "minecraft:ore_drops" => BonusFormula::OreDrops,
                    "minecraft:uniform_bonus_count" => BonusFormula::UniformBonusCount {
                        bonus_multiplier: req(j, "parameters", |v| req(v, "bonusMultiplier", int))?,
                    },
                    "minecraft:binomial_with_bonus_count" => {
                        let (extra, probability) = req(j, "parameters", |v| {
                            Ok((req(v, "extra", int)?, req(v, "probability", crate::parse::float)?))
                        })?;
                        BonusFormula::BinomialWithBonusCount { extra, probability }
                    }
                    other => return fail(format!("unknown formula {other}")).map_err(|e: ParseError| e.at("formula")),
                };
                FunctionKind::ApplyBonus { enchantment, formula }
            }
            "minecraft:set_loot_table" => FunctionKind::SetLootTable {
                table: req(j, "loot_table_id", |v| {
                    let id = ident(v)?;
                    if p.names.index(Kind::Table, &id).is_none() {
                        return fail(format!("unknown loot_table {id}"));
                    }
                    Ok(id)
                })?,
                seed: opt_or(j, "seed", 0, long)?,
            },
            "minecraft:explosion_decay" => FunctionKind::ExplosionDecay,
            "minecraft:set_lore" => FunctionKind::SetLore {
                lore: req(j, "lore", |v| {
                    let l = list(v, text)?;
                    if l.len() > 256 {
                        return fail("more than 256 lore lines");
                    }
                    Ok(l)
                })?,
                mode: ListOperation::parse(j, 256)?,
                entity: opt(j, "entity", entity_target)?,
            },
            "minecraft:fill_player_head" => FunctionKind::FillPlayerHead(req(j, "entity", entity_target)?),
            "minecraft:copy_custom_data" => FunctionKind::CopyCustomData {
                source: req(j, "source", |v| match v {
                    Json::Str(s) => Source::by_name(s)
                        .filter(|s| matches!(s, Source::Entity(_) | Source::BlockEntity))
                        .map(NbtSource::Context)
                        .ok_or_else(|| ParseError::new(format!("unknown nbt source {s:?}"))),
                    _ => match req(v, "type", ident)?.as_str() {
                        "minecraft:storage" => Ok(NbtSource::Storage(req(v, "source", ident)?)),
                        "minecraft:context" => req(v, "target", |t| {
                            let s = string(t)?;
                            Source::by_name(&s)
                                .filter(|s| matches!(s, Source::Entity(_) | Source::BlockEntity))
                                .map(NbtSource::Context)
                                .ok_or_else(|| ParseError::new(format!("unknown nbt source {s:?}")))
                        }),
                        other => fail(format!("unknown loot nbt provider type {other}")),
                    },
                })?,
                ops: req(j, "ops", |v| {
                    list(v, |o| {
                        Ok(CopyOperation {
                            source: req(o, "source", |s| parse_nbt_path(&string(s)?))?,
                            target: req(o, "target", |s| parse_nbt_path(&string(s)?))?,
                            op: req(o, "op", |s| match string(s)?.as_str() {
                                "replace" => Ok(MergeStrategy::Replace),
                                "append" => Ok(MergeStrategy::Append),
                                "merge" => Ok(MergeStrategy::Merge),
                                other => fail(format!("unknown merge strategy {other}")),
                            })?,
                        })
                    })
                })?,
            },
            "minecraft:copy_state" => {
                let block = req(j, "block", |v| p.id(v, registry::BLOCK))?;
                let name = registry::BLOCK.name(block).unwrap_or("minecraft:air");
                let info = kiln_data::blocks_types::block_by_name(name);
                let properties = req(j, "properties", |v| list(v, string))?
                    .into_iter()
                    .filter(|n| info.is_some_and(|b| b.properties.iter().any(|p| p.name == n)))
                    .collect();
                FunctionKind::CopyState { properties }
            }
            "minecraft:set_banner_pattern" => FunctionKind::SetBannerPattern {
                patterns: req(j, "patterns", |v| value(v, <BannerPatternLayers as kiln_item::ComponentValue>::from_value))?,
                append: req(j, "append", boolean)?,
            },
            "minecraft:set_potion" => FunctionKind::SetPotion(req(j, "id", |v| p.id(v, registry::POTION))?),
            "minecraft:set_random_dyes" => FunctionKind::SetRandomDyes(int_ref("number_of_dyes")?),
            "minecraft:set_random_potion" => {
                FunctionKind::SetRandomPotion(opt(j, "options", |v| p.id_set(v, registry::POTION))?)
            }
            "minecraft:set_instrument" => FunctionKind::SetInstrument(req(j, "options", |v| p.id_set(v, registry::INSTRUMENT))?),
            "minecraft:sequence" => FunctionKind::Sequence(req(j, "functions", |v| Function::parse_list(p, v))?),
            "minecraft:copy_components" => FunctionKind::CopyComponents {
                source: req(j, "source", |v| {
                    let s = string(v)?;
                    Source::by_name(&s).ok_or_else(|| ParseError::new(format!("unknown component source {s:?}")))
                })?,
                include: opt(j, "include", |v| list(v, component_type))?,
                exclude: opt(j, "exclude", |v| list(v, component_type))?,
            },
            "minecraft:set_fireworks" => FunctionKind::SetFireworks {
                explosions: opt(j, "explosions", |v| {
                    stand_alone(v, 256, |e| value(e, <FireworkExplosion as kiln_item::ComponentValue>::from_value))
                })?,
                flight_duration: opt(j, "flight_duration", |v| {
                    let n = int(v)?;
                    if !(0..=255).contains(&n) {
                        return fail(format!("value out of range [0;255]: {n}"));
                    }
                    Ok(n)
                })?,
            },
            "minecraft:set_firework_explosion" => FunctionKind::SetFireworkExplosion {
                shape: opt(j, "shape", |v| value(v, FireworkShape::from_value))?,
                colors: opt(j, "colors", |v| value(v, |x| x.as_int_stream()))?,
                fade_colors: opt(j, "fade_colors", |v| value(v, |x| x.as_int_stream()))?,
                trail: opt(j, "trail", boolean)?,
                twinkle: opt(j, "twinkle", boolean)?,
            },
            "minecraft:set_book_cover" => FunctionKind::SetBookCover {
                title: opt(j, "title", |v| filterable(v, string))?,
                author: opt(j, "author", string)?,
                generation: opt(j, "generation", |v| {
                    let n = int(v)?;
                    if !(0..=3).contains(&n) {
                        return fail(format!("value out of range [0;3]: {n}"));
                    }
                    Ok(n)
                })?,
            },
            "minecraft:set_written_book_pages" => FunctionKind::SetWrittenBookPages {
                pages: req(j, "pages", |v| list(v, |pg| filterable(pg, text)))?,
                mode: ListOperation::parse(j, 100)?,
            },
            "minecraft:set_writable_book_pages" => FunctionKind::SetWritableBookPages {
                pages: req(j, "pages", |v| list(v, |pg| filterable(pg, string)))?,
                mode: ListOperation::parse(j, 100)?,
            },
            "minecraft:toggle_tooltips" => FunctionKind::ToggleTooltips(req(j, "toggles", |v| {
                obj(v)?
                    .iter()
                    .map(|(k, b)| {
                        let id = comp::by_name(k).ok_or_else(|| ParseError::new(format!("unknown data component type {k}")))?;
                        Ok((id, boolean(b).map_err(|e| e.at(k))?))
                    })
                    .collect()
            })?),
            "minecraft:set_ominous_bottle_amplifier" => FunctionKind::SetOminousBottleAmplifier(int_ref("amplifier")?),
            "minecraft:set_custom_model_data" => FunctionKind::SetCustomModelData {
                floats: opt(j, "floats", |v| stand_alone(v, usize::MAX, |e| FloatProvider::parse_ref(p, e)))?,
                flags: opt(j, "flags", |v| stand_alone(v, usize::MAX, boolean))?,
                strings: opt(j, "strings", |v| stand_alone(v, usize::MAX, string))?,
                colors: opt(j, "colors", |v| {
                    stand_alone(v, usize::MAX, |e| match e {
                        // `ExtraCodecs.RGB_COLOR_CODEC`'s alternative: `[r, g, b]` floats, made
                        // opaque (`ARGB.colorFromFloat(1, r, g, b)`, channels floored).
                        Json::Arr(c) if c.len() == 3 && c.iter().all(Json::is_number) => {
                            let ch = |i: usize| kiln_javamath::math::floor_f32(c[i].as_f32().unwrap_or(0.0) * 255.0) & 0xFF;
                            Ok(Ref::direct(IntProvider::Constant((0xFF << 24) | ch(0) << 16 | ch(1) << 8 | ch(2))))
                        }
                        _ => IntProvider::parse_ref(p, e),
                    })
                })?,
            },
            "minecraft:discard" => FunctionKind::Discard,
            other => return fail(format!("unknown loot function type {other}")),
        };
        Ok(Function { condition, kind })
    }
}

/// Mob effects that apply at once (`MobEffect.isInstantaneous`).
fn instantaneous(effect: i32) -> bool {
    matches!(
        registry::MOB_EFFECT.name(effect),
        Some("minecraft:instant_health" | "minecraft:instant_damage" | "minecraft:saturation")
    )
}

/// `DyeColor.getTextureDiffuseColor()` in `DyeColor.VALUES` order.
const DYE_COLORS: [i32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97, 0x169C9C, 0x8932B8, 0x3C44AA,
    0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

/// `DyedItemColor.applyDyes(DyedItemColor, List<DyeColor>)`.
fn mix_dyes(old: Option<i32>, dyes: &[i32]) -> i32 {
    let (mut r, mut g, mut b, mut total, mut count) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let mut add = |rgb: i32| {
        let (rr, gg, bb) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
        total += rr.max(gg.max(bb));
        r += rr;
        g += gg;
        b += bb;
        count += 1;
    };
    if let Some(rgb) = old {
        add(rgb);
    }
    for &d in dyes {
        add(d);
    }
    let (ar, ag, ab) = (r / count, g / count, b / count);
    let avg_max = total as f32 / count as f32;
    let cur_max = ar.max(ag.max(ab)) as f32;
    let scale = |c: i32| (c as f32 * avg_max / cur_max) as i32;
    ((scale(ar) & 0xFF) << 16) | ((scale(ag) & 0xFF) << 8) | (scale(ab) & 0xFF)
}

impl Eval<'_> {
    /// `LootItemFunction.apply`.
    pub fn apply_fn(&mut self, f: &Ref<Function>, stack: ItemStack) -> ItemStack {
        let data = self.data;
        let f: &Function = match f {
            Ref::Direct(v) => v,
            Ref::Named(i) => match data.modifiers.get(*i) {
                Some(v) => v,
                None => return stack,
            },
        };
        if let Some(c) = &f.condition
            && !self.test(c)
        {
            return stack;
        }
        self.run(&f.kind, stack)
    }

    fn run(&mut self, kind: &FunctionKind, mut stack: ItemStack) -> ItemStack {
        let data = self.data;
        match kind {
            FunctionKind::SetCount { count, add } => {
                let base = if *add { stack.count() } else { 0 };
                let n = self.int(count);
                stack.set_count(base.wrapping_add(n));
                stack
            }
            FunctionKind::SetItem(item) => stack::transmute_copy(&stack, *item),
            FunctionKind::EnchantWithLevels { levels, options, include_additional_cost_component } => {
                let levels = self.int(levels);
                let candidates = data.enchantment_candidates(options.as_ref());
                let out = crate::enchant::enchant_item(self.rng, stack, levels, &candidates);
                let mut out = out;
                if *include_additional_cost_component
                    && self.ctx.additional_cost_component_allowed()
                    && !out.is_empty()
                    && levels > 0
                {
                    out.insert(keys::ADDITIONAL_TRADE_COST, levels);
                }
                out
            }
            FunctionKind::EnchantRandomly { options, only_compatible, include_additional_cost_component } => {
                let book = stack.item_name() == "minecraft:book";
                let only_compatible = !book && *only_compatible;
                let candidates: Vec<_> = data
                    .enchantment_candidates(options.as_ref())
                    .into_iter()
                    .filter(|e| !only_compatible || e.can_enchant(&stack))
                    .collect();
                if candidates.is_empty() {
                    return stack;
                }
                let pick = candidates[self.rng.bounded(candidates.len() as i32) as usize];
                let level = self.rng.next_int_between(pick.min_level(), pick.max_level);
                let mut out = if book { ItemStack::of("minecraft:enchanted_book", 1).expect("enchanted_book") } else { stack };
                crate::enchant::enchant(&mut out, pick.id, level);
                if *include_additional_cost_component && self.ctx.additional_cost_component_allowed() {
                    let cost = 2 + self.rng.bounded(5 + level * 10) + 3 * level;
                    out.insert(keys::ADDITIONAL_TRADE_COST, cost);
                }
                out
            }
            FunctionKind::SetEnchantments { enchantments, add } => {
                if stack.item_name() == "minecraft:book" {
                    stack = stack::transmute_copy(&stack, registry::ITEM.id("minecraft:enchanted_book").expect("enchanted_book"));
                }
                let mut levels = Vec::with_capacity(enchantments.len());
                for (e, n) in enchantments {
                    levels.push((*e, n));
                }
                // Vanilla iterates an immutable hash map here; with random levels its order is
                // not reproducible, so kiln keeps file order.
                let mut resolved = Vec::with_capacity(levels.len());
                for (e, n) in levels {
                    resolved.push((e, self.int(n)));
                }
                crate::enchant::update(&mut stack, |m| {
                    for (e, n) in resolved {
                        let level = if *add { m.level(e).wrapping_add(n) } else { n };
                        m.set(e, level.clamp(0, 255));
                    }
                });
                stack
            }
            FunctionKind::SetCustomData(tag) => {
                let mut current = stack.get(keys::CUSTOM_DATA).map(|c| c.0.clone()).unwrap_or(Tag::Compound(Vec::new()));
                stack::merge_compound(&mut current, tag);
                stack::set_custom_data(&mut stack, current);
                stack
            }
            FunctionKind::SetComponents(patch) => {
                stack::apply_patch(&mut stack, patch);
                stack
            }
            FunctionKind::FurnaceSmelt { use_input_count } => {
                if stack.is_empty() {
                    return stack;
                }
                match self.ctx.smelt(&stack) {
                    Some(result) if !result.is_empty() => {
                        let n = if *use_input_count { stack.count() } else { 1 } * result.count();
                        result.with_count(n.min(result.max_stack_size()))
                    }
                    _ => stack,
                }
            }
            FunctionKind::EnchantedCountIncrease { enchantment, count, limit } => {
                let level = self.entity_enchantment_level(EntityTarget::Attacker, *enchantment);
                if level == 0 {
                    return stack;
                }
                let f = level as f32 * self.float(count);
                // `ItemStack.grow`: `setCount(getCount() + n)`, where an empty stack (count <= 0)
                // counts as 0.
                stack.set_count(stack.count().wrapping_add(crate::number::java_round(f)));
                if *limit > 0 {
                    stack::limit_size(&mut stack, *limit);
                }
                stack
            }
            FunctionKind::SetDamage { damage, add } => {
                if stack.is_damageable_item() {
                    let max = stack.max_damage();
                    let f = if *add { 1.0 - stack.damage() as f32 / max as f32 } else { 0.0 };
                    let g = 1.0 - kiln_javamath::math::clamp(self.float(damage) + f, 0.0, 1.0);
                    let value = kiln_javamath::math::floor_f32(g * max as f32);
                    stack.insert(keys::DAMAGE, value.clamp(0, max.max(0)));
                }
                stack
            }
            FunctionKind::SetAttributes { modifiers, replace } => {
                let mut current = if *replace {
                    AttributeModifiers::default()
                } else {
                    stack.get(keys::ATTRIBUTE_MODIFIERS).cloned().unwrap_or_default()
                };
                for m in modifiers {
                    let slot = m.slots[self.rng.bounded(m.slots.len() as i32) as usize];
                    let amount = self.float(&m.amount) as f64;
                    current.0.retain(|e| !(e.attribute == m.attribute && e.id == m.id));
                    current.0.push(AttributeModifier {
                        attribute: m.attribute,
                        id: m.id.clone(),
                        amount,
                        operation: m.operation,
                        slot,
                        display: AttributeDisplay::Default,
                    });
                }
                stack.insert(keys::ATTRIBUTE_MODIFIERS, current);
                stack
            }
            FunctionKind::SetName { name, entity, item_name } => {
                if let Some(name) = name {
                    let resolved = match entity {
                        Some(t) if self.ctx.has_entity(*t) => self.ctx.resolve_text(*t, name),
                        _ => name.clone(),
                    };
                    if *item_name { stack.insert(keys::ITEM_NAME, resolved) } else { stack.insert(keys::CUSTOM_NAME, resolved) }
                }
                stack
            }
            FunctionKind::ExplorationMap { destination, decoration, zoom, search_radius, skip_existing_chunks } => {
                if stack.is_empty() || self.ctx.origin().is_none() {
                    return stack;
                }
                let request = ExplorationMap {
                    destination,
                    decoration: *decoration,
                    zoom: *zoom,
                    search_radius: *search_radius,
                    skip_existing_chunks: *skip_existing_chunks,
                };
                self.ctx.exploration_map(&stack, &request).unwrap_or(stack)
            }
            FunctionKind::SetStewEffect(effects) => {
                if stack.item_name() != "minecraft:suspicious_stew" || effects.is_empty() {
                    return stack;
                }
                let (effect, duration) = &effects[self.rng.bounded(effects.len() as i32) as usize];
                let mut duration = self.int(duration);
                if !instantaneous(*effect) {
                    duration = duration.wrapping_mul(20);
                }
                let mut current = stack.get(keys::SUSPICIOUS_STEW_EFFECTS).cloned().unwrap_or(SuspiciousStewEffects(Vec::new()));
                current.0.push(StewEffect { effect: *effect, duration });
                stack.insert(keys::SUSPICIOUS_STEW_EFFECTS, current);
                stack
            }
            FunctionKind::CopyName(source) => {
                if let Some(name) = self.ctx.custom_name(*source) {
                    match name {
                        Some(n) => stack.insert(keys::CUSTOM_NAME, n),
                        None => stack.remove(ids::CUSTOM_NAME),
                    }
                }
                stack
            }
            FunctionKind::SetContents { component, entries } => {
                if stack.is_empty() {
                    return stack;
                }
                let mut items = Vec::new();
                for e in entries {
                    let mut expanded = Vec::new();
                    self.expand_entry(e, &mut expanded);
                    for pe in expanded {
                        self.create_items(&pe, &mut |ev: &mut Eval<'_>, s| split_into(ev.ctx, s, &mut items));
                    }
                }
                stack::set_container_contents(&mut stack, *component, items);
                stack
            }
            FunctionKind::ModifyContents { component, modifier } => {
                if stack.is_empty() {
                    return stack;
                }
                let Some(items) = stack::container_contents(&stack, *component) else { return stack };
                // `modifyItems`: empty slots stay empty, modified stacks are limited to their size.
                let modified: Vec<ItemStack> = items
                    .into_iter()
                    .map(|i| {
                        if i.is_empty() {
                            return i;
                        }
                        let mut s = self.apply_fn(modifier, i);
                        let max = s.max_stack_size();
                        stack::limit_size(&mut s, max);
                        s
                    })
                    .collect();
                stack::set_container_contents(&mut stack, *component, modified);
                stack
            }
            FunctionKind::Filtered { filter, on_pass, on_fail } => {
                let branch = if predicate::item_matches(&data.tags, filter, &stack) { on_pass } else { on_fail };
                match branch {
                    Some(f) => self.apply_fn(f, stack),
                    None => stack,
                }
            }
            FunctionKind::LimitCount { min, max } => {
                let mut n = stack.count();
                match (min, max) {
                    (None, None) => {}
                    (Some(min), None) => n = self.int(min).max(n),
                    (None, Some(max)) => n = self.int(max).min(n),
                    (Some(min), Some(max)) => {
                        let lo = self.int(min);
                        let hi = self.int(max);
                        n = n.max(lo).min(hi);
                    }
                }
                stack.set_count(n);
                stack
            }
            FunctionKind::ApplyBonus { enchantment, formula } => {
                let Some(tool) = self.ctx.tool() else { return stack };
                let level = crate::enchant::item_level(tool, *enchantment);
                let count = stack.count();
                let n = match *formula {
                    BonusFormula::OreDrops => {
                        if level > 0 {
                            let bonus = (self.rng.bounded(level.wrapping_add(2)) - 1).max(0);
                            count.wrapping_mul(bonus + 1)
                        } else {
                            count
                        }
                    }
                    BonusFormula::UniformBonusCount { bonus_multiplier } => {
                        count.wrapping_add(self.rng.bounded(bonus_multiplier.wrapping_mul(level).wrapping_add(1)))
                    }
                    BonusFormula::BinomialWithBonusCount { extra, probability } => {
                        let mut n = count;
                        for _ in 0..level.wrapping_add(extra) {
                            if self.rng.next_float() < probability {
                                n += 1;
                            }
                        }
                        n
                    }
                };
                stack.set_count(n);
                stack
            }
            FunctionKind::SetLootTable { table, seed } => {
                if stack.is_empty() {
                    return stack;
                }
                stack.insert(keys::CONTAINER_LOOT, SeededContainerLoot { loot_table: table.clone(), seed: *seed });
                stack
            }
            FunctionKind::ExplosionDecay => {
                if let Some(radius) = self.ctx.explosion_radius() {
                    let chance = 1.0 / radius;
                    let count = stack.count();
                    let mut kept = 0;
                    for _ in 0..count {
                        if self.rng.next_float() <= chance {
                            kept += 1;
                        }
                    }
                    stack.set_count(kept);
                }
                stack
            }
            FunctionKind::SetLore { lore, mode, entity } => {
                let old = stack.get(keys::LORE).cloned().unwrap_or_default();
                let resolved: Vec<Text> = match entity {
                    Some(t) if self.ctx.has_entity(*t) => lore.iter().map(|l| self.ctx.resolve_text(*t, l)).collect(),
                    _ => lore.clone(),
                };
                stack.insert(keys::LORE, Lore(mode.apply(&old.0, &resolved, 256)));
                stack
            }
            FunctionKind::FillPlayerHead(target) => {
                if stack.item_name() == "minecraft:player_head"
                    && let Some(profile) = self.ctx.player_profile(*target)
                {
                    stack.insert(keys::PROFILE, profile);
                }
                stack
            }
            FunctionKind::CopyCustomData { source, ops } => {
                let src = match source {
                    NbtSource::Storage(id) => Some(self.ctx.storage(id).unwrap_or(Tag::Compound(Vec::new()))),
                    NbtSource::Context(s) => self.ctx.nbt(*s),
                };
                let Some(src) = src else { return stack };
                let mut target: Option<Tag> = None;
                for op in ops {
                    let found = op.source.get(&src);
                    if found.is_empty() {
                        continue;
                    }
                    let values: Vec<Tag> = found.into_iter().cloned().collect();
                    let t = target.get_or_insert_with(|| {
                        stack.get(keys::CUSTOM_DATA).map(|c| c.0.clone()).unwrap_or(Tag::Compound(Vec::new()))
                    });
                    stack::apply_copy(t, &op.target, op.op, &values);
                }
                if let Some(t) = target {
                    stack::set_custom_data(&mut stack, t);
                }
                stack
            }
            FunctionKind::CopyState { properties } => {
                let Some(state) = self.ctx.block_state() else { return stack };
                let block = kiln_data::blocks_types::block_of(state);
                let mut current = stack.get(keys::BLOCK_STATE).cloned().unwrap_or(BlockItemStateProperties(Vec::new()));
                for name in properties {
                    if let Some(v) = block.property(state, name) {
                        match current.0.iter_mut().find(|(k, _)| k == name) {
                            Some(slot) => slot.1 = v.to_owned(),
                            None => current.0.push((name.clone(), v.to_owned())),
                        }
                    }
                }
                stack.insert(keys::BLOCK_STATE, current);
                stack
            }
            FunctionKind::SetBannerPattern { patterns, append } => {
                if *append {
                    let mut current = stack.get(keys::BANNER_PATTERNS).cloned().unwrap_or(BannerPatternLayers(Vec::new()));
                    current.0.extend(patterns.0.iter().cloned());
                    stack.insert(keys::BANNER_PATTERNS, current);
                } else {
                    stack.insert(keys::BANNER_PATTERNS, patterns.clone());
                }
                stack
            }
            FunctionKind::SetPotion(potion) => {
                let mut current = stack.get(keys::POTION_CONTENTS).cloned().unwrap_or_else(empty_potion);
                current.potion = Some(*potion);
                stack.insert(keys::POTION_CONTENTS, current);
                stack
            }
            FunctionKind::SetRandomDyes(n) => {
                let n = self.int(n);
                if n <= 0 {
                    return stack;
                }
                let dyes: Vec<i32> = (0..n).map(|_| DYE_COLORS[self.rng.bounded(16) as usize]).collect();
                let old = stack.get(keys::DYED_COLOR).map(|c| c.0);
                let mut out = stack.with_count(1);
                out.insert(keys::DYED_COLOR, DyedColor(mix_dyes(old, &dyes)));
                out
            }
            FunctionKind::SetRandomPotion(options) => {
                let pick = match options {
                    Some(set) if !set.ids().is_empty() => Some(set.ids()[self.rng.bounded(set.ids().len() as i32) as usize]),
                    Some(_) => None,
                    None => {
                        let n = registry::POTION.len() as i32;
                        (n > 0).then(|| self.rng.bounded(n))
                    }
                };
                if let Some(potion) = pick {
                    let mut current = stack.get(keys::POTION_CONTENTS).cloned().unwrap_or_else(empty_potion);
                    current.potion = Some(potion);
                    stack.insert(keys::POTION_CONTENTS, current);
                }
                stack
            }
            FunctionKind::SetInstrument(options) => {
                if !options.ids().is_empty() {
                    let id = options.ids()[self.rng.bounded(options.ids().len() as i32) as usize];
                    stack.insert(keys::INSTRUMENT, InstrumentComponent(Holder::Reference(id)));
                }
                stack
            }
            FunctionKind::Sequence(functions) => {
                for f in functions {
                    stack = self.apply_fn(f, stack);
                }
                stack
            }
            FunctionKind::CopyComponents { source, include, exclude } => {
                let Some(components) = self.ctx.components(*source) else { return stack };
                let excluded = |id: ComponentId| exclude.as_ref().is_some_and(|e| e.contains(&id));
                if self.ctx.components_are_map(*source) {
                    for c in components {
                        if include.as_ref().is_none_or(|i| i.contains(&c.id())) && !excluded(c.id()) {
                            stack.set(c);
                        }
                    }
                } else {
                    let order: Vec<ComponentId> = match include {
                        Some(i) => i.clone(),
                        None => (0..comp::count() as ComponentId).collect(),
                    };
                    for id in order {
                        if excluded(id) {
                            continue;
                        }
                        if let Some(c) = components.iter().find(|c| c.id() == id) {
                            stack.set(c.clone());
                        }
                    }
                }
                stack
            }
            FunctionKind::SetFireworks { explosions, flight_duration } => {
                let current = stack.get(keys::FIREWORKS).cloned().unwrap_or(Fireworks { flight_duration: 0, explosions: Vec::new() });
                let updated = Fireworks {
                    flight_duration: flight_duration.unwrap_or(current.flight_duration),
                    explosions: match explosions {
                        Some((new, op)) => op.apply(&current.explosions, new, 256),
                        None => current.explosions.clone(),
                    },
                };
                stack.insert(keys::FIREWORKS, updated);
                stack
            }
            FunctionKind::SetFireworkExplosion { shape, colors, fade_colors, trail, twinkle } => {
                let current = stack.get(keys::FIREWORK_EXPLOSION).cloned().unwrap_or(FireworkExplosion {
                    shape: FireworkShape::SmallBall,
                    colors: Vec::new(),
                    fade_colors: Vec::new(),
                    has_trail: false,
                    has_twinkle: false,
                });
                let updated = FireworkExplosion {
                    shape: shape.unwrap_or(current.shape),
                    colors: colors.clone().unwrap_or(current.colors),
                    fade_colors: fade_colors.clone().unwrap_or(current.fade_colors),
                    has_trail: trail.unwrap_or(current.has_trail),
                    has_twinkle: twinkle.unwrap_or(current.has_twinkle),
                };
                stack.insert(keys::FIREWORK_EXPLOSION, updated);
                stack
            }
            FunctionKind::SetBookCover { title, author, generation } => {
                let current = stack.get(keys::WRITTEN_BOOK_CONTENT).cloned().unwrap_or_else(empty_written_book);
                let updated = WrittenBookContent {
                    title: title.clone().unwrap_or(current.title),
                    author: author.clone().unwrap_or(current.author),
                    generation: generation.unwrap_or(current.generation),
                    pages: current.pages,
                    resolved: current.resolved,
                };
                stack.insert(keys::WRITTEN_BOOK_CONTENT, updated);
                stack
            }
            FunctionKind::SetWrittenBookPages { pages, mode } => {
                let current = stack.get(keys::WRITTEN_BOOK_CONTENT).cloned().unwrap_or_else(empty_written_book);
                let new_pages = mode.apply(&current.pages, pages, usize::MAX);
                // `withReplacedPages` resets `resolved`.
                let updated = WrittenBookContent { pages: new_pages, resolved: false, ..current };
                stack.insert(keys::WRITTEN_BOOK_CONTENT, updated);
                stack
            }
            FunctionKind::SetWritableBookPages { pages, mode } => {
                let current = stack.get(keys::WRITABLE_BOOK_CONTENT).cloned().unwrap_or(WritableBookContent { pages: Vec::new() });
                let updated = WritableBookContent { pages: mode.apply(&current.pages, pages, 100) };
                stack.insert(keys::WRITABLE_BOOK_CONTENT, updated);
                stack
            }
            FunctionKind::ToggleTooltips(toggles) => {
                let mut current =
                    stack.get(keys::TOOLTIP_DISPLAY).cloned().unwrap_or(TooltipDisplay { hide_tooltip: false, hidden_components: Vec::new() });
                for &(id, shown) in toggles {
                    let hidden = !shown;
                    let present = current.hidden_components.contains(&id);
                    if present != hidden {
                        if hidden {
                            current.hidden_components.push(id);
                        } else {
                            current.hidden_components.retain(|&x| x != id);
                        }
                    }
                }
                stack.insert(keys::TOOLTIP_DISPLAY, current);
                stack
            }
            FunctionKind::SetOminousBottleAmplifier(amplifier) => {
                let n = self.int(amplifier).clamp(0, 4);
                stack.insert(keys::OMINOUS_BOTTLE_AMPLIFIER, OminousBottleAmplifier(n));
                stack
            }
            FunctionKind::SetCustomModelData { floats, flags, strings, colors } => {
                let current = stack.get(keys::CUSTOM_MODEL_DATA).cloned().unwrap_or(CustomModelData {
                    floats: Vec::new(),
                    flags: Vec::new(),
                    strings: Vec::new(),
                    colors: Vec::new(),
                });
                let floats = match floats {
                    Some((values, op)) => {
                        let v: Vec<f32> = values.iter().map(|p| self.float(p)).collect();
                        op.apply(&current.floats, &v, usize::MAX)
                    }
                    None => current.floats,
                };
                let flags = match flags {
                    Some((values, op)) => op.apply(&current.flags, values, usize::MAX),
                    None => current.flags,
                };
                let strings = match strings {
                    Some((values, op)) => op.apply(&current.strings, values, usize::MAX),
                    None => current.strings,
                };
                let colors = match colors {
                    Some((values, op)) => {
                        let v: Vec<i32> = values.iter().map(|p| self.int(p)).collect();
                        op.apply(&current.colors, &v, usize::MAX)
                    }
                    None => current.colors,
                };
                stack.insert(keys::CUSTOM_MODEL_DATA, CustomModelData { floats, flags, strings, colors });
                stack
            }
            FunctionKind::Discard => ItemStack::empty(),
        }
    }
}

fn empty_potion() -> PotionContents {
    PotionContents { potion: None, custom_color: None, custom_effects: Vec::new(), custom_name: None }
}

fn empty_written_book() -> WrittenBookContent {
    WrittenBookContent {
        title: Filterable::pass_through(String::new()),
        author: String::new(),
        generation: 0,
        pages: Vec::new(),
        resolved: true,
    }
}

/// `LootTable.createStackSplitter` feeding a list.
fn split_into(ctx: &dyn crate::LootContext, stack: ItemStack, out: &mut Vec<ItemStack>) {
    stack::split_stack(ctx, stack, &mut |s| out.push(s));
}

