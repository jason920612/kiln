//! Loot conditions (`LootItemCondition`, datapack registry `predicate/`).

use crate::context::EntityTarget;
use crate::data::Kind;
use crate::eval::Eval;
use crate::json::Json;
use crate::number::{FloatProvider, IntProvider, LevelBasedValue, entity_target};
use crate::parse::{PResult, Parser, Ref, boolean, fail, float, ident, list, long, obj, opt, opt_or, req};
use crate::predicate::{self, BlockPredicate, DamageSourcePredicate, EntityPredicate, LocationPredicate};
use kiln_item::Identifier;
use kiln_item::component::ItemPredicate;
use kiln_item::registry;

/// `IntRangePredicate`: a single value, or optional inclusive bounds.
#[derive(Debug, Clone)]
pub enum IntRange {
    Point(Ref<IntProvider>),
    Line { min: Option<Ref<IntProvider>>, max: Option<Ref<IntProvider>> },
}

/// `FloatRangePredicate`.
#[derive(Debug, Clone)]
pub enum FloatRange {
    Point(Ref<FloatProvider>),
    Line { min: Option<Ref<FloatProvider>>, max: Option<Ref<FloatProvider>> },
}

#[derive(Debug, Clone)]
pub enum Condition {
    Inverted(Ref<Condition>),
    AnyOf(Vec<Ref<Condition>>),
    AllOf(Vec<Ref<Condition>>),
    RandomChance(Ref<FloatProvider>),
    RandomChanceWithEnchantedBonus { unenchanted_chance: f32, enchanted_chance: LevelBasedValue, enchantment: i32 },
    EntityProperties { target: EntityTarget, predicate: Option<Box<EntityPredicate>> },
    KilledByPlayer,
    EntityScores { target: EntityTarget, scores: Vec<(String, IntRange)> },
    MatchBlock(Box<BlockPredicate>),
    MatchTool(Option<Box<ItemPredicate>>),
    TableBonus { enchantment: i32, chances: Vec<f32> },
    SurvivesExplosion,
    DamageSourceProperties(Option<Box<DamageSourcePredicate>>),
    LocationCheck { predicate: Option<Box<LocationPredicate>>, offset: [i32; 3] },
    WeatherCheck { raining: Option<bool>, thundering: Option<bool> },
    TimeCheck { clock: Identifier, period: Option<i64>, value: IntRange },
    IntValueCheck { value: Ref<IntProvider>, range: IntRange },
    FloatValueCheck { value: Ref<FloatProvider>, range: FloatRange },
    EnchantmentActiveCheck(bool),
    EnvironmentAttributeCheck { attribute: Identifier, value: Json },
}

impl IntRange {
    /// `IntRangePredicate.CODEC`: `either(provider, {min, max})`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<IntRange> {
        match IntProvider::parse_ref(p, j) {
            Ok(v) => Ok(IntRange::Point(v)),
            Err(point_err) => {
                if j.as_object().is_none() {
                    return Err(point_err);
                }
                Ok(IntRange::Line {
                    min: opt(j, "min", |v| IntProvider::parse_ref(p, v))?,
                    max: opt(j, "max", |v| IntProvider::parse_ref(p, v))?,
                })
            }
        }
    }
}

impl FloatRange {
    pub fn parse(p: &Parser, j: &Json) -> PResult<FloatRange> {
        match FloatProvider::parse_ref(p, j) {
            Ok(v) => Ok(FloatRange::Point(v)),
            Err(point_err) => {
                if j.as_object().is_none() {
                    return Err(point_err);
                }
                Ok(FloatRange::Line {
                    min: opt(j, "min", |v| FloatProvider::parse_ref(p, v))?,
                    max: opt(j, "max", |v| FloatProvider::parse_ref(p, v))?,
                })
            }
        }
    }
}

impl Condition {
    /// `LootItemCondition.CODEC`: a reference to `predicate/` or an inline condition.
    pub fn parse_ref(p: &Parser, j: &Json) -> PResult<Ref<Condition>> {
        p.holder(j, Kind::Predicate, Condition::parse)
    }

    /// `LootItemCondition.LIST_CODEC` (a holder set: a list, or a single holder).
    pub fn parse_list(p: &Parser, j: &Json) -> PResult<Vec<Ref<Condition>>> {
        p.holder_list(j, Kind::Predicate, Condition::parse)
    }

