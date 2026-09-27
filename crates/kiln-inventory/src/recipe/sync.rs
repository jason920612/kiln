//! What the client learns about recipes: `ClientboundUpdateRecipesPacket` (sent when the player
//! joins): the recipe property sets (items furnaces, smithing tables and brewing stands accept)
//! and the stonecutter's recipes.

use super::{Ingredient, Recipe, RecipeManager};
use bytes::{Bytes, BytesMut};
use kiln_item::HolderSet;
use kiln_proto::WriteExt;

/// `RecipePropertySet`: the items some recipe of a kind takes in one slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySet {
    /// `minecraft:furnace_input`, ...
    pub key: &'static str,
    /// Item ids, ascending (a set in vanilla).
    pub items: Vec<i32>,
}

/// `RecipeManager.RECIPE_PROPERTY_SETS`, computed from the loaded recipes.
pub fn property_sets(recipes: &RecipeManager) -> Vec<PropertySet> {
    type Extract = fn(&Recipe) -> Option<&Ingredient>;
    let sets: [(&'static str, Extract); 9] = [
        ("minecraft:smithing_addition", |r| match r {
            Recipe::SmithingTransform(s) => s.addition.as_ref(),
            Recipe::SmithingTrim(s) => Some(&s.addition),
            _ => None,
        }),
        ("minecraft:smithing_base", |r| match r {
            Recipe::SmithingTransform(s) => Some(&s.base),
            Recipe::SmithingTrim(s) => Some(&s.base),
            _ => None,
        }),
        ("minecraft:smithing_template", |r| match r {
            Recipe::SmithingTransform(s) => s.template.as_ref(),
            Recipe::SmithingTrim(s) => Some(&s.template),
            _ => None,
        }),
        ("minecraft:furnace_input", |r| cooking(r, super::CookingKind::Smelting)),
        ("minecraft:blast_furnace_input", |r| cooking(r, super::CookingKind::Blasting)),
        ("minecraft:smoker_input", |r| cooking(r, super::CookingKind::Smoking)),
        ("minecraft:campfire_input", |r| cooking(r, super::CookingKind::Campfire)),
        ("minecraft:brewing_input", |r| match r {
            Recipe::Brewing(b) => Some(&b.input.item),
            _ => None,
        }),
        ("minecraft:brewing_reagent", |r| match r {
            Recipe::Brewing(b) => Some(&b.reagent.item),
            _ => None,
        }),
    ];
    sets.iter()
        .map(|(key, extract)| {
            let mut items: Vec<i32> = recipes.recipes().iter().filter_map(|h| extract(&h.recipe)).flat_map(|i| i.items.iter().copied()).collect();
            items.sort_unstable();
            items.dedup();
            PropertySet { key, items }
        })
        .collect()
}

fn cooking(r: &Recipe, kind: super::CookingKind) -> Option<&Ingredient> {
    match r {
        Recipe::Cooking(c) if c.kind == kind => Some(&c.ingredient),
        _ => None,
    }
}

/// `Ingredient.CONTENTS_STREAM_CODEC` (`ByteBufCodecs.holderSet`).
pub fn write_ingredient(ing: &Ingredient, out: &mut BytesMut) {
    let set = match &ing.tag {
        Some(tag) => HolderSet::Tag(kiln_item::Identifier::parse(tag).expect("tag id")),
        None => HolderSet::Direct(ing.items.clone()),
    };
    set.write(out);
}

/// `ClientboundUpdateRecipesPacket`: property sets, then the stonecutter entries (input
/// ingredient, result display) in recipe order.
pub fn update_recipes(recipes: &RecipeManager) -> Bytes {
    let mut b = BytesMut::with_capacity(8192);
    b.put_varint(kiln_data::packets::play::clientbound::UPDATE_RECIPES);
    let sets = recipes.property_sets();
    b.put_varint(sets.len() as i32);
    for s in sets {
        b.put_string(s.key);
        b.put_varint(s.items.len() as i32);
        for &i in &s.items {
            b.put_varint(i);
        }
    }
    let stonecutter: Vec<_> = recipes.recipes().iter().filter_map(|h| match &h.recipe {
        Recipe::Stonecutting(s) => Some(s),
        _ => None,
    }).collect();
    b.put_varint(stonecutter.len() as i32);
    let item_stack_display = kiln_data::builtin_id("minecraft:slot_display", "minecraft:item_stack").expect("slot display type");
    for s in stonecutter {
        write_ingredient(&s.ingredient, &mut b);
        b.put_varint(item_stack_display);
        s.result.write(&mut b);
    }
    b.freeze()
}
