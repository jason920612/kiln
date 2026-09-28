//! The villager trading screen: `MerchantMenu`, `MerchantContainer` and `MerchantResultSlot`.
//!
//! Slots: payment A 0, payment B 1, result 2, main inventory 3-29, hotbar 30-38. The menu
//! keeps a copy of the merchant's offers; what the merchant must hear about (a trade was made,
//! the result preview changed, the screen closed) queues up as [`MerchantEvent`]s for the
//! simulation to drain ([`MerchantState::drain`]) and apply to the villager.

use crate::container::{Container, remove_item, take_item};
use crate::menu::{CraftGrid, Env, Menu};
use crate::menus::MenuKind;
use crate::slot::{Slot, SlotKind, Source};
use crate::stack::{StackExt, same_item_same_components};
use kiln_item::ItemStack;
use kiln_item::trading::{self, ItemCost, MerchantOffer};

pub const PAYMENT_A: usize = 0;
pub const PAYMENT_B: usize = 1;
pub const RESULT: usize = 2;
const INV_START: usize = 3;
const INV_END: usize = 30;
const HOTBAR_END: usize = 39;

/// What the merchant (villager) is told, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum MerchantEvent {
    /// `Merchant.notifyTrade(offer)`: offer `index` was used once (the menu's copy already
    /// counts the use).
    Trade { index: usize },
    /// `Merchant.notifyTradeUpdated(result)`: the preview changed (yes/no sound).
    TradeUpdated { has_result: bool },
    /// `Stats.TRADED_WITH_VILLAGER`.
    TradedStat,
    /// `MerchantMenu.removed`: the merchant stops trading.
    Closed,
}

/// `MerchantContainer` with the menu's view of the merchant.
#[derive(Debug, Clone)]
pub struct MerchantState {
    /// The merchant's entity id.
    pub merchant: i32,
    pub offers: Vec<MerchantOffer>,
    pub items: [ItemStack; 3],
    /// `activeOffer` (index into `offers`).
    pub active_offer: Option<usize>,
    pub selection_hint: i32,
    pub future_xp: i32,
    /// `MerchantResultSlot.removeCount`.
    remove_count: i32,
    events: Vec<MerchantEvent>,
}

impl MerchantState {
    pub fn new(merchant: i32, offers: Vec<MerchantOffer>) -> Self {
        MerchantState {
            merchant,
            offers,
            items: std::array::from_fn(|_| ItemStack::empty()),
            active_offer: None,
            selection_hint: 0,
            future_xp: 0,
            remove_count: 0,
            events: Vec::new(),
        }
    }

    /// Events for the merchant since the last call.
    pub fn drain(&mut self) -> Vec<MerchantEvent> {
        std::mem::take(&mut self.events)
    }

    fn is_payment(slot: usize) -> bool {
        slot == PAYMENT_A || slot == PAYMENT_B
    }

    /// `updateSellItem`: the offer the payments satisfy (either order) fills the result slot.
    pub fn update_sell_item(&mut self) {
        self.active_offer = None;
        let (a, b) = if self.items[PAYMENT_A].is_empty() {
            (self.items[PAYMENT_B].clone(), ItemStack::empty())
        } else {
            (self.items[PAYMENT_A].clone(), self.items[PAYMENT_B].clone())
        };
        if a.is_empty() {
            self.set_item(RESULT, ItemStack::empty());
            self.future_xp = 0;
            return;
        }
        if !self.offers.is_empty() {
            let mut offer = trading::recipe_for(&self.offers, &a, &b, self.selection_hint);
            if offer.is_none_or(|o| self.offers[o].is_out_of_stock()) {
                self.active_offer = offer;
                offer = trading::recipe_for(&self.offers, &b, &a, self.selection_hint);
            }
            match offer.filter(|&o| !self.offers[o].is_out_of_stock()) {
                Some(o) => {
                    self.active_offer = Some(o);
                    let result = self.offers[o].result.copy();
                    self.set_item(RESULT, result);
                    self.future_xp = self.offers[o].xp;
                }
                None => {
                    self.set_item(RESULT, ItemStack::empty());
                    self.future_xp = 0;
                }
            }
        }
        let has_result = !self.items[RESULT].is_empty();
        self.events.push(MerchantEvent::TradeUpdated { has_result });
    }

    /// `setSelectionHint`.
    pub fn set_selection_hint(&mut self, hint: i32) {
        self.selection_hint = hint;
        self.update_sell_item();
    }
}

impl Container for MerchantState {
    fn size(&self) -> usize {
        3
    }

