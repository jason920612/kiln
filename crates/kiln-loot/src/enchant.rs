//! The parts of enchantment definitions (`data/*/enchantment/*.json`) loot needs, and
//! `EnchantmentHelper`'s level lookups and random selection.

use crate::json::Json;
use crate::parse::{IdSet, PResult, Parser, int, list, obj, opt, req, value};
use crate::random::RngExt;
use kiln_item::component::{EquipmentSlotGroup, Enchantments, keys};
use kiln_item::registry;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// `Enchantment.Cost`: `base + per_level_above_first * (level - 1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    pub base: i32,
    pub per_level_above_first: i32,
}

impl Cost {
    pub fn calculate(self, level: i32) -> i32 {
        self.base.wrapping_add(self.per_level_above_first.wrapping_mul(level.wrapping_sub(1)))
    }

    fn parse(j: &Json) -> PResult<Cost> {
        Ok(Cost { base: req(j, "base", int)?, per_level_above_first: req(j, "per_level_above_first", int)? })
    }
}

/// `Enchantment` (its definition and exclusive set; effects are not modelled).
#[derive(Debug, Clone, PartialEq)]
pub struct Enchantment {
    /// `minecraft:enchantment` network id.
    pub id: i32,
    pub supported_items: IdSet,
    pub primary_items: Option<IdSet>,
    pub weight: i32,
    pub max_level: i32,
    pub min_cost: Cost,
    pub max_cost: Cost,
    pub anvil_cost: i32,
    pub slots: Vec<EquipmentSlotGroup>,
    pub exclusive_set: IdSet,
}

impl Enchantment {
    pub fn parse(p: &Parser, id: i32, j: &Json) -> PResult<Enchantment> {
        obj(j)?;
        let weight = req(j, "weight", int)?;
        let max_level = req(j, "max_level", int)?;
        if !(1..=1024).contains(&weight) {
            return crate::parse::fail(format!("weight out of range [1;1024]: {weight}"));
        }
        if !(1..=255).contains(&max_level) {
            return crate::parse::fail(format!("max_level out of range [1;255]: {max_level}"));
        }
        Ok(Enchantment {
            id,
            supported_items: req(j, "supported_items", |v| p.id_set(v, registry::ITEM))?,
            primary_items: opt(j, "primary_items", |v| p.id_set(v, registry::ITEM))?,
            weight,
            max_level,
            min_cost: req(j, "min_cost", Cost::parse)?,
            max_cost: req(j, "max_cost", Cost::parse)?,
            anvil_cost: req(j, "anvil_cost", int)?,
            slots: req(j, "slots", |v| list(v, |s| value(s, EquipmentSlotGroup::from_value)))?,
            exclusive_set: opt(j, "exclusive_set", |v| p.id_set(v, registry::ENCHANTMENT))?
                .unwrap_or_else(|| IdSet::new(None, Vec::new())),
        })
    }

    /// `getMinLevel()` is always 1.
    pub fn min_level(&self) -> i32 {
        1
    }

    /// `Enchantment.canEnchant`: the item is supported.
    pub fn can_enchant(&self, stack: &ItemStack) -> bool {
        self.supported_items.contains(stack.item())
    }

    /// `Enchantment.isPrimaryItem`.
    pub fn is_primary_item(&self, stack: &ItemStack) -> bool {
        self.can_enchant(stack) && self.primary_items.as_ref().is_none_or(|p| p.contains(stack.item()))
    }
}

/// `Enchantment.areCompatible`.
pub fn compatible(a: &Enchantment, b: &Enchantment) -> bool {
    a.id != b.id && !a.exclusive_set.contains(b.id) && !b.exclusive_set.contains(a.id)
}

/// `EnchantmentHelper.getItemEnchantmentLevel`: the level in the `enchantments` component.
pub fn item_level(stack: &ItemStack, enchantment: i32) -> i32 {
    stack.get(keys::ENCHANTMENTS).map_or(0, |e| e.level(enchantment))
}

fn is(stack: &ItemStack, name: &str) -> bool {
    stack.item_name() == name
}

