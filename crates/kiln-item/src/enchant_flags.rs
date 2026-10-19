//! The enchantments whose definitions carry the marker effect components `prevent_armor_change`
//! (the curse of binding) and `prevent_equipment_drop` (the curse of vanishing), for the code
//! that cannot hold the loaded enchantment definitions (inventory menus, mobs).
//!
//! The simulation installs the sets when the world loads the enchantments; they are the ids as the
//! registry numbers them then. Until something is installed vanilla's own two curses are assumed.

use crate::ItemStack;
use crate::component::keys;
use crate::registry;
use std::sync::{Arc, RwLock};

/// A marker component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// `minecraft:prevent_armor_change`.
    PreventArmorChange,
    /// `minecraft:prevent_equipment_drop`.
    PreventEquipmentDrop,
}

struct Installed {
    armor: Vec<i32>,
    drop: Vec<i32>,
}

static INSTALLED: RwLock<Option<Arc<Installed>>> = RwLock::new(None);

/// Installs the enchantments that have `prevent_armor_change` and `prevent_equipment_drop`
/// (`minecraft:enchantment` ids as numbered now).
pub fn install(armor: Vec<i32>, drop: Vec<i32>) {
    let installed = Arc::new(Installed { armor, drop });
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = Some(installed);
}

/// Forgets what was installed (vanilla's curses again).
pub fn reset() {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The ids of the enchantments with the marker.
pub fn enchantments(marker: Marker) -> Vec<i32> {
    let installed = INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone();
    match installed {
        Some(i) => match marker {
            Marker::PreventArmorChange => i.armor.clone(),
            Marker::PreventEquipmentDrop => i.drop.clone(),
        },
        None => {
            let name = match marker {
                Marker::PreventArmorChange => "minecraft:binding_curse",
                Marker::PreventEquipmentDrop => "minecraft:vanishing_curse",
            };
            registry::ENCHANTMENT.id(name).into_iter().collect()
        }
    }
}

/// `EnchantmentHelper.has(stack, marker)`: any enchantment on the stack has the marker.
pub fn has(stack: &ItemStack, marker: Marker) -> bool {
    let Some(list) = stack.get(keys::ENCHANTMENTS) else { return false };
    if list.is_empty() {
        return false;
    }
    let ids = enchantments(marker);
    list.0.iter().any(|(e, _)| ids.contains(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_curses_until_installed() {
        reset();
        let binding = registry::ENCHANTMENT.id("minecraft:binding_curse").unwrap();
        assert_eq!(enchantments(Marker::PreventArmorChange), [binding]);
        install(vec![7], vec![]);
        assert_eq!(enchantments(Marker::PreventArmorChange), [7]);
        assert!(enchantments(Marker::PreventEquipmentDrop).is_empty());
        reset();
    }
}
