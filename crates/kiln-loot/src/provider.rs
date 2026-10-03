//! Enchantment providers (`data/*/enchantment_provider/*.json`, `EnchantmentProvider`): what
//! mobs spawn their equipment enchanted with (`minecraft:mob_spawn_equipment`), a pillager's
//! crossbow, raiders' weapons and the axe an enderman's drops are mined with.

use crate::data::LootData;
use crate::enchant;
use crate::json::Json;
use crate::parse::{IdSet, PResult, Parser, fail, int, obj, opt, req};
use kiln_item::{Identifier, ItemStack, registry};
use kiln_javamath::random::RandomSource;

/// A vanilla `IntProvider` of the kinds providers use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ints {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    BiasedToBottom { min: i32, max: i32 },
}

impl Ints {
    fn parse(j: &Json) -> PResult<Ints> {
        if matches!(j, Json::Num(_)) {
            return Ok(Ints::Constant(int(j)?));
        }
        obj(j)?;
        let kind = req(j, "type", crate::parse::string)?;
        let kind = kind.strip_prefix("minecraft:").unwrap_or(&kind);
        let range = |j: &Json| -> PResult<(i32, i32)> {
            let (min, max) = (req(j, "min_inclusive", int)?, req(j, "max_inclusive", int)?);
            if max < min {
                return fail(format!("max_inclusive {max} below min_inclusive {min}"));
            }
            Ok((min, max))
        };
        match kind {
            "constant" => Ok(Ints::Constant(req(j, "value", int)?)),
            "uniform" => range(j).map(|(min, max)| Ints::Uniform { min, max }),
            "biased_to_bottom" => range(j).map(|(min, max)| Ints::BiasedToBottom { min, max }),
            other => fail(format!("unsupported int provider type {other}")),
        }
    }

    /// `IntProvider.sample`.
    pub fn sample(self, random: &mut dyn RandomSource) -> i32 {
        match self {
            Ints::Constant(v) => v,
            Ints::Uniform { min, max } => random.next_int_bounded(max - min + 1) + min,
            Ints::BiasedToBottom { min, max } => {
                let inner = random.next_int_bounded(max - min + 1) + 1;
                min + random.next_int_bounded(inner)
            }
        }
    }
}

/// An `EnchantmentProvider`.
#[derive(Debug, Clone)]
pub enum Provider {
    /// `minecraft:by_cost`: enchantments picked as for an enchanting table at a sampled cost.
    ByCost { enchantments: IdSet, cost: Ints },
    /// `minecraft:by_cost_with_difficulty`: the cost between `min_cost` and
    /// `min_cost + multiplier * max_cost_span`, the multiplier the difficulty's special one.
    ByCostWithDifficulty { enchantments: IdSet, min_cost: i32, max_cost_span: i32 },
    /// `minecraft:single`: one enchantment at a (clamped) level.
    Single { enchantment: i32, level: Ints },
}

impl Provider {
    pub fn parse(p: &Parser, j: &Json) -> PResult<Provider> {
        obj(j)?;
        let kind = req(j, "type", crate::parse::string)?;
        match kind.strip_prefix("minecraft:").unwrap_or(&kind) {
            "by_cost" => Ok(Provider::ByCost {
                enchantments: req(j, "enchantments", |v| p.id_set(v, registry::ENCHANTMENT))?,
                cost: req(j, "cost", Ints::parse)?,
            }),
            "by_cost_with_difficulty" => {
                let (min_cost, max_cost_span) = (req(j, "min_cost", int)?, req(j, "max_cost_span", int)?);
                if min_cost < 1 || max_cost_span < 1 {
                    return fail("min_cost and max_cost_span must be positive");
                }
                Ok(Provider::ByCostWithDifficulty {
                    enchantments: req(j, "enchantments", |v| p.id_set(v, registry::ENCHANTMENT))?,
                    min_cost,
                    max_cost_span,
                })
            }
            "single" => Ok(Provider::Single {
                enchantment: req(j, "enchantment", |v| p.id(v, registry::ENCHANTMENT))?,
                level: opt(j, "level", Ints::parse)?.unwrap_or(Ints::Constant(1)),
            }),
            other => fail(format!("unknown enchantment provider type {other}")),
        }
    }
}

impl LootData {
    /// The provider `id` (`enchantment_provider/`).
    pub fn enchantment_provider(&self, id: &Identifier) -> Option<&Provider> {
        self.providers.get(id)
    }

    /// `EnchantmentHelper.enchantItemFromProvider`: enchants `stack` as the provider `id` says,
    /// drawing from `random`, for a difficulty of `special_multiplier`
    /// (`DifficultyInstance.getSpecialMultiplier`). An unknown provider does nothing.
    pub fn enchant_from_provider(&self, id: &str, stack: &mut ItemStack, special_multiplier: f32, random: &mut dyn RandomSource) {
        let Some(id) = Identifier::parse(id) else { return };
        let Some(provider) = self.providers.get(&id) else { return };
        let picks = |random: &mut dyn RandomSource, stack: &ItemStack, cost: i32, set: &IdSet| {
            let candidates = self.enchantment_candidates(Some(set));
            enchant::select(random, stack, cost, &candidates)
        };
        match provider {
            Provider::ByCost { enchantments, cost } => {
                let cost = cost.sample(random);
                for (e, level) in picks(random, stack, cost, enchantments) {
                    enchant::enchant(stack, e, level);
                }
            }
            Provider::ByCostWithDifficulty { enchantments, min_cost, max_cost_span } => {
                // `Mth.randomBetweenInclusive(random, minCost, minCost + (int)(multiplier * span))`.
                let max = min_cost.wrapping_add((special_multiplier * *max_cost_span as f32) as i32);
                let cost = random.next_int_bounded(max.wrapping_sub(*min_cost).wrapping_add(1)).wrapping_add(*min_cost);
                for (e, level) in picks(random, stack, cost, enchantments) {
                    enchant::enchant(stack, e, level);
                }
            }
            Provider::Single { enchantment, level } => {
                let sampled = level.sample(random);
                let max = self.enchantment(*enchantment).map_or(1, |e| e.max_level);
                // `Mth.clamp(level, enchantment.getMinLevel(), enchantment.getMaxLevel())`.
                enchant::enchant(stack, *enchantment, sampled.clamp(1, max.max(1)));
            }
        }
    }
}
