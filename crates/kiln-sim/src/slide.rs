//! Sliding down honey blocks (`HoneyBlock.entityInside`): a player falling along the side of a
//! honey block they are inside earns `slide_down_block`, checked once every 20 game ticks.

use crate::Player;
use kiln_data::blocks::default_state as d;
use kiln_entity::math::BlockPos;

/// `HoneyBlock.isSlidingDown`.
fn sliding_down(p: &Player, pos: BlockPos) -> bool {
    if p.on_ground || p.pos[1] > pos.y as f64 + 0.9375 - 1.0E-7 || p.known_movement[1] >= -0.08 {
        return false;
    }
    let dx = (pos.x as f64 + 0.5 - p.pos[0]).abs();
    let dz = (pos.z as f64 + 0.5 - p.pos[2]).abs();
    let reach = 0.4375 + 0.6f32 as f64 / 2.0;
    dx + 1.0E-7 > reach || dz + 1.0E-7 > reach
}

impl Player {
    /// `maybeDoSlideAchievement` for the honey blocks the player's box overlaps.
    pub(crate) fn tick_honey_slide(&mut self, block: &dyn Fn(BlockPos) -> u16, game_time: i64) {
        if game_time % 20 != 0 || self.dead {
            return;
        }
        let bb = self.bounding_box();
        let (lo, hi) = ([bb.min_x + 1.0E-5, bb.min_y + 1.0E-5, bb.min_z + 1.0E-5], [bb.max_x - 1.0E-5, bb.max_y - 1.0E-5, bb.max_z - 1.0E-5]);
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    let pos = BlockPos::new(x, y, z);
                    let s = block(pos);
                    if kiln_blocks::state::same_block(s, d::HONEY_BLOCK) && sliding_down(self, pos) {
                        self.slid_down_block(s);
                        return;
                    }
                }
            }
        }
    }
}
