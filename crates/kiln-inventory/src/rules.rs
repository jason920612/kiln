//! Data-driven rules menus consult: recipes, and the enchantments that lock armor on.

use crate::recipe::RecipeManager;
use kiln_item::component::EquipmentSlot;
use kiln_item::enchant_flags::{self, Marker};
use kiln_item::{HolderSet, ItemStack, keys, registry};
use std::path::Path;

/// Everything menus read from the datapack (loaded at runtime, like vanilla's reload).
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub recipes: RecipeManager,
}

impl Rules {
    /// Loads recipes from a datapack directory (the one holding `data/`, like `work/generated`).
    pub fn load(datapack: &Path) -> Result<Rules, crate::recipe::LoadError> {
        Ok(Rules { recipes: RecipeManager::load(datapack)? })
    }

    /// [`load`](Self::load) over several packs in order (later packs override files).
    pub fn load_packs(packs: &[&Path]) -> Result<Rules, crate::recipe::LoadError> {
        Ok(Rules { recipes: RecipeManager::load_packs(packs)? })
    }

    pub fn with_recipes(recipes: RecipeManager) -> Rules {
        Rules { recipes }
    }

    /// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`: an enchantment of the world's
    /// definitions has the marker (see [`kiln_item::enchant_flags`]).
    pub fn prevents_armor_change(&self, stack: &ItemStack) -> bool {
        enchant_flags::has(stack, Marker::PreventArmorChange)
    }

    /// `LivingEntity.isEquippableInSlot` for a player.
    pub fn is_equippable_in_slot(&self, stack: &ItemStack, slot: EquipmentSlot) -> bool {
        match stack.get(keys::EQUIPPABLE) {
            None => slot == EquipmentSlot::MainHand,
            Some(e) => slot == e.slot && e.allowed_entities.as_ref().is_none_or(contains_player),
        }
    }
}

fn contains_player(set: &HolderSet) -> bool {
    let Some(player) = registry::ENTITY_TYPE.id("minecraft:player") else { return false };
    match set {
        HolderSet::Direct(ids) => ids.contains(&player),
        HolderSet::Tag(tag) => crate::tags::contains("minecraft:entity_type", tag.as_str(), player),
    }
}

/// `EnchantmentHelper.has(stack, PREVENT_EQUIPMENT_DROP)`.
pub fn prevents_equipment_drop(stack: &ItemStack) -> bool {
    enchant_flags::has(stack, Marker::PreventEquipmentDrop)
}
