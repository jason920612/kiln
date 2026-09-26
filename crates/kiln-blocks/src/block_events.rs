//! Block events (`Level.blockEvent`, `ServerLevel.runBlockEvents`): pistons, note blocks,
//! chests, bells... queued during the tick and run in the block event phase.

use crate::behaviour;
use crate::level::{Effect, Level};
use crate::pos::BlockPos;
use crate::state::BlockId;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockEvent {
    pub pos: BlockPos,
    pub block: BlockId,
    pub a: i32,
    pub b: i32,
}

/// Insertion-ordered set of pending events (vanilla's `ObjectLinkedOpenHashSet`: an event
/// equal to a queued one is dropped).
#[derive(Default)]
pub struct BlockEvents {
    queue: VecDeque<BlockEvent>,
    queued: HashSet<BlockEvent>,
}

impl BlockEvents {
    pub fn push(&mut self, e: BlockEvent) {
        if self.queued.insert(e) {
            self.queue.push_back(e);
        }
    }

    pub fn pop(&mut self) -> Option<BlockEvent> {
        let e = self.queue.pop_front()?;
        self.queued.remove(&e);
        Some(e)
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Drops the events inside the box (inclusive), as structure placement does.
    pub fn clear_area(&mut self, min: BlockPos, max: BlockPos) {
        let inside = |p: BlockPos| (min.x..=max.x).contains(&p.x) && (min.y..=max.y).contains(&p.y) && (min.z..=max.z).contains(&p.z);
        self.queue.retain(|e| !inside(e.pos));
        self.queued.retain(|e| !inside(e.pos));
    }
}

/// `Level.blockEvent`.
pub fn block_event<L: Level + ?Sized>(level: &mut L, pos: BlockPos, block: BlockId, a: i32, b: i32) {
    level.data().block_events.push(BlockEvent { pos, block, a, b });
}

/// `ServerLevel.runBlockEvents`: runs queued events, including ones queued while running;
/// events at positions `can_tick` rejects wait for a later tick.
pub fn run_block_events<L: Level>(level: &mut L, mut can_tick: impl FnMut(BlockPos) -> bool) {
    let mut later = Vec::new();
    while let Some(e) = level.data().block_events.pop() {
        if !can_tick(e.pos) {
            later.push(e);
            continue;
        }
        let state = level.block(e.pos);
        if BlockId::of(state) == e.block && behaviour::trigger_event(level, state, e.pos, e.a, e.b) {
            level.effect(Effect::BlockEvent { pos: e.pos, block: e.block, a: e.a, b: e.b });
        }
    }
    for e in later {
        level.data().block_events.push(e);
    }
}
