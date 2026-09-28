//! The menu kinds: slot layouts, `quickMoveStack` (shift-click) rules and per-kind hooks.

use crate::container::{Container, SimpleContainer};
use crate::inventory::{HOTBAR_SIZE, MAIN_SIZE, SLOT_OFFHAND};
use crate::menu::{CraftGrid, Env, Menu};
use crate::slot::{Slot, SlotKind, Source, equipment_slot_for_item};
use crate::stack::StackExt;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

/// Which furnace (`AbstractFurnaceMenu` subclass): they differ in the recipes they accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FurnaceKind {
    Furnace,
    BlastFurnace,
    Smoker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuKind {
    /// The player's own inventory (`InventoryMenu`, container id 0).
    Inventory,
    /// `ChestMenu` with 1-6 rows (chests, barrels, ender chests...).
    Generic { rows: u8 },
    /// `DispenserMenu` (dispensers and droppers).
    Generic3x3,
    /// `HopperMenu`.
    Hopper,
    /// `ShulkerBoxMenu`.
    ShulkerBox,
    /// `CraftingMenu` (crafting table).
    Crafting,
    /// `AbstractFurnaceMenu`.
    Furnace(FurnaceKind),
    /// `StonecutterMenu`.
    Stonecutter,
    /// `SmithingMenu`.
    Smithing,
    /// `MerchantMenu` (see [`crate::merchant`]).
    Merchant,
}

impl MenuKind {
    /// The `minecraft:menu` registry name (for `open_screen`); `None` for the inventory menu,
    /// which the client always has open.
    pub fn menu_type(self) -> Option<&'static str> {
        Some(match self {
            MenuKind::Inventory => return None,
            MenuKind::Generic { rows } => {
                ["minecraft:generic_9x1", "minecraft:generic_9x2", "minecraft:generic_9x3", "minecraft:generic_9x4", "minecraft:generic_9x5", "minecraft:generic_9x6"]
                    [rows.clamp(1, 6) as usize - 1]
            }
            MenuKind::Generic3x3 => "minecraft:generic_3x3",
            MenuKind::Hopper => "minecraft:hopper",
            MenuKind::ShulkerBox => "minecraft:shulker_box",
            MenuKind::Crafting => "minecraft:crafting",
            MenuKind::Furnace(FurnaceKind::Furnace) => "minecraft:furnace",
            MenuKind::Furnace(FurnaceKind::BlastFurnace) => "minecraft:blast_furnace",
            MenuKind::Furnace(FurnaceKind::Smoker) => "minecraft:smoker",
            MenuKind::Stonecutter => "minecraft:stonecutter",
            MenuKind::Smithing => "minecraft:smithing",
            MenuKind::Merchant => "minecraft:merchant",
        })
    }

    /// Network id of [`MenuKind::menu_type`].
    pub fn menu_type_id(self) -> Option<i32> {
        kiln_data::builtin_id("minecraft:menu", self.menu_type()?)
    }

    /// Slots of the block container, before the player inventory slots.
    pub fn block_size(self) -> usize {
        match self {
            MenuKind::Inventory | MenuKind::Crafting | MenuKind::Stonecutter | MenuKind::Smithing | MenuKind::Merchant => 0,
            MenuKind::Generic { rows } => rows as usize * 9,
            MenuKind::Generic3x3 => 9,
            MenuKind::Hopper => 5,
            MenuKind::ShulkerBox => 27,
            MenuKind::Furnace(_) => 3,
        }
    }

    /// `canDragTo`: result slots take no drag.
    pub fn can_drag_to(self, slot: Slot) -> bool {
        match self {
            MenuKind::Inventory | MenuKind::Crafting => slot.source != Source::Result,
            _ => true,
        }
    }

    /// `canTakeItemForPickAll`: a double click never collects from a result slot.
    pub fn can_take_item_for_pick_all(self, slot: Slot) -> bool {
        match self {
            MenuKind::Inventory | MenuKind::Crafting | MenuKind::Stonecutter | MenuKind::Smithing => slot.source != Source::Result,
            MenuKind::Merchant => false,
            _ => true,
        }
    }
}