    /// `LootItemCondition.DIRECT_CODEC`: dispatched on `type`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<Condition> {
        let ty = req(j, "type", ident)?;
        Ok(match ty.as_str() {
            "minecraft:inverted" => Condition::Inverted(req(j, "term", |v| Condition::parse_ref(p, v))?),
            "minecraft:any_of" => Condition::AnyOf(req(j, "terms", |v| Condition::parse_list(p, v))?),
            "minecraft:all_of" => Condition::AllOf(req(j, "terms", |v| Condition::parse_list(p, v))?),
            "minecraft:random_chance" => Condition::RandomChance(req(j, "chance", |v| FloatProvider::parse_ref(p, v))?),
            "minecraft:random_chance_with_enchanted_bonus" => {
                let unenchanted_chance = req(j, "unenchanted_chance", float)?;
                if !(0.0..=1.0).contains(&unenchanted_chance) {
                    return fail(format!("unenchanted_chance outside of range [0:1]: {unenchanted_chance}"));
                }
                Condition::RandomChanceWithEnchantedBonus {
                    unenchanted_chance,
                    enchanted_chance: req(j, "enchanted_chance", LevelBasedValue::parse)?,
                    enchantment: req(j, "enchantment", |v| p.id(v, registry::ENCHANTMENT))?,
                }
            }
            "minecraft:entity_properties" => Condition::EntityProperties {
                target: req(j, "entity", entity_target)?,
                predicate: opt(j, "predicate", |v| EntityPredicate::parse(p, v).map(Box::new))?,
            },
            "minecraft:killed_by_player" => Condition::KilledByPlayer,
            "minecraft:entity_scores" => Condition::EntityScores {
                target: req(j, "entity", entity_target)?,
                scores: req(j, "scores", |v| {
                    obj(v)?.iter().map(|(k, r)| Ok((k.clone(), IntRange::parse(p, r).map_err(|e| e.at(k))?))).collect()
                })?,
            },
            "minecraft:match_block" => Condition::MatchBlock(Box::new(BlockPredicate::parse(p, j)?)),
            "minecraft:match_tool" => Condition::MatchTool(opt(j, "predicate", |v| predicate::item_predicate(v).map(Box::new))?),
            "minecraft:table_bonus" => Condition::TableBonus {
                enchantment: req(j, "enchantment", |v| p.id(v, registry::ENCHANTMENT))?,
                chances: req(j, "chances", |v| {
                    let c = list(v, float)?;
                    if c.is_empty() {
                        return fail("list must have contents");
                    }
                    Ok(c)
                })?,
            },
            "minecraft:survives_explosion" => Condition::SurvivesExplosion,
            "minecraft:damage_source_properties" => {
                Condition::DamageSourceProperties(opt(j, "predicate", |v| DamageSourcePredicate::parse(p, v).map(Box::new))?)
            }
            "minecraft:location_check" => {
                let off = |k: &str| opt_or(j, k, 0, crate::parse::int);
                Condition::LocationCheck {
                    predicate: opt(j, "predicate", |v| LocationPredicate::parse(p, v).map(Box::new))?,
                    offset: [off("offsetX")?, off("offsetY")?, off("offsetZ")?],
                }
            }
            "minecraft:weather_check" => {
                Condition::WeatherCheck { raining: opt(j, "raining", boolean)?, thundering: opt(j, "thundering", boolean)? }
            }
            "minecraft:time_check" => Condition::TimeCheck {
                clock: req(j, "clock", ident)?,
                period: opt(j, "period", long)?,
                value: req(j, "value", |v| IntRange::parse(p, v))?,
            },
            "minecraft:int_value_check" => Condition::IntValueCheck {
                value: req(j, "value", |v| IntProvider::parse_ref(p, v))?,
                range: req(j, "test", |v| IntRange::parse(p, v))?,
            },
            "minecraft:float_value_check" => Condition::FloatValueCheck {
                value: req(j, "value", |v| FloatProvider::parse_ref(p, v))?,
                range: req(j, "test", |v| FloatRange::parse(p, v))?,
            },
            "minecraft:enchantment_active_check" => Condition::EnchantmentActiveCheck(req(j, "active", boolean)?),
            "minecraft:environment_attribute_check" => Condition::EnvironmentAttributeCheck {
                attribute: req(j, "attribute", ident)?,
                value: req(j, "value", |v| Ok(v.clone()))?,
            },
            other => return fail(format!("unknown loot condition type {other}")),
        })
    }
}