    fn item(&self, slot: usize) -> &ItemStack {
        &self.items[slot]
    }

    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        &mut self.items[slot]
    }

    fn set_item(&mut self, slot: usize, mut stack: ItemStack) {
        let max = self.max_stack_size_for(&stack);
        if stack.count() > max {
            stack.set_count(max);
        }
        self.items[slot] = stack;
        if Self::is_payment(slot) {
            self.update_sell_item();
        }
    }

    /// The result slot always gives its whole stack, without an update.
    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        if slot == RESULT && !self.items[RESULT].is_empty() {
            let n = self.items[RESULT].count();
            return remove_item(&mut self.items, slot, n);
        }
        let removed = remove_item(&mut self.items, slot, count);
        if !removed.is_empty() && Self::is_payment(slot) {
            self.update_sell_item();
        }
        removed
    }

    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        take_item(&mut self.items, slot)
    }

    fn set_changed(&mut self) {
        self.update_sell_item();
    }
}

impl Menu {
    /// `MerchantMenu` over `state` (the merchant's offers).
    pub fn merchant(container_id: i32, state: MerchantState) -> Menu {
        let mut slots = vec![
            Slot::new(Source::Merchant, PAYMENT_A, SlotKind::Normal),
            Slot::new(Source::Merchant, PAYMENT_B, SlotKind::Normal),
            Slot::new(Source::Merchant, RESULT, SlotKind::MerchantResult),
        ];
        slots.extend((crate::inventory::HOTBAR_SIZE..crate::inventory::MAIN_SIZE).map(|i| Slot::new(Source::Player, i, SlotKind::Normal)));
        slots.extend((0..crate::inventory::HOTBAR_SIZE).map(|i| Slot::new(Source::Player, i, SlotKind::Normal)));
        let mut menu = Menu::with_slots(MenuKind::Merchant, container_id, slots, 0, CraftGrid::default());
        menu.merchant = Some(Box::new(state));
        menu
    }

    pub fn merchant_state(&self) -> Option<&MerchantState> {
        self.merchant.as_deref()
    }

    pub fn merchant_state_mut(&mut self) -> Option<&mut MerchantState> {
        self.merchant.as_deref_mut()
    }

    /// `ServerGamePacketListenerImpl.handleSelectTrade`: `setSelectionHint` then
    /// `tryMoveItems` (the payments go back and the selected offer's items come in, up to full
    /// stacks).
    pub fn select_trade(&mut self, env: &mut Env, selected: i32) {
        let Some(st) = self.merchant.as_deref_mut() else { return };
        st.set_selection_hint(selected);
        self.try_move_items(env, selected);
    }

    /// `MerchantMenu.tryMoveItems`.
    fn try_move_items(&mut self, env: &mut Env, selected: i32) {
        let Some(offer) = usize::try_from(selected).ok().and_then(|i| self.merchant.as_deref()?.offers.get(i).cloned()) else { return };
        for slot in [PAYMENT_A, PAYMENT_B] {
            let mut stack = self.merchant.as_deref().map_or_else(ItemStack::empty, |m| m.items[slot].clone());
            if stack.is_empty() {
                continue;
            }
            // `moveItemStackTo` mutates the payment stack in place, then it is set back.
            let moved = self.move_item_stack_to(env, &mut stack, INV_START, HOTBAR_END, true);
            if let Some(m) = self.merchant.as_deref_mut() {
                m.items[slot] = stack.clone();
            }
            if !moved {
                return;
            }
            if let Some(m) = self.merchant.as_deref_mut() {
                m.set_item(slot, stack);
            }
        }
        let empty = self.merchant.as_deref().is_some_and(|m| m.items[PAYMENT_A].is_empty() && m.items[PAYMENT_B].is_empty());
        if empty {
            self.move_from_inventory_to_payment_slot(env, PAYMENT_A, &offer.cost_a);
            if let Some(b) = &offer.cost_b {
                self.move_from_inventory_to_payment_slot(env, PAYMENT_B, b);
            }
        }
    }

    /// `moveFromInventoryToPaymentSlot`: merges matching stacks of slots 3-38 (in order) into
    /// the payment slot, up to a full stack.
    fn move_from_inventory_to_payment_slot(&mut self, env: &mut Env, payment: usize, cost: &ItemCost) {
        for i in INV_START..HOTBAR_END {
            let inv = self.item(env, i).clone();
            if inv.is_empty() || !cost.test(&inv) {
                continue;
            }
            let Some(cur) = self.merchant.as_deref().map(|m| m.items[payment].clone()) else { return };
            if !cur.is_empty() && !same_item_same_components(&inv, &cur) {
                continue;
            }
            let max = inv.max_stack_size();
            let n = (max - cur.count()).min(inv.count());
            let merged = inv.copy_with_count(cur.count() + n);
            // `inv.shrink(n)`: in place, without notifying the inventory.
            self.item_mut(env, i).shrink_count(n);
            if let Some(m) = self.merchant.as_deref_mut() {
                m.set_item(payment, merged.clone());
            }
            if merged.count() >= max {
                break;
            }
        }
    }
}

