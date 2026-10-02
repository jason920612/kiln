//! Menu slots (`net.minecraft.world.inventory.Slot` and its subclasses) and the per-kind rules.

use crate::rules::Rules;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};

/// Which container a slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The player's [`crate::PlayerInventory`].
    Player,
    /// The block (or entity) container the menu was opened on.
    Block,
    /// The menu's crafting grid (`TransientCraftingContainer`).
    Craft,
    /// The menu's result container (`ResultContainer`).
    Result,
    /// The menu's own input container (stonecutter, smithing table): a `SimpleContainer` whose
    /// changes make the menu update its result.
    Input,
    /// The merchant menu's trade container (`MerchantContainer`, see [`crate::merchant`]).
    Merchant,
}

/// The slot subclass: placement, pickup and stack size rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// `Slot`.
    Normal,
    /// `ArmorSlot`: one item that is equippable in this slot; a curse of binding keeps it on.
    Armor(EquipmentSlot),
    /// `ArmorSlot` of a mount's screen (`HorseInventoryMenu`): the saddle or body slot of an
    /// animal of entity type `entity` (a `minecraft:entity_type` id); it takes only what that
    /// type may wear, and nothing when the animal cannot use the slot (`usable`).
    Mount { slot: EquipmentSlot, entity: i32, usable: bool },
    /// The inventory menu's off hand slot (`InventoryMenu$1`).
    Offhand,
    /// `ResultSlot` of a crafting grid.
    CraftResult,
    /// `FurnaceFuelSlot`.
    FurnaceFuel,
    /// `FurnaceResultSlot`.
    FurnaceResult,
    /// `ShulkerBoxSlot`: no shulker boxes.
    ShulkerBox,
    /// The stonecutter's result slot (`StonecutterMenu$2`).
    StonecutterResult,
    /// A smithing table input: 0 template, 1 base, 2 addition (items of the matching recipe
    /// property set).
    SmithingInput(u8),
    /// The smithing table's result slot (`ItemCombinerMenu$3`).
    SmithingResult,
    /// `MerchantResultSlot`: taking the result makes the trade.
    MerchantResult,
    /// A grindstone input: damageable or enchanted items (`GrindstoneMenu$2`, `$3`).
    GrindstoneInput,
    /// The grindstone's result slot (`GrindstoneMenu$4`).
    GrindstoneResult,
    /// The anvil's result slot (`ItemCombinerMenu$3` with `AnvilMenu.mayPickup`).
    AnvilResult,
    /// The loom's banner, dye and pattern slots, and its result.
    LoomBanner,
    LoomDye,
    LoomPattern,
    LoomResult,
    /// The cartography table's map slot (anything with a map id), its additional slot (paper,
    /// an empty map, a glass pane) and its result.
    CartographyMap,
    CartographyAdditional,
    CartographyResult,
    /// The enchanting table's item slot (one item) and lapis slot.
    EnchantItem,
    EnchantLapis,
    /// A brewing stand's bottle slot (`BrewingStandMenu$PotionSlot`): one potion input.
    BrewingPotion,
    /// Its ingredient slot (`IngredientsSlot`): a brewing reagent.
    BrewingIngredient,
    /// Its fuel slot (`FuelSlot`): items with `brewing_fuel`.
    BrewingFuel,
    /// A beacon's payment slot (`BeaconMenu$PaymentSlot`): one beacon payment item.
    BeaconPayment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub source: Source,
    /// Index in the source container.
    pub index: usize,
    pub kind: SlotKind,
}

/// `PotionIngredient.isPotionInput`: an item brewing recipes take as their input, or one in
/// `#minecraft:brewing_potion_inputs`.
pub fn is_potion_input(stack: &ItemStack, rules: &Rules) -> bool {
    rules.recipes.property_set_accepts("minecraft:brewing_input", stack)
        || crate::tags::contains("minecraft:item", "minecraft:brewing_potion_inputs", crate::stack::StackExt::effective_item(stack))
}

impl Slot {
    pub const fn new(source: Source, index: usize, kind: SlotKind) -> Self {
        Slot { source, index, kind }
    }

