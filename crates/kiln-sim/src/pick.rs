//! Middle click: `ServerboundPickItemFromBlock` (`handlePickItemFromBlock`,
//! `BlockState.getCloneItemStack`, `tryPickItem`).
//!
//! The player looks for the item in the main inventory (`Inventory.findSlotMatchingItem`): one
//! in the hotbar is selected, one further in is swapped into a hotbar slot
//! (`getSuitableHotbarSlot`); a creative player without it gets it added
//! (`addAndPickItem`). The held slot is sent back.
//!
//! Not simulated: block entity data on the picked item (`include_data`, creative: the block
//! entity's saved fields and components), the pot decorations, banner patterns of the block
//! entity that `getCloneItemStack` carries over. (Picking from entities is `entities::pick_item_from_entity`.)

use crate::Player;
use kiln_blocks::BlockPos;
use kiln_item::component::BlockItemStateProperties;
use kiln_item::{ItemStack, keys};
use kiln_proto::packets;

/// `Block.getCloneItemStack` (without data): the item of the block, or what the block that
/// has no item of its own stands for (a seed for its crop, the flower in a pot...).
pub(crate) fn clone_item(state: u16) -> Option<ItemStack> {
    let name = kiln_data::blocks_types::block_of(state).name;
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let item: String = match short {
        "attached_melon_stem" => "melon_seeds".into(),
        "attached_pumpkin_stem" => "pumpkin_seeds".into(),
        "bamboo_sapling" => "bamboo".into(),
        "big_dripleaf_stem" => "big_dripleaf".into(),
        "cave_vines_plant" => "glow_berries".into(),
        "kelp_plant" => "kelp".into(),
        "tall_seagrass" => "seagrass".into(),
        "twisting_vines_plant" => "twisting_vines".into(),
        "weeping_vines_plant" => "weeping_vines".into(),
        "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => "cauldron".into(),
        "piston_head" => {
            let sticky = kiln_blocks::state::get(state, "type") == Some("sticky");
            (if sticky { "sticky_piston" } else { "piston" }).to_owned()
        }
        // `FlowerPotBlock.getCloneItemStack`: the plant in the pot.
        "potted_azalea_bush" => "azalea".into(),
        "potted_flowering_azalea_bush" => "flowering_azalea".into(),
        s if s.starts_with("potted_") => s["potted_".len()..].into(),
        // `CandleCakeBlock.getCloneItemStack`: the cake.
        s if s.ends_with("candle_cake") => "cake".into(),
        _ => kiln_data::block_logic::item_of_block(name)?.strip_prefix("minecraft:")?.to_owned(),
    };
    let mut stack = ItemStack::of(&format!("minecraft:{item}"), 1)?;
    // `LightBlock` and `TestBlock` keep their state in the item.
    match short {
        "light" => stack.insert(keys::BLOCK_STATE, BlockItemStateProperties(vec![("level".into(), kiln_blocks::state::get(state, "level")?.to_owned())])),
        "test_block" => stack.insert(keys::BLOCK_STATE, BlockItemStateProperties(vec![("mode".into(), kiln_blocks::state::get(state, "mode")?.to_owned())])),
        _ => {}
    }
    Some(stack)
}

impl Player {
    /// `ServerGamePacketListenerImpl.tryPickItem`.
    pub(crate) fn try_pick_item(&mut self, stack: &ItemStack) {
        let found = self.inv.items.iter().position(|s| !s.is_empty() && stack.is_same_item_same_components(s));
        match found {
            Some(slot) if slot < 9 => self.inv.selected = slot,
            // `Inventory.pickSlot`: the item swaps with the hotbar slot that is free or plain.
            Some(slot) => {
                self.inv.selected = self.suitable_hotbar_slot();
                self.inv.items.swap(self.inv.selected, slot);
            }
            // `Inventory.addAndPickItem`.
            None if self.game_mode == 1 => {
                self.inv.selected = self.suitable_hotbar_slot();
                if !self.inv.items[self.inv.selected].is_empty()
                    && let Some(free) = self.inv.free_slot()
                {
                    self.inv.items[free] = self.inv.items[self.inv.selected].clone();
                }
                self.inv.items[self.inv.selected] = stack.clone();
            }
            None => {}
        }
        self.inv.times_changed += 1;
        let held = self.inv.selected as i32;
        self.send(packets::set_held_slot(held));
    }

    /// `Inventory.getSuitableHotbarSlot`: the first empty hotbar slot from the selected one on,
    /// else the first one without enchantments, else the selected one.
    fn suitable_hotbar_slot(&self) -> usize {
        let from = self.inv.selected;
        let at = |i: usize| (from + i) % 9;
        if let Some(i) = (0..9).map(at).find(|&s| self.inv.items[s].is_empty()) {
            return i;
        }
        let enchanted = |s: &ItemStack| s.get(keys::ENCHANTMENTS).is_some_and(|e| !e.0.is_empty());
        (0..9).map(at).find(|&s| !enchanted(&self.inv.items[s])).unwrap_or(from)
    }

    /// `handlePickItemFromBlock`: `block` is the state at the position (`None`: not loaded).
    pub(crate) fn pick_item_from_block(&mut self, pos: BlockPos, block: Option<u16>) {
        if !self.can_reach_block([pos.x, pos.y, pos.z], 1.0) {
            return;
        }
        let Some(state) = block else { return };
        let Some(stack) = clone_item(state) else { return };
        self.try_pick_item(&stack);
    }
}
