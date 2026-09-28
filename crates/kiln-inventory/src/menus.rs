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
    /// `GrindstoneMenu`.
    Grindstone,
    /// `AnvilMenu`.
    Anvil,
    /// `LoomMenu`.
    Loom,
    /// `CartographyTableMenu`.
    CartographyTable,
    /// `EnchantmentMenu`.
    Enchantment,
    /// `BrewingStandMenu`.
    BrewingStand,
    /// `BeaconMenu`.
    Beacon,
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
            MenuKind::Grindstone => "minecraft:grindstone",
            MenuKind::Anvil => "minecraft:anvil",
            MenuKind::Loom => "minecraft:loom",
            MenuKind::CartographyTable => "minecraft:cartography_table",
            MenuKind::Enchantment => "minecraft:enchantment",
            MenuKind::BrewingStand => "minecraft:brewing_stand",
            MenuKind::Beacon => "minecraft:beacon",
        })
    }

    /// Network id of [`MenuKind::menu_type`].
    pub fn menu_type_id(self) -> Option<i32> {
        kiln_data::builtin_id("minecraft:menu", self.menu_type()?)
    }

    /// Slots of the block container, before the player inventory slots.
    pub fn block_size(self) -> usize {
        match self {
            MenuKind::Inventory
            | MenuKind::Crafting
            | MenuKind::Stonecutter
            | MenuKind::Smithing
            | MenuKind::Grindstone
            | MenuKind::Anvil
            | MenuKind::Loom
            | MenuKind::CartographyTable
            | MenuKind::Enchantment
            | MenuKind::Beacon
            | MenuKind::Merchant => 0,
            MenuKind::Generic { rows } => rows as usize * 9,
            MenuKind::Generic3x3 => 9,
            MenuKind::Hopper => 5,
            MenuKind::ShulkerBox => 27,
            MenuKind::Furnace(_) => 3,
            MenuKind::BrewingStand => 5,
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

    /// `BrewingStandMenu`: bottles 0-2, ingredient 3, fuel 4 over the brewing stand's container,
    /// and its four data values (brew time, fuel, total brew time, total fuel).
    pub fn brewing_stand(container_id: i32) -> Menu {
        let mut slots = vec![
            Slot::new(Source::Block, 0, SlotKind::BrewingPotion),
            Slot::new(Source::Block, 1, SlotKind::BrewingPotion),
            Slot::new(Source::Block, 2, SlotKind::BrewingPotion),
            Slot::new(Source::Block, 3, SlotKind::BrewingIngredient),
            Slot::new(Source::Block, 4, SlotKind::BrewingFuel),
        ];
        player_slots(&mut slots);
        Menu::with_slots(MenuKind::BrewingStand, container_id, slots, 4, CraftGrid::default())
    }

    /// `BeaconMenu`: the payment slot 0 (the menu's own), main 1-27, hotbar 28-36, and the
    /// beacon's three data values (levels, primary and secondary power).
    pub fn beacon(container_id: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Input, 0, SlotKind::BeaconPayment)];
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Beacon, container_id, slots, 3, CraftGrid::default());
        menu.input = SimpleContainer::new(1);
        menu
    }

    /// `BeaconMenu.hasPayment`.
    pub fn has_beacon_payment(&self) -> bool {
        self.kind == MenuKind::Beacon && self.input.items.first().is_some_and(|s| !s.is_empty())
    }

    /// `paymentSlot.remove(1)` once the powers are set (`BeaconMenu.updateEffects`).
    pub fn take_beacon_payment(&mut self) {
        if let Some(s) = self.input.items.first_mut() {
            s.shrink(1);
        }
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

    /// `GrindstoneMenu`: inputs 0 and 1, result 2, main 3-29, hotbar 30-38.
    pub fn grindstone(container_id: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Input, 0, SlotKind::GrindstoneInput), Slot::new(Source::Input, 1, SlotKind::GrindstoneInput)];
        slots.push(Slot::new(Source::Result, 2, SlotKind::GrindstoneResult));
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Grindstone, container_id, slots, 0, CraftGrid::default());
        menu.input = SimpleContainer::new(2);
        menu
    }

    /// `AnvilMenu`: inputs 0 and 1, result 2, main 3-29, hotbar 30-38, and the level cost as
    /// its data value.
    pub fn anvil(container_id: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Input, 0, SlotKind::Normal), Slot::new(Source::Input, 1, SlotKind::Normal)];
        slots.push(Slot::new(Source::Result, 2, SlotKind::AnvilResult));
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Anvil, container_id, slots, 1, CraftGrid::default());
        menu.input = SimpleContainer::new(2);
        menu.local_data = vec![0];
        menu
    }

    /// `LoomMenu`: banner 0, dye 1, pattern 2, result 3, main 4-30, hotbar 31-39, and the
    /// selected pattern as its data value.
    pub fn loom(container_id: i32) -> Menu {
        let mut slots = vec![
            Slot::new(Source::Input, 0, SlotKind::LoomBanner),
            Slot::new(Source::Input, 1, SlotKind::LoomDye),
            Slot::new(Source::Input, 2, SlotKind::LoomPattern),
            Slot::new(Source::Result, 3, SlotKind::LoomResult),
        ];
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Loom, container_id, slots, 1, CraftGrid::default());
        menu.input = SimpleContainer::new(3);
        menu.local_data = vec![0];
        menu
    }

    /// `CartographyTableMenu`: map 0, additional 1, result 2, main 3-29, hotbar 30-38.
    pub fn cartography_table(container_id: i32) -> Menu {
        let mut slots = vec![
            Slot::new(Source::Input, 0, SlotKind::CartographyMap),
            Slot::new(Source::Input, 1, SlotKind::CartographyAdditional),
            Slot::new(Source::Result, 2, SlotKind::CartographyResult),
        ];
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::CartographyTable, container_id, slots, 0, CraftGrid::default());
        menu.input = SimpleContainer::new(2);
        menu
    }

    /// `EnchantmentMenu`: item 0, lapis 1, main 2-28, hotbar 29-37, and ten data values (the
    /// three costs, the seed, the three enchantment clues and their levels).
    pub fn enchantment(container_id: i32, seed: i32) -> Menu {
        let mut slots = vec![Slot::new(Source::Input, 0, SlotKind::EnchantItem), Slot::new(Source::Input, 1, SlotKind::EnchantLapis)];
        player_slots(&mut slots);
        let mut menu = Menu::with_slots(MenuKind::Enchantment, container_id, slots, 10, CraftGrid::default());
        menu.input = SimpleContainer::new(2);
        menu.local_data = vec![0, 0, 0, seed, -1, -1, -1, -1, -1, -1];
        menu.enchant.seed = seed;
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
        match self.kind {
            MenuKind::Loom => return crate::stations::loom_click(self, env, button),
            MenuKind::Enchantment => {
                let done = crate::stations::enchantment_click(self, env, button);
                self.local_data[3] = self.enchant.seed;
                return done;
            }
            MenuKind::Stonecutter => {}
            _ => return false,
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
        MenuKind::BrewingStand => {
            // `BrewingStandMenu.quickMoveStack`.
            let rules = env.rules;
            let ingredient = rules.recipes.property_set_accepts("minecraft:brewing_reagent", &stack);
            let ok = match i {
                0..5 => {
                    let ok = menu.move_item_stack_to(env, &mut stack, 5, 41, true);
                    if ok {
                        let mut old = copy.clone();
                        menu.on_quick_craft(env, i, &stack, &mut old);
                    }
                    ok
                }
                _ if copy.has(kiln_item::component::ids::BREWING_FUEL) => {
                    menu.move_item_stack_to(env, &mut stack, 4, 5, false)
                        || !ingredient
                        || menu.move_item_stack_to(env, &mut stack, 3, 4, false)
                }
                _ if ingredient => menu.move_item_stack_to(env, &mut stack, 3, 4, false),
                _ if crate::slot::is_potion_input(&copy, rules) => menu.move_item_stack_to(env, &mut stack, 0, 3, false),
                5..32 => menu.move_item_stack_to(env, &mut stack, 32, 41, false),
                32..41 => menu.move_item_stack_to(env, &mut stack, 5, 32, false),
                _ => menu.move_item_stack_to(env, &mut stack, 5, 41, false),
            };
            if !ok {
                return ItemStack::empty();
            }
            *menu.item_mut(env, i) = stack.clone();
            if stack.is_empty() {
                menu.set_by_player(env, i, ItemStack::empty());
            } else {
                menu.set_changed(env, i);
            }
            if stack.count() == copy.count() {
                return ItemStack::empty();
            }
            // `slot.onTake(player, copy)`: the stack as it was (a bottle slot's potion).
            let mut taken = copy.clone();
            menu.on_take(env, i, &mut taken);
            copy
        }
        MenuKind::Beacon => {
            // `BeaconMenu.quickMoveStack`.
            let payment_empty = menu.item(env, 0).is_empty();
            let ok = match i {
                0 => {
                    let ok = menu.move_item_stack_to(env, &mut stack, 1, 37, true);
                    if ok {
                        let mut old = copy.clone();
                        menu.on_quick_craft(env, i, &stack, &mut old);
                    }
                    ok
                }
                _ if payment_empty && menu.slots[0].may_place(&stack, env.rules) && stack.count() == 1 => {
                    menu.move_item_stack_to(env, &mut stack, 0, 1, false)
                }
                1..28 => menu.move_item_stack_to(env, &mut stack, 28, 37, false),
                28..37 => menu.move_item_stack_to(env, &mut stack, 1, 28, false),
                _ => menu.move_item_stack_to(env, &mut stack, 1, 37, false),
            };
            if !ok {
                return ItemStack::empty();
            }
            menu.finish_quick_move(env, i, stack, copy, false).0
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
        MenuKind::Grindstone | MenuKind::Anvil => {
            let grindstone = menu.kind == MenuKind::Grindstone;
            let ok = match i {
                2 => menu.move_item_stack_to(env, &mut stack, 3, 39, true),
                0 | 1 => menu.move_item_stack_to(env, &mut stack, 3, 39, false),
                // The grindstone takes an item while an input is free; the anvil always tries.
                _ if (!grindstone || menu.input.items[0].is_empty() || menu.input.items[1].is_empty()) && (grindstone || (3..39).contains(&i)) => {
                    menu.move_item_stack_to(env, &mut stack, 0, 2, false)
                }
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
            menu.finish_quick_move(env, i, stack, copy, false).0
        }
        MenuKind::Loom | MenuKind::CartographyTable => {
            let loom = menu.kind == MenuKind::Loom;
            let (result, inv) = if loom { (3, 4) } else { (2, 3) };
            let (use_row, end) = (inv + 27, inv + 36);
            if i == result && !loom {
                // `Item.onCraftedBy` on the result (map post-processing).
                menu.post_process_result(env, &mut stack);
            }
            let ok = if i == result {
                menu.move_item_stack_to(env, &mut stack, inv, end, true)
            } else if i < result {
                menu.move_item_stack_to(env, &mut stack, inv, end, false)
            } else if loom && crate::stations::is_banner(&stack) {
                menu.move_item_stack_to(env, &mut stack, 0, 1, false)
            } else if loom && crate::stations::is_loom_dye(&stack) {
                menu.move_item_stack_to(env, &mut stack, 1, 2, false)
            } else if loom && crate::stations::is_loom_pattern(&stack) {
                menu.move_item_stack_to(env, &mut stack, 2, 3, false)
            } else if !loom && stack.has(kiln_item::component::ids::MAP_ID) {
                menu.move_item_stack_to(env, &mut stack, 0, 1, false)
            } else if !loom && crate::stations::is_cartography_additional(&stack) {
                menu.move_item_stack_to(env, &mut stack, 1, 2, false)
            } else if (inv..use_row).contains(&i) {
                menu.move_item_stack_to(env, &mut stack, use_row, end, false)
            } else if (use_row..end).contains(&i) {
                menu.move_item_stack_to(env, &mut stack, inv, use_row, false)
            } else {
                true
            };
            if !ok {
                return ItemStack::empty();
            }
            let mut old = copy.clone();
            if i == result {
                menu.on_quick_craft(env, i, &stack, &mut old);
            }
            menu.finish_quick_move(env, i, stack, copy, false).0
        }
        MenuKind::Enchantment => {
            let ok = match i {
                0 | 1 => menu.move_item_stack_to(env, &mut stack, 2, 38, true),
                _ if !stack.is_empty() && stack.item_name() == "minecraft:lapis_lazuli" => menu.move_item_stack_to(env, &mut stack, 1, 2, true),
                _ if menu.input.items[0].is_empty() => {
                    let one = stack.copy_with_count(1);
                    stack.shrink_count(1);
                    menu.set_by_player(env, 0, one);
                    true
                }
                _ => false,
            };
            if !ok {
                return ItemStack::empty();
            }
            menu.finish_quick_move(env, i, stack, copy, false).0
        }
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
