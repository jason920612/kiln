//! Pistons: `PistonBaseBlock` (signal check, block events, `moveBlocks`),
//! `PistonStructureResolver`, `PistonHeadBlock`, and the moving blocks
//! (`MovingPistonBlock` with its `PistonMovingBlockEntity`), which live in
//! [`LevelData::pistons`](crate::LevelData) and advance in the level's block-entity phase
//! ([`tick_moving_pistons`]).

use crate::block_events::block_event;
use crate::java_map::JavaHashMap;
use crate::level::{Effect, Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::redstone::has_signal;
use crate::state::{self, BlockId};
use crate::update::{
    destroy_block, neighbor_changed_at, remove_block, set_block, set_block_and_update, update_from_neighbour_shapes,
    update_neighbors_at, update_neighbour_shapes, update_or_destroy,
};
use kiln_data::block_logic::{self as logic, BlockClass, PushReaction};
use kiln_data::block_props;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::collections::HashMap;

/// `PistonStructureResolver.MAX_PUSH_DEPTH`.
pub const MAX_PUSH: usize = 12;

/// `PistonMovingBlockEntity`: a block travelling through a `moving_piston`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingPiston {
    /// The block in motion (for the piston itself: its head when extending, its base when
    /// retracting).
    pub moved: u16,
    /// The piston's facing.
    pub direction: Direction,
    pub extending: bool,
    /// The piston's own head or base rather than a pushed block.
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    /// Game time of the last tick (not saved).
    pub last_ticked: i64,
}

impl MovingPiston {
    pub fn new(moved: u16, direction: Direction, extending: bool, source: bool) -> Self {
        Self { moved, direction, extending, source, progress: 0.0, progress_o: 0.0, last_ticked: 0 }
    }

    /// `getPushDirection` / `getMovementDirection`: where the block is going.
    pub fn push_direction(&self) -> Direction {
        if self.extending { self.direction } else { self.direction.opposite() }
    }

    /// The block entity's saved fields (`saveAdditional`; the caller adds `id`, `x`, `y`,
    /// `z`). `progress` is written from `progressO`, as vanilla does.
    pub fn to_nbt(&self) -> Tag {
        Tag::Compound(vec![
            ("blockState".into(), state_to_nbt(self.moved)),
            ("facing".into(), Tag::Byte(self.direction.index() as i8)),
            ("progress".into(), Tag::Float(self.progress_o)),
            ("extending".into(), Tag::Byte(self.extending as i8)),
            ("source".into(), Tag::Byte(self.source as i8)),
        ])
    }

    /// `loadAdditional`, with vanilla's defaults for missing fields.
    pub fn from_nbt(tag: &Tag) -> Self {
        let moved = tag.get("blockState").and_then(state_from_nbt).unwrap_or(d::AIR);
        let direction = tag.get("facing").and_then(Tag::as_i64).map_or(Direction::Down, |i| Direction::from_index(i.rem_euclid(6) as usize));
        let progress = match tag.get("progress") {
            Some(Tag::Float(f)) => *f,
            _ => 0.0,
        };
        let flag = |k: &str| tag.get(k).and_then(Tag::as_i64).is_some_and(|v| v != 0);
        Self { moved, direction, extending: flag("extending"), source: flag("source"), progress, progress_o: progress, last_ticked: 0 }
    }
}

/// `BlockState.CODEC` form: the block id for a default state, else `{id, properties}`.
pub fn state_to_nbt(s: u16) -> Tag {
    let b = BlockId::of(s).info();
    if s == b.default {
        return Tag::String(b.name.to_string());
    }
    let props = b.properties.iter().map(|p| (p.name.to_string(), Tag::String(state::get(s, p.name).unwrap_or("").to_string())));
    Tag::Compound(vec![("id".to_string(), Tag::String(b.name.to_string())), ("properties".into(), Tag::Compound(props.collect()))])
}

/// Reads a block id, `{id, properties}` or the older `{Name, Properties}`.
pub fn state_from_nbt(tag: &Tag) -> Option<u16> {
    if let Tag::String(name) = tag {
        return BlockId::by_name(name).map(BlockId::default_state);
    }
    let name = tag.get("id").or_else(|| tag.get("Name"))?.as_str()?;
    let mut s = BlockId::by_name(name)?.default_state();
    if let Some(Tag::Compound(props)) = tag.get("properties").or_else(|| tag.get("Properties")) {
        for (k, v) in props {
            if let Some(v) = v.as_str() {
                s = state::set(s, k, v);
            }
        }
    }
    Some(s)
}