/// `MerchantResultSlot.mayPickup`: the active offer must still be paid for.
pub(crate) fn result_may_pickup(menu: &Menu) -> bool {
    let Some(m) = menu.merchant.as_deref() else { return false };
    let Some(o) = m.active_offer.and_then(|i| m.offers.get(i)) else { return false };
    let (a, b) = (&m.items[PAYMENT_A], &m.items[PAYMENT_B]);
    o.satisfied_by(a, b) || o.satisfied_by(b, a)
}

/// `MerchantResultSlot.remove` counting and `checkTakeAchievements` share the menu's counter.
pub(crate) fn take_remove_count(menu: &mut Menu) -> i32 {
    menu.merchant.as_deref_mut().map_or(0, |m| std::mem::take(&mut m.remove_count))
}

pub(crate) fn add_remove_count(menu: &mut Menu, n: i32) {
    if let Some(m) = menu.merchant.as_deref_mut() {
        m.remove_count += n;
    }
}

/// `MerchantResultSlot.onTake` after `checkTakeAchievements`: the trade.
pub(crate) fn on_take(menu: &mut Menu) {
    let Some(m) = menu.merchant.as_deref_mut() else { return };
    let Some(index) = m.active_offer else { return };
    let offer = m.offers[index].clone();
    let (mut a, mut b) = (m.items[PAYMENT_A].clone(), m.items[PAYMENT_B].clone());
    if offer.take(&mut a, &mut b) || offer.take(&mut b, &mut a) {
        m.offers[index].increase_uses();
        m.events.push(MerchantEvent::Trade { index });
        m.events.push(MerchantEvent::TradedStat);
        m.set_item(PAYMENT_A, a);
        m.set_item(PAYMENT_B, b);
    }
}

/// `MerchantMenu.quickMoveStack`: the result and payments go to the inventory; the inventory
/// only moves between its main part and the hotbar (never into the payment slots).
pub(crate) fn quick_move_stack(menu: &mut Menu, env: &mut Env, i: usize) -> ItemStack {
    let mut stack = menu.item(env, i).clone();
    let copy = stack.copy();
    let ok = match i {
        RESULT => menu.move_item_stack_to(env, &mut stack, INV_START, HOTBAR_END, true),
        PAYMENT_A | PAYMENT_B => menu.move_item_stack_to(env, &mut stack, INV_START, HOTBAR_END, false),
        INV_START..INV_END => menu.move_item_stack_to(env, &mut stack, INV_END, HOTBAR_END, false),
        INV_END..HOTBAR_END => menu.move_item_stack_to(env, &mut stack, INV_START, INV_END, false),
        _ => true,
    };
    if !ok {
        return ItemStack::empty();
    }
    let mut old = copy.clone();
    if i == RESULT {
        menu.on_quick_craft(env, i, &stack, &mut old);
    }
    menu.finish_quick_move(env, i, stack, old, false).0
}