/// `addStandardInventorySlots`: the 27 main slots, then the hotbar.
fn player_slots(slots: &mut Vec<Slot>) {
    slots.extend((HOTBAR_SIZE..MAIN_SIZE).map(|i| Slot::new(Source::Player, i, SlotKind::Normal)));
    slots.extend((0..HOTBAR_SIZE).map(|i| Slot::new(Source::Player, i, SlotKind::Normal)));
}

fn block_slots(n: usize, kind: SlotKind) -> Vec<Slot> {
    (0..n).map(|i| Slot::new(Source::Block, i, kind)).collect()
}

impl Menu {
    /// `InventoryMenu`: result 0, crafting grid 1-4, armor 5-8 (head to feet), main 9-35,
    /// hotbar 36-44, off hand 45.
    pub fn inventory() -> Menu {
        let mut slots = vec![Slot::new(Source::Result, 0, SlotKind::CraftResult)];
        slots.extend((0..4).map(|i| Slot::new(Source::Craft, i, SlotKind::Normal)));
        for (i, eq) in [EquipmentSlot::Head, EquipmentSlot::Chest, EquipmentSlot::Legs, EquipmentSlot::Feet].into_iter().enumerate() {
            slots.push(Slot::new(Source::Player, 39 - i, SlotKind::Armor(eq)));
        }
        player_slots(&mut slots);
        slots.push(Slot::new(Source::Player, SLOT_OFFHAND, SlotKind::Offhand));
        Menu::with_slots(MenuKind::Inventory, 0, slots, 0, CraftGrid::new(2, 2))
    }

    /// `ChestMenu` of `rows` rows over a block container of `rows * 9` slots.
    pub fn generic(container_id: i32, rows: u8) -> Menu {
        let rows = rows.clamp(1, 6);
        Self::over_block(MenuKind::Generic { rows }, container_id, SlotKind::Normal, 0)
    }

    /// `DispenserMenu` (dispenser, dropper).
    pub fn generic_3x3(container_id: i32) -> Menu {
        Self::over_block(MenuKind::Generic3x3, container_id, SlotKind::Normal, 0)
    }

    /// `HopperMenu`.
    pub fn hopper(container_id: i32) -> Menu {
        Self::over_block(MenuKind::Hopper, container_id, SlotKind::Normal, 0)
    }

    /// `ShulkerBoxMenu`: its slots refuse shulker boxes.
    pub fn shulker_box(container_id: i32) -> Menu {
        Self::over_block(MenuKind::ShulkerBox, container_id, SlotKind::ShulkerBox, 0)
    }

