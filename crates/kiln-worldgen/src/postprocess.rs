//! Which placed blocks mark a position for post-processing (`BlockState.getPostProcessPos`,
//! the `hasPostProcess` block property).

use crate::pos::BlockPos;
use crate::region::Region;

/// `BlockState.getPostProcessPos(level, pos)`: the position to mark when `state` is set at
/// `pos` during generation, if any.
pub fn post_process_pos(_state: u16, _region: &mut Region, _pos: BlockPos) -> Option<BlockPos> {
    None
}
