//! Scheduled block and fluid ticks (`LevelTicks`, `LevelChunkTicks`, `ScheduledTick`,
//! `SavedTick`) and their chunk NBT form (`block_ticks`, `fluid_ticks`).
//!
//! Order within a tick is vanilla's: containers are merged by (priority, sub-tick) of their
//! next tick, each container drains in (trigger tick, priority, sub-tick) order, and at most
//! `max` ticks run per game tick. Sub-tick numbers come from one counter per level (per region
//! in Kiln), so ties only arise between chunks reloaded in the same tick (both number their
//! loaded ticks -n..-1); vanilla breaks those by hash-map iteration order, Kiln by chunk
//! position.

use crate::pos::BlockPos;
use kiln_proto::nbt::Tag;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// `TickPriority`, ordered from most to least urgent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}

impl TickPriority {
    pub fn value(self) -> i32 {
        self as i32
    }

    /// `TickPriority.byValue`: out-of-range values clamp to the extremes.
    pub fn from_value(v: i32) -> Self {
        match v {
            i32::MIN..=-3 => Self::ExtremelyHigh,
            -2 => Self::VeryHigh,
            -1 => Self::High,
            0 => Self::Normal,
            1 => Self::Low,
            2 => Self::VeryLow,
            _ => Self::ExtremelyLow,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScheduledTick<T> {
    pub kind: T,
    pub pos: BlockPos,
    pub trigger: i64,
    pub priority: TickPriority,
    pub sub: i64,
}

impl<T> ScheduledTick<T> {
    /// `DRAIN_ORDER`.
    fn drain_key(&self) -> (i64, TickPriority, i64) {
        (self.trigger, self.priority, self.sub)
    }

    /// `INTRA_TICK_DRAIN_ORDER`.
    fn intra_key(&self) -> (TickPriority, i64) {
        (self.priority, self.sub)
    }
}

/// A tick as saved in chunk NBT: delay relative to the save time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedTick<T> {
    pub kind: T,
    pub pos: BlockPos,
    pub delay: i32,
    pub priority: TickPriority,
}

struct Queued<T>(ScheduledTick<T>);

impl<T> PartialEq for Queued<T> {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl<T> Eq for Queued<T> {}
impl<T> PartialOrd for Queued<T> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Queued<T> {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.drain_key().cmp(&o.0.drain_key()).then_with(|| self.0.pos.cmp(&o.0.pos))
    }
}

/// One chunk's scheduled ticks (`LevelChunkTicks`).
pub struct ChunkTicks<T> {
    queue: BinaryHeap<Reverse<Queued<T>>>,
    /// Loaded ticks not yet given trigger times (`pendingTicks`).
    pending: Option<Vec<SavedTick<T>>>,
    scheduled: HashSet<(T, BlockPos)>,
}

impl<T: Copy + Eq + Hash> Default for ChunkTicks<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Eq + Hash> ChunkTicks<T> {
    pub fn new() -> Self {
        Self { queue: BinaryHeap::new(), pending: None, scheduled: HashSet::new() }
    }

    /// A container holding ticks loaded from chunk NBT; they get trigger times on `unpack`.
    pub fn from_saved(saved: Vec<SavedTick<T>>) -> Self {
        let scheduled = saved.iter().map(|t| (t.kind, t.pos)).collect();
        Self { queue: BinaryHeap::new(), pending: Some(saved), scheduled }
    }

    pub fn peek(&self) -> Option<&ScheduledTick<T>> {
        self.queue.peek().map(|r| &r.0.0)
    }

    pub fn poll(&mut self) -> Option<ScheduledTick<T>> {
        let t = self.queue.pop()?.0.0;
        self.scheduled.remove(&(t.kind, t.pos));
        Some(t)
    }

    /// Adds the tick unless one of the same kind is already scheduled at its position.
    /// Returns whether it was added.
    pub fn schedule(&mut self, tick: ScheduledTick<T>) -> bool {
        if !self.scheduled.insert((tick.kind, tick.pos)) {
            return false;
        }
        self.queue.push(Reverse(Queued(tick)));
        true
    }

    pub fn has_scheduled_tick(&self, pos: BlockPos, kind: T) -> bool {
        self.scheduled.contains(&(kind, pos))
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&ScheduledTick<T>) -> bool) {
        let ticks = std::mem::take(&mut self.queue).into_vec();
        for Reverse(Queued(t)) in ticks {
            if keep(&t) {
                self.queue.push(Reverse(Queued(t)));
            } else {
                self.scheduled.remove(&(t.kind, t.pos));
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ScheduledTick<T>> {
        self.queue.iter().map(|r| &r.0.0)
    }

    pub fn count(&self) -> usize {
        self.queue.len() + self.pending.as_ref().map_or(0, Vec::len)
    }

    /// `pack`: still-pending loaded ticks first, then queued ones by sub-tick order.
    pub fn pack(&self, game_time: i64) -> Vec<SavedTick<T>> {
        let mut out: Vec<SavedTick<T>> = self.pending.clone().unwrap_or_default();
        let mut queued: Vec<&ScheduledTick<T>> = self.iter().collect();
        queued.sort_by_key(|t| t.sub);
        out.extend(queued.into_iter().map(|t| SavedTick {
            kind: t.kind,
            pos: t.pos,
            delay: (t.trigger - game_time) as i32,
            priority: t.priority,
        }));
        out
    }

    /// Moves every queued tick `delta` ticks later (a region that fell behind rejoins the
    /// server's clock: what was due in n of its ticks stays due in n ticks). Loaded ticks not
    /// unpacked yet are relative and stay as they are; the order is unchanged.
    pub fn shift(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }
        let ticks = std::mem::take(&mut self.queue).into_vec();
        self.queue = ticks
            .into_iter()
            .map(|Reverse(Queued(mut t))| {
                t.trigger += delta;
                Reverse(Queued(t))
            })
            .collect();
    }

    /// `unpack`: loaded ticks trigger `delay` ticks after `game_time`, numbered -n..-1.
    pub fn unpack(&mut self, game_time: i64) {
        let Some(pending) = self.pending.take() else { return };
        let first = -(pending.len() as i64);
        for (sub, t) in (first..).zip(pending) {
            self.queue.push(Reverse(Queued(ScheduledTick {
                kind: t.kind,
                pos: t.pos,
                trigger: game_time + t.delay as i64,
                priority: t.priority,
                sub,
            })));
        }
    }
}

/// Chunk coordinates keying the per-chunk containers.
pub type ChunkKey = (i32, i32);

struct Head<T>(ScheduledTick<T>, ChunkKey);

impl<T> PartialEq for Head<T> {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl<T> Eq for Head<T> {}
impl<T> PartialOrd for Head<T> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Head<T> {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.intra_key().cmp(&o.0.intra_key()).then_with(|| self.1.cmp(&o.1))
    }
}

/// All scheduled ticks of one kind in a level (`LevelTicks`).
pub struct LevelTicks<T> {
    containers: HashMap<ChunkKey, ChunkTicks<T>>,
    next_tick: HashMap<ChunkKey, i64>,
    to_run: VecDeque<ScheduledTick<T>>,
    to_run_set: HashSet<(T, BlockPos)>,
}

impl<T: Copy + Eq + Hash> Default for LevelTicks<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Eq + Hash> LevelTicks<T> {
    pub fn new() -> Self {
        Self { containers: HashMap::new(), next_tick: HashMap::new(), to_run: VecDeque::new(), to_run_set: HashSet::new() }
    }

    pub fn add_container(&mut self, chunk: ChunkKey, ticks: ChunkTicks<T>) {
        if let Some(t) = ticks.peek() {
            self.next_tick.insert(chunk, t.trigger);
        }
        self.containers.insert(chunk, ticks);
    }

    pub fn remove_container(&mut self, chunk: ChunkKey) -> Option<ChunkTicks<T>> {
        self.next_tick.remove(&chunk);
        self.containers.remove(&chunk)
    }

    pub fn container(&self, chunk: ChunkKey) -> Option<&ChunkTicks<T>> {
        self.containers.get(&chunk)
    }

    pub fn container_mut(&mut self, chunk: ChunkKey) -> Option<&mut ChunkTicks<T>> {
        self.containers.get_mut(&chunk)
    }

    /// Chunks with a container, in no particular order (for moving containers between
    /// levels).
    pub fn chunks(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        self.containers.keys().copied()
    }

    /// Unpacks a chunk's loaded ticks and makes it eligible for ticking.
    pub fn unpack(&mut self, chunk: ChunkKey, game_time: i64) {
        if let Some(c) = self.containers.get_mut(&chunk) {
            c.unpack(game_time);
            if let Some(t) = c.peek() {
                self.next_tick.insert(chunk, t.trigger);
            }
        }
    }

    /// Schedules a tick; dropped (returns false) if its chunk has no container or a tick of
    /// the same kind is already scheduled there.
    pub fn schedule(&mut self, tick: ScheduledTick<T>) -> bool {
        let chunk = tick.pos.chunk();
        let Some(c) = self.containers.get_mut(&chunk) else { return false };
        if !c.schedule(tick) {
            return false;
        }
        if c.peek() == Some(&tick) {
            self.next_tick.insert(chunk, tick.trigger);
        }
        true
    }

    pub fn has_scheduled_tick(&self, pos: BlockPos, kind: T) -> bool {
        self.containers.get(&pos.chunk()).is_some_and(|c| c.has_scheduled_tick(pos, kind))
    }

    /// Whether a tick of `kind` at `pos` is among those still to run in the current tick.
    pub fn will_tick_this_tick(&mut self, pos: BlockPos, kind: T) -> bool {
        if self.to_run_set.is_empty() && !self.to_run.is_empty() {
            self.to_run_set.extend(self.to_run.iter().map(|t| (t.kind, t.pos)));
        }
        self.to_run_set.contains(&(kind, pos))
    }

    /// Collects up to `max` ticks due at `time` from chunks `can_tick` accepts, in run order
    /// (`LevelTicks.collectTicks`). Run them with [`LevelTicks::next_to_run`].
    pub fn collect(&mut self, time: i64, max: usize, mut can_tick: impl FnMut(ChunkKey) -> bool) {
        let mut heads: BinaryHeap<Reverse<Head<T>>> = BinaryHeap::new();
        let containers = &self.containers;
        self.next_tick.retain(|&chunk, next| {
            if *next > time {
                return true;
            }
            let Some(c) = containers.get(&chunk) else { return false };
            let Some(head) = c.peek() else { return false };
            if head.trigger > time {
                *next = head.trigger;
                return true;
            }
            if can_tick(chunk) {
                heads.push(Reverse(Head(*head, chunk)));
                return false;
            }
            true
        });
        while self.to_run.len() < max {
            let Some(Reverse(Head(_, chunk))) = heads.pop() else { break };
            let c = self.containers.get_mut(&chunk).expect("container");
            let t = c.poll().expect("head");
            self.to_run.push_back(t);
            // drainFromCurrentContainer: keep taking while this container stays ahead.
            let rival = heads.peek().map(|h| h.0.0);
            while self.to_run.len() < max {
                let Some(&t) = c.peek() else { break };
                if t.trigger > time || rival.is_some_and(|r| t.intra_key() > r.intra_key()) {
                    break;
                }
                c.poll();
                self.to_run.push_back(t);
            }
            if let Some(&next) = c.peek() {
                if next.trigger <= time && self.to_run.len() < max {
                    heads.push(Reverse(Head(next, chunk)));
                } else {
                    self.next_tick.insert(chunk, next.trigger);
                }
            }
        }
        for Reverse(Head(_, chunk)) in heads {
            if let Some(next) = self.containers.get(&chunk).and_then(|c| c.peek()) {
                self.next_tick.insert(chunk, next.trigger);
            }
        }
    }

    /// The next collected tick to run (`runCollectedTicks`).
    pub fn next_to_run(&mut self) -> Option<ScheduledTick<T>> {
        let t = self.to_run.pop_front()?;
        if !self.to_run_set.is_empty() {
            self.to_run_set.remove(&(t.kind, t.pos));
        }
        Some(t)
    }

    /// `cleanupAfterTick`.
    pub fn finish_tick(&mut self) {
        self.to_run.clear();
        self.to_run_set.clear();
    }

    pub fn count(&self) -> usize {
        self.containers.values().map(ChunkTicks::count).sum()
    }

    /// [`ChunkTicks::shift`] for every chunk. Call between ticks (nothing collected).
    pub fn shift(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }
        for c in self.containers.values_mut() {
            c.shift(delta);
        }
        for next in self.next_tick.values_mut() {
            *next += delta;
        }
    }

    /// Removes the ticks inside the box (inclusive corners), as `/fill` and structures do.
    pub fn clear_area(&mut self, min: BlockPos, max: BlockPos) {
        let inside = |p: BlockPos| (min.x..=max.x).contains(&p.x) && (min.y..=max.y).contains(&p.y) && (min.z..=max.z).contains(&p.z);
        for cx in min.x >> 4..=max.x >> 4 {
            for cz in min.z >> 4..=max.z >> 4 {
                if let Some(c) = self.containers.get_mut(&(cx, cz)) {
                    c.retain(|t| !inside(t.pos));
                    match c.peek() {
                        Some(t) => {
                            let t = t.trigger;
                            self.next_tick.insert((cx, cz), t);
                        }
                        None => {
                            self.next_tick.remove(&(cx, cz));
                        }
                    }
                }
            }
        }
    }
}

/// Encodes saved ticks as a chunk NBT list (`block_ticks` / `fluid_ticks`): compounds
/// `{i, x, y, z, t, p}`.
pub fn ticks_to_nbt<T: Copy>(ticks: &[SavedTick<T>], name: impl Fn(T) -> &'static str) -> Tag {
    Tag::List(
        ticks
            .iter()
            .map(|t| {
                Tag::Compound(vec![
                    ("i".into(), Tag::String(name(t.kind).into())),
                    ("x".into(), Tag::Int(t.pos.x)),
                    ("y".into(), Tag::Int(t.pos.y)),
                    ("z".into(), Tag::Int(t.pos.z)),
                    ("t".into(), Tag::Int(t.delay)),
                    ("p".into(), Tag::Int(t.priority.value())),
                ])
            })
            .collect(),
    )
}

/// Decodes a chunk NBT tick list, keeping entries inside `chunk` whose id `parse` knows
/// (`SavedTick.filterTickListForChunk`).
pub fn ticks_from_nbt<T>(tag: &Tag, chunk: ChunkKey, parse: impl Fn(&str) -> Option<T>) -> Vec<SavedTick<T>> {
    let Some(list) = tag.as_list() else { return Vec::new() };
    list.iter()
        .filter_map(|e| {
            let int = |k| e.get(k).and_then(Tag::as_i64).map(|v| v as i32);
            let kind = parse(e.get("i")?.as_str()?)?;
            let pos = BlockPos::new(int("x")?, int("y")?, int("z")?);
            let t = SavedTick { kind, pos, delay: int("t")?, priority: TickPriority::from_value(int("p")?) };
            (pos.chunk() == chunk).then_some(t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(kind: u16, x: i32, trigger: i64, priority: TickPriority, sub: i64) -> ScheduledTick<u16> {
        ScheduledTick { kind, pos: BlockPos::new(x, 0, 0), trigger, priority, sub }
    }

    fn run_all(t: &mut LevelTicks<u16>, time: i64) -> Vec<(u16, i32)> {
        t.collect(time, 65536, |_| true);
        let mut out = Vec::new();
        while let Some(x) = t.next_to_run() {
            out.push((x.kind, x.pos.x));
        }
        t.finish_tick();
        out
    }

    #[test]
    fn shifted_ticks_keep_their_distance_and_order() {
        use TickPriority::*;
        let mut t = LevelTicks::new();
        t.add_container((0, 0), ChunkTicks::new());
        t.add_container((1, 0), ChunkTicks::new());
        assert!(t.schedule(tick(1, 0, 12, Normal, 0)));
        assert!(t.schedule(tick(2, 16, 12, High, 1)));
        assert!(t.schedule(tick(3, 1, 15, Normal, 2)));
        // A region that ran one tick (time 10) while the server ran four more.
        t.shift(3);
        assert_eq!(run_all(&mut t, 14), vec![], "due at 15 now");
        assert_eq!(run_all(&mut t, 15), vec![(2, 16), (1, 0)]);
        assert_eq!(run_all(&mut t, 17), vec![]);
        assert_eq!(run_all(&mut t, 18), vec![(3, 1)]);
    }

    #[test]
    fn order_is_priority_then_sub_tick_across_chunks() {
        let mut t = LevelTicks::new();
        t.add_container((0, 0), ChunkTicks::new());
        t.add_container((1, 0), ChunkTicks::new());
        use TickPriority::*;
        assert!(t.schedule(tick(1, 0, 10, Normal, 0)));
        assert!(t.schedule(tick(2, 16, 10, Normal, 1)));
        assert!(t.schedule(tick(3, 1, 10, High, 2)));
        assert!(t.schedule(tick(4, 17, 10, Normal, 3)));
        assert!(t.schedule(tick(5, 2, 11, ExtremelyHigh, 4)));
        // Duplicate (kind, pos) is dropped.
        assert!(!t.schedule(tick(1, 0, 12, Normal, 5)));
        assert!(!t.schedule(tick(9, 400, 10, Normal, 6)), "no container");
        assert_eq!(run_all(&mut t, 9), vec![]);
        assert_eq!(run_all(&mut t, 10), vec![(3, 1), (1, 0), (2, 16), (4, 17)]);
        assert_eq!(run_all(&mut t, 11), vec![(5, 2)]);
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn overdue_ticks_drain_by_trigger_within_a_container() {
        let mut t = LevelTicks::new();
        t.add_container((0, 0), ChunkTicks::new());
        use TickPriority::*;
        t.schedule(tick(1, 0, 5, Low, 0));
        t.schedule(tick(2, 1, 6, High, 1));
        t.schedule(tick(3, 2, 6, Normal, 2));
        // With a limit of 1 per tick the rest become overdue.
        t.collect(6, 1, |_| true);
        assert_eq!(t.next_to_run().map(|x| x.kind), Some(1));
        t.finish_tick();
        assert_eq!(run_all(&mut t, 7), vec![(2, 1), (3, 2)]);
    }

    #[test]
    fn will_tick_this_tick_sees_remaining_ticks() {
        let mut t = LevelTicks::new();
        t.add_container((0, 0), ChunkTicks::new());
        t.schedule(tick(1, 0, 1, TickPriority::Normal, 0));
        t.schedule(tick(2, 1, 1, TickPriority::Normal, 1));
        t.collect(1, 100, |_| true);
        assert!(t.will_tick_this_tick(BlockPos::new(1, 0, 0), 2));
        t.next_to_run();
        assert!(!t.will_tick_this_tick(BlockPos::new(0, 0, 0), 1));
        assert!(t.will_tick_this_tick(BlockPos::new(1, 0, 0), 2));
        t.next_to_run();
        assert!(!t.will_tick_this_tick(BlockPos::new(1, 0, 0), 2));
    }

    /// A model check of the whole life of scheduled ticks: scheduling, running when due in ticking chunks, chunks that
    /// do not tick for a while, chunks leaving (packed, relative delays) and coming back (unpacked), containers moving
    /// between two levels' tick sets (region splits and merges). A tick a chunk holds must report as scheduled until it
    /// runs, and must run once its chunk ticks at or after its time; a tick must never run early or twice.
    #[test]
    fn scheduled_ticks_are_never_lost_or_run_twice() {
        use std::collections::BTreeMap;
        // A small deterministic generator (xorshift).
        struct Rng(u64);
        impl Rng {
            fn next(&mut self, n: u64) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0 % n
            }
        }
        for seed in 1..=40u64 {
            let mut rng = Rng(seed * 0x9E37_79B9_7F4A_7C15);
            let mut sets = [LevelTicks::<u16>::new(), LevelTicks::<u16>::new()];
            // Which set holds each chunk (None: unloaded), the model of ticks by (chunk, kind, x): trigger time, and the
            // saved (relative) ticks of unloaded chunks.
            let mut home: [Option<usize>; 5] = [None; 5];
            let mut model: BTreeMap<(usize, u16, i32), i64> = BTreeMap::new();
            let mut saved: [Vec<SavedTick<u16>>; 5] = Default::default();
            let mut time = 100i64;
            let mut sub = 0i64;
            for c in 0..5usize {
                let s = rng.next(2) as usize;
                sets[s].add_container((c as i32, 0), ChunkTicks::from_saved(Vec::new()));
                sets[s].unpack((c as i32, 0), time);
                home[c] = Some(s);
            }
            for step in 0..400 {
                match rng.next(10) {
                    0..=3 => {
                        // Schedule.
                        let c = rng.next(5) as usize;
                        let (kind, x) = (rng.next(3) as u16, rng.next(4) as i32);
                        let delay = rng.next(8) as i64;
                        sub += 1;
                        let t = ScheduledTick {
                            kind,
                            pos: BlockPos::new(c as i32 * 16 + x, 0, 0),
                            trigger: time + delay,
                            priority: TickPriority::from_value(rng.next(5) as i32 - 2),
                            sub,
                        };
                        match home[c] {
                            Some(s) => {
                                let had = model.contains_key(&(c, kind, x));
                                assert_eq!(sets[s].has_scheduled_tick(t.pos, kind), had, "seed {seed} step {step}");
                                let added = sets[s].schedule(t);
                                assert_eq!(added, !had, "seed {seed} step {step}: schedule");
                                if added {
                                    model.insert((c, kind, x), t.trigger);
                                }
                            }
                            None => assert!(!sets[0].schedule(t) && !sets[1].schedule(t)),
                        }
                    }
                    4..=6 => {
                        // A game tick: the chunks that tick this time are those whose bit is set.
                        time += 1;
                        let mask = rng.next(32);
                        let ticks_now = |k: ChunkKey| mask & (1 << k.0) != 0;
                        let mut ran = Vec::new();
                        for s in 0..2 {
                            sets[s].collect(time, 65536, ticks_now);
                            while let Some(t) = sets[s].next_to_run() {
                                ran.push(((t.pos.x >> 4) as usize, t.kind, t.pos.x & 15, t.trigger));
                            }
                            sets[s].finish_tick();
                        }
                        for (c, kind, x, trigger) in ran {
                            assert!(trigger <= time, "seed {seed} step {step}: ran early");
                            assert!(mask & (1 << c) != 0, "seed {seed} step {step}: ran in a chunk that does not tick");
                            assert_eq!(model.remove(&(c, kind, x)), Some(trigger), "seed {seed} step {step}: ran twice or never scheduled");
                        }
                        // Everything due in a ticking chunk has run.
                        for (&(c, _, _), &trigger) in &model {
                            if home[c].is_some() && mask & (1 << c) != 0 {
                                assert!(trigger > time, "seed {seed} step {step}: a due tick was left");
                            }
                        }
                    }
                    7 => {
                        // A chunk leaves: its ticks are packed relative to now.
                        let c = rng.next(5) as usize;
                        if let Some(s) = home[c].take() {
                            let container = sets[s].remove_container((c as i32, 0)).unwrap();
                            saved[c] = container.pack(time);
                            let mine: Vec<_> = model.keys().filter(|k| k.0 == c).copied().collect();
                            assert_eq!(saved[c].len(), mine.len(), "seed {seed} step {step}");
                            for k in mine {
                                model.remove(&k);
                            }
                        }
                    }
                    8 => {
                        // A chunk comes back, into either set.
                        let c = rng.next(5) as usize;
                        if home[c].is_none() {
                            let s = rng.next(2) as usize;
                            let ticks = std::mem::take(&mut saved[c]);
                            // The model keeps absolute times: now plus the delay it was saved with.
                            let by_pos: BTreeMap<(u16, i32), i64> = ticks.iter().map(|t| ((t.kind, t.pos.x & 15), time + t.delay as i64)).collect();
                            sets[s].add_container((c as i32, 0), ChunkTicks::from_saved(ticks));
                            sets[s].unpack((c as i32, 0), time);
                            for ((kind, x), trigger) in by_pos {
                                model.insert((c, kind, x), trigger);
                            }
                            home[c] = Some(s);
                        }
                    }
                    _ => {
                        // A region split or merge moves a container to the other set.
                        let c = rng.next(5) as usize;
                        if let Some(s) = home[c] {
                            let container = sets[s].remove_container((c as i32, 0)).unwrap();
                            sets[1 - s].add_container((c as i32, 0), container);
                            home[c] = Some(1 - s);
                        }
                    }
                }
                // What the sets report is what the model holds.
                for ((c, kind, x), _) in model.iter() {
                    let s = home[*c].expect("a model tick of an unloaded chunk");
                    assert!(sets[s].has_scheduled_tick(BlockPos::new(*c as i32 * 16 + x, 0, 0), *kind), "seed {seed} step {step}: lost");
                }
            }
            // Finally every chunk loaded and ticking: everything left runs.
            for c in 0..5usize {
                if home[c].is_none() {
                    sets[0].add_container((c as i32, 0), ChunkTicks::from_saved(std::mem::take(&mut saved[c])));
                    sets[0].unpack((c as i32, 0), time);
                    home[c] = Some(0);
                }
            }
            time += 100;
            for s in 0..2 {
                sets[s].collect(time, 65536, |_| true);
                while sets[s].next_to_run().is_some() {}
                sets[s].finish_tick();
                assert_eq!(sets[s].count(), 0, "seed {seed}: ticks stuck in a set");
            }
        }
    }

    #[test]
    fn pack_unpack_roundtrip_keeps_order() {
        let mut c = ChunkTicks::new();
        use TickPriority::*;
        c.schedule(tick(7, 3, 105, Normal, 40));
        c.schedule(tick(8, 4, 103, High, 41));
        c.schedule(tick(9, 5, 103, High, 12));
        let saved = c.pack(100);
        assert_eq!(saved.iter().map(|t| (t.kind, t.delay)).collect::<Vec<_>>(), vec![(9, 3), (7, 5), (8, 3)]);
        let nbt = ticks_to_nbt(&saved, |k| if k == 7 { "minecraft:a" } else if k == 8 { "minecraft:b" } else { "minecraft:c" });
        let back = ticks_from_nbt(&nbt, (0, 0), |s| Some(match s { "minecraft:a" => 7, "minecraft:b" => 8, _ => 9 }));
        assert_eq!(back, saved);
        let mut loaded = ChunkTicks::from_saved(back);
        assert!(loaded.has_scheduled_tick(BlockPos::new(3, 0, 0), 7));
        assert_eq!(loaded.pack(0), saved, "pending ticks are saved as loaded");
        loaded.unpack(200);
        let order: Vec<_> = std::iter::from_fn(|| loaded.poll()).map(|t| (t.kind, t.trigger, t.sub)).collect();
        assert_eq!(order, vec![(9, 203, -3), (8, 203, -1), (7, 205, -2)]);
    }
}