/// The level's moving-piston block entities and their place in the block-entity ticker
/// list (`Level.blockEntityTickers` with `LevelChunk`'s rebindable wrappers): a piston
/// placed where one is still registered takes over its wrapper, and so its position in
/// the list; one placed while pistons are ticking starts ticking next tick.
#[derive(Default)]
pub struct MovingPistons {
    entities: HashMap<BlockPos, MovingPiston>,
    /// Wrapper slots: the position each wrapper ticks, `None` once unbound.
    slots: Vec<Option<BlockPos>>,
    free: Vec<usize>,
    order: Vec<usize>,
    pending: Vec<usize>,
    /// `LevelChunk.tickersInLevel`.
    at: HashMap<BlockPos, usize>,
    ticking: bool,
}

impl MovingPistons {
    pub fn get(&self, pos: BlockPos) -> Option<&MovingPiston> {
        self.entities.get(&pos)
    }

    pub fn get_mut(&mut self, pos: BlockPos) -> Option<&mut MovingPiston> {
        self.entities.get_mut(&pos)
    }

    /// `LevelChunk.addAndRegisterBlockEntity` (also for pistons loaded from a chunk, in the
    /// chunk's block-entity order).
    pub fn insert(&mut self, pos: BlockPos, piston: MovingPiston) {
        self.entities.insert(pos, piston);
        if self.at.contains_key(&pos) {
            return;
        }
        let slot = match self.free.pop() {
            Some(i) => {
                self.slots[i] = Some(pos);
                i
            }
            None => {
                self.slots.push(Some(pos));
                self.slots.len() - 1
            }
        };
        self.at.insert(pos, slot);
        if self.ticking { self.pending.push(slot) } else { self.order.push(slot) }
    }

    /// `LevelChunk.removeBlockEntity`.
    pub fn remove(&mut self, pos: BlockPos) -> Option<MovingPiston> {
        if let Some(slot) = self.at.remove(&pos) {
            self.slots[slot] = None;
        }
        self.entities.remove(&pos)
    }

    /// Removes every moving piston, in ticker order (to move them to another level; insert
    /// them there in the same order).
    pub fn take_all(&mut self) -> Vec<(BlockPos, MovingPiston)> {
        let mut out = Vec::with_capacity(self.entities.len());
        for &slot in self.order.iter().chain(&self.pending) {
            if let Some(pos) = self.slots[slot]
                && let Some(m) = self.entities.remove(&pos)
            {
                out.push((pos, m));
            }
        }
        let mut rest: Vec<_> = self.entities.drain().collect();
        rest.sort_by_key(|(p, _)| *p);
        out.extend(rest);
        *self = Self::default();
        out
    }

