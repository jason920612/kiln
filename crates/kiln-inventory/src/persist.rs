//! Item lists in saved data, with full item stacks: the player's `Inventory` list and
//! `equipment` compound (playerdata), and container block entities' `Items` list
//! (`ContainerHelper.loadAllItems` / `saveAllItems`).
//!
//! Loading is vanilla's (`ValueInput.list` with the `ItemStackWithSlot` codec): an entry that does not decode (an unknown item, no
//! id) or whose slot is outside the container is dropped, and is gone when the data is saved again; a count that is missing or out
//! of range is 1; components that do not decode are left off (see [`ItemStack::from_nbt`]).

use crate::container::Container;
use crate::inventory::{EQUIPMENT, MAIN_SIZE, PlayerInventory};
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_proto::nbt::Tag;

/// Items by slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemList {
    pub stacks: Vec<ItemStack>,
    /// Whether loading dropped an entry, so that what is saved differs from what was read.
    pub dropped: bool,
}

/// `ItemStackWithSlot.Slot`: an unsigned byte (0 when absent).
fn slot_of(entry: &Tag) -> i32 {
    entry.get("Slot").and_then(Tag::as_i64).map_or(0, |s| (s as i8) as u8 as i32)
}

fn without_slot(entry: &Tag) -> Tag {
    match entry {
        Tag::Compound(fields) => Tag::Compound(fields.iter().filter(|(k, _)| k != "Slot").cloned().collect()),
        other => other.clone(),
    }
}

fn with_slot(slot: i32, item: Tag) -> Tag {
    let Tag::Compound(mut fields) = item else { return item };
    fields.retain(|(k, _)| k != "Slot");
    fields.insert(0, ("Slot".into(), Tag::Byte(slot as u8 as i8)));
    Tag::Compound(fields)
}

impl ItemList {
    /// `ContainerHelper.loadAllItems` for a container of `size` slots.
    pub fn load(list: Option<&Tag>, size: usize) -> ItemList {
        let mut out = ItemList { stacks: vec![ItemStack::empty(); size], dropped: false };
        for entry in list.and_then(Tag::as_list).unwrap_or(&[]) {
            let slot = slot_of(entry);
            match ItemStack::from_nbt(&without_slot(entry)) {
                Ok(stack) if (slot as usize) < size => out.stacks[slot as usize] = stack,
                _ => out.dropped = true,
            }
        }
        out
    }

    /// `ContainerHelper.saveAllItems`: the non-empty slots in slot order.
    pub fn save(&self) -> Tag {
        Tag::List(self.stacks.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| with_slot(i as i32, s.to_nbt())).collect())
    }

    /// The stacks as a container (block entity contents).
    pub fn into_container(self) -> crate::container::SimpleContainer {
        crate::container::SimpleContainer::from_items(self.stacks)
    }
}

/// `EquipmentSlot.getSerializedName`.
pub fn equipment_key(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::MainHand => "mainhand",
        EquipmentSlot::OffHand => "offhand",
        EquipmentSlot::Feet => "feet",
        EquipmentSlot::Legs => "legs",
        EquipmentSlot::Chest => "chest",
        EquipmentSlot::Head => "head",
        EquipmentSlot::Body => "body",
        EquipmentSlot::Saddle => "saddle",
    }
}

/// Reads `Inventory`, `equipment` and `SelectedItemSlot` from a player compound
/// (`Player.readAdditionalSaveData`): what does not decode is dropped.
pub fn load_player_inventory(player: &Tag) -> PlayerInventory {
    let mut inv = PlayerInventory::new();
    inv.items = ItemList::load(player.get("Inventory"), MAIN_SIZE).stacks;
    if let Some(Tag::Compound(eq)) = player.get("equipment") {
        for (key, item) in eq {
            if let (Some(i), Ok(stack)) = (EQUIPMENT.iter().position(|s| equipment_key(*s) == key), ItemStack::from_nbt(item)) {
                inv.equipment[i] = stack;
            }
        }
    }
    inv.selected = player.get("SelectedItemSlot").and_then(Tag::as_i64).filter(|s| (0..9).contains(s)).unwrap_or(0) as usize;
    inv
}

