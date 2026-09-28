//! Menus (`AbstractContainerMenu`): slots over containers, the carried stack, clicks, and the
//! synchronization of slot contents with the client.
//!
//! The logic mirrors vanilla statement by statement, including where vanilla mutates a slot's
//! stack in place without notifying its container (which decides, for instance, whether a
//! crafting grid recomputes its result).

use crate::click::{ClickAction, ContainerInput};
use crate::container::Container;
use crate::effect::Effect;
use crate::inventory::PlayerInventory;
use crate::menus::MenuKind;
use crate::recipe::CraftingInput;
use crate::remote::RemoteSlot;
use crate::rules::Rules;
use crate::slot::{Slot, SlotKind, Source};
use crate::stack::{StackExt, matches, same_item, same_item_same_components};
use kiln_item::{HashedStack, ItemStack};

/// The player state menus consult.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerFlags {
    /// `isCreative()` (game mode).
    pub creative: bool,
    /// `hasInfiniteMaterials()` (the `instabuild` ability).
    pub infinite_materials: bool,
    pub spectator: bool,
    /// `isDeadOrDying()`.
    pub dead: bool,
    /// Removed from the world (other than by changing dimension) or disconnected: items that
    /// would go back into the inventory are dropped instead.
    pub removed: bool,
    /// `experienceLevel` (anvil costs).
    pub xp_level: i32,
    /// `Player.enchantmentSeed` (enchanting table offers).
    pub enchantment_seed: i32,
}

/// World state that some recipes and results depend on (maps, the recipe book, game rules).
pub trait World {
    /// The `limitedCrafting` game rule.
    fn limited_crafting(&self) -> bool {
        false
    }

    /// Whether the player's recipe book contains the recipe.
    fn knows_recipe(&self, _recipe: &str) -> bool {
        true
    }

    /// `MapItemSavedData` scale of a map (`MapExtendingRecipe`); `None` if the map has no data
    /// or is an exploration map.
    fn map_scale(&self, _map_id: i32) -> Option<i8> {
        None
    }

    /// `MapItem.onCraftedPostProcess`: applies and removes `minecraft:map_post_processing`
    /// (vanilla locks or scales the map, creating new map data).
    fn post_process_map(&mut self, _stack: &mut ItemStack) {}

    /// The server's `en_us` text for a translation key (item names the anvil compares with).
    fn translate(&self, _key: &str) -> Option<String> {
        None
    }

    /// `Enchantment.canEnchant(stack)` of a `minecraft:enchantment` id.
    fn enchantment_can_enchant(&self, _enchantment: i32, _stack: &ItemStack) -> bool {
        false
    }

    /// `Enchantment.areCompatible`.
    fn enchantments_compatible(&self, a: i32, b: i32) -> bool {
        a != b
    }

    /// `Enchantment.getMaxLevel`.
    fn enchantment_max_level(&self, _enchantment: i32) -> i32 {
        1
    }

    /// `Enchantment.getAnvilCost`.
    fn enchantment_anvil_cost(&self, _enchantment: i32) -> i32 {
        1
    }

    /// `Enchantment.getMinCost(level)`.
    fn enchantment_min_cost(&self, _enchantment: i32, _level: i32) -> i32 {
        0
    }

    /// `level.getRandom().nextInt(bound)` (grindstone experience).
    fn random_int(&mut self, _bound: i32) -> i32 {
        0
    }

    /// `EnchantmentHelper.selectEnchantment(random, stack, cost, #in_enchanting_table)`:
    /// (`minecraft:enchantment` id, level) pairs.
    fn select_enchantments(&self, _rng: &mut dyn kiln_javamath::random::RandomSource, _stack: &ItemStack, _cost: i32) -> Vec<(i32, i32)> {
        Vec::new()
    }

    /// Bookshelves around the enchanting table (`EnchantingTableBlock.isValidBookShelf`).
    fn enchanting_bookshelves(&self) -> i32 {
        0
    }

    /// `player.getRandom().nextInt()` (the new enchantment seed).
    fn next_player_int(&mut self) -> i32 {
        0
    }
}

/// A world with no maps, the recipe book open and `limitedCrafting` off.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoWorld;

impl World for NoWorld {}

/// Everything a menu operation needs besides the menu itself.
pub struct Env<'a> {
    pub inventory: &'a mut PlayerInventory,
    /// The block (or entity) container, for menus opened on one.
    pub block: Option<&'a mut dyn Container>,
    pub player: PlayerFlags,
    pub rules: &'a Rules,
    pub world: &'a mut dyn World,
    /// Packets and side effects, in order.
    pub out: &'a mut Vec<Effect>,
}

impl Env<'_> {
    fn drop_item(&mut self, stack: ItemStack, retain_ownership: bool) {
        if !stack.is_empty() {
            self.out.push(Effect::Drop { stack, retain_ownership });
        }
    }
}

/// Vanilla would throw here (the click reaches a slot index that does not exist); the server
/// disconnects the player.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("container click on invalid slot {slot}")]
pub struct ClickCrash {
    pub slot: i32,
}

/// `TransientCraftingContainer`: the crafting grid of crafting menus (row-major). Its
/// `setItem` and non-empty `removeItem` make the menu recompute the result; the menu does that
/// around these plain container operations.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CraftGrid {
    pub width: usize,
    pub height: usize,
    pub items: Vec<ItemStack>,
}

impl CraftGrid {
    pub fn new(width: usize, height: usize) -> Self {
        CraftGrid { width, height, items: vec![ItemStack::empty(); width * height] }
    }
}

/// `AnvilMenu`'s own state besides its cost data slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AnvilState {
    /// `itemName`: the name the client typed.
    pub item_name: Option<String>,
    /// `repairItemCountCost`: materials a repair uses up.
    pub repair_item_count_cost: i32,
    /// `onlyRenaming`.
    pub only_renaming: bool,
}

/// `EnchantmentMenu`'s seed and random.
#[derive(Debug, Clone)]
pub(crate) struct EnchantState {
    /// `enchantmentSeed` (also data value 3).
    pub seed: i32,
    pub rng: kiln_javamath::random::LegacyRandom,
}

impl Default for EnchantState {
    fn default() -> Self {
        EnchantState { seed: 0, rng: kiln_javamath::random::LegacyRandom::new(0) }
    }
}

