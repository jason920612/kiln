//! What a dispenser does to the living things in front of it (`EquipmentDispenseItemBehavior.dispenseEquipment`
//! and the other behaviours that look for an entity there).

use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction};
use kiln_item::ItemStack;

/// `EquipmentDispenseItemBehavior.dispenseEquipment`: one of `stack` is put on the first living thing in front of
/// the dispenser that can wear it. Whether it was.
pub(super) fn dispense_equipment(level: &mut RegionLevel, pos: BlockPos, facing: Direction, stack: &mut ItemStack) -> bool {
    let _ = (level, pos, facing, stack);
    false
}
