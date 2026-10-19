//! Block edits that commands make, gathered while they run (`Host::begin_block_edits`): what
//! vanilla does once per change only later and for all of them together waits for the end of
//! the outermost command (a console line, a player's command, a function run, a command block).
//!
//! - The clients get the changed blocks then, a Section Blocks Update per section (or a Block
//!   Update for a lone change) with the block entities' data after it: vanilla's `ChunkHolder`
//!   collects a tick's changes per section and sends them in `broadcastChanges`.
//! - The light of the changed blocks is worked out then, all at once: vanilla's light engine
//!   queues `checkBlock` and runs the queue after the tick's work, so light read meanwhile is
//!   the light from before the changes there too.
//! - A block command's block work sees one environment ([`crate::blocks::BlockEnv`]: rules,
//!   time, players), made once: nothing a block command does changes it.

use crate::blocks::{self, BlockEnv};
use crate::{DimId, Sim};
use kiln_world::{Blocks, ChunkPos};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct EditBatch {
    /// Open batches: block commands and the runs around them.
    depth: u32,
    /// Open block commands.
    commands: u32,
    /// Each level's environment while a block command is open.
    envs: Vec<Option<BlockEnv>>,
    /// Positions set with `UPDATE_CLIENTS`, in order (may repeat), by level.
    changed: Vec<Vec<[i32; 3]>>,
    /// Positions whose new state may change light that is not worked out yet, by level.
    unlit: Vec<Vec<[i32; 3]>>,
}

impl EditBatch {
    pub(crate) fn open(&self) -> bool {
        self.depth > 0
    }

    /// Room for every level's lists.
    fn levels(&mut self, dim: DimId) {
        if self.envs.len() <= dim {
            self.envs.resize_with(dim + 1, || None);
            self.changed.resize_with(dim + 1, Vec::new);
            self.unlit.resize_with(dim + 1, Vec::new);
        }
    }

    pub(crate) fn add(&mut self, dim: DimId, changed: &mut Vec<[i32; 3]>, unlit: Option<Vec<[i32; 3]>>) {
        self.levels(dim);
        self.changed[dim].append(changed);
        if let Some(mut u) = unlit {
            self.unlit[dim].append(&mut u);
        }
    }
}

impl Sim {
    /// Opens a batch of edits; `command` for a block command (its environment then stays).
    pub(crate) fn begin_edits(&mut self, command: bool) {
        let e = &mut self.world.edits;
        e.depth += 1;
        e.commands += u32::from(command);
    }

    /// Closes what [`Self::begin_edits`] opened; the last one sends and lights the edits.
    pub(crate) fn end_edits(&mut self, command: bool) {
        let e = &mut self.world.edits;
        if command {
            e.commands -= 1;
            if e.commands == 0 {
                e.envs = Default::default();
            }
        }
        e.depth -= 1;
        if e.depth == 0 {
            self.flush_edits();
        }
    }

    /// Runs `f` as one batch of edits (a whole command run).
    pub(crate) fn editing<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.begin_edits(false);
        let r = f(self);
        self.end_edits(false);
        r
    }

    /// The environment of block work in level `dim`: a block command's, or a new one.
    pub(crate) fn take_block_env(&mut self, dim: DimId) -> BlockEnv {
        self.world.edits.levels(dim);
        match self.world.edits.envs[dim].take() {
            Some(env) => env,
            None => self.block_env(dim),
        }
    }

    /// Gives back what [`Self::take_block_env`] lent, if a block command keeps it.
    pub(crate) fn return_block_env(&mut self, dim: DimId, env: BlockEnv) {
        if self.world.edits.commands > 0 {
            self.world.edits.levels(dim);
            self.world.edits.envs[dim] = Some(env);
        }
    }

    /// The light of the batch's changes, then the changes to the players who have their chunks.
    fn flush_edits(&mut self) {
        for dim in 0..self.dims.len() {
            self.world.edits.levels(dim);
            let unlit = std::mem::take(&mut self.world.edits.unlit[dim]);
            let changed = std::mem::take(&mut self.world.edits.changed[dim]);
            if unlit.is_empty() && changed.is_empty() {
                continue;
            }
            let d = &mut self.dims[dim];
            // By the region holding them (light does not cross into other regions' chunks here
            // either; a position no region holds any more has nothing to light or send).
            let by_region = |positions: Vec<[i32; 3]>| {
                let mut by: BTreeMap<kiln_region::RegionId, Vec<[i32; 3]>> = BTreeMap::new();
                for p in positions {
                    if let Some(r) = d.regions.owner(ChunkPos::of_block(p[0], p[2]).cell()) {
                        by.entry(r).or_default().push(p);
                    }
                }
                by
            };
            let unlit = by_region(unlit);
            let changed = by_region(changed);
            for (r, positions) in unlit {
                if let Some(region) = d.regions.get_mut(r) {
                    region.cells_and_part_mut().0.relight(&positions);
                }
            }
            let mut everyone: Vec<&mut crate::Player> = self.players.values_mut().filter(|p| p.dim == dim).collect();
            for (r, positions) in changed {
                if let Some(region) = d.regions.get(r) {
                    blocks::send_changes(region.cells(), &positions, &mut everyone);
                }
            }
        }
    }
}