/// `ResultContainer`: one stack whatever the index; removing takes it whole.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ResultBox {
    pub item: ItemStack,
    /// `recipeUsed`.
    pub recipe_used: Option<usize>,
}

/// A menu over containers (`AbstractContainerMenu`), with vanilla's slot synchronization.
#[derive(Debug, Clone)]
pub struct Menu {
    pub kind: MenuKind,
    pub container_id: i32,
    pub(crate) slots: Vec<Slot>,
    pub(crate) carried: ItemStack,
    state_id: i32,
    last_slots: Vec<ItemStack>,
    /// Slots whose last-seen stack and remote stack both equal the stack at the last
    /// broadcast, and whose remote nothing touched since: an unchanged stack needs no work.
    settled: Vec<bool>,
    remote: Vec<RemoteSlot>,
    remote_carried: RemoteSlot,
    remote_data: Vec<i32>,
    data_count: usize,
    /// A synchronizer is attached (`setSynchronizer`); before that, remote slots are
    /// placeholders that match anything.
    synchronized: bool,
    suppress_remote: bool,
    quickcraft_type: i32,
    quickcraft_status: i32,
    quickcraft_slots: Vec<usize>,
    pub(crate) craft: CraftGrid,
    pub(crate) result: ResultBox,
    /// `CraftingMenu.placingRecipe`.
    pub(crate) placing_recipe: bool,
    /// `ResultSlot.removeCount` / `FurnaceResultSlot.removeCount`.
    remove_count: i32,
    /// The input container of stonecutters and smithing tables.
    pub(crate) input: crate::container::SimpleContainer,
    /// The menu's own data slots (stonecutter selection, smithing error flag); other menus
    /// show their block's `ContainerData`.
    pub(crate) local_data: Vec<i32>,
    /// `StonecutterMenu.input`: the last input, to notice when its item changes.
    pub(crate) last_input: ItemStack,
    /// `StonecutterMenu.recipesForInput` (recipe indices).
    pub(crate) visible_recipes: Vec<usize>,
    /// The merchant menu's trade container and offers.
    pub(crate) merchant: Option<Box<crate::merchant::MerchantState>>,
    /// The anvil's name and repair bookkeeping.
    pub(crate) anvil: AnvilState,
    /// `LoomMenu.selectablePatterns` (`minecraft:banner_pattern` ids).
    pub(crate) visible_patterns: Vec<i32>,
    /// The enchanting table's seed and random.
    pub(crate) enchant: EnchantState,
}

