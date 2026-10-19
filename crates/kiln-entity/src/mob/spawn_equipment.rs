//! `Mob.equip(EquipmentTable)` (`EquipmentUser.equip`): a spawn data's `equipment` (`{loot_table, slot_drop_chances}`) is rolled when the mob
//! is made, and what comes out is put on in the slots it belongs to (trial chamber mobs wear the `equipment/trial_chamber*` tables).

use super::{FEET, HEAD, MAINHAND, SLOT_NAMES, data_mut};
use crate::entity::Entity;
use crate::level::EntityLevel;
use kiln_proto::nbt::Tag;

/// `EquipmentTable.CODEC`'s `slot_drop_chances`: one number for every slot, or a number per named slot.
fn drop_chances(equipment: &Tag) -> Vec<(usize, f32)> {
    match equipment.get("slot_drop_chances") {
        Some(Tag::Compound(slots)) => slots.iter().filter_map(|(k, v)| Some((SLOT_NAMES.iter().position(|s| s == k)?, v.as_f64()? as f32))).collect(),
        Some(single) => single.as_f64().map(|f| (0..SLOT_NAMES.len()).map(|i| (i, f as f32)).collect()).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// `Mob.equip(table, slotDropChances)`: the table (`LootContextParamSets.EQUIPMENT`: the mob and where it stands) rolls; each stack goes into its
/// own `equippable` slot, or the main hand, and a slot is filled once (`resolveSlot`). Armor slots hold one item (`EquipmentSlot.limit`).
pub fn equip_from_table(e: &mut Entity, level: &mut dyn EntityLevel, equipment: &Tag) {
    let Some(table) = equipment.get("loot_table").and_then(Tag::as_str) else { return };
    let chances = drop_chances(equipment);
    let stacks = level.equipment_loot(e, table);
    let Some(m) = data_mut(e) else { return };
    let mut filled: Vec<usize> = Vec::new();
    for stack in stacks {
        if stack.is_empty() {
            continue;
        }
        let slot = match stack.get(kiln_item::keys::EQUIPPABLE) {
            Some(q) => q.slot as usize,
            None => MAINHAND,
        };
        // (The body and saddle slots are other types' own.)
        if slot >= SLOT_NAMES.len() || filled.contains(&slot) {
            continue;
        }
        let stack = if (FEET..=HEAD).contains(&slot) && stack.count() > 1 { stack.with_count(1) } else { stack };
        m.equipment[slot] = stack;
        if let Some(&(_, chance)) = chances.iter().find(|(i, _)| *i == slot) {
            m.drop_chances[slot] = chance;
        }
        filled.push(slot);
    }
}
