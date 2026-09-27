//! The block-setting halves of `/setblock` and `/fill` (`SetBlockCommand`, `FillCommand`,
//! `BlockInput.place`), with vanilla's update semantics: the placed state is first shaped by
//! its neighbours (except for properties the command names), set without neighbour updates,
//! then neighbours are notified as for any block change (not in `strict` mode).

use crate::level::{Level, flags};
use crate::pos::BlockPos;
use crate::state::{self, BlockId};
use crate::update::{destroy_block, set_block, update_from_neighbour_shapes, update_neighbors_at, update_neighbour_for_output_signal};
use kiln_data::block_logic as logic;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;

/// A block state argument: the state and the properties it names explicitly.
#[derive(Clone, Debug)]
pub struct BlockInput {
    pub state: u16,
    pub defined: Vec<(String, String)>,
}

impl BlockInput {
    /// Parses `minecraft:name[prop=value,...]`.
    pub fn parse(text: &str) -> Option<Self> {
        let (name, props) = match text.find('[') {
            Some(i) => (&text[..i], text[i + 1..].strip_suffix(']')?),
            None => (text, ""),
        };
        let mut s = BlockId::by_name(name)?.default_state();
        let mut defined = Vec::new();
        for kv in props.split(',').filter(|kv| !kv.trim().is_empty()) {
            let (k, v) = kv.split_once('=')?;
            let (k, v) = (k.trim(), v.trim());
            s = state::set(s, k, v);
            if state::get(s, k) != Some(v) {
                return None;
            }
            defined.push((k.to_string(), v.to_string()));
        }
        Some(Self { state: s, defined })
    }

    fn overwrite_defined(&self, mut s: u16) -> u16 {
        for (k, v) in &self.defined {
            s = state::set(s, k, v);
        }
        s
    }

    /// `BlockInput.place`: returns whether the block changed.
    pub fn place<L: Level>(&self, level: &mut L, pos: BlockPos, flags: u32) -> bool {
        let mut s = if flags & flags::KNOWN_SHAPE != 0 { self.state } else { update_from_neighbour_shapes(level, self.state, pos) };
        if is_air(s) {
            s = self.state;
        }
        let s = self.overwrite_defined(s);
        set_block(level, pos, s, flags)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetMode {
    Replace,
    Destroy,
    Keep,
}

fn place_flags(strict: bool) -> u32 {
    flags::CLIENTS | if strict { flags::SKIP_ALL_SIDEEFFECTS } else { flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS }
}

/// `ServerLevel.updateNeighboursOnBlockSet`.
pub fn update_neighbours_on_block_set<L: Level>(level: &mut L, pos: BlockPos, old: u16) {
    let new = level.block(pos);
    if !state::same_block(old, new) {
        crate::behaviour::affect_neighbors_after_removal(level, old, pos, false);
    }
    update_neighbors_at(level, pos, BlockId::of(new));
    if logic::has_analog_output(new) {
        update_neighbour_for_output_signal(level, pos, BlockId::of(new));
    }
}

/// `/setblock`. `Err` when vanilla reports "Could not set the block".
pub fn setblock<L: Level>(level: &mut L, pos: BlockPos, input: &BlockInput, mode: SetMode, strict: bool) -> Result<(), ()> {
    if mode == SetMode::Keep && !is_air(level.block(pos)) {
        return Err(());
    }
    let place = if mode == SetMode::Destroy {
        destroy_block(level, pos, true, flags::LIMIT);
        !(is_air(input.state) && is_air(level.block(pos)))
    } else {
        true
    };
    let old = level.block(pos);
    if place && !input.place(level, pos, place_flags(strict)) {
        return Err(());
    }
    if !strict {
        update_neighbours_on_block_set(level, pos, old);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    Replace,
    Outline,
    Hollow,
    Destroy,
    /// `replace` with an air filter (`keep`).
    Keep,
}

/// `/fill` over the inclusive box. Returns the count vanilla reports, `Err` for zero.
pub fn fill<L: Level>(level: &mut L, a: BlockPos, b: BlockPos, input: &BlockInput, mode: FillMode, strict: bool) -> Result<usize, ()> {
    let min = BlockPos::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
    let max = BlockPos::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
    let air = BlockInput { state: d::AIR, defined: Vec::new() };
    let mut placed = Vec::new();
    let mut count = 0;
    for z in min.z..=max.z {
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                let pos = BlockPos::new(x, y, z);
                if mode == FillMode::Keep && !is_air(level.block(pos)) {
                    continue;
                }
                let old = level.block(pos);
                let affected = mode == FillMode::Destroy && destroy_block(level, pos, true, flags::LIMIT);
                let edge = x == min.x || x == max.x || y == min.y || y == max.y || z == min.z || z == max.z;
                let what = match mode {
                    FillMode::Outline if !edge => None,
                    FillMode::Hollow if !edge => Some(&air),
                    _ => Some(input),
                };
                let Some(what) = what else {
                    count += affected as usize;
                    continue;
                };
                if !what.place(level, pos, place_flags(strict)) {
                    count += affected as usize;
                    continue;
                }
                if !strict {
                    placed.push((pos, old));
                }
                count += 1;
            }
        }
    }
    for (pos, old) in placed {
        update_neighbours_on_block_set(level, pos, old);
    }
    if count == 0 { Err(()) } else { Ok(count) }
}
