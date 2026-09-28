//! Serverbound menu packets and their handlers (`ServerGamePacketListenerImpl.handleContainerClick`
//! and friends).

use crate::effect::Effect;
use crate::menu::{ClickCrash, Env, Menu};
use kiln_item::{HashedStack, ItemStack};
use kiln_proto::{DecodeError, Reader};

/// `ContainerInput` (formerly `ClickType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerInput {
    Pickup,
    QuickMove,
    Swap,
    Clone,
    Throw,
    QuickCraft,
    PickupAll,
}

impl ContainerInput {
    /// `BY_ID` with `OutOfBoundsStrategy.ZERO`: unknown ids are `PICKUP`.
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => ContainerInput::QuickMove,
            2 => ContainerInput::Swap,
            3 => ContainerInput::Clone,
            4 => ContainerInput::Throw,
            5 => ContainerInput::QuickCraft,
            6 => ContainerInput::PickupAll,
            _ => ContainerInput::Pickup,
        }
    }

    pub fn id(self) -> i32 {
        self as i32
    }
}

/// `ClickAction`: left (primary) or right (secondary) button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    Primary,
    Secondary,
}

/// `ServerboundContainerClickPacket`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerClick {
    pub container_id: i32,
    pub state_id: i32,
    pub slot: i16,
    pub button: i8,
    pub input: ContainerInput,
    /// What the client predicts each changed slot now holds (a map on the wire: at most 128).
    pub changed: Vec<(i16, HashedStack)>,
    pub carried: HashedStack,
}

/// At most 128 changed slots (`MAX_SLOT_COUNT`).
const MAX_CHANGED: usize = 128;

impl ContainerClick {
    /// Decodes the packet body (after the packet id).
    pub fn read(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let container_id = r.varint()?;
        let state_id = r.varint()?;
        let slot = r.i16()?;
        let button = r.i8()?;
        let input = ContainerInput::from_id(r.varint()?);
        let n = r.varint()?;
        let n = usize::try_from(n).map_err(|_| DecodeError::Invalid("negative changed slot count"))?;
        if n > MAX_CHANGED {
            return Err(DecodeError::Invalid("too many changed slots"));
        }
        let mut changed = Vec::with_capacity(n);
        for _ in 0..n {
            changed.push((r.i16()?, HashedStack::read(r)?));
        }
        let carried = HashedStack::read(r)?;
        Ok(ContainerClick { container_id, state_id, slot, button, input, changed, carried })
    }

    /// Decodes a whole packet body and rejects trailing bytes.
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let c = Self::read(&mut r)?;
        r.finish()?;
        Ok(c)
    }

    pub fn write(&self, out: &mut bytes::BytesMut) {
        use kiln_proto::WriteExt;
        out.put_varint(self.container_id);
        out.put_varint(self.state_id);
        bytes::BufMut::put_i16(out, self.slot);
        bytes::BufMut::put_i8(out, self.button);
        out.put_varint(self.input.id());
        out.put_varint(self.changed.len() as i32);
        for (slot, h) in &self.changed {
            bytes::BufMut::put_i16(out, *slot);
            h.write(out);
        }
        self.carried.write(out);
    }
}

/// `handleContainerClick`. `still_valid` is the menu's `stillValid(player)` (distance to the
/// block, block still there). The click is applied even with a stale state id; the client's
/// predicted slots are then trusted only as hashes, and a stale id or any mismatch makes the
/// server send the corrections vanilla sends.
pub fn handle_container_click(menu: &mut Menu, env: &mut Env, click: &ContainerClick, still_valid: bool) -> Result<(), ClickCrash> {
    if menu.container_id != click.container_id {
        return Ok(());
    }
    if env.player.spectator || env.player.dead {
        menu.send_all_data_to_remote(env);
        return Ok(());
    }
    if !still_valid {
        return Ok(());
    }
    let slot = click.slot as i32;
    if !menu.is_valid_slot_index(slot) {
        return Ok(());
    }
    let full = click.state_id != menu.state_id();
    menu.suppress_remote_updates();
    menu.clicked(env, slot, click.button as i32, click.input)?;
    // A map on the wire: the last entry for a slot wins.
    for (slot, hash) in &click.changed {
        menu.set_remote_slot_unsafe(*slot as i32, hash.clone());
    }
    menu.set_remote_carried(click.carried.clone());
    menu.resume_remote_updates();
    if full {
        menu.broadcast_full_state(env);
    } else {
        menu.broadcast_changes(env);
    }
    Ok(())
}