/// `MerchantMenu.removed` after the carried stack went back: the merchant stops trading and
/// the payments return to the inventory (dropped for a dead or disconnected player).
pub(crate) fn removed(menu: &mut Menu, env: &mut Env) {
    let Some(m) = menu.merchant.as_deref_mut() else { return };
    m.events.push(MerchantEvent::Closed);
    let a = m.remove_item_no_update(PAYMENT_A);
    let b = m.remove_item_no_update(PAYMENT_B);
    for s in [a, b] {
        if s.is_empty() {
            continue;
        }
        if env.player.dead || env.player.removed {
            env.out.push(crate::effect::Effect::Drop { stack: s, retain_ownership: false });
        } else {
            let (updates, left) = env.inventory.place_item_back(s, env.player.infinite_materials);
            for u in updates {
                env.out.push(crate::effect::Effect::SetPlayerInventory { slot: u.slot as i32, stack: u.stack });
            }
            if let Some(left) = left {
                env.out.push(crate::effect::Effect::Drop { stack: left, retain_ownership: false });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::click::ContainerInput;
    use crate::menu::{NoWorld, PlayerFlags};
    use crate::{Effect, PlayerInventory, Rules};

    fn stack(name: &str, n: i32) -> ItemStack {
        ItemStack::of(name, n).unwrap()
    }

    fn cost(name: &str, n: i32) -> ItemCost {
        ItemCost::new(kiln_item::registry::ITEM.id(name).unwrap(), n)
    }

    fn offers() -> Vec<MerchantOffer> {
        vec![
            MerchantOffer::new(cost("minecraft:wheat", 20), None, stack("minecraft:emerald", 1), 2, 2, 0.05),
            MerchantOffer::new(cost("minecraft:emerald", 1), None, stack("minecraft:bread", 6), 16, 1, 0.05),
        ]
    }

    struct P {
        inv: PlayerInventory,
        out: Vec<Effect>,
        world: NoWorld,
    }

    impl P {
        fn env<'a>(&'a mut self, rules: &'a Rules) -> Env<'a> {
            Env { inventory: &mut self.inv, block: None, player: PlayerFlags::default(), rules, world: &mut self.world, out: &mut self.out }
        }
    }

    #[test]
    fn select_take_and_run_out() {
        let rules = Rules::with_recipes(Default::default());
        let mut p = P { inv: PlayerInventory::new(), out: Vec::new(), world: NoWorld };
        p.inv.set_item(0, stack("minecraft:wheat", 30));
        p.inv.set_item(9, stack("minecraft:wheat", 64));
        let mut menu = Menu::merchant(3, MerchantState::new(77, offers()));
        assert_eq!(menu.len(), 39);
        assert_eq!(menu.kind.menu_type(), Some("minecraft:merchant"));
        menu.open(&mut p.env(&rules));
        // Selecting the wheat trade pulls wheat in, the main inventory first, up to a stack.
        menu.select_trade(&mut p.env(&rules), 0);
        let st = menu.merchant_state().unwrap();
        assert_eq!(st.items[PAYMENT_A], stack("minecraft:wheat", 64));
        assert_eq!(st.items[RESULT], stack("minecraft:emerald", 1));
        assert_eq!(p.inv.item(9).count(), 0);
        assert_eq!(p.inv.item(0).count(), 30);
        // Taking the result pays and trades.
        menu.clicked(&mut p.env(&rules), RESULT as i32, 0, ContainerInput::Pickup).unwrap();
        assert_eq!(menu.carried(), &stack("minecraft:emerald", 1));
        let st = menu.merchant_state_mut().unwrap();
        assert_eq!(st.items[PAYMENT_A].count(), 44);
        assert_eq!(st.offers[0].uses, 1);
        let events = st.drain();
        assert!(events.contains(&MerchantEvent::Trade { index: 0 }), "{events:?}");
        // Shift-clicking the result trades until the offer is out of stock (2 uses).
        menu.clicked(&mut p.env(&rules), RESULT as i32, 0, ContainerInput::QuickMove).unwrap();
        let st = menu.merchant_state_mut().unwrap();
        assert_eq!(st.offers[0].uses, 2);
        assert!(st.items[RESULT].is_empty());
        assert_eq!(st.items[PAYMENT_A].count(), 24);
        assert_eq!(st.drain().iter().filter(|e| matches!(e, MerchantEvent::Trade { .. })).count(), 1);
        // Closing returns the payment.
        menu.removed(&mut p.env(&rules));
        assert_eq!(menu.merchant_state_mut().unwrap().drain().last(), Some(&MerchantEvent::Closed));
        let wheat: i32 = (0..36).map(|i| p.inv.item(i)).filter(|s| !s.is_empty() && s.item_name() == "minecraft:wheat").map(ItemStack::count).sum();
        assert_eq!(wheat, 30 + 24);
    }

    #[test]
    fn payments_in_either_slot() {
        let rules = Rules::with_recipes(Default::default());
        let mut p = P { inv: PlayerInventory::new(), out: Vec::new(), world: NoWorld };
        let mut menu = Menu::merchant(1, MerchantState::new(5, offers()));
        menu.open(&mut p.env(&rules));
        menu.set_carried(stack("minecraft:emerald", 3));
        menu.clicked(&mut p.env(&rules), PAYMENT_B as i32, 0, ContainerInput::Pickup).unwrap();
        let st = menu.merchant_state().unwrap();
        assert_eq!(st.items[RESULT], stack("minecraft:bread", 6));
        assert_eq!(st.active_offer, Some(1));
        // Shift-clicking from the inventory never fills the payment slots.
        p.inv.set_item(10, stack("minecraft:emerald", 5));
        menu.clicked(&mut p.env(&rules), 4, 0, ContainerInput::QuickMove).unwrap();
        assert_eq!(menu.merchant_state().unwrap().items[PAYMENT_A], ItemStack::empty());
    }
}
