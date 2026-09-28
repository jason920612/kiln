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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub source: Source,
    /// Index in the source container.
    pub index: usize,
    pub kind: SlotKind,
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
            SlotKind::CraftResult | SlotKind::FurnaceResult | SlotKind::StonecutterResult | SlotKind::SmithingResult | SlotKind::MerchantResult => false,
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
            SlotKind::Armor(_) => item.is_empty() || creative || !rules.prevents_armor_change(item),
            _ => true,
        }
    }

    /// `getMaxStackSize()`, given the container's.
    pub fn max_stack_size(&self, container_max: i32) -> i32 {
        match self.kind {
            SlotKind::Armor(_) => 1,
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