    /// All moving pistons (for saving), in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (BlockPos, &MovingPiston)> {
        self.entities.iter().map(|(p, m)| (*p, m))
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

/// The level's block-entity phase for moving pistons (`Level.tickBlockEntities`): each
/// registered piston in ticker order, if `can_tick` accepts its position.
pub fn tick_moving_pistons<L: Level>(level: &mut L, mut can_tick: impl FnMut(BlockPos) -> bool) {
    let p = &mut level.data().pistons;
    p.ticking = true;
    let pending = std::mem::take(&mut p.pending);
    p.order.extend(pending);
    // Nothing else touches `order` while ticking (new tickers go to `pending`).
    let (mut read, mut write) = (0, 0);
    loop {
        let p = &mut level.data().pistons;
        let Some(&slot) = p.order.get(read) else { break };
        read += 1;
        let Some(pos) = p.slots[slot] else {
            p.free.push(slot);
            continue;
        };
        p.order[write] = slot;
        write += 1;
        if can_tick(pos) && state::is(level.block(pos), d::MOVING_PISTON) {
            tick_entity(level, pos);
        }
    }
    let p = &mut level.data().pistons;
    p.order.truncate(write);
    p.ticking = false;
}

/// `PistonMovingBlockEntity.tick`.
fn tick_entity<L: Level>(level: &mut L, pos: BlockPos) {
    let time = level.game_time();
    let Some(be) = level.data().pistons.get_mut(pos) else { return };
    be.last_ticked = time;
    be.progress_o = be.progress;
    let be = *be;
    if be.progress_o >= 1.0 {
        level.data().pistons.remove(pos);
        if state::is(level.block(pos), d::MOVING_PISTON) {
            let s = update_from_neighbour_shapes(level, be.moved, pos);
            if is_air(s) {
                set_block(level, pos, be.moved, 340);
                update_or_destroy(level, be.moved, s, pos, flags::ALL, flags::LIMIT);
            } else {
                let s = if state::get_bool(s, "waterlogged") { state::set_bool(s, "waterlogged", false) } else { s };
                set_block(level, pos, s, 67);
                neighbor_changed_at(level, pos, BlockId::of(s));
            }
        }
        return;
    }
    let next = be.progress + 0.5;
    level.effect(Effect::PistonMove { pos, piston: be, progress: next });
    if let Some(be) = level.data().pistons.get_mut(pos) {
        be.progress = next.min(1.0);
    }
}

/// `PistonMovingBlockEntity.finalTick`: jumps the move to its end.
pub fn final_tick<L: Level>(level: &mut L, pos: BlockPos) {
    let Some(be) = level.data().pistons.get(pos).copied() else { return };
    if be.progress_o >= 1.0 {
        return;
    }
    level.data().pistons.remove(pos);
    if state::is(level.block(pos), d::MOVING_PISTON) {
        let s = if be.source { d::AIR } else { update_from_neighbour_shapes(level, be.moved, pos) };
        set_block_and_update(level, pos, s);
        neighbor_changed_at(level, pos, BlockId::of(s));
    }
}

/// `Level.setBlockEntity` for a new moving piston (ignored unless `pos` holds a
/// `moving_piston`).
fn set_moving<L: Level>(level: &mut L, pos: BlockPos, piston: MovingPiston) {
    if level.in_bounds(pos) && state::is(level.block(pos), d::MOVING_PISTON) {
        level.data().pistons.insert(pos, piston);
    }
}

fn is_base(s: u16) -> bool {
    logic::block_class(s) == BlockClass::PistonBaseBlock
}

fn facing(s: u16) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

fn piston_type(sticky: bool) -> &'static str {
    if sticky { "sticky" } else { "normal" }
}

/// `PistonBaseBlock.isPushable`.
pub fn is_pushable<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction, allow_destroy: bool, piston_facing: Direction) -> bool {
    if !level.in_bounds(pos) {
        return false;
    }
    if is_air(s) {
        return true;
    }
    if (dir == Direction::Down || dir == Direction::Up) && !level.in_bounds(pos.relative(dir)) {
        return false;
    }
    if is_base(s) {
        return !state::get_bool(s, "extended");
    }
    if block_props::hardness(s) == -1.0 {
        return false;
    }
    match logic::push_reaction(s) {
        PushReaction::Immoveable => false,
        PushReaction::Popped => allow_destroy,
        PushReaction::Push => dir == piston_facing,
        _ => !block_props::has_block_entity(s),
    }
}

fn is_sticky(s: u16) -> bool {
    state::is(s, d::SLIME_BLOCK) || state::is(s, d::HONEY_BLOCK)
}

fn can_stick(a: u16, b: u16) -> bool {
    if (state::is(a, d::HONEY_BLOCK) && state::is(b, d::SLIME_BLOCK)) || (state::is(a, d::SLIME_BLOCK) && state::is(b, d::HONEY_BLOCK)) {
        return false;
    }
    is_sticky(a) || is_sticky(b)
}

/// `PistonStructureResolver`: the blocks a piston would move and break.
pub struct Resolver {
    piston: BlockPos,
    extending: bool,
    start: BlockPos,
    push_dir: Direction,
    piston_dir: Direction,
    pub to_push: Vec<BlockPos>,
    pub to_destroy: Vec<BlockPos>,
}

impl Resolver {
    pub fn new(piston: BlockPos, dir: Direction, extending: bool) -> Self {
        let (push_dir, start) = if extending { (dir, piston.relative(dir)) } else { (dir.opposite(), piston.relative_by(dir, 2)) };
        Self { piston, extending, start, push_dir, piston_dir: dir, to_push: Vec::new(), to_destroy: Vec::new() }
    }

    pub fn push_direction(&self) -> Direction {
        self.push_dir
    }

