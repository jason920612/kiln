//! Comparator input (`BlockState.getAnalogOutputSignal`): blocks whose output follows from
//! their state are computed here; block entities (containers, lecterns, jukeboxes, sculk
//! sensors, ...) and minecarts on detector rails are the level's.

use crate::level::Level;
use crate::pos::{BlockPos, Direction};
use crate::state;
use kiln_data::block_logic::{self as logic, BlockClass};

/// `getAnalogOutputSignal` of the block `s` at `pos`, read from the side `dir`.
pub fn output<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> i32 {
    use BlockClass as C;
    let int = |p: &str| state::get_int(s, p);
    match logic::block_class(s) {
        C::BeehiveBlock => int("honey_level"),
        C::CakeBlock => (7 - int("bites")) * 2,
        C::CandleCakeBlock => 14,
        C::ComposterBlock => int("level"),
        C::LayeredCauldronBlock => int("level"),
        C::LavaCauldronBlock => 3,
        C::EndPortalFrameBlock => {
            if state::get_bool(s, "eye") { 15 } else { 0 }
        }
        C::RespawnAnchorBlock => (int("charges") as f32 / 4.0 * 15.0).floor() as i32,
        _ if logic::is_instance(s, C::CopperBulbBlock) => {
            if state::get_bool(level.block(pos), "lit") { 15 } else { 0 }
        }
        _ if logic::is_instance(s, C::CopperGolemStatueBlock) => state::value_index(s, "copper_golem_pose").unwrap_or(0) as i32 + 1,
        _ => level.block_entity_analog(pos, s, dir),
    }
}