/// `EnchantmentHelper.updateEnchantments` with `Mutable.upgrade`: stored enchantments on an
/// enchanted book, else `enchantments`; nothing happens to an item without the component.
pub fn enchant(stack: &mut ItemStack, enchantment: i32, level: i32) {
    let key = if is(stack, "minecraft:enchanted_book") { keys::STORED_ENCHANTMENTS } else { keys::ENCHANTMENTS };
    let Some(current) = stack.get(key) else { return };
    let mut e: Enchantments = current.clone();
    if level > 0 {
        let level = level.min(255);
        let merged = e.level(enchantment).max(level);
        e.set(enchantment, merged);
    }
    stack.insert(key, e);
}

/// `EnchantmentHelper.updateEnchantments` with an arbitrary edit.
pub fn update(stack: &mut ItemStack, edit: impl FnOnce(&mut Enchantments)) {
    let key = if is(stack, "minecraft:enchanted_book") { keys::STORED_ENCHANTMENTS } else { keys::ENCHANTMENTS };
    let Some(current) = stack.get(key) else { return };
    let mut e = current.clone();
    edit(&mut e);
    stack.insert(key, e);
}

/// `EnchantmentHelper.getAvailableEnchantmentResults`: for each candidate usable on the item
/// (or any, for a book), the highest level whose cost range contains `level`.
pub fn available_results(level: i32, stack: &ItemStack, candidates: &[&Enchantment]) -> Vec<(i32, i32)> {
    let book = is(stack, "minecraft:book");
    let mut out = Vec::new();
    for e in candidates {
        if !(e.is_primary_item(stack) || book) {
            continue;
        }
        let mut l = e.max_level;
        while l >= e.min_level() {
            if level >= e.min_cost.calculate(l) && level <= e.max_cost.calculate(l) {
                out.push((e.id, l));
                break;
            }
            l -= 1;
        }
    }
    out
}

/// `WeightedRandom.getRandomItem` over enchantment instances weighted by the enchantment.
fn weighted_pick(rng: &mut dyn RandomSource, list: &[(i32, i32)], weight: &dyn Fn(i32) -> i32) -> Option<(i32, i32)> {
    let total: i64 = list.iter().map(|(e, _)| weight(*e) as i64).sum();
    if total == 0 {
        return None;
    }
    let mut r = rng.bounded(total as i32);
    for &(e, l) in list {
        r -= weight(e);
        if r < 0 {
            return Some((e, l));
        }
    }
    None
}

/// `EnchantmentHelper.selectEnchantment`.
pub fn select(
    rng: &mut dyn RandomSource,
    stack: &ItemStack,
    level: i32,
    candidates: &[&Enchantment],
) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let Some(enchantable) = stack.get(keys::ENCHANTABLE).map(|e| e.0) else { return out };
    let mut level = level
        .wrapping_add(1)
        .wrapping_add(rng.bounded(enchantable / 4 + 1))
        .wrapping_add(rng.bounded(enchantable / 4 + 1));
    let spread = (rng.next_float() + rng.next_float() - 1.0) * 0.15;
    level = crate::number::java_round(level as f32 + level as f32 * spread).clamp(1, i32::MAX);
    let mut available = available_results(level, stack, candidates);
    if available.is_empty() {
        return out;
    }
    let by_id = |id: i32| candidates.iter().find(|e| e.id == id).copied();
    let weight = |id: i32| by_id(id).map_or(0, |e| e.weight);
    if let Some(pick) = weighted_pick(rng, &available, &weight) {
        out.push(pick);
    }
    while rng.bounded(50) <= level {
        if let Some(&(last, _)) = out.last() {
            let last = by_id(last).expect("selected enchantment");
            available.retain(|(e, _)| by_id(*e).is_some_and(|e| compatible(last, e)));
        }
        if available.is_empty() {
            break;
        }
        if let Some(pick) = weighted_pick(rng, &available, &weight) {
            out.push(pick);
        }
        level /= 2;
    }
    out
}

/// `EnchantmentHelper.enchantItem`: a book becomes a fresh enchanted book.
pub fn enchant_item(rng: &mut dyn RandomSource, stack: ItemStack, level: i32, candidates: &[&Enchantment]) -> ItemStack {
    let picks = select(rng, &stack, level, candidates);
    let mut stack = if is(&stack, "minecraft:book") { ItemStack::of("minecraft:enchanted_book", 1).expect("enchanted_book") } else { stack };
    for (e, l) in picks {
        enchant(&mut stack, e, l);
    }
    stack
}