    pub fn resolve<L: Level + ?Sized>(&mut self, level: &L) -> bool {
        self.to_push.clear();
        self.to_destroy.clear();
        let s = level.block(self.start);
        if !is_pushable(level, s, self.start, self.push_dir, false, self.piston_dir) {
            if self.extending && logic::push_reaction(s) == PushReaction::Popped {
                self.to_destroy.push(self.start);
                return true;
            }
            return false;
        }
        if !self.add_block_line(level, self.start, self.push_dir) {
            return false;
        }
        let mut i = 0;
        while i < self.to_push.len() {
            let p = self.to_push[i];
            if is_sticky(level.block(p)) && !self.add_branching_blocks(level, p) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn add_block_line<L: Level + ?Sized>(&mut self, level: &L, origin: BlockPos, dir: Direction) -> bool {
        let mut s = level.block(origin);
        if is_air(s) || !is_pushable(level, s, origin, self.push_dir, false, dir) || origin == self.piston || self.to_push.contains(&origin) {
            return true;
        }
        let back = self.push_dir.opposite();
        let mut n = 1;
        if n + self.to_push.len() > MAX_PUSH {
            return false;
        }
        while is_sticky(s) {
            let p = origin.relative_by(back, n as i32);
            let prev = s;
            s = level.block(p);
            if is_air(s) || !can_stick(prev, s) || !is_pushable(level, s, p, self.push_dir, false, back) || p == self.piston {
                break;
            }
            n += 1;
            if n + self.to_push.len() > MAX_PUSH {
                return false;
            }
        }
        let mut added = 0;
        for k in (0..n).rev() {
            self.to_push.push(origin.relative_by(back, k as i32));
            added += 1;
        }
        let mut k = 1;
        loop {
            let p = origin.relative_by(self.push_dir, k);
            if let Some(idx) = self.to_push.iter().position(|&q| q == p) {
                self.reorder_at_collision(added, idx);
                for l in 0..=idx + added {
                    let q = self.to_push[l];
                    if is_sticky(level.block(q)) && !self.add_branching_blocks(level, q) {
                        return false;
                    }
                }
                return true;
            }
            let s = level.block(p);
            if is_air(s) {
                return true;
            }
            if !is_pushable(level, s, p, self.push_dir, true, self.push_dir) || p == self.piston {
                return false;
            }
            if logic::push_reaction(s) == PushReaction::Popped {
                self.to_destroy.push(p);
                return true;
            }
            if self.to_push.len() >= MAX_PUSH {
                return false;
            }
            self.to_push.push(p);
            added += 1;
            k += 1;
        }
    }

    /// `reorderListAtCollision`: moves the `added` newest entries in front of entry `at`.
    fn reorder_at_collision(&mut self, added: usize, at: usize) {
        let len = self.to_push.len();
        let mut v = Vec::with_capacity(len);
        v.extend_from_slice(&self.to_push[..at]);
        v.extend_from_slice(&self.to_push[len - added..]);
        v.extend_from_slice(&self.to_push[at..len - added]);
        self.to_push = v;
    }

    fn add_branching_blocks<L: Level + ?Sized>(&mut self, level: &L, pos: BlockPos) -> bool {
        let s = level.block(pos);
        for dir in Direction::ALL {
            if dir.axis() == self.push_dir.axis() {
                continue;
            }
            let n = pos.relative(dir);
            if can_stick(level.block(n), s) && !self.add_block_line(level, n, dir) {
                return false;
            }
        }
        true
    }
}

/// `PistonBaseBlock.getNeighborSignal`: powered from any side but the front, or (quasi-
/// connectivity) at the block above from any side but below.
fn neighbor_signal<L: Level + ?Sized>(level: &L, pos: BlockPos, front: Direction) -> bool {
    if Direction::ALL.iter().any(|&dir| dir != front && has_signal(level, pos.relative(dir), dir)) {
        return true;
    }
    if has_signal(level, pos, Direction::Down) {
        return true;
    }
    let up = pos.above();
    Direction::ALL.iter().any(|&dir| dir != Direction::Down && has_signal(level, up.relative(dir), dir))
}

/// `PistonBaseBlock.checkIfExtend`: queues the extend (0), retract (1) or instant-retract
/// (2) block event.
pub fn check_if_extend<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let dir = facing(s);
    let signal = neighbor_signal(level, pos, dir);
    let extended = state::get_bool(s, "extended");
    if signal && !extended {
        if Resolver::new(pos, dir, true).resolve(level) {
            block_event(level, pos, BlockId::of(s), 0, dir.index() as i32);
        }
    } else if !signal && extended {
        let p2 = pos.relative_by(dir, 2);
        let s2 = level.block(p2);
        let mut kind = 1;
        if state::is(s2, d::MOVING_PISTON) && facing(s2) == dir {
            let time = level.game_time();
            let handling = level.data().handling_tick;
            if let Some(be) = level.data().pistons.get(p2)
                && be.extending
                && (be.progress_o < 0.5 || time == be.last_ticked || handling)
            {
                kind = 2;
            }
        }
        block_event(level, pos, BlockId::of(s), kind, dir.index() as i32);
    }
}

/// `PistonBaseBlock.onPlace`.
pub fn on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if !state::same_block(old, s) {
        check_if_extend(level, s, pos);
    }
}

