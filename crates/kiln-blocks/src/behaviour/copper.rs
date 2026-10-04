//! Copper oxidation: `ChangeOverTimeBlock.changeOverTime` / `getNextState` of every
//! `WeatheringCopper` block (full blocks, cut copper, stairs, slabs, doors, trapdoors, bulbs,
//! grates, chains, bars, lanterns, lightning rods, chests, golem statues). Waxed blocks are plain
//! classes and never change.

use crate::level::Level;
use crate::pos::BlockPos;
use crate::state::{self, BlockId};
use crate::update::set_block_and_update;
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::blocks::BLOCKS;
use kiln_javamath::random::RandomSource;
use std::sync::OnceLock;

/// `ChangeOverTimeBlock.SCAN_DISTANCE`.
const SCAN_DISTANCE: i32 = 4;

/// `WeatheringCopper.WeatherState` ordinal and the block the next stage is, per block index.
struct Weathering {
    age: u8,
    next: Option<BlockId>,
}

fn is_weathering_class(class: BlockClass) -> bool {
    use BlockClass as C;
    matches!(
        class,
        C::WeatheringCopperBarsBlock
            | C::WeatheringCopperBulbBlock
            | C::WeatheringCopperChainBlock
            | C::WeatheringCopperChestBlock
            | C::WeatheringCopperDoorBlock
            | C::WeatheringCopperFullBlock
            | C::WeatheringCopperGolemStatueBlock
            | C::WeatheringCopperGrateBlock
            | C::WeatheringCopperSlabBlock
            | C::WeatheringCopperStairBlock
            | C::WeatheringCopperTrapDoorBlock
            | C::WeatheringLanternBlock
            | C::WeatheringLightningRodBlock
    )
}

fn table() -> &'static [Option<Weathering>] {
    static TABLE: OnceLock<Vec<Option<Weathering>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        BLOCKS
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if !is_weathering_class(logic::class_info(i).classes[0]) {
                    return None;
                }
                let name = b.name.strip_prefix("minecraft:").unwrap_or(b.name);
                // `WeatheringCopperCollection`: the stages of one family, the first one unprefixed
                // (copper block / exposed copper is the one name that does not follow).
                let (age, base) = match name.split_once('_') {
                    Some(("exposed", rest)) => (1, rest),
                    Some(("weathered", rest)) => (2, rest),
                    Some(("oxidized", rest)) => (3, rest),
                    _ => (0, name),
                };
                let base = if base == "copper_block" { "copper" } else { base };
                let next_prefix = match age {
                    0 => Some("exposed"),
                    1 => Some("weathered"),
                    2 => Some("oxidized"),
                    _ => None,
                };
                let next = next_prefix.and_then(|p| BlockId::by_name(&format!("{p}_{base}")));
                Some(Weathering { age, next })
            })
            .collect()
    })
}

fn weathering(s: u16) -> Option<&'static Weathering> {
    table()[logic::block_index(s)].as_ref()
}

/// `WeatheringCopper.getNext(state)`: the next stage with the state's properties.
pub fn next_state(s: u16) -> Option<u16> {
    let next = weathering(s)?.next?;
    Some(state::with_properties_of(next.default_state(), s))
}

/// `ChangeOverTimeBlock.changeOverTime`: a 5.7% chance per random tick of looking at the
/// neighbourhood; an older block within 4 (Manhattan) blocks stops the change, else the chance
/// is `((older + 1) / (older + same + 1))^2` (times 0.75 for fresh copper).
pub fn change_over_time<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    const CHANCE: f32 = 0.05688889;
    if level.random().next_float() >= CHANCE {
        return;
    }
    let Some(me) = weathering(s) else { return };
    let (mut older, mut same) = (0i32, 0i32);
    for dx in -SCAN_DISTANCE..=SCAN_DISTANCE {
        for dy in -(SCAN_DISTANCE - dx.abs())..=(SCAN_DISTANCE - dx.abs()) {
            let rest = SCAN_DISTANCE - dx.abs() - dy.abs();
            for dz in -rest..=rest {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let Some(other) = weathering(level.block(BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz))) else { continue };
                if other.age < me.age {
                    return;
                }
                if other.age > me.age {
                    older += 1;
                } else {
                    same += 1;
                }
            }
        }
    }
    let f = (older + 1) as f32 / (older + same + 1) as f32;
    let modifier = if me.age == 0 { 0.75f32 } else { 1.0 };
    let chance = f * f * modifier;
    if level.random().next_float() < chance {
        if let Some(next) = next_state(s) {
            set_block_and_update(level, pos, next);
        }
    }
}

/// `randomTick` of every weathering block; the chest only ages with nobody looking into it (and
/// never as the right half of a double chest).
pub fn random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    match logic::block_class(s) {
        BlockClass::WeatheringCopperChestBlock if state::get(s, "type") == Some("right") || level.container_openers(pos) != 0 => return,
        // `WeatheringCopperDoorBlock.randomTick`: the lower half; the upper one follows its shape update.
        BlockClass::WeatheringCopperDoorBlock if state::get(s, "half") != Some("lower") => return,
        _ => {}
    }
    change_over_time(level, s, pos);
}

pub fn is_weathering(s: u16) -> bool {
    weathering(s).is_some()
}