    /// `CraftingMenu`: result 0, grid 1-9, main 10-36, hotbar 37-45.
    pub fn crafting(container_id: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Result, 0, SlotKind::CraftResult)];
        slots.extend((0..9).map(|i| Slot::new(Source::Craft, i, SlotKind::Normal)));
        player_slots(&mut slots);
        Menu::with_slots(MenuKind::Crafting, container_id, slots, 0, CraftGrid::new(3, 3))
    }

    /// `AbstractFurnaceMenu`: ingredient 0, fuel 1, result 2 over the furnace's container, and
    /// its four data values (burn time, burn duration, cook progress, cook time).
    pub fn furnace(container_id: i32, kind: FurnaceKind) -> Menu {
        let mut slots = vec![
            Slot::new(Source::Block, 0, SlotKind::Normal),
            Slot::new(Source::Block, 1, SlotKind::FurnaceFuel),
            Slot::new(Source::Block, 2, SlotKind::FurnaceResult),
        ];
        player_slots(&mut slots);
        Menu::with_slots(MenuKind::Furnace(kind), container_id, slots, 4, CraftGrid::default())
    }

    /// `StonecutterMenu`: input 0, result 1, main 2-28, hotbar 29-37, and the selected recipe
    /// as its data value.
    pub fn stonecutter(container_id: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Input, 0, SlotKind::Normal), Slot::new(Source::Result, 1, SlotKind::StonecutterResult)];
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Stonecutter, container_id, slots, 1, CraftGrid::default());
        menu.input = SimpleContainer::new(1);
        menu.local_data = vec![0];
        menu
    }

    /// `SmithingMenu`: template 0, base 1, addition 2, result 3, main 4-30, hotbar 31-39, and
    /// the recipe error flag as its data value.
    pub fn smithing(container_id: i32) -> Menu {
        let mut slots: Vec<Slot> = (0..3).map(|k| Slot::new(Source::Input, k, SlotKind::SmithingInput(k as u8))).collect();
        slots.push(Slot::new(Source::Result, 3, SlotKind::SmithingResult));
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Smithing, container_id, slots, 1, CraftGrid::default());
        menu.input = SimpleContainer::new(3);
        menu.local_data = vec![0];
        menu
    }

    /// Stonecutter recipes the current input offers (indices into the recipe manager), in the
    /// order of the client's list.
    pub fn stonecutter_recipes(&self) -> &[usize] {
        &self.visible_recipes
    }

    fn over_block(kind: MenuKind, container_id: i32, slot_kind: SlotKind, data: usize) -> Menu {
        let mut slots = block_slots(kind.block_size(), slot_kind);
        player_slots(&mut slots);
        Menu::with_slots(kind, container_id, slots, data, CraftGrid::default())
    }

    /// `clickMenuButton`: the stonecutter selects a recipe; other menus here have no buttons.
    pub fn click_menu_button(&mut self, env: &mut Env, button: i32) -> bool {
        if self.kind != MenuKind::Stonecutter {
            return false;
        }
        if self.local_data[0] == button {
            return false;
        }
        if self.stonecutter_index(button).is_some() {
            self.local_data[0] = button;
            stonecutter_setup_result(self, env, button);
        }
        true
    }

    /// `StonecutterMenu.isValidRecipeIndex`: the recipe at a list position.
    fn stonecutter_index(&self, i: i32) -> Option<usize> {
        usize::try_from(i).ok().and_then(|i| self.visible_recipes.get(i).copied())
    }
}

/// `StonecutterMenu.slotsChanged`: a new input item resets the selection and the recipe list.
pub(crate) fn stonecutter_slots_changed(menu: &mut Menu, env: &mut Env) {
    let input = menu.input.items[0].clone();
    if input.effective_item() == menu.last_input.effective_item() {
        return;
    }
    menu.last_input = input.copy();
    menu.local_data[0] = -1;
    menu.set_slot(env, 1, ItemStack::empty());
    menu.visible_recipes = if input.is_empty() { Vec::new() } else { env.rules.recipes.stonecutter_for(&input) };
}

/// `StonecutterMenu.setupResultSlot`.
pub(crate) fn stonecutter_setup_result(menu: &mut Menu, env: &mut Env, selected: i32) {
    match menu.stonecutter_index(selected) {
        Some(recipe) => {
            menu.result.recipe_used = Some(recipe);
            let out = match &env.rules.recipes.recipes()[recipe].recipe {
                crate::recipe::Recipe::Stonecutting(s) => s.assemble(),
                _ => ItemStack::empty(),
            };
            menu.set_slot(env, 1, out);
        }
        None => {
            menu.set_slot(env, 1, ItemStack::empty());
            menu.result.recipe_used = None;
        }
    }
    menu.broadcast_changes(env);
}

/// `SmithingMenu.slotsChanged`: broadcasts, recomputes the result when the inputs changed, and
/// flags a full input without a result.
pub(crate) fn smithing_slots_changed(menu: &mut Menu, env: &mut Env, source: Source) {
    menu.broadcast_changes(env);
    if source == Source::Input {
        let [t, b, a] = [0, 1, 2].map(|k| &menu.input.items[k]);
        let recipes = &env.rules.recipes;
        match recipes.find_smithing(t, b, a) {
            Some(r) => {
                let out = recipes.assemble_smithing(r, b, a);
                menu.result.recipe_used = Some(r);
                menu.result.item = out;
            }
            None => {
                menu.result.recipe_used = None;
                menu.result.item = ItemStack::empty();
            }
        }
    }
    let full = menu.input.items.iter().all(|s| !s.is_empty());
    menu.local_data[0] = (full && menu.result.item.is_empty()) as i32;
}