/// `PistonBaseBlock.triggerEvent`.
pub fn trigger_event<L: Level>(level: &mut L, s: u16, pos: BlockPos, kind: i32, data: i32) -> bool {
    let dir = facing(s);
    let extended = state::set_bool(s, "extended", true);
    let signal = neighbor_signal(level, pos, dir);
    if signal && (kind == 1 || kind == 2) {
        set_block(level, pos, extended, flags::CLIENTS);
        return false;
    }
    if !signal && kind == 0 {
        return false;
    }
    let sticky = state::is(s, d::STICKY_PISTON);
    if kind == 0 {
        if !move_blocks(level, pos, dir, true, sticky) {
            return false;
        }
        set_block(level, pos, extended, 67);
        let pitch = level.random().next_float() * 0.25 + 0.6;
        level.effect(Effect::Sound { pos, sound: "minecraft:block.piston.extend", volume: 0.5, pitch });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
    } else if kind == 1 || kind == 2 {
        let front = pos.relative(dir);
        final_tick(level, front);
        let moving = state::set(state::set_dir(d::MOVING_PISTON, "facing", dir), "type", piston_type(sticky));
        set_block(level, pos, moving, 276);
        let base = state::set_dir(BlockId::of(s).default_state(), "facing", Direction::from_index(((data & 7) % 6) as usize));
        set_moving(level, pos, MovingPiston::new(base, dir, false, true));
        update_neighbors_at(level, pos, BlockId::of(moving));
        update_neighbour_shapes(level, moving, pos, flags::CLIENTS, flags::LIMIT);
        if sticky {
            let p2 = pos.relative_by(dir, 2);
            let s2 = level.block(p2);
            let mut pulled = false;
            if state::is(s2, d::MOVING_PISTON)
                && let Some(be) = level.data().pistons.get(p2)
                && be.direction == dir
                && be.extending
            {
                final_tick(level, p2);
                pulled = true;
            }
            if !pulled {
                let pull = kind == 1
                    && !is_air(s2)
                    && is_pushable(level, s2, p2, dir.opposite(), false, dir)
                    && (logic::push_reaction(s2) == PushReaction::PushPull || is_base(s2));
                if pull {
                    move_blocks(level, pos, dir, false, sticky);
                } else {
                    remove_block(level, front, false);
                }
            }
        } else {
            remove_block(level, front, false);
        }
        let pitch = level.random().next_float() * 0.15 + 0.6;
        level.effect(Effect::Sound { pos, sound: "minecraft:block.piston.contract", volume: 0.5, pitch });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_deactivate" });
    }
    true
}

