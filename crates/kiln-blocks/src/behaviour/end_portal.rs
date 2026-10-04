//! The end portal frame and the eye of ender put into it (`EnderEyeItem.useOn` as far as the
//! level goes, `EndPortalFrameBlock.getOrCreatePortalShape`): the eye shows, comparators beside
//! the frame read 15, and a ring of twelve filled frames turns the 3x3 inside into `end_portal`.

use crate::level::{Effect, Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::update;
use kiln_data::blocks::default_state as d;

/// Whether `s` is an end portal frame.
pub fn is_frame(s: u16) -> bool {
    state::is(s, d::END_PORTAL_FRAME)
}

/// What the pattern `?vvv? / >???< / >???< / >???< / ?^^^?` needs at (`col`, `row`): `None`
/// anywhere, else a frame with its eye and this facing (`v` north, `^` south, `>` west, `<` east).
fn wanted(col: usize, row: usize) -> Option<&'static str> {
    match (row, col) {
        (0, 1..=3) => Some("north"),
        (4, 1..=3) => Some("south"),
        (1..=3, 0) => Some("west"),
        (1..=3, 4) => Some("east"),
        _ => None,
    }
}

/// `Vec3i.cross`.
fn cross(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `BlockPattern.matches` of the one-layer portal pattern with its front top left at `origin`.
fn matches<L: Level>(level: &L, origin: BlockPos, forwards: Direction, up: Direction) -> bool {
    let (f, u) = (forwards.step(), up.step());
    let left = cross(f, u);
    for row in 0..5i32 {
        for col in 0..5i32 {
            let Some(facing) = wanted(col as usize, row as usize) else { continue };
            // `translateAndRotate(origin, forwards, up, left = col, down = row, forward = 0)`.
            let at = BlockPos::new(
                origin.x + u[0] * -row + left[0] * col,
                origin.y + u[1] * -row + left[1] * col,
                origin.z + u[2] * -row + left[2] * col,
            );
            let s = level.block(at);
            if !is_frame(s) || !state::get_bool(s, "eye") || state::get(s, "facing") != Some(facing) {
                return false;
            }
        }
    }
    true
}

/// `BlockPattern.find(level, pos)`: the front top left of the first match among the 5x5x5
/// blocks from `pos` (`BlockPos.betweenClosed`: x fastest, then y, then z), trying every
/// `forwards` then `up` direction.
pub fn find_portal_shape<L: Level>(level: &L, pos: BlockPos) -> Option<BlockPos> {
    for z in 0..5 {
        for y in 0..5 {
            for x in 0..5 {
                let origin = BlockPos::new(pos.x + x, pos.y + y, pos.z + z);
                for forwards in Direction::ALL {
                    for up in Direction::ALL {
                        if up != forwards && up != forwards.opposite() && matches(level, origin, forwards, up) {
                            return Some(origin);
                        }
                    }
                }
            }
        }
    }
    None
}

/// `EnderEyeItem.useOn` without the item: `false` (pass) unless `pos` holds an empty frame;
/// else the frame gets its eye (clients only, no neighbour updates), comparators are told,
/// the fill sound and particles play and a complete ring opens its portal.
pub fn insert_eye<L: Level>(level: &mut L, pos: BlockPos) -> bool {
    let old = level.block(pos);
    if !is_frame(old) || state::get_bool(old, "eye") {
        return false;
    }
    // (`Block.pushEntitiesUp`, which lifts entities standing on the frame, is the caller's.)
    crate::set_block(level, pos, state::set_bool(old, "eye", true), flags::CLIENTS);
    update::update_neighbour_for_output_signal(level, pos, BlockId::of(old));
    level.effect(Effect::LevelEvent { id: 1503, pos, data: 0 });
    if let Some(top_left) = find_portal_shape(level, pos) {
        let first = BlockPos::new(top_left.x - 3, top_left.y, top_left.z - 3);
        for i in 0..3 {
            for j in 0..3 {
                let at = BlockPos::new(first.x + i, first.y, first.z + j);
                update::destroy_block(level, at, true, flags::LIMIT);
                crate::set_block(level, at, d::END_PORTAL, flags::CLIENTS);
            }
        }
        level.effect(Effect::GlobalLevelEvent { id: 1038, pos: BlockPos::new(first.x + 1, first.y, first.z + 1), data: 0 });
    }
    true
}
