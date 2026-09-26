//! `Ingredient`: a set of items (an item, a list of items or an item tag).

use crate::stack::StackExt;
use kiln_item::{ItemStack, registry};
use serde_json::Value as Json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingredient {
    /// The tag, for `#tag` ingredients.
    pub tag: Option<String>,
    /// Item ids (resolved from the tag).
    pub items: Vec<i32>,
}

impl Ingredient {
    pub fn of_items(items: Vec<i32>) -> Self {
        Ingredient { tag: None, items }
    }

    /// `Ingredient.test`: the stack's item (air if empty) is in the set.
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.items.contains(&stack.effective_item())
    }

    pub fn accepts(&self, item: i32) -> bool {
        self.items.contains(&item)
    }

    /// `Ingredient.CODEC`: `"id"`, `"#tag"` or `["id", ...]`; empty sets and air are errors.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let air = kiln_item::stack::air();
        let ing = match v {
            Json::String(s) if s.starts_with('#') => {
                let items = crate::tags::entries("minecraft:item", s).ok_or_else(|| format!("unknown item tag {s}"))?;
                Ingredient { tag: Some(s[1..].to_owned()), items: items.to_vec() }
            }
            Json::String(s) => Ingredient::of_items(vec![item_id(s)?]),
            Json::Array(list) => {
                let items = list.iter().map(|e| e.as_str().ok_or("ingredient list entry is not a string".to_owned()).and_then(item_id)).collect::<Result<Vec<_>, _>>()?;
                Ingredient::of_items(items)
            }
            _ => return Err("invalid ingredient".into()),
        };
        if ing.items.is_empty() {
            return Err("empty ingredient".into());
        }
        if ing.items.contains(&air) {
            return Err("ingredient can't contain air".into());
        }
        Ok(ing)
    }
}

pub(crate) fn item_id(name: &str) -> Result<i32, String> {
    registry::ITEM.id(name).ok_or_else(|| format!("unknown item {name}"))
}