impl Menu {
    pub(crate) fn with_slots(kind: MenuKind, container_id: i32, slots: Vec<Slot>, data_count: usize, craft: CraftGrid) -> Self {
        let n = slots.len();
        Menu {
            kind,
            container_id,
            slots,
            carried: ItemStack::empty(),
            state_id: 0,
            last_slots: vec![ItemStack::empty(); n],
            settled: vec![false; n],
            remote: vec![RemoteSlot::default(); n],
            remote_carried: RemoteSlot::default(),
            remote_data: vec![0; data_count],
            data_count,
            synchronized: false,
            suppress_remote: false,
            quickcraft_type: -1,
            quickcraft_status: 0,
            quickcraft_slots: Vec::new(),
            craft,
            result: ResultBox::default(),
            placing_recipe: false,
            remove_count: 0,
            input: crate::container::SimpleContainer::new(0),
            local_data: Vec::new(),
            last_input: ItemStack::empty(),
            visible_recipes: Vec::new(),
            merchant: None,
            anvil: AnvilState::default(),
            visible_patterns: Vec::new(),
            enchant: EnchantState::default(),
        }
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn carried(&self) -> &ItemStack {
        &self.carried
    }

    /// `setCarried`.
    pub fn set_carried(&mut self, stack: ItemStack) {
        self.carried = stack;
    }

    pub fn state_id(&self) -> i32 {
        self.state_id
    }

    /// `incrementStateId`: 15-bit wrap-around.
    pub fn increment_state_id(&mut self) -> i32 {
        self.state_id = (self.state_id + 1) & 32767;
        self.state_id
    }

    /// The crafting grid (row-major) of crafting menus.
    pub fn craft_grid(&self) -> &[ItemStack] {
        &self.craft.items
    }

    /// Stacks of every slot, in slot order.
    pub fn items(&self, env: &Env) -> Vec<ItemStack> {
        (0..self.slots.len()).map(|i| self.item(env, i).clone()).collect()
    }

    // ---- slot access ------------------------------------------------------------------------

    fn index(&self, slot: i32) -> Result<usize, ClickCrash> {
        usize::try_from(slot).ok().filter(|&i| i < self.slots.len()).ok_or(ClickCrash { slot })
    }

    fn container<'s>(&'s self, env: &'s Env, source: Source) -> &'s dyn Container {
        match source {
            Source::Player => &*env.inventory,
            Source::Block => env.block.as_deref().expect("menu without its block container"),
            Source::Craft => &self.craft,
            Source::Result => &self.result,
            Source::Input => &self.input,
            Source::Merchant => self.merchant.as_deref().expect("merchant menu"),
        }
    }

    fn container_mut<'s>(&'s mut self, env: &'s mut Env, source: Source) -> &'s mut dyn Container {
        match source {
            Source::Player => &mut *env.inventory,
            Source::Block => env.block.as_deref_mut().expect("menu without its block container"),
            Source::Craft => &mut self.craft,
            Source::Result => &mut self.result,
            Source::Input => &mut self.input,
            Source::Merchant => self.merchant.as_deref_mut().expect("merchant menu"),
        }
    }

    /// `Slot.getItem`.
    pub fn item<'s>(&'s self, env: &'s Env, i: usize) -> &'s ItemStack {
        let s = self.slots[i];
        self.container(env, s.source).item(s.index)
    }

    pub(crate) fn item_mut<'s>(&'s mut self, env: &'s mut Env, i: usize) -> &'s mut ItemStack {
        let s = self.slots[i];
        self.container_mut(env, s.source).item_mut(s.index)
    }

    fn has_item(&self, env: &Env, i: usize) -> bool {
        !self.item(env, i).is_empty()
    }

    fn may_place(&self, env: &Env, i: usize, stack: &ItemStack) -> bool {
        self.slots[i].may_place(stack, env.rules)
    }

    fn may_pickup(&self, env: &Env, i: usize) -> bool {
        if self.slots[i].kind == SlotKind::MerchantResult {
            return crate::merchant::result_may_pickup(self);
        }
        if self.slots[i].kind == SlotKind::AnvilResult {
            return crate::workstation::anvil_may_pickup(self, env);
        }
        self.slots[i].may_pickup(self.item(env, i), env.player.creative, env.rules)
    }

    /// `Slot.getMaxStackSize(ItemStack)`.
    fn slot_max_for(&self, env: &Env, i: usize, stack: &ItemStack) -> i32 {
        let s = self.slots[i];
        s.max_stack_size_for(self.container(env, s.source).max_stack_size(), stack)
    }

    /// `Slot.allowModification`.
    pub(crate) fn allow_modification(&self, env: &Env, i: usize) -> bool {
        self.may_pickup(env, i) && self.may_place(env, i, self.item(env, i))
    }

    /// `Slot.set`: `setItem` (a crafting grid then recomputes its result) and `setChanged`.
    fn set(&mut self, env: &mut Env, i: usize, stack: ItemStack) {
        let s = self.slots[i];
        self.container_mut(env, s.source).set_item(s.index, stack);
        // A crafting grid's setItem, and an input container's own setChanged inside setItem.
        if matches!(s.source, Source::Craft | Source::Input) {
            self.slots_changed(env, s.source);
        }
        self.set_changed(env, i);
    }

    /// `Slot.set` from outside a click (a crafting grid recomputes and sends its result).
    pub fn set_slot(&mut self, env: &mut Env, i: usize, stack: ItemStack) {
        self.set(env, i, stack);
    }

    /// `Slot.setByPlayer(stack)`.
    pub(crate) fn set_by_player(&mut self, env: &mut Env, i: usize, stack: ItemStack) {
        let old = self.item(env, i).clone();
        self.set_by_player_old(env, i, stack, old);
    }

    /// `Slot.setByPlayer(stack, previous)`.
    fn set_by_player_old(&mut self, env: &mut Env, i: usize, stack: ItemStack, old: ItemStack) {
        if let Some(slot) = self.slots[i].equip_slot() {
            env.out.push(Effect::Equip { slot, old, new: stack.clone() });
        }
        self.set(env, i, stack);
    }

    /// `Slot.setChanged`.
    pub(crate) fn set_changed(&mut self, env: &mut Env, i: usize) {
        let s = self.slots[i];
        self.container_mut(env, s.source).set_changed();
        match (self.kind, s.source) {
            (_, Source::Input) | (MenuKind::Smithing, Source::Result) => self.slots_changed(env, s.source),
            _ => {}
        }
    }

    /// `Slot.remove`.
    fn remove(&mut self, env: &mut Env, i: usize, count: i32) -> ItemStack {
        let s = self.slots[i];
        if matches!(s.kind, SlotKind::CraftResult | SlotKind::FurnaceResult) && self.has_item(env, i) {
            self.remove_count += count.min(self.item(env, i).count());
        }
        if s.kind == SlotKind::MerchantResult && self.has_item(env, i) {
            let n = count.min(self.item(env, i).count());
            crate::merchant::add_remove_count(self, n);
        }
        let removed = self.container_mut(env, s.source).remove_item(s.index, count);
        if matches!(s.source, Source::Craft | Source::Input) && !removed.is_empty() {
            self.slots_changed(env, s.source);
        }
        removed
    }

    /// `Slot.tryRemove`.
    fn try_remove(&mut self, env: &mut Env, i: usize, count: i32, max: i32) -> Option<ItemStack> {
        if !self.may_pickup(env, i) {
            return None;
        }
        if !self.allow_modification(env, i) && max < self.item(env, i).count() {
            return None;
        }
        let removed = self.remove(env, i, count.min(max));
        if removed.is_empty() {
            return None;
        }
        if self.item(env, i).is_empty() {
            self.set_by_player_old(env, i, ItemStack::empty(), removed.clone());
        }
        Some(removed)
    }

    /// `Slot.safeTake`.
    pub(crate) fn safe_take(&mut self, env: &mut Env, i: usize, count: i32, max: i32) -> ItemStack {
        match self.try_remove(env, i, count, max) {
            Some(mut taken) => {
                self.on_take(env, i, &mut taken);
                taken
            }
            None => ItemStack::empty(),
        }
    }

    /// `Slot.safeInsert(stack, count)`: returns what is left of `stack`.
    pub(crate) fn safe_insert(&mut self, env: &mut Env, i: usize, mut stack: ItemStack, count: i32) -> ItemStack {
        if stack.is_empty() || !self.may_place(env, i, &stack) {
            return stack;
        }
        let current = self.item(env, i);
        let n = count.min(stack.count()).min(self.slot_max_for(env, i, &stack) - current.count());
        if n <= 0 {
            return stack;
        }
        if current.is_empty() {
            let part = stack.split_count(n);
            self.set_by_player(env, i, part);
        } else if same_item_same_components(current, &stack) {
            stack.shrink_count(n);
            // Vanilla grows the slot's stack in place, then sets it (as new and old stack).
            let mut grown = current.clone();
            grown.grow_count(n);
            *self.item_mut(env, i) = grown.clone();
            self.set_by_player_old(env, i, grown.clone(), grown);
        }
        stack
    }

    /// `Slot.safeClone`.
    fn safe_clone(&mut self, env: &mut Env, i: usize) -> ItemStack {
        let item = self.item(env, i);
        let mut clone = item.copy_with_count(item.max_stack_size());
        if self.slots[i].kind == SlotKind::CraftResult {
            self.crafted_post_process(env, &mut clone);
        }
        clone
    }

    /// `Item.onCraftedBy(stack, player)`: map post-processing.
    fn crafted_post_process(&mut self, env: &mut Env, stack: &mut ItemStack) {
        if stack.has(kiln_item::component::ids::MAP_POST_PROCESSING) {
            env.world.post_process_map(stack);
        }
    }

    /// `checkTakeAchievements` of result slots.
    fn check_take_achievements(&mut self, env: &mut Env, i: usize, stack: &mut ItemStack) {
        match self.slots[i].kind {
            SlotKind::CraftResult => {
                if self.remove_count > 0 {
                    self.on_crafted_by(env, stack, self.remove_count, self.result.recipe_used);
                }
                self.remove_count = 0;
            }
            SlotKind::FurnaceResult => {
                self.on_crafted_by(env, stack, self.remove_count, None);
                self.remove_count = 0;
            }
            SlotKind::StonecutterResult | SlotKind::SmithingResult => {
                let n = stack.count();
                self.on_crafted_by(env, stack, n, self.result.recipe_used);
            }
            SlotKind::MerchantResult => {
                let n = crate::merchant::take_remove_count(self);
                self.on_crafted_by(env, stack, n, None);
            }
            // `BrewingStandMenu$PotionSlot.onTake`.
            SlotKind::BrewingPotion => {
                if let Some(contents) = stack.get(kiln_item::keys::POTION_CONTENTS) {
                    env.out.push(Effect::BrewedPotion { potion: contents.potion });
                }
            }
            _ => {}
        }
    }

    /// `ItemStack.onCraftedBy(player, amount)`, with the recipe `RecipeCraftingHolder`
    /// awards (crafting, stonecutting and smithing results; furnaces award theirs with the
    /// experience).
    fn on_crafted_by(&mut self, env: &mut Env, stack: &mut ItemStack, amount: i32, recipe: Option<usize>) {
        env.out.push(Effect::Crafted { item: stack.effective_item(), amount, recipe });
        self.crafted_post_process(env, stack);
    }

    /// `Slot.onTake`.
    pub(crate) fn on_take(&mut self, env: &mut Env, i: usize, stack: &mut ItemStack) {
        self.check_take_achievements(env, i, stack);
        self.on_take_effects(env, i);
    }

    /// `onTake` for a stack vanilla has already moved into player inventory slot `inv`.
    fn on_take_in_inventory(&mut self, env: &mut Env, i: usize, inv: usize) {
        let mut stack = env.inventory.item(inv).clone();
        self.check_take_achievements(env, i, &mut stack);
        env.inventory.set_item(inv, stack);
        self.on_take_effects(env, i);
    }

    /// What `onTake` does besides `checkTakeAchievements`.
    fn on_take_effects(&mut self, env: &mut Env, i: usize) {
        match self.slots[i].kind {
            SlotKind::CraftResult => self.consume_crafting_grid(env),
            SlotKind::StonecutterResult => {
                if !self.remove(env, 0, 1).is_empty() {
                    let selected = self.local_data[0];
                    crate::menus::stonecutter_setup_result(self, env, selected);
                }
                self.set_changed(env, i);
            }
            SlotKind::SmithingResult => crate::menus::smithing_take(self, env),
            SlotKind::MerchantResult => crate::merchant::on_take(self),
            SlotKind::GrindstoneResult => crate::workstation::grindstone_take(self, env),
            SlotKind::AnvilResult => crate::workstation::anvil_take(self, env),
            SlotKind::LoomResult => {
                crate::stations::loom_take(self, env);
                self.set_changed(env, i);
            }
            _ => self.set_changed(env, i),
        }
    }

    /// `Slot.onQuickCraft(new, old)`.
    pub(crate) fn on_quick_craft(&mut self, env: &mut Env, i: usize, new: &ItemStack, old: &mut ItemStack) {
        let n = old.count() - new.count();
        if n > 0 && matches!(self.slots[i].kind, SlotKind::CraftResult | SlotKind::FurnaceResult) {
            self.remove_count += n;
            self.check_take_achievements(env, i, old);
        }
        if n > 0 && self.slots[i].kind == SlotKind::MerchantResult {
            crate::merchant::add_remove_count(self, n);
            self.check_take_achievements(env, i, old);
        }
    }

    /// `Slot.onSwapCraft`.
    fn on_swap_craft(&mut self, i: usize, n: i32) {
        if self.slots[i].kind == SlotKind::CraftResult {
            self.remove_count += n;
        }
    }

    /// `ResultSlot.onTake` after `checkTakeAchievements`: consumes one of each ingredient and
    /// puts back the remainders.
    fn consume_crafting_grid(&mut self, env: &mut Env) {
        let (input, left, top) = CraftingInput::positioned(self.craft.width, self.craft.height, &self.craft.items);
        let remaining = env.rules.recipes.remaining_items(&input, &*env.world);
        let width = self.craft.width;
        for y in 0..input.height() {
            for x in 0..input.width() {
                let slot = x + left + (y + top) * width;
                let mut rem = remaining[x + y * input.width()].clone();
                if !self.craft.items[slot].is_empty() {
                    let removed = crate::container::remove_item(&mut self.craft.items, slot, 1);
                    if !removed.is_empty() {
                        self.slots_changed(env, Source::Craft);
                    }
                }
                if rem.is_empty() {
                    continue;
                }
                let current = self.craft.items[slot].clone();
                if current.is_empty() {
                    self.craft.items[slot] = rem;
                    self.slots_changed(env, Source::Craft);
                } else if same_item_same_components(&current, &rem) {
                    rem.grow_count(current.count());
                    self.craft.items[slot] = rem;
                    self.slots_changed(env, Source::Craft);
                } else if !env.inventory.add(None, &mut rem, env.player.infinite_materials) {
                    env.drop_item(rem, false);
                }
            }
        }
    }

    /// `slotsChanged(container)`: crafting menus recompute their result, workstations react
    /// to their input, others broadcast changes.
    pub(crate) fn slots_changed(&mut self, env: &mut Env, source: Source) {
        match self.kind {
            MenuKind::Inventory => self.slot_changed_crafting_grid(env, None),
            MenuKind::Crafting if !self.placing_recipe => self.slot_changed_crafting_grid(env, None),
            MenuKind::Crafting => {}
            MenuKind::Stonecutter => crate::menus::stonecutter_slots_changed(self, env),
            MenuKind::Smithing => crate::menus::smithing_slots_changed(self, env, source),
            MenuKind::Grindstone => crate::workstation::grindstone_slots_changed(self, env, source),
            MenuKind::Anvil => crate::workstation::anvil_slots_changed(self, env, source),
            MenuKind::Loom if source == Source::Input => crate::stations::loom_slots_changed(self, env),
            MenuKind::CartographyTable if source == Source::Input => crate::stations::cartography_slots_changed(self, env),
            MenuKind::Enchantment if source == Source::Input => crate::stations::enchantment_slots_changed(self, env),
            MenuKind::Loom | MenuKind::CartographyTable | MenuKind::Enchantment => {}
            _ => self.broadcast_changes(env),
        }
    }

    /// `CraftingMenu.slotChangedCraftingGrid`: recomputes the result and sends it at once.
    pub(crate) fn slot_changed_crafting_grid(&mut self, env: &mut Env, hint: Option<usize>) {
        let input = CraftingInput::new(self.craft.width, self.craft.height, &self.craft.items);
        let mut result = ItemStack::empty();
        if let Some(id) = env.rules.recipes.find_crafting(&input, hint, &*env.world) {
            let recipes = &env.rules.recipes;
            if recipes.is_special(id) || !env.world.limited_crafting() || env.world.knows_recipe(recipes.id(id)) {
                self.result.recipe_used = Some(id);
                result = recipes.assemble(id, &input, &*env.world);
            }
        }
        self.result.item = result.clone();
        self.set_remote_slot(0, &result);
        let state_id = self.increment_state_id();
        env.out.push(Effect::SetSlot { container_id: self.container_id, state_id, slot: 0, stack: result });
    }

    // ---- synchronization ----------------------------------------------------------------------

    /// `addSlotListener` + `setSynchronizer` (`ServerPlayer.initMenu`): sends the full state.
    pub fn open(&mut self, env: &mut Env) {
        self.broadcast_changes(env);
        self.synchronized = true;
        self.remote = vec![RemoteSlot::default(); self.slots.len()];
        self.settled.fill(false);
        self.remote_carried = RemoteSlot::default();
        self.send_all_data_to_remote(env);
    }

    /// `sendAllDataToRemote`.
    pub fn send_all_data_to_remote(&mut self, env: &mut Env) {
        if !self.synchronized {
            return;
        }
        let items = self.items(env);
        self.settled.fill(false);
        for (r, s) in self.remote.iter_mut().zip(&items) {
            r.force(s);
        }
        self.remote_carried.force(&self.carried);
        for i in 0..self.data_count {
            self.remote_data[i] = self.data(env, i);
        }
        let state_id = self.increment_state_id();
        env.out.push(Effect::SetContent { container_id: self.container_id, state_id, items, carried: self.carried.copy() });
        for i in 0..self.data_count {
            env.out.push(Effect::SetData { container_id: self.container_id, id: i as i16, value: self.remote_data[i] as i16 });
        }
    }

    fn data(&self, env: &Env, i: usize) -> i32 {
        match self.local_data.get(i) {
            Some(v) => *v,
            None => env.block.as_deref().map_or(0, |b| b.data(i)),
        }
    }

    /// The slot listener (`ServerPlayer`'s `containerListener`): inventory change triggers.
    fn trigger_slot_listeners(&mut self, env: &mut Env, i: usize) {
        if matches(&self.last_slots[i], self.item(env, i)) {
            return;
        }
        let now = self.item(env, i).copy();
        self.last_slots[i] = now.clone();
        let s = self.slots[i];
        if s.kind != SlotKind::CraftResult && s.source == Source::Player {
            env.out.push(Effect::InventoryChanged { slot: s.index, stack: now });
        }
    }

    /// `broadcastChanges`: sends every slot, the carried stack and the data values the client
    /// does not have yet.
    pub fn broadcast_changes(&mut self, env: &mut Env) {
        for i in 0..self.slots.len() {
            if self.settled[i] && matches(&self.last_slots[i], self.item(env, i)) {
                continue;
            }
            self.trigger_slot_listeners(env, i);
            if self.suppress_remote || !self.synchronized {
                continue;
            }
            let item = self.item(env, i).clone();
            if !self.remote[i].matches(&item) {
                self.remote[i].force(&item);
                let state_id = self.increment_state_id();
                env.out.push(Effect::SetSlot { container_id: self.container_id, state_id, slot: i as i16, stack: item.copy() });
            }
            self.settled[i] = true;
        }
        if !self.suppress_remote && self.synchronized && !self.remote_carried.matches(&self.carried) {
            self.remote_carried.force(&self.carried);
            env.out.push(Effect::SetCursor { stack: self.carried.copy() });
        }
        for i in 0..self.data_count {
            let value = self.data(env, i);
            if self.suppress_remote || self.remote_data[i] == value {
                continue;
            }
            self.remote_data[i] = value;
            if self.synchronized {
                env.out.push(Effect::SetData { container_id: self.container_id, id: i as i16, value: value as i16 });
            }
        }
    }

    /// `broadcastFullState`.
    pub fn broadcast_full_state(&mut self, env: &mut Env) {
        for i in 0..self.slots.len() {
            self.trigger_slot_listeners(env, i);
        }
        self.send_all_data_to_remote(env);
    }

    /// `setRemoteSlot`.
    pub fn set_remote_slot(&mut self, i: usize, stack: &ItemStack) {
        if self.synchronized {
            self.remote[i].force(stack);
            self.settled[i] = false;
        }
    }

    /// `setRemoteSlotUnsafe`: a slot the client reported; out-of-range slots are ignored.
    pub fn set_remote_slot_unsafe(&mut self, slot: i32, hash: HashedStack) {
        if self.synchronized
            && let Some(r) = usize::try_from(slot).ok().and_then(|i| self.remote.get_mut(i))
        {
            r.receive(hash);
            self.settled[slot as usize] = false;
        }
    }

    /// `setRemoteCarried`.
    pub fn set_remote_carried(&mut self, hash: HashedStack) {
        if self.synchronized {
            self.remote_carried.receive(hash);
        }
    }

    pub fn suppress_remote_updates(&mut self) {
        self.suppress_remote = true;
        self.settled.fill(false);
    }

    pub fn resume_remote_updates(&mut self) {
        self.suppress_remote = false;
        self.settled.fill(false);
    }

    /// `transferState(from)`: the client's view of slots over the same (container, index),
    /// kept when switching menus (closing a chest back to the inventory menu).
    pub fn transfer_state(&mut self, from: &Menu) {
        self.settled.fill(false);
        for (i, s) in self.slots.iter().enumerate() {
            if s.source != Source::Player {
                continue;
            }
            if let Some(j) = from.slots.iter().rposition(|o| o.source == s.source && o.index == s.index) {
                self.last_slots[i] = from.last_slots[j].clone();
                if self.synchronized && from.synchronized {
                    self.remote[i] = from.remote[j].clone();
                }
            }
        }
    }

    /// `isValidSlotIndex`: -1, -999 and any index below the slot count (so every negative).
    pub fn is_valid_slot_index(&self, slot: i32) -> bool {
        slot == -1 || slot == -999 || slot < self.slots.len() as i32
    }

    // ---- clicks -------------------------------------------------------------------------------

    fn reset_quick_craft(&mut self) {
        self.quickcraft_status = 0;
        self.quickcraft_slots.clear();
    }

    /// `AbstractContainerMenu.clicked`.
    pub fn clicked(&mut self, env: &mut Env, slot: i32, button: i32, input: ContainerInput) -> Result<(), ClickCrash> {
        if input == ContainerInput::QuickCraft {
            return self.quick_craft(env, slot, button);
        }
        if self.quickcraft_status != 0 {
            self.reset_quick_craft();
            return Ok(());
        }
        match input {
            ContainerInput::Pickup | ContainerInput::QuickMove if button == 0 || button == 1 => {
                let action = if button == 0 { ClickAction::Primary } else { ClickAction::Secondary };
                if slot == -999 {
                    if !self.carried.is_empty() {
                        let dropped = match action {
                            ClickAction::Primary => std::mem::take(&mut self.carried),
                            ClickAction::Secondary => self.carried.split_count(1),
                        };
                        env.drop_item(dropped, true);
                    }
                } else if input == ContainerInput::QuickMove {
                    if slot < 0 {
                        return Ok(());
                    }
                    let i = self.index(slot)?;
                    if !self.may_pickup(env, i) {
                        return Ok(());
                    }
                    let mut moved = self.quick_move_stack(env, i);
                    while !moved.is_empty() && same_item(self.item(env, i), &moved) {
                        moved = self.quick_move_stack(env, i);
                    }
                } else {
                    if slot < 0 {
                        return Ok(());
                    }
                    let i = self.index(slot)?;
                    self.pickup(env, i, action)?;
                }
            }
            ContainerInput::Swap if (0..9).contains(&button) || button == 40 => {
                let i = self.index(slot)?;
                self.swap(env, i, button as usize);
            }
            ContainerInput::Clone if env.player.infinite_materials && self.carried.is_empty() && slot >= 0 => {
                let i = self.index(slot)?;
                if self.has_item(env, i) {
                    self.carried = self.safe_clone(env, i);
                }
            }
            ContainerInput::Throw if self.carried.is_empty() && slot >= 0 => {
                let i = self.index(slot)?;
                let n = if button == 0 { 1 } else { self.item(env, i).count() };
                let mut taken = self.safe_take(env, i, n, i32::MAX);
                env.drop_item(taken.clone(), true);
                if button == 1 {
                    while !taken.is_empty() && same_item(self.item(env, i), &taken) {
                        taken = self.safe_take(env, i, n, i32::MAX);
                        env.drop_item(taken.clone(), true);
                    }
                }
            }
            ContainerInput::PickupAll if slot >= 0 => {
                let i = self.index(slot)?;
                self.pickup_all(env, i, button);
            }
            _ => {}
        }
        Ok(())
    }

    fn quick_craft(&mut self, env: &mut Env, slot: i32, button: i32) -> Result<(), ClickCrash> {
        let previous = self.quickcraft_status;
        self.quickcraft_status = button & 3;
        let valid_transition = (previous == 1 && self.quickcraft_status == 2) || previous == self.quickcraft_status;
        if !valid_transition || self.carried.is_empty() {
            self.reset_quick_craft();
        } else if self.quickcraft_status == 0 {
            self.quickcraft_type = (button >> 2) & 3;
            let valid = match self.quickcraft_type {
                0 | 1 => true,
                2 => env.player.infinite_materials,
                _ => false,
            };
            if valid {
                self.quickcraft_status = 1;
                self.quickcraft_slots.clear();
            } else {
                self.reset_quick_craft();
            }
        } else if self.quickcraft_status == 1 {
            let i = self.index(slot)?;
            let carried = &self.carried;
            if can_item_quick_replace(self.item(env, i), carried, true)
                && self.may_place(env, i, carried)
                && (self.quickcraft_type == 2 || carried.count() as usize > self.quickcraft_slots.len())
                && self.kind.can_drag_to(self.slots[i])
                && !self.quickcraft_slots.contains(&i)
            {
                self.quickcraft_slots.push(i);
            }
        } else if self.quickcraft_status == 2 {
            if !self.quickcraft_slots.is_empty() {
                if self.quickcraft_slots.len() == 1 {
                    let i = self.quickcraft_slots[0];
                    self.reset_quick_craft();
                    let kind = self.quickcraft_type;
                    return self.clicked(env, i as i32, kind, ContainerInput::Pickup);
                }
                let mut stack = self.carried.copy();
                if stack.is_empty() {
                    self.reset_quick_craft();
                    return Ok(());
                }
                let mut remaining = self.carried.count();
                let slots = self.quickcraft_slots.clone();
                for &i in &slots {
                    let carried = &self.carried;
                    if can_item_quick_replace(self.item(env, i), carried, true)
                        && self.may_place(env, i, carried)
                        && (self.quickcraft_type == 2 || carried.count() as usize >= slots.len())
                        && self.kind.can_drag_to(self.slots[i])
                    {
                        let existing = if self.has_item(env, i) { self.item(env, i).count() } else { 0 };
                        let max = stack.max_stack_size().min(self.slot_max_for(env, i, &stack));
                        let place = (quick_craft_place_count(slots.len(), self.quickcraft_type, &stack) + existing).min(max);
                        remaining -= place - existing;
                        self.set_by_player(env, i, stack.copy_with_count(place));
                    }
                }
                stack.set_count(remaining);
                self.carried = stack;
            }
            self.reset_quick_craft();
        } else {
            self.reset_quick_craft();
        }
        Ok(())
    }

    fn pickup(&mut self, env: &mut Env, i: usize, action: ClickAction) -> Result<(), ClickCrash> {
        if !crate::bundle::click_override(self, env, i, action)? {
            let slot_empty = self.item(env, i).is_empty();
            if slot_empty {
                if !self.carried.is_empty() {
                    let n = if action == ClickAction::Primary { self.carried.count() } else { 1 };
                    let carried = std::mem::take(&mut self.carried);
                    self.carried = self.safe_insert(env, i, carried, n);
                }
            } else if self.may_pickup(env, i) {
                if self.carried.is_empty() {
                    let count = self.item(env, i).count();
                    let n = if action == ClickAction::Primary { count } else { (count + 1) / 2 };
                    if let Some(taken) = self.try_remove(env, i, n, i32::MAX) {
                        self.carried = taken;
                        let mut carried = std::mem::take(&mut self.carried);
                        self.on_take(env, i, &mut carried);
                        self.carried = carried;
                    }
                } else if self.may_place(env, i, &self.carried) {
                    if same_item_same_components(self.item(env, i), &self.carried) {
                        let n = if action == ClickAction::Primary { self.carried.count() } else { 1 };
                        let carried = std::mem::take(&mut self.carried);
                        self.carried = self.safe_insert(env, i, carried, n);
                    } else if self.carried.count() <= self.slot_max_for(env, i, &self.carried) {
                        let in_slot = self.item(env, i).clone();
                        let carried = std::mem::replace(&mut self.carried, in_slot.clone());
                        self.set_by_player_old(env, i, carried, in_slot);
                    }
                } else if same_item_same_components(self.item(env, i), &self.carried) {
                    let count = self.item(env, i).count();
                    let max = self.carried.max_stack_size() - self.carried.count();
                    if let Some(mut taken) = self.try_remove(env, i, count, max) {
                        self.carried.grow_count(taken.count());
                        self.on_take(env, i, &mut taken);
                    }
                }
            }
        }
        self.set_changed(env, i);
        Ok(())
    }

    fn swap(&mut self, env: &mut Env, i: usize, button: usize) {
        let hotbar = env.inventory.item(button).clone();
        let in_slot = self.item(env, i).clone();
        let s = self.slots[i];
        let aliased = s.source == Source::Player && s.index == button;
        if hotbar.is_empty() && in_slot.is_empty() {
            return;
        }
        if hotbar.is_empty() {
            if self.may_pickup(env, i) {
                env.inventory.set_item(button, in_slot.clone());
                self.on_swap_craft(i, in_slot.count());
                self.set_by_player(env, i, ItemStack::empty());
                self.on_take_in_inventory(env, i, button);
            }
        } else if in_slot.is_empty() {
            if self.may_place(env, i, &hotbar) {
                let max = self.slot_max_for(env, i, &hotbar);
                if hotbar.count() > max {
                    let part = env.inventory.item_mut(button).split_count(max);
                    self.set_by_player(env, i, part);
                } else {
                    env.inventory.set_item(button, ItemStack::empty());
                    self.set_by_player(env, i, hotbar);
                }
            }
        } else if self.may_pickup(env, i) && self.may_place(env, i, &hotbar) {
            let max = self.slot_max_for(env, i, &hotbar);
            if hotbar.count() > max {
                let part = env.inventory.item_mut(button).split_count(max);
                let old = self.item(env, i).clone();
                self.set_by_player_old(env, i, part, old.clone());
                // The slot's previous stack; when the slot is the hotbar slot itself, that is
                // the stack just split.
                let mut previous = if aliased { old } else { in_slot };
                self.on_take(env, i, &mut previous);
                if !env.inventory.add(None, &mut previous, env.player.infinite_materials) {
                    env.drop_item(previous, true);
                }
            } else {
                env.inventory.set_item(button, in_slot);
                self.set_by_player(env, i, hotbar);
                self.on_take_in_inventory(env, i, button);
            }
        }
    }

    fn pickup_all(&mut self, env: &mut Env, i: usize, button: i32) {
        if self.carried.is_empty() || (self.has_item(env, i) && self.may_pickup(env, i)) {
            return;
        }
        let n = self.slots.len() as i32;
        let (start, step) = if button == 0 { (0, 1) } else { (n - 1, -1) };
        for pass in 0..2 {
            let mut j = start;
            while j >= 0 && j < n && self.carried.count() < self.carried.max_stack_size() {
                let k = j as usize;
                if self.has_item(env, k)
                    && can_item_quick_replace(self.item(env, k), &self.carried, true)
                    && self.may_pickup(env, k)
                    && self.kind.can_take_item_for_pick_all(self.slots[k])
                {
                    let item = self.item(env, k);
                    if !(pass == 0 && item.count() == item.max_stack_size()) {
                        let count = item.count();
                        let max = self.carried.max_stack_size() - self.carried.count();
                        let taken = self.safe_take(env, k, count, max);
                        self.carried.grow_count(taken.count());
                    }
                }
                j += step;
            }
        }
    }

    /// `moveItemStackTo`: merges `stack` (the stack of the slot being moved, mutated as vanilla
    /// mutates it in place) into slots `start..end`.
    pub(crate) fn move_item_stack_to(&mut self, env: &mut Env, stack: &mut ItemStack, start: usize, end: usize, reverse: bool) -> bool {
        let mut moved = false;
        let (start, end) = (start as i32, end as i32);
        let in_range = |i: i32| if reverse { i >= start } else { i < end };
        let step = if reverse { -1 } else { 1 };
        if stack.is_stackable() {
            let mut i = if reverse { end - 1 } else { start };
            while !stack.is_empty() && in_range(i) {
                let k = i as usize;
                let dest = self.item(env, k);
                if !dest.is_empty() && same_item_same_components(stack, dest) {
                    let total = dest.count() + stack.count();
                    let max = self.slot_max_for(env, k, dest);
                    if total <= max {
                        stack.set_count(0);
                        self.item_mut(env, k).set_count(total);
                        self.set_changed(env, k);
                        moved = true;
                    } else if dest.count() < max {
                        stack.shrink_count(max - dest.count());
                        self.item_mut(env, k).set_count(max);
                        self.set_changed(env, k);
                        moved = true;
                    }
                }
                i += step;
            }
        }
        if !stack.is_empty() {
            let mut i = if reverse { end - 1 } else { start };
            while in_range(i) {
                let k = i as usize;
                if self.item(env, k).is_empty() && self.may_place(env, k, stack) {
                    let max = self.slot_max_for(env, k, stack);
                    let part = stack.split_count(stack.count().min(max));
                    self.set_by_player(env, k, part);
                    self.set_changed(env, k);
                    moved = true;
                    break;
                }
                i += step;
            }
        }
        moved
    }

    /// The common tail of `quickMoveStack`: writes the moved stack back into its slot the way
    /// vanilla leaves it (in place), then empties or marks the slot. Returns the `quickMoveStack`
    /// result.
    pub(crate) fn finish_quick_move(&mut self, env: &mut Env, i: usize, mut stack: ItemStack, copy: ItemStack, old_is_copy: bool) -> (ItemStack, ItemStack) {
        *self.item_mut(env, i) = stack.clone();
        if stack.is_empty() {
            if old_is_copy {
                self.set_by_player_old(env, i, ItemStack::empty(), copy.clone());
            } else {
                self.set_by_player(env, i, ItemStack::empty());
            }
        } else {
            self.set_changed(env, i);
        }
        if stack.count() == copy.count() {
            return (ItemStack::empty(), stack);
        }
        self.on_take(env, i, &mut stack);
        (copy, stack)
    }

    /// The tail of `ChestMenu.quickMoveStack` (and hopper, shulker box): no count check, no
    /// `onTake`.
    pub(crate) fn finish_move_simple(&mut self, env: &mut Env, i: usize, stack: ItemStack) {
        *self.item_mut(env, i) = stack.clone();
        if stack.is_empty() {
            self.set_by_player(env, i, ItemStack::empty());
        } else {
            self.set_changed(env, i);
        }
    }

    /// `Item.onCraftedBy(stack, player)` on the result slot's own stack (`CraftingMenu`
    /// shift-click).
    pub(crate) fn post_process_result(&mut self, env: &mut Env, stack: &mut ItemStack) {
        if stack.has(kiln_item::component::ids::MAP_POST_PROCESSING) {
            env.world.post_process_map(stack);
            self.result.item = stack.clone();
        }
    }

    /// `quickMoveStack` of this menu's kind.
    fn quick_move_stack(&mut self, env: &mut Env, i: usize) -> ItemStack {
        crate::menus::quick_move_stack(self, env, i)
    }

    /// `removed(player)`: the carried stack (and a crafting grid) go back to the inventory, or
    /// are dropped for a player that left.
    pub fn removed(&mut self, env: &mut Env) {
        if !self.carried.is_empty() {
            let carried = std::mem::take(&mut self.carried);
            drop_or_place_in_inventory(env, carried);
        }
        match self.kind {
            MenuKind::Inventory => {
                self.result.item = ItemStack::empty();
                for i in 0..self.craft.items.len() {
                    let stack = crate::container::take_item(&mut self.craft.items, i);
                    clear_container_item(env, stack);
                }
            }
            MenuKind::Crafting => {
                for i in 0..self.craft.items.len() {
                    let stack = crate::container::take_item(&mut self.craft.items, i);
                    clear_container_item(env, stack);
                }
            }
            MenuKind::Stonecutter
            | MenuKind::Smithing
            | MenuKind::Grindstone
            | MenuKind::Anvil
            | MenuKind::Loom
            | MenuKind::CartographyTable
            | MenuKind::Enchantment => {
                if self.kind == MenuKind::Stonecutter {
                    self.result.item = ItemStack::empty();
                }
                for i in 0..self.input.items.len() {
                    let stack = crate::container::take_item(&mut self.input.items, i);
                    clear_container_item(env, stack);
                }
            }
            MenuKind::Merchant => crate::merchant::removed(self, env),
            // `BeaconMenu.removed`: the payment is dropped.
            MenuKind::Beacon => {
                let stack = crate::container::take_item(&mut self.input.items, 0);
                env.drop_item(stack, false);
            }
            _ => {}
        }
    }
}

/// `AbstractContainerMenu.clearContainer` for one stack: dropped for a dead or removed
/// player.
fn clear_container_item(env: &mut Env, stack: ItemStack) {
    if env.player.dead {
        env.drop_item(stack, false);
    } else {
        drop_or_place_in_inventory(env, stack);
    }
}

/// `AbstractContainerMenu.dropOrPlaceInInventory`.
fn drop_or_place_in_inventory(env: &mut Env, stack: ItemStack) {
    if env.player.removed {
        env.drop_item(stack, false);
        return;
    }
    let (updates, left) = env.inventory.place_item_back(stack, env.player.infinite_materials);
    for u in updates {
        env.out.push(Effect::SetPlayerInventory { slot: u.slot as i32, stack: u.stack });
    }
    if let Some(left) = left {
        env.drop_item(left, false);
    }
}

/// `canItemQuickReplace(slot, stack, stackSizeMatters)`.
pub(crate) fn can_item_quick_replace(in_slot: &ItemStack, stack: &ItemStack, size_matters: bool) -> bool {
    if in_slot.is_empty() {
        return true;
    }
    same_item_same_components(stack, in_slot)
        && in_slot.count() + if size_matters { 0 } else { stack.count() } <= stack.max_stack_size()
}

/// `getQuickCraftPlaceCount`.
fn quick_craft_place_count(slots: usize, kind: i32, stack: &ItemStack) -> i32 {
    match kind {
        0 => (stack.count() as f32 / slots as f32).floor() as i32,
        1 => 1,
        2 => stack.max_stack_size(),
        _ => stack.count(),
    }
}

impl Container for CraftGrid {
    fn size(&self) -> usize {
        self.items.len()
    }

    fn item(&self, slot: usize) -> &ItemStack {
        &self.items[slot]
    }

    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        &mut self.items[slot]
    }

    fn set_item(&mut self, slot: usize, stack: ItemStack) {
        self.items[slot] = stack;
    }

    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        crate::container::remove_item(&mut self.items, slot, count)
    }

    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        crate::container::take_item(&mut self.items, slot)
    }
}

impl Container for ResultBox {
    fn size(&self) -> usize {
        1
    }

    fn item(&self, _slot: usize) -> &ItemStack {
        &self.item
    }

    fn item_mut(&mut self, _slot: usize) -> &mut ItemStack {
        &mut self.item
    }

    fn set_item(&mut self, _slot: usize, stack: ItemStack) {
        self.item = stack;
    }

    fn remove_item(&mut self, _slot: usize, _count: i32) -> ItemStack {
        std::mem::take(&mut self.item)
    }

    fn remove_item_no_update(&mut self, _slot: usize) -> ItemStack {
        std::mem::take(&mut self.item)
    }
}
