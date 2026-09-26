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

/// `Ingredient.testOptionalIngredient`: no ingredient means an empty slot.
pub fn test_optional(ing: &Option<Ingredient>, stack: &ItemStack) -> bool {
    match ing {
        Some(i) => i.test(stack),
        None => stack.is_empty(),
    }
}