fn put(compound: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(fields) = compound {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some(f) => f.1 = value,
            None => fields.push((key.to_owned(), value)),
        }
    }
}

fn remove(compound: &mut Tag, key: &str) {
    if let Tag::Compound(fields) = compound {
        fields.retain(|(k, _)| k != key);
    }
}

/// Writes `Inventory`, `equipment` and `SelectedItemSlot` into a player compound.
pub fn save_player_inventory(inv: &PlayerInventory, player: &mut Tag) {
    put(player, "Inventory", ItemList { stacks: inv.items.clone(), dropped: false }.save());
    let mut eq: Vec<(String, Tag)> = Vec::new();
    for (i, slot) in EQUIPMENT.iter().enumerate() {
        let stack = inv.item(MAIN_SIZE + i);
        if !stack.is_empty() {
            eq.push((equipment_key(*slot).to_owned(), stack.to_nbt()));
        }
    }
    if eq.is_empty() {
        remove(player, "equipment");
    } else {
        put(player, "equipment", Tag::Compound(eq));
    }
    put(player, "SelectedItemSlot", Tag::Int(inv.selected as i32));
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_item::keys;

    fn item(name: &str, count: i32) -> Tag {
        Tag::Compound(vec![("id".into(), Tag::String(name.into())), ("count".into(), Tag::Int(count))])
    }

    fn slotted(slot: i8, t: Tag) -> Tag {
        with_slot(slot as u8 as i32, t)
    }

    #[test]
    fn container_items_drop_what_vanilla_drops() {
        let list = Tag::List(vec![
            slotted(0, item("minecraft:stone", 5)),
            slotted(3, item("minecraft:not_an_item", 1)),
            slotted(40, item("minecraft:dirt", 1)),
            slotted(4, item("minecraft:dirt", 120)),
        ]);
        let loaded = ItemList::load(Some(&list), 27);
        assert_eq!(loaded.stacks[0].count(), 5);
        // The unknown item and the slot outside the container are gone; a count of 120 is 1.
        assert!(loaded.dropped);
        assert_eq!(loaded.stacks[4].count(), 1);
        let saved = loaded.save();
        let slots: Vec<i32> = saved.as_list().unwrap().iter().map(slot_of).collect();
        assert_eq!(slots, [0, 4]);
        assert_eq!(ItemList::load(Some(&saved), 27).stacks, loaded.stacks);
    }

    #[test]
    fn player_inventory_and_equipment() {
        let mut enchanted = ItemStack::of("diamond_helmet", 1).unwrap();
        enchanted.insert(keys::DAMAGE, 7);
        let player = Tag::Compound(vec![
            ("Inventory".into(), Tag::List(vec![slotted(2, item("minecraft:stone", 3)), slotted(9, item("minecraft:unknown_thing", 1))])),
            (
                "equipment".into(),
                Tag::Compound(vec![
                    ("head".into(), enchanted.to_nbt()),
                    ("body".into(), item("minecraft:stone", 1)),
                    ("feet".into(), item("minecraft:no_such_boots", 1)),
                ]),
            ),
            ("SelectedItemSlot".into(), Tag::Int(2)),
        ]);
        let inv = load_player_inventory(&player);
        assert_eq!(inv.items[2].count(), 3);
        assert_eq!(inv.equipped(EquipmentSlot::Head), &enchanted);
        assert_eq!(inv.equipped(EquipmentSlot::Body).item_name(), "minecraft:stone");
        assert_eq!(inv.selected, 2);
        // (The unknown item and the unknown boots are not kept.)
        assert!(inv.equipped(EquipmentSlot::Feet).is_empty());
        let mut out = Tag::Compound(vec![("Health".into(), Tag::Float(20.0))]);
        save_player_inventory(&inv, &mut out);
        let again = load_player_inventory(&out);
        assert_eq!(again, inv);
        assert_eq!(out.get("equipment").and_then(|e| e.get("feet")), None);
        assert_eq!(out.get("Health"), Some(&Tag::Float(20.0)));
    }
}
