//! Which placed blocks mark a position for post-processing (`BlockState.getPostProcessPos`,
//! the `BlockBehaviour.Properties.postProcess` hook).
//!
//! In 26.3 only four blocks set the hook: mushrooms mark themselves (`Blocks.postProcessSelf`:
//! they re-check survival once the chunk is full) and soul sand / magma mark the block above
//! (`Blocks.postProcessAbove`: a bubble column may form there).

use crate::blocks::is_block;
use crate::pos::BlockPos;
use crate::region::Region;

/// `BlockState.getPostProcessPos(level, pos)`: the position to mark when `state` is set at
/// `pos` during generation, if any.
pub fn post_process_pos(state: u16, _region: &mut Region, pos: BlockPos) -> Option<BlockPos> {
    if is_block(state, "minecraft:brown_mushroom") || is_block(state, "minecraft:red_mushroom") {
        Some(pos)
    } else if is_block(state, "minecraft:soul_sand") || is_block(state, "minecraft:magma_block") {
        Some(pos.above())
    } else {
        None
    }
}
