//! What the entity phase reaches of a region's blocks: the region itself (mutable, as the
//! serial entity tick has it), or, for an island of entities that ticks beside the others
//! ([`crate::entities::tick`]), the blocks to read and a log of what it changes. The log
//! (block changes, block packets) is applied after the islands, in the entities' order; until
//! then the island reads its own block changes from an overlay.

use crate::blocks::{BlockEnv, RegionLevel};
use bytes::Bytes;
use kiln_blocks::{BlockPos, Level};
use kiln_region::CellSet;
use kiln_world::chunk::LightLayer;
use kiln_world::{Blocks, Cell, ChunkPos};

/// A change to the region an island makes, applied after the islands ticked.
pub(crate) type Deferred = Box<dyn FnOnce(&mut RegionLevel) + Send>;

/// One island's view of the blocks and its log.
pub(crate) struct IslandWorld<'l> {
    pub cells: &'l CellSet<Cell>,
    pub env: &'l BlockEnv,
    /// Blocks this island set, as it reads them until the changes are applied.
    pub overlay: crate::FastMap<(i32, i32, i32), u16>,
    pub deferred: Vec<Deferred>,
    /// Block packets (particles), with the position and range they go out to.
    pub packets: Vec<([f64; 3], f64, Bytes)>,
}

impl<'l> IslandWorld<'l> {
    pub fn new(cells: &'l CellSet<Cell>, env: &'l BlockEnv) -> Self {
        IslandWorld { cells, env, overlay: Default::default(), deferred: Vec::new(), packets: Vec::new() }
    }
}

pub(crate) enum World<'a, 'l> {
    Region(&'a mut RegionLevel<'l>),
    Island(IslandWorld<'l>),
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::new(pos.x >> 4, pos.z >> 4)
}

impl<'a, 'l> World<'a, 'l> {
    pub fn env(&self) -> &'l BlockEnv {
        match self {
            World::Region(l) => l.env,
            World::Island(i) => i.env,
        }
    }

    pub fn cells(&self) -> &CellSet<Cell> {
        match self {
            World::Region(l) => l.cells,
            World::Island(i) => i.cells,
        }
    }

    /// The region, for what only the serial entity tick does.
    pub fn region(&mut self) -> Option<&mut RegionLevel<'l>> {
        match self {
            World::Region(l) => Some(l),
            World::Island(_) => None,
        }
    }

    pub fn region_ref(&self) -> Option<&RegionLevel<'l>> {
        match self {
            World::Region(l) => Some(l),
            World::Island(_) => None,
        }
    }

    /// The region of a serial entity level.
    pub fn into_region(self) -> &'a mut RegionLevel<'l> {
        match self {
            World::Region(l) => l,
            World::Island(_) => unreachable!("an island's entity level has no region"),
        }
    }

    pub fn block(&self, pos: BlockPos) -> u16 {
        match self {
            World::Region(l) => l.block(pos),
            World::Island(i) => match i.overlay.get(&(pos.x, pos.y, pos.z)) {
                Some(&s) => s,
                None => i.cells.get_block(pos.x, pos.y, pos.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR),
            },
        }
    }

    pub fn is_loaded(&self, pos: BlockPos) -> bool {
        match self {
            World::Region(l) => l.is_loaded(pos),
            World::Island(i) => i.cells.chunk(chunk_of(pos)).is_some(),
        }
    }

    /// `Level.getRawBrightness`, as [`RegionLevel`] computes it.
    pub fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        match self {
            World::Region(l) => l.raw_brightness(pos, sky_darken),
            World::Island(i) => {
                let top = i.env.min_y + i.env.height;
                let sky = i.cells.light_at(LightLayer::Sky, pos.x, pos.y, pos.z).map_or(if pos.y >= top { 15 } else { 0 }, i32::from);
                let block = i.cells.light_at(LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from);
                (sky - sky_darken).max(block)
            }
        }
    }

    /// Whether any sculk listener could hear game events (never in an island: a region with
    /// listeners ticks its entities serially).
    pub fn listening(&self) -> bool {
        self.region_ref().is_some_and(crate::sculk::listening)
    }

    pub fn push_packet(&mut self, packet: ([f64; 3], f64, Bytes)) {
        match self {
            World::Region(l) => l.out.packets.push(packet),
            World::Island(i) => i.packets.push(packet),
        }
    }

    /// Changes the region now, or (an island) once the islands have ticked.
    pub fn change(&mut self, f: impl FnOnce(&mut RegionLevel) + Send + 'static) {
        match self {
            World::Region(l) => f(l),
            World::Island(i) => i.deferred.push(Box::new(f)),
        }
    }

    /// `Level.setBlock`.
    pub fn set_block(&mut self, pos: BlockPos, state: u16, flags: u32) -> bool {
        match self {
            World::Region(l) => kiln_blocks::set_block(*l, pos, state, flags),
            World::Island(_) => {
                let old = self.block(pos);
                if old == state || !self.is_loaded(pos) {
                    return false;
                }
                let World::Island(i) = self else { unreachable!() };
                i.overlay.insert((pos.x, pos.y, pos.z), state);
                i.deferred.push(Box::new(move |l| {
                    kiln_blocks::set_block(l, pos, state, flags);
                }));
                true
            }
        }
    }

    /// `Level.destroyBlock`.
    pub fn destroy_block(&mut self, pos: BlockPos, drop: bool) -> bool {
        match self {
            World::Region(l) => kiln_blocks::destroy_block(*l, pos, drop, 512),
            World::Island(_) => {
                let old = self.block(pos);
                if kiln_data::blocks_types::is_air(old) || !self.is_loaded(pos) {
                    return false;
                }
                let World::Island(i) = self else { unreachable!() };
                i.overlay.insert((pos.x, pos.y, pos.z), kiln_data::blocks::default_state::AIR);
                i.deferred.push(Box::new(move |l| {
                    kiln_blocks::destroy_block(l, pos, drop, 512);
                }));
                true
            }
        }
    }
}