/// `handleContainerButtonClick` (enchanting table options, stonecutter recipes, loom
/// patterns...): returns whether the menu accepted the button.
pub fn handle_container_button_click(menu: &mut Menu, env: &mut Env, container_id: i32, button: i32, still_valid: bool) -> bool {
    if menu.container_id != container_id || env.player.spectator || !still_valid {
        return false;
    }
    let accepted = menu.click_menu_button(env, button);
    if accepted {
        menu.broadcast_changes(env);
    }
    accepted
}

/// `handleRenameItem`: the anvil's name field (ignored unless an anvil menu is open).
pub fn handle_rename_item(menu: &mut Menu, env: &mut Env, name: &str, still_valid: bool) -> bool {
    if menu.kind != crate::MenuKind::Anvil || !still_valid {
        return false;
    }
    crate::workstation::anvil_set_item_name(menu, env, name)
}

/// `handleSetCreativeModeSlot`: `slot` is an inventory-menu slot (1-45), or negative to drop
/// the stack. Only players with infinite materials may use it; stacks over their maximum size
/// are ignored. `drop_allowed` is vanilla's drop spam throttle (20 drops per second).
pub fn handle_set_creative_slot(inventory_menu: &mut Menu, env: &mut Env, slot: i16, stack: ItemStack, drop_allowed: bool) {
    if !env.player.infinite_materials {
        return;
    }
    let drop = slot < 0;
    let in_menu = (1..=45).contains(&slot);
    let valid = stack.is_empty() || stack.count() <= stack.max_stack_size();
    if in_menu && valid {
        let i = slot as usize;
        inventory_menu.set_by_player(env, i, stack.clone());
        inventory_menu.set_remote_slot(i, &stack);
        inventory_menu.broadcast_changes(env);
    } else if drop && valid && drop_allowed && !stack.is_empty() {
        env.out.push(Effect::Drop { stack, retain_ownership: true });
    }
}

/// Decodes `ServerboundSetCreativeModeSlotPacket` from kiln-proto's raw form.
pub fn creative_stack(raw: Option<&kiln_proto::packets::ItemStack>) -> Result<ItemStack, DecodeError> {
    match raw {
        None => Ok(ItemStack::empty()),
        Some(s) => ItemStack::from_untrusted(s),
    }
}

/// `handleContainerClose` / `ServerPlayer.doCloseContainer`: closes the open menu (whatever id
/// the packet names) and hands its view of the player inventory back to the inventory menu.
/// Call with the open menu (or the inventory menu itself when none is open).
pub fn close_container(open: &mut Menu, inventory_menu: Option<&mut Menu>, env: &mut Env) {
    open.removed(env);
    if let Some(inv) = inventory_menu {
        inv.transfer_state(open);
    }
}

/// `ServerPlayer.nextContainerCounter`: container ids cycle through 1-100.
pub fn next_container_id(counter: &mut i32) -> i32 {
    *counter = *counter % 100 + 1;
    *counter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_packet_round_trips() {
        let stone = ItemStack::of("stone", 3).unwrap();
        let c = ContainerClick {
            container_id: 2,
            state_id: 7,
            slot: -999,
            button: 1,
            input: ContainerInput::QuickCraft,
            changed: vec![(4, HashedStack::of(&stone).unwrap()), (5, HashedStack::Empty)],
            carried: HashedStack::Empty,
        };
        let mut out = bytes::BytesMut::new();
        c.write(&mut out);
        assert_eq!(ContainerClick::decode(&out).unwrap(), c);
        assert_eq!(ContainerInput::from_id(99), ContainerInput::Pickup);
    }
}