/// `PistonBaseBlock.moveBlocks`: turns every moved block into a `moving_piston`, breaks
/// the popped ones, then updates the neighbourhood in vanilla's order (including the
/// `HashMap` walk over vacated positions).
fn move_blocks<L: Level>(level: &mut L, pos: BlockPos, dir: Direction, extending: bool, sticky: bool) -> bool {
    let front = pos.relative(dir);
    if !extending && state::is(level.block(front), d::PISTON_HEAD) {
        set_block(level, front, d::AIR, 276);
    }
    let mut resolver = Resolver::new(pos, dir, extending);
    if !resolver.resolve(level) {
        return false;
    }
    let mut vacated = JavaHashMap::new();
    let to_push = std::mem::take(&mut resolver.to_push);
    let to_destroy = std::mem::take(&mut resolver.to_destroy);
    let mut pushed_states = Vec::with_capacity(to_push.len());
    for &p in &to_push {
        let s = level.block(p);
        pushed_states.push(s);
        vacated.put(p, s);
    }
    let mut affected = Vec::with_capacity(to_push.len() + to_destroy.len());
    let push_dir = if extending { dir } else { dir.opposite() };
    for &p in to_destroy.iter().rev() {
        let s = level.block(p);
        level.effect(Effect::Drop { pos: p, state: s });
        set_block(level, p, d::AIR, 18);
        level.effect(Effect::GameEvent { pos: p, event: "minecraft:block_destroy" });
        affected.push(s);
    }
    let moving = state::set_dir(d::MOVING_PISTON, "facing", dir);
    for (i, &p) in to_push.iter().enumerate().rev() {
        let s = level.block(p);
        let dest = p.relative(push_dir);
        vacated.remove(dest);
        set_block(level, dest, moving, 324);
        set_moving(level, dest, MovingPiston::new(pushed_states[i], dir, extending, false));
        affected.push(s);
    }
    if extending {
        let head = state::set(state::set_dir(d::PISTON_HEAD, "facing", dir), "type", piston_type(sticky));
        let moving = state::set(moving, "type", piston_type(sticky));
        vacated.remove(front);
        set_block(level, front, moving, 324);
        set_moving(level, front, MovingPiston::new(head, dir, true, true));
    }
    let keys: Vec<BlockPos> = vacated.iter().map(|(p, _)| p).collect();
    for &p in &keys {
        set_block(level, p, d::AIR, 82);
    }
    let entries: Vec<(BlockPos, u16)> = vacated.iter().map(|(p, &s)| (p, s)).collect();
    for (p, s) in entries {
        super::update_indirect_neighbour_shapes(level, s, p, flags::CLIENTS, flags::LIMIT);
        update_neighbour_shapes(level, d::AIR, p, flags::CLIENTS, flags::LIMIT);
        super::update_indirect_neighbour_shapes(level, d::AIR, p, flags::CLIENTS, flags::LIMIT);
    }
    let mut k = 0;
    for &p in to_destroy.iter().rev() {
        let s = affected[k];
        k += 1;
        super::affect_neighbors_after_removal(level, s, p, false);
        super::update_indirect_neighbour_shapes(level, s, p, flags::CLIENTS, flags::LIMIT);
        update_neighbors_at(level, p, BlockId::of(s));
    }
    for &p in to_push.iter().rev() {
        let s = affected[k];
        k += 1;
        update_neighbors_at(level, p, BlockId::of(s));
    }
    if extending {
        update_neighbors_at(level, front, BlockId::of(d::PISTON_HEAD));
    }
    true
}

/// `PistonHeadBlock.isFittingBase`.
fn fitting_base(head: u16, base: u16) -> bool {
    let want = if state::get(head, "type") == Some("sticky") { d::STICKY_PISTON } else { d::PISTON };
    state::is(base, want) && state::get_bool(base, "extended") && facing(base) == facing(head)
}

/// `PistonHeadBlock.canSurvive`.
pub fn head_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let base = level.block(pos.relative(facing(s).opposite()));
    fitting_base(s, base) || (state::is(base, d::MOVING_PISTON) && facing(base) == facing(s))
}

/// `PistonHeadBlock.updateShape`: gone once its base is.
pub fn head_update_shape<L: Level>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if dir.opposite() == facing(s) && !head_can_survive(level, s, pos) { d::AIR } else { s }
}

/// `PistonHeadBlock.neighborChanged`: passed on to the base.
pub fn head_neighbor_changed<L: Level>(level: &mut L, s: u16, pos: BlockPos, source: BlockId) {
    if head_can_survive(level, s, pos) {
        neighbor_changed_at(level, pos.relative(facing(s).opposite()), source);
    }
}