    /// `mayPlace`.
    pub fn may_place(&self, stack: &ItemStack, rules: &Rules) -> bool {
        match self.kind {
            SlotKind::Normal | SlotKind::Offhand => true,
            SlotKind::Armor(slot) => rules.is_equippable_in_slot(stack, slot),
            SlotKind::Mount { slot, entity, usable } => usable && mount_may_wear(stack, slot, entity),
            SlotKind::CraftResult
            | SlotKind::FurnaceResult
            | SlotKind::StonecutterResult
            | SlotKind::SmithingResult
            | SlotKind::GrindstoneResult
            | SlotKind::AnvilResult
            | SlotKind::LoomResult
            | SlotKind::CartographyResult
            | SlotKind::MerchantResult => false,
            SlotKind::LoomBanner => crate::stations::is_banner(stack),
            SlotKind::LoomDye => crate::stations::is_loom_dye(stack),
            SlotKind::LoomPattern => crate::stations::is_loom_pattern(stack),
            SlotKind::CartographyMap => stack.has(kiln_item::component::ids::MAP_ID),
            SlotKind::CartographyAdditional => crate::stations::is_cartography_additional(stack),
            SlotKind::EnchantItem => true,
            SlotKind::EnchantLapis => stack.effective_item_name() == "minecraft:lapis_lazuli",
            SlotKind::BrewingPotion => is_potion_input(stack, rules),
            SlotKind::BrewingIngredient => rules.recipes.property_set_accepts("minecraft:brewing_reagent", stack),
            SlotKind::BrewingFuel => stack.has(kiln_item::component::ids::BREWING_FUEL),
            SlotKind::BeaconPayment => crate::tags::contains("minecraft:item", "minecraft:beacon_payment_items", crate::stack::StackExt::effective_item(stack)),
            SlotKind::GrindstoneInput => stack.is_damageable_item() || crate::workstation::has_any_enchantments(stack),
            SlotKind::SmithingInput(k) => {
                let key = ["minecraft:smithing_template", "minecraft:smithing_base", "minecraft:smithing_addition"][k as usize];
                rules.recipes.property_set_accepts(key, stack)
            }
            SlotKind::FurnaceFuel => stack.has(kiln_item::component::ids::COOKING_FUEL) || is_bucket(stack),
            SlotKind::ShulkerBox => can_fit_inside_container_items(stack),
        }
    }

    /// `mayPickup`.
    pub fn may_pickup(&self, item: &ItemStack, creative: bool, rules: &Rules) -> bool {
        match self.kind {
            SlotKind::Armor(_) | SlotKind::Mount { .. } => item.is_empty() || creative || !rules.prevents_armor_change(item),
            _ => true,
        }
    }

    /// `getMaxStackSize()`, given the container's.
    pub fn max_stack_size(&self, container_max: i32) -> i32 {
        match self.kind {
            SlotKind::Armor(_) | SlotKind::Mount { .. } | SlotKind::EnchantItem | SlotKind::BrewingPotion | SlotKind::BeaconPayment => 1,
            _ => container_max,
        }
    }

    /// `getMaxStackSize(ItemStack)`.
    pub fn max_stack_size_for(&self, container_max: i32, stack: &ItemStack) -> i32 {
        match self.kind {
            SlotKind::FurnaceFuel if is_bucket(stack) => 1,
            _ => self.max_stack_size(container_max).min(stack.max_stack_size()),
        }
    }

    /// The equipment slot whose `onEquipItem` a player change of this slot triggers.
    pub fn equip_slot(&self) -> Option<EquipmentSlot> {
        match self.kind {
            SlotKind::Armor(slot) => Some(slot),
            SlotKind::Offhand => Some(EquipmentSlot::OffHand),
            _ => None,
        }
    }
}

/// `LivingEntity.isEquippableInSlot` less `canUseSlot`: `stack`'s `equippable` names `slot` and
/// lets entity type `entity` wear it.
pub fn mount_may_wear(stack: &ItemStack, slot: EquipmentSlot, entity: i32) -> bool {
    use kiln_item::HolderSet;
    let Some(e) = stack.get(keys::EQUIPPABLE) else { return false };
    if e.slot != slot {
        return false;
    }
    match &e.allowed_entities {
        None => true,
        Some(HolderSet::Direct(ids)) => ids.contains(&entity),
        Some(HolderSet::Tag(tag)) => crate::tags::contains("minecraft:entity_type", tag.as_str(), entity),
    }
}

/// `FurnaceFuelSlot.isBucket`.
pub fn is_bucket(stack: &ItemStack) -> bool {
    stack.effective_item_name() == "minecraft:bucket"
}

/// `Item.canFitInsideContainerItems`: false for shulker boxes.
pub fn can_fit_inside_container_items(stack: &ItemStack) -> bool {
    !stack.effective_item_name().ends_with("shulker_box")
}

/// `getEquipmentSlotForItem` for a player: the `equippable` slot, else the main hand.
pub fn equipment_slot_for_item(stack: &ItemStack) -> EquipmentSlot {
    stack.get(keys::EQUIPPABLE).map_or(EquipmentSlot::MainHand, |e| e.slot)
}

trait EffectiveName {
    fn effective_item_name(&self) -> &'static str;
}

impl EffectiveName for ItemStack {
    fn effective_item_name(&self) -> &'static str {
        if self.is_empty() { "minecraft:air" } else { self.item_name() }
    }
}