impl Eval<'_> {
    /// `LootItemCondition.test`.
    pub fn test(&mut self, c: &Ref<Condition>) -> bool {
        let data = self.data;
        let c: &Condition = match c {
            Ref::Direct(v) => v,
            Ref::Named(i) => match data.predicates.get(*i) {
                Some(v) => v,
                None => return false,
            },
        };
        match c {
            Condition::Inverted(term) => !self.test(term),
            Condition::AnyOf(terms) => terms.iter().any(|t| self.test(t)),
            Condition::AllOf(terms) => terms.iter().all(|t| self.test(t)),
            Condition::RandomChance(chance) => {
                let chance = self.float(chance);
                self.rng.next_float() < chance
            }
            Condition::RandomChanceWithEnchantedBonus { unenchanted_chance, enchanted_chance, enchantment } => {
                let level = self.entity_enchantment_level(EntityTarget::Attacker, *enchantment);
                let chance = if level > 0 { enchanted_chance.calculate(level) } else { *unenchanted_chance };
                self.rng.next_float() < chance
            }
            // Without a predicate the condition holds even when the entity is absent.
            Condition::EntityProperties { target, predicate } => match predicate {
                None => true,
                Some(p) => self.ctx.has_entity(*target) && self.ctx.entity_matches(*target, p),
            },
            Condition::KilledByPlayer => self.ctx.has_entity(EntityTarget::AttackingPlayer),
            Condition::EntityScores { target, scores } => {
                if !self.ctx.has_entity(*target) {
                    return false;
                }
                for (objective, range) in scores {
                    let holder = crate::context::ScoreHolder::Entity(*target);
                    let Some(score) = self.ctx.score(&holder, objective) else { return false };
                    if !self.int_range(range, score) {
                        return false;
                    }
                }
                true
            }
            Condition::MatchBlock(p) => {
                let Some(state) = self.ctx.block_state() else { return false };
                p.matches_state(state) && (p.block_entity.is_empty() || self.ctx.block_entity_matches(&p.block_entity))
            }
            Condition::MatchTool(p) => match self.ctx.tool() {
                Some(tool) => p.as_ref().is_none_or(|p| predicate::item_matches(&data.tags, p, tool)),
                None => false,
            },
            Condition::TableBonus { enchantment, chances } => {
                let level = self.ctx.tool().map_or(0, |t| crate::enchant::item_level(t, *enchantment));
                let chance = chances[(level.max(0) as usize).min(chances.len() - 1)];
                self.rng.next_float() < chance
            }
            Condition::SurvivesExplosion => match self.ctx.explosion_radius() {
                Some(radius) => {
                    let chance = 1.0 / radius;
                    self.rng.next_float() <= chance
                }
                None => true,
            },
            Condition::DamageSourceProperties(p) => {
                if !self.ctx.has_damage_source() || self.ctx.origin().is_none() {
                    return false;
                }
                p.as_ref().is_none_or(|p| self.ctx.damage_source_matches(p))
            }
            Condition::LocationCheck { predicate, offset } => {
                let Some(o) = self.ctx.origin() else { return false };
                let pos = [o[0] + offset[0] as f64, o[1] + offset[1] as f64, o[2] + offset[2] as f64];
                predicate.as_ref().is_none_or(|p| self.ctx.location_matches(p, pos))
            }
            Condition::WeatherCheck { raining, thundering } => {
                raining.is_none_or(|r| r == self.ctx.is_raining()) && thundering.is_none_or(|t| t == self.ctx.is_thundering())
            }
            Condition::TimeCheck { clock, period, value } => {
                let mut ticks = self.ctx.clock_total_ticks(clock);
                if let Some(p) = period {
                    ticks = if *p == 0 { 0 } else { ticks.wrapping_rem(*p) };
                }
                self.int_range(value, ticks as i32)
            }
            Condition::IntValueCheck { value, range } => match range {
                IntRange::Line { min: None, max: None } => true,
                IntRange::Line { .. } => {
                    let v = self.int(value);
                    self.int_range(range, v)
                }
                IntRange::Point(_) => {
                    let v = self.int(value);
                    self.int_range(range, v)
                }
            },
            Condition::FloatValueCheck { value, range } => match range {
                FloatRange::Line { min: None, max: None } => true,
                _ => {
                    let v = self.float(value);
                    self.float_range(range, v)
                }
            },
            Condition::EnchantmentActiveCheck(active) => self.ctx.enchantment_active() == Some(*active),
            Condition::EnvironmentAttributeCheck { attribute, value } => self.ctx.environment_attribute_equals(attribute, value),
        }
    }

    /// `IntRangePredicate.test(context, value)`.
    pub fn int_range(&mut self, r: &IntRange, v: i32) -> bool {
        match r {
            IntRange::Point(p) => self.int(p) == v,
            IntRange::Line { min, max } => {
                if let Some(min) = min
                    && v < self.int(min)
                {
                    return false;
                }
                if let Some(max) = max
                    && v > self.int(max)
                {
                    return false;
                }
                true
            }
        }
    }

    /// `FloatRangePredicate.test(context, value)`.
    pub fn float_range(&mut self, r: &FloatRange, v: f32) -> bool {
        match r {
            FloatRange::Point(p) => self.float(p) == v,
            FloatRange::Line { min, max } => {
                if let Some(min) = min {
                    let m = self.float(min);
                    if !matches!(v.partial_cmp(&m), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)) {
                        return false;
                    }
                }
                if let Some(max) = max {
                    let m = self.float(max);
                    if !matches!(v.partial_cmp(&m), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)) {
                        return false;
                    }
                }
                true
            }
        }
    }
}