/// `PistonHeadBlock.affectNeighborsAfterRemoval`: breaks the base with it.
pub fn head_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let base = pos.relative(facing(s).opposite());
    if fitting_base(s, level.block(base)) {
        destroy_block(level, base, true, flags::LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::parse_state;
    use crate::test_level::TestLevel;

    fn level() -> TestLevel {
        let mut l = TestLevel::flat(-64, 384, &[d::BEDROCK, d::STONE, d::STONE, d::STONE]);
        l.load_chunks((-2, -2), (2, 2));
        l
    }

    #[test]
    fn extends_and_retracts_through_block_entities() {
        let mut l = level();
        let base = BlockPos::new(0, -60, 0);
        set_block(&mut l, base, parse_state("minecraft:piston[facing=east]").unwrap(), flags::ALL);
        set_block(&mut l, base.relative(Direction::East), d::STONE, flags::ALL);
        set_block(&mut l, base.relative(Direction::West), d::REDSTONE_BLOCK, flags::ALL);
        l.tick(0, &[]);
        // Block event ran: the head and the stone are moving.
        assert!(state::is(l.block(base.offset(1, 0, 0)), d::MOVING_PISTON));
        assert!(state::is(l.block(base.offset(2, 0, 0)), d::MOVING_PISTON));
        assert!(state::get_bool(l.block(base), "extended"));
        l.tick(0, &[]);
        l.tick(0, &[]);
        assert!(state::is(l.block(base.offset(1, 0, 0)), d::PISTON_HEAD));
        assert_eq!(l.block(base.offset(2, 0, 0)), d::STONE);
        set_block(&mut l, base.relative(Direction::West), d::AIR, flags::ALL);
        for _ in 0..3 {
            l.tick(0, &[]);
        }
        assert!(!state::get_bool(l.block(base), "extended"));
        assert_eq!(l.block(base.offset(1, 0, 0)), d::AIR);
        assert_eq!(l.block(base.offset(2, 0, 0)), d::STONE);
    }

    #[test]
    fn ticker_order_follows_wrappers() {
        let (a, b, c) = (BlockPos::new(0, 0, 0), BlockPos::new(1, 0, 0), BlockPos::new(2, 0, 0));
        let m = MovingPiston::new(d::STONE, Direction::East, true, false);
        let mut p = MovingPistons::default();
        p.insert(a, m);
        p.insert(b, m);
        p.insert(c, m);
        // Replacing a registered piston keeps its wrapper; removing one frees its place.
        p.insert(a, MovingPiston { progress: 0.5, ..m });
        p.remove(b);
        p.insert(b, m);
        let order: Vec<BlockPos> = p.order.iter().filter_map(|&s| p.slots[s]).collect();
        assert_eq!(order, [a, c, b]);
        assert_eq!(p.get(a).unwrap().progress, 0.5);
        let back = MovingPiston::from_nbt(&MovingPiston { progress_o: 0.5, ..m }.to_nbt());
        assert_eq!((back.moved, back.direction, back.progress, back.extending, back.source), (d::STONE, Direction::East, 0.5, true, false));
    }

    #[test]
    fn slime_structures_and_push_limit() {
        let mut l = level();
        let base = BlockPos::new(0, -50, 0);
        for x in 1..=12 {
            set_block(&mut l, base.offset(x, 0, 0), d::STONE, flags::ALL);
        }
        let mut r = Resolver::new(base, Direction::East, true);
        assert!(r.resolve(&l) && r.to_push.len() == 12);
        set_block(&mut l, base.offset(13, 0, 0), d::STONE, flags::ALL);
        assert!(!Resolver::new(base, Direction::East, true).resolve(&l));
        // A slime block drags the stone beside it along, but not honey.
        let mut l = level();
        set_block(&mut l, base.offset(1, 0, 0), d::SLIME_BLOCK, flags::ALL);
        set_block(&mut l, base.offset(1, 0, 1), d::STONE, flags::ALL);
        set_block(&mut l, base.offset(1, 1, 0), d::HONEY_BLOCK, flags::ALL);
        let mut r = Resolver::new(base, Direction::East, true);
        assert!(r.resolve(&l));
        assert_eq!(r.to_push, [base.offset(1, 0, 0), base.offset(1, 0, 1)]);
        // On the ground, the slime would drag the whole floor: too many blocks.
        let ground = BlockPos::new(0, -60, 0);
        set_block(&mut l, ground.offset(1, 0, 0), d::SLIME_BLOCK, flags::ALL);
        assert!(!Resolver::new(ground, Direction::East, true).resolve(&l));
        // Obsidian stops the line.
        set_block(&mut l, base.offset(2, 0, 0), d::OBSIDIAN, flags::ALL);
        assert!(!Resolver::new(base, Direction::East, true).resolve(&l));
    }
}