/// `SmithingMenu.onTake` after the craft is counted: one of each input is used up.
pub(crate) fn smithing_take(menu: &mut Menu, env: &mut Env) {
    for k in 0..3 {
        if menu.input.items[k].is_empty() {
            continue;
        }
        let mut stack = menu.input.items[k].clone();
        stack.shrink_count(1);
        menu.input.set_item(k, stack);
        menu.slots_changed(env, Source::Input);
    }
}

/// `SmithingMenu.canMoveIntoInputSlots`: an empty input slot takes the item.
fn smithing_accepts(menu: &Menu, env: &Env, stack: &ItemStack) -> bool {
    let keys = ["minecraft:smithing_template", "minecraft:smithing_base", "minecraft:smithing_addition"];
    keys.iter().enumerate().any(|(k, key)| env.rules.recipes.property_set_accepts(key, stack) && menu.input.items[k].is_empty())
}

/// `quickMoveStack` for each menu kind.
pub(crate) fn quick_move_stack(menu: &mut Menu, env: &mut Env, i: usize) -> ItemStack {
    if menu.item(env, i).is_empty() {
        return ItemStack::empty();
    }
    let mut stack = menu.item(env, i).clone();
    let copy = stack.copy();
    let len = menu.len();
    match menu.kind {
        MenuKind::Inventory => {
            let eq = equipment_slot_for_item(&copy);
            let ok = match i {
                0 => menu.move_item_stack_to(env, &mut stack, 9, 45, true),
                1..9 => menu.move_item_stack_to(env, &mut stack, 9, 45, false),
                _ if is_humanoid_armor(eq) && menu.item(env, 8 - armor_index(eq)).is_empty() => {
                    let t = 8 - armor_index(eq);
                    menu.move_item_stack_to(env, &mut stack, t, t + 1, false)
                }
                _ if eq == EquipmentSlot::OffHand && menu.item(env, 45).is_empty() => menu.move_item_stack_to(env, &mut stack, 45, 46, false),
                9..36 => menu.move_item_stack_to(env, &mut stack, 36, 45, false),
                36..45 => menu.move_item_stack_to(env, &mut stack, 9, 36, false),
                _ => menu.move_item_stack_to(env, &mut stack, 9, 45, false),
            };
            if !ok {
                return ItemStack::empty();
            }
            let mut old = copy.clone();
            if i == 0 {
                menu.on_quick_craft(env, i, &stack, &mut old);
            }
            let (result, left) = menu.finish_quick_move(env, i, stack, old, true);
            if i == 0 && !result.is_empty() {
                env_drop(env, left);
            }
            result
        }
        MenuKind::Crafting => {
            if i == 0 {
                menu.post_process_result(env, &mut stack);
            }
            let ok = match i {
                0 => menu.move_item_stack_to(env, &mut stack, 10, 46, true),
                10..46 => {
                    menu.move_item_stack_to(env, &mut stack, 1, 10, false)
                        || if i < 37 {
                            menu.move_item_stack_to(env, &mut stack, 37, 46, false)
                        } else {
                            menu.move_item_stack_to(env, &mut stack, 10, 37, false)
                        }
                }
                _ => menu.move_item_stack_to(env, &mut stack, 10, 46, false),
            };
            if !ok {
                return ItemStack::empty();
            }
            let mut old = copy.clone();
            if i == 0 {
                menu.on_quick_craft(env, i, &stack, &mut old);
            }
            let (result, left) = menu.finish_quick_move(env, i, stack, old, false);
            if i == 0 && !result.is_empty() {
                env_drop(env, left);
            }
            result
        }
        MenuKind::Furnace(kind) => {
            let ok = match i {
                2 => menu.move_item_stack_to(env, &mut stack, 3, 39, true),
                0 | 1 => menu.move_item_stack_to(env, &mut stack, 3, 39, false),
                _ if env.rules.recipes.furnace_accepts(kind, &stack) => menu.move_item_stack_to(env, &mut stack, 0, 1, false),
                _ if stack.has(kiln_item::component::ids::COOKING_FUEL) => menu.move_item_stack_to(env, &mut stack, 1, 2, false),
                3..30 => menu.move_item_stack_to(env, &mut stack, 30, 39, false),
                30..39 => menu.move_item_stack_to(env, &mut stack, 3, 30, false),
                _ => true,
            };
            if !ok {
                return ItemStack::empty();
            }
            let mut old = copy.clone();
            if i == 2 {
                menu.on_quick_craft(env, i, &stack, &mut old);
            }
            menu.finish_quick_move(env, i, stack, old, false).0
        }
        MenuKind::Stonecutter => {
            if i == 1 {
                menu.post_process_result(env, &mut stack);
            }
            let ok = match i {
                1 => menu.move_item_stack_to(env, &mut stack, 2, 38, true),
                0 => menu.move_item_stack_to(env, &mut stack, 2, 38, false),
                _ if env.rules.recipes.stonecutter().any(|(_, s)| s.matches(&stack)) => menu.move_item_stack_to(env, &mut stack, 0, 1, false),
                2..29 => menu.move_item_stack_to(env, &mut stack, 29, 38, false),
                29..38 => menu.move_item_stack_to(env, &mut stack, 2, 29, false),
                _ => true,
            };
            if !ok {
                return ItemStack::empty();
            }
            *menu.item_mut(env, i) = stack.clone();
            if stack.is_empty() {
                menu.set_by_player(env, i, ItemStack::empty());
            }
            menu.set_changed(env, i);
            if stack.count() == copy.count() {
                return ItemStack::empty();
            }
            menu.on_take(env, i, &mut stack);
            if i == 1 {
                env_drop(env, stack);
            }
            menu.broadcast_changes(env);
            copy
        }
        MenuKind::Smithing => {
            let ok = match i {
                3 => menu.move_item_stack_to(env, &mut stack, 4, 40, true),
                0..3 => menu.move_item_stack_to(env, &mut stack, 4, 40, false),
                4..40 if smithing_accepts(menu, env, &stack) => menu.move_item_stack_to(env, &mut stack, 0, 3, false),
                4..31 => menu.move_item_stack_to(env, &mut stack, 31, 40, false),
                31..40 => menu.move_item_stack_to(env, &mut stack, 4, 31, false),
                _ => true,
            };
            if !ok {
                return ItemStack::empty();
            }
            menu.finish_quick_move(env, i, stack, copy, false).0
        }
        MenuKind::Merchant => crate::merchant::quick_move_stack(menu, env, i),
        MenuKind::Generic3x3 => {
            let ok = if i < 9 {
                menu.move_item_stack_to(env, &mut stack, 9, 45, true)
            } else {
                menu.move_item_stack_to(env, &mut stack, 0, 9, false)
            };
            if !ok {
                return ItemStack::empty();
            }
            menu.finish_quick_move(env, i, stack, copy, false).0
        }
        MenuKind::Generic { .. } | MenuKind::Hopper | MenuKind::ShulkerBox => {
            let n = menu.kind.block_size();
            let ok = if i < n {
                menu.move_item_stack_to(env, &mut stack, n, len, true)
            } else {
                menu.move_item_stack_to(env, &mut stack, 0, n, false)
            };
            if !ok {
                return ItemStack::empty();
            }
            menu.finish_move_simple(env, i, stack);
            copy
        }
    }
}

fn env_drop(env: &mut Env, stack: ItemStack) {
    if !stack.is_empty() {
        env.out.push(crate::effect::Effect::Drop { stack, retain_ownership: false });
    }
}

fn is_humanoid_armor(slot: EquipmentSlot) -> bool {
    matches!(slot, EquipmentSlot::Feet | EquipmentSlot::Legs | EquipmentSlot::Chest | EquipmentSlot::Head)
}

/// `EquipmentSlot.getIndex()` for armor: feet 0 to head 3.
fn armor_index(slot: EquipmentSlot) -> usize {
    match slot {
        EquipmentSlot::Feet => 0,
        EquipmentSlot::Legs => 1,
        EquipmentSlot::Chest => 2,
        _ => 3,
    }
}
