//! Data-driven rules menus consult: recipes, and the enchantments that lock armor on.

use crate::recipe::RecipeManager;
use kiln_item::component::EquipmentSlot;
use kiln_item::{HolderSet, ItemStack, keys, registry};
use std::path::Path;

/// Everything menus read from the datapack (loaded at runtime, like vanilla's reload).
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub recipes: RecipeManager,
    /// `minecraft:enchantment` ids whose effects include `minecraft:prevent_armor_change`.
    armor_lock: Option<Vec<i32>>,
    /// `minecraft:enchantment` ids whose effects include `minecraft:prevent_equipment_drop`
    /// (leaked: a datapack reload makes a new set, and the damage code holds it by reference).
    equipment_drop_lock: Option<&'static [i32]>,
}

impl Rules {
    /// Loads recipes and enchantment effects from a datapack directory (the one holding
    /// `data/`, like `work/generated`).
    pub fn load(datapack: &Path) -> Result<Rules, crate::recipe::LoadError> {
        let recipes = RecipeManager::load(datapack)?;
        let armor_lock = load_effect_enchantments(datapack, "minecraft:prevent_armor_change");
        let equipment_drop_lock = load_effect_enchantments(datapack, "minecraft:prevent_equipment_drop").map(|v| &*Box::leak(v.into_boxed_slice()));
        Ok(Rules { recipes, armor_lock, equipment_drop_lock })
    }

    /// [`load`](Self::load) over several packs in order (later packs override files).
    pub fn load_packs(packs: &[&Path]) -> Result<Rules, crate::recipe::LoadError> {
        let recipes = RecipeManager::load_packs(packs)?;
        let armor_lock = packs.iter().rev().find_map(|p| load_effect_enchantments(p, "minecraft:prevent_armor_change"));
        let equipment_drop_lock = packs
            .iter()
            .rev()
            .find_map(|p| load_effect_enchantments(p, "minecraft:prevent_equipment_drop"))
            .map(|v| &*Box::leak(v.into_boxed_slice()));
        Ok(Rules { recipes, armor_lock, equipment_drop_lock })
    }

    pub fn with_recipes(recipes: RecipeManager) -> Rules {
        Rules { recipes, armor_lock: None, equipment_drop_lock: None }
    }

    /// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`. Without loaded enchantments,
    /// vanilla's only such enchantment (the curse of binding) is assumed.
    pub fn prevents_armor_change(&self, stack: &ItemStack) -> bool {
        let Some(ench) = stack.get(keys::ENCHANTMENTS) else { return false };
        match &self.armor_lock {
            Some(ids) => ench.0.iter().any(|(e, _)| ids.contains(e)),
            None => {
                let curse = registry::ENCHANTMENT.id("minecraft:binding_curse");
                ench.0.iter().any(|(e, _)| Some(*e) == curse)
            }
        }
    }

    /// The enchantments with `prevent_equipment_drop` (the curse of vanishing), for code that
    /// cannot hold the rules; `None` before a datapack is loaded.
    pub fn equipment_drop_lock(&self) -> Option<&'static [i32]> {
        self.equipment_drop_lock
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

/// `EnchantmentHelper.has(stack, PREVENT_EQUIPMENT_DROP)` against the set of
/// [`Rules::equipment_drop_lock`] (the curse of vanishing when no datapack is loaded).
pub fn prevents_equipment_drop(lock: Option<&[i32]>, stack: &ItemStack) -> bool {
    let Some(ench) = stack.get(keys::ENCHANTMENTS) else { return false };
    match lock {
        Some(ids) => ench.0.iter().any(|(e, _)| ids.contains(e)),
        None => {
            let curse = registry::ENCHANTMENT.id("minecraft:vanishing_curse");
            ench.0.iter().any(|(e, _)| Some(*e) == curse)
        }
    }
}

fn load_effect_enchantments(datapack: &Path, effect: &str) -> Option<Vec<i32>> {
    let dir = datapack.join("data/minecraft/enchantment");
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let locks = json.get("effects").and_then(|e| e.get(effect)).is_some();
        if let (true, Some(id)) = (locks, registry::ENCHANTMENT.id(&format!("minecraft:{name}"))) {
            ids.push(id);
        }
    }
    Some(ids)
}
