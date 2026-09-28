//! What menu operations produce: clientbound packets for the player, in the order vanilla sends
//! them, and world side effects for the simulation to carry out.

use bytes::{Bytes, BytesMut};
use kiln_data::packets::play::clientbound as cb;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_proto::WriteExt;

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// `ClientboundContainerSetContentPacket`.
    SetContent { container_id: i32, state_id: i32, items: Vec<ItemStack>, carried: ItemStack },
    /// `ClientboundContainerSetSlotPacket`.
    SetSlot { container_id: i32, state_id: i32, slot: i16, stack: ItemStack },
    /// `ClientboundSetCursorItemPacket`.
    SetCursor { stack: ItemStack },
    /// `ClientboundContainerSetDataPacket`.
    SetData { container_id: i32, id: i16, value: i16 },
    /// `ClientboundSetPlayerInventoryPacket` (an inventory index, not a menu slot).
    SetPlayerInventory { slot: i32, stack: ItemStack },
    /// `Player.drop(stack, retainOwnership, ...)`: spawn an item entity (vanilla throws it
    /// from the player's eyes).
    Drop { stack: ItemStack, retain_ownership: bool },
    /// `LivingEntity.onEquipItem`: equip sound and game event when armor or the off hand
    /// changes through a menu.
    Equip { slot: EquipmentSlot, old: ItemStack, new: ItemStack },
    /// `ItemStack.onCraftedBy(player, amount)`: the `crafted` statistic (and map post-processing,
    /// already applied through [`crate::World::post_process_map`]).
    Crafted { item: i32, amount: i32 },
    /// A player inventory slot changed (`InventoryChangeTrigger`, from the menu's slot listener).
    InventoryChanged { slot: usize, stack: ItemStack },
    /// The grindstone's result was taken: experience orbs at the grindstone and its sound
    /// (level event 1042).
    GrindstoneUsed { experience: i32 },
    /// The anvil's result was taken: the player loses `levels` experience levels, and the anvil
    /// wears (`AnvilMenu.onTake`'s block part: level events 1030 and 1029).
    AnvilUsed { levels: i32 },
}

impl Effect {
    /// The packet body (id + data) for packet effects.
    pub fn encode(&self) -> Option<Bytes> {
        let mut b = BytesMut::with_capacity(32);
        match self {
            Effect::SetContent { container_id, state_id, items, carried } => {
                b.put_varint(cb::CONTAINER_SET_CONTENT);
                b.put_varint(*container_id);
                b.put_varint(*state_id);
                b.put_varint(items.len() as i32);
                for s in items {
                    s.write_optional(&mut b);
                }
                carried.write_optional(&mut b);
            }
            Effect::SetSlot { container_id, state_id, slot, stack } => {
                b.put_varint(cb::CONTAINER_SET_SLOT);
                b.put_varint(*container_id);
                b.put_varint(*state_id);
                bytes::BufMut::put_i16(&mut b, *slot);
                stack.write_optional(&mut b);
            }
            Effect::SetCursor { stack } => {
                b.put_varint(cb::SET_CURSOR_ITEM);
                stack.write_optional(&mut b);
            }
            Effect::SetData { container_id, id, value } => {
                b.put_varint(cb::CONTAINER_SET_DATA);
                b.put_varint(*container_id);
                bytes::BufMut::put_i16(&mut b, *id);
                bytes::BufMut::put_i16(&mut b, *value);
            }
            Effect::SetPlayerInventory { slot, stack } => {
                b.put_varint(cb::SET_PLAYER_INVENTORY);
                b.put_varint(*slot);
                stack.write_optional(&mut b);
            }
            _ => return None,
        }
        Some(b.freeze())
    }

    pub fn is_packet(&self) -> bool {
        matches!(
            self,
            Effect::SetContent { .. }
                | Effect::SetSlot { .. }
                | Effect::SetCursor { .. }
                | Effect::SetData { .. }
                | Effect::SetPlayerInventory { .. }
        )
    }
}

/// `ClientboundContainerClosePacket`.
pub fn container_close(container_id: i32) -> Bytes {
    let mut b = BytesMut::with_capacity(4);
    b.put_varint(cb::CONTAINER_CLOSE);
    b.put_varint(container_id);
    b.freeze()
}

/// `ClientboundOpenScreenPacket`: container id, `minecraft:menu` id and the title (NBT text).
pub fn open_screen(container_id: i32, menu_type: i32, title: &kiln_proto::nbt::Tag) -> Bytes {
    let mut b = BytesMut::with_capacity(32);
    b.put_varint(cb::OPEN_SCREEN);
    b.put_varint(container_id);
    b.put_varint(menu_type);
    title.write_network(&mut b);
    b.freeze()
}
