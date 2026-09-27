//! Furnace, stonecutter and smithing recipes.

use super::crafting::with_original_components;
use super::ingredient::Ingredient;
use super::template_from_json;
use kiln_item::{ItemStack, ItemStackTemplate};
use serde_json::Value as Json;

fn ing(v: &Json, field: &str) -> Result<Ingredient, String> {
    Ingredient::from_json(v.get(field).ok_or_else(|| format!("missing {field}"))?)
}

fn opt_ing(v: &Json, field: &str) -> Result<Option<Ingredient>, String> {
    v.get(field).map(Ingredient::from_json).transpose()
}

fn result(v: &Json) -> Result<ItemStackTemplate, String> {
    template_from_json(v.get("result").ok_or("missing result")?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CookingKind {
    Smelting,
    Blasting,
    Smoking,
    Campfire,
}

/// `AbstractCookingRecipe`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cooking {
    pub kind: CookingKind,
    pub ingredient: Ingredient,
    pub result: ItemStackTemplate,
    pub experience: f32,
    pub cooking_time: i32,
}

impl Cooking {
    pub fn from_json(kind: &str, v: &Json) -> Result<Self, String> {
        let (kind, default_time) = match kind {
            "smelting" => (CookingKind::Smelting, 200),
            "blasting" => (CookingKind::Blasting, 100),
            "smoking" => (CookingKind::Smoking, 100),
            _ => (CookingKind::Campfire, 100),
        };
        Ok(Cooking {
            kind,
            ingredient: ing(v, "ingredient")?,
            result: result(v)?,
            experience: v.get("experience").and_then(Json::as_f64).unwrap_or(0.0) as f32,
            cooking_time: v.get("cookingtime").and_then(Json::as_i64).map_or(default_time, |t| t as i32),
        })
    }

    pub fn matches(&self, input: &ItemStack) -> bool {
        self.ingredient.test(input)
    }
}

/// `StonecutterRecipe`.
#[derive(Debug, Clone, PartialEq)]
pub struct Stonecutting {
    pub ingredient: Ingredient,
    pub result: ItemStackTemplate,
}

impl Stonecutting {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        Ok(Stonecutting { ingredient: ing(v, "ingredient")?, result: result(v)? })
    }

    pub fn matches(&self, input: &ItemStack) -> bool {
        self.ingredient.test(input)
    }

    pub fn assemble(&self) -> ItemStack {
        self.result.create()
    }
}

/// `SmithingTransformRecipe`: the base item turned into the result, keeping its components.
#[derive(Debug, Clone, PartialEq)]
pub struct SmithingTransform {
    pub template: Option<Ingredient>,
    pub base: Ingredient,
    pub addition: Option<Ingredient>,
    pub result: ItemStackTemplate,
}

impl SmithingTransform {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        Ok(SmithingTransform { template: opt_ing(v, "template")?, base: ing(v, "base")?, addition: opt_ing(v, "addition")?, result: result(v)? })
    }

    pub fn matches(&self, template: &ItemStack, base: &ItemStack, addition: &ItemStack) -> bool {
        test_optional(&self.template, template) && self.base.test(base) && test_optional(&self.addition, addition)
    }

    pub fn assemble(&self, base: &ItemStack) -> ItemStack {
        with_original_components(&self.result, base, 0)
    }
}

/// `SmithingTrimRecipe`.
#[derive(Debug, Clone, PartialEq)]
pub struct SmithingTrim {
    pub template: Ingredient,
    pub base: Ingredient,
    pub addition: Ingredient,
    /// `minecraft:trim_pattern` entry.
    pub pattern: String,
}

impl SmithingTrim {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        Ok(SmithingTrim {
            template: ing(v, "template")?,
            base: ing(v, "base")?,
            addition: ing(v, "addition")?,
            pattern: v.get("pattern").and_then(Json::as_str).ok_or("missing pattern")?.to_owned(),
        })
    }
}

/// `PotionIngredient`: an item ingredient and, optionally, the potions its `potion_contents`
/// must hold (`PotionsPredicate`).
#[derive(Debug, Clone, PartialEq)]
pub struct PotionIngredient {
    pub item: Ingredient,
    /// `minecraft:potion` ids.
    pub potions: Option<Vec<i32>>,
}

impl PotionIngredient {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let item = ing(v, "item")?;
        let potions = match v.get("potion_contents").and_then(|p| p.get("potions")) {
            None => None,
            Some(set) => {
                let set = kiln_item::HolderSet::from_value(kiln_item::registry::POTION, &super::json_value(set)).map_err(|e| e.0)?;
                Some(match set {
                    kiln_item::HolderSet::Direct(ids) => ids,
                    kiln_item::HolderSet::Tag(t) => crate::tags::entries("minecraft:potion", t.as_str()).unwrap_or(&[]).to_vec(),
                })
            }
        };
        Ok(PotionIngredient { item, potions })
    }

    pub fn test(&self, stack: &ItemStack) -> bool {
        self.item.test(stack)
            && self.potions.as_ref().is_none_or(|set| {
                stack.get(kiln_item::keys::POTION_CONTENTS).and_then(|c| c.potion).is_some_and(|p| set.contains(&p))
            })
    }
}

/// `BrewingRecipe` (26.x data-driven brewing): `input` + `reagent` make `output`.
#[derive(Debug, Clone, PartialEq)]
pub struct Brewing {
    pub input: PotionIngredient,
    pub reagent: PotionIngredient,
    pub output: ItemStackTemplate,
}

impl Brewing {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let part = |f: &str| PotionIngredient::from_json(v.get(f).ok_or_else(|| format!("missing {f}"))?);
        Ok(Brewing {
            input: part("input")?,
            reagent: part("reagent")?,
            output: template_from_json(v.get("output").ok_or("missing output")?)?,
        })
    }

    pub fn matches(&self, input: &ItemStack, reagent: &ItemStack) -> bool {
        self.input.test(input) && self.reagent.test(reagent)
    }
}

/// `Ingredient.testOptionalIngredient`: no ingredient means an empty slot.
pub fn test_optional(ing: &Option<Ingredient>, stack: &ItemStack) -> bool {
    match ing {
        Some(i) => i.test(stack),
        None => stack.is_empty(),
    }
}
