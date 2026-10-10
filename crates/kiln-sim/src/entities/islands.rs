//! Islands: a region's entities in groups too far apart to meet within a tick, ticked side by
//! side on the tick pool.
//!
//! A crowd keeps many mobs in one region, and the entity phase is serial (each entity sees what
//! the ones before it did). Entities closer than [`LINK`] blocks to each other (on every axis),
//! the players near them, riders and their vehicles, leads and their holders, pets and their
//! owners, fishing hooks and their anglers form an island; each island ticks its entities in
//! list order as the serial phase would, against its own players, and the islands run in
//! parallel. An island reads the region's blocks and logs what it changes (block changes,
//! block packets); everything an island leaves behind (spawns, deaths, events, the log) is
//! merged in the order of the entities that made it, so the serial phase's order is kept and
//! the result does not depend on the workers.
//!
//! What differs from the serial phase: an entity does not see entities or players of another
//! island (they are at least [`LINK`] blocks away), and a block an entity changes reaches the
//! region (with its neighbour updates) after the islands, though its own island reads the new
//! state at once. Regions with what an island cannot leave for later (villages' points of
//! interest, sculk listeners, creaking hearts, lightning, hopper minecarts, the dragon and the
//! wither) tick their entities serially.
//!
//! Mobs that roam join the islands into one, which leaves nothing to share out. Then the region
//! ticks in tiles instead ([`TILE`] blocks square, on fixed coordinates): nine passes, one per
//! tile colour (`x mod 3`, `z mod 3`), and in each pass every tile of that colour ticks its
//! entities in list order against the entities and players of its tile and the eight around
//! it, which no other tile of the pass reaches, so the pass's tiles run in parallel. The
//! entities tick in the order of the passes rather than the list, and see no further than the
//! neighbouring tiles; what they leave behind is merged per pass in list order. There is no
//! barrier between the passes: a tile's group runs once the groups of earlier passes it shares
//! an entity or a player with are done ([`run_tiles`]), and the blocks the groups change reach
//! the region after all the passes.

use super::*;
use crate::entity_world::{Deferred, IslandWorld, World};
use kiln_sched::{Ctx, Strategy, Window};

/// Entities closer than this (blocks, on each axis) to each other or to a player share an
/// island: more than the reach of the entities' area queries (a 16-block sensor box reaches
/// 23 blocks along a diagonal).
const LINK: f64 = 24.0;
/// Regions with fewer entities tick them serially.
const MIN_ENTITIES: usize = 32;
/// Islands are used while none holds more than this share of the entities (one in four), else
/// tiles; never the workers, so the result does not depend on them.
const ISLAND_SHARE: usize = 4;
/// Entity types whose tick needs region state an island cannot change later.
pub(super) const SERIAL: [&str; 11] = [
    "minecraft:copper_golem",
    "minecraft:sulfur_cube",
    "minecraft:villager",
    "minecraft:warden",
    "minecraft:allay",
    "minecraft:creaking",
    "minecraft:hopper_minecart",
    "minecraft:lightning_bolt",
    "minecraft:ender_dragon",
    "minecraft:end_crystal",
    "minecraft:wither",
];
/// Placeholder ids each island or tile may hand out.
const PLACEHOLDERS: i32 = 10_000;
/// The tiles' side (blocks): an entity reaches at least this far into the tiles around its own.
const TILE: f64 = 16.0;

/// Where an island's outputs stood when one of its entities' turns began.
#[derive(Clone, Copy)]
struct Mark {
    /// The entity's index in the region's list.
    global: usize,
    spawns: usize,
    deaths: usize,
    events: usize,
    deferred: usize,
    packets: usize,
}

struct Job<'p> {
    /// The island's entities' indices in the region's list (ascending), and the entities.
    global: Vec<usize>,
    list: Vec<Entity>,
    players: Vec<&'p mut Player>,
    proxies: Vec<Proxy>,
    views: Vec<PlayerView>,
    placeholder: i32,
    spawns: Vec<Spawn>,
    deaths: Vec<health::Death>,
    events: Vec<Event>,
    deferred: Vec<Deferred>,
    packets: Vec<([f64; 3], f64, Bytes)>,
    marks: Vec<Mark>,
    /// Which of `list` tick (`None`: all).
    ticks: Option<Vec<bool>>,
    /// Where `proxies` came from in the region's.
    proxy_slots: Vec<usize>,
}

/// What every island reads.
struct Shared<'s> {
    cells: &'s kiln_region::CellSet<kiln_world::Cell>,
    env: &'s blocks::BlockEnv,
    pistons: &'s kiln_blocks::MovingPistons,
    ticking: &'s blocks::Ticking,
    /// The region's players, for `Mob.checkDespawn` (the nearest player decides).
    nearest: &'s Nearest,
    any_player: bool,
}

/// Union-find over the entities (first) and the players.
struct Sets(Vec<u32>);

impl Sets {
    fn find(&mut self, mut a: usize) -> usize {
        while self.0[a] as usize != a {
            let up = self.0[self.0[a] as usize];
            self.0[a] = up;
            a = up as usize;
        }
        a
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            self.0[hi] = lo as u32;
        }
    }
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    (a[0] - b[0]).abs() <= LINK && (a[1] - b[1]).abs() <= LINK && (a[2] - b[2]).abs() <= LINK
}

/// The islands: (entity indices, player indices), ordered by their first entity. Nodes in one
/// cell of the link distance are always close; nodes in neighbouring cells are compared
/// until one close pair (with an entity in it) joins the cells' sets.
fn partition(list: &[Entity], players: &[&mut Player]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let (n, m) = (list.len(), players.len());
    let pos = |k: usize| if k < n { list[k].pos } else { players[k - n].pos };
    let mut sets = Sets((0..(n + m) as u32).collect());
    const CELL: f64 = LINK;
    let cell_of = |p: [f64; 3]| ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32);
    // Cell → (entities, players).
    let mut cells: crate::FastMap<(i32, i32, i32), (Vec<usize>, Vec<usize>)> = Default::default();
    for k in 0..n + m {
        let c = cells.entry(cell_of(pos(k))).or_default();
        if k < n { c.0.push(k) } else { c.1.push(k) }
    }
    let mut keys: Vec<(i32, i32, i32)> = cells.keys().copied().collect();
    keys.sort_unstable();
    for key in &keys {
        let (ents, pls) = &cells[key];
        let all: Vec<usize> = ents.iter().chain(pls).copied().collect();
        for w in all.windows(2) {
            sets.union(w[0], w[1]);
        }
    }
    for key in &keys {
        let (ents, _) = &cells[key];
        if ents.is_empty() {
            continue;
        }
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let other = (key.0 + dx, key.1 + dy, key.2 + dz);
                    if other == *key {
                        continue;
                    }
                    let Some((oents, opls)) = cells.get(&other) else { continue };
                    let (a, b) = (ents[0], *oents.first().or(opls.first()).expect("a cell has nodes"));
                    if sets.find(a) == sets.find(b) {
                        continue;
                    }
                    if let Some(&q) = oents.iter().chain(opls).find(|&&q| ents.iter().any(|&e| close(pos(e), pos(q)))) {
                        sets.union(a, q);
                    }
                }
            }
        }
    }
    // Riders and vehicles, leads, pets and owners, fishing hooks and anglers.
    let by_id = |id: i32| list.binary_search_by_key(&id, |e| e.id).ok().or_else(|| players.iter().position(|p| p.entity_id == id).map(|j| n + j));
    for (i, e) in list.iter().enumerate() {
        let Some(phys) = e.phys.as_deref() else { continue };
        let mut related: SmallVec<[usize; 4]> = SmallVec::new();
        related.extend(phys.vehicle.and_then(by_id));
        related.extend(phys.passengers.iter().filter_map(|&p| by_id(p)));
        related.extend(phys.leash.as_ref().and_then(|l| l.holder).and_then(by_id));
        if let Some(owner) = kiln_entity::mob::data(phys).and_then(kiln_entity::mob::kinds::tame::get).and_then(|t| t.owner) {
            related.extend(players.iter().position(|p| p.uuid.as_u128() == owner).map(|j| n + j));
        }
        if let Some(hook) = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::fishing_hook::FishingHook>(phys) {
            related.extend(by_id(hook.owner));
        }
        for r in related {
            sets.union(i, r);
        }
    }
    let mut islands: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut at: crate::FastMap<usize, usize> = Default::default();
    for k in 0..n + m {
        let root = sets.find(k);
        let slot = match at.get(&root) {
            Some(&s) => s,
            None if k < n => {
                islands.push(Default::default());
                at.insert(root, islands.len() - 1);
                islands.len() - 1
            }
            // Players with no entity near them stay out.
            None => continue,
        };
        if k < n { islands[slot].0.push(k) } else { islands[slot].1.push(k - n) }
    }
    islands
}

/// A job of one parallel batch: entities (indices in the region's list, ascending), players
/// (indices), and which of the entities tick (`None`: all).
struct Group {
    ents: Vec<usize>,
    players: Vec<usize>,
    ticks: Option<Vec<bool>>,
    /// The centre tile of a tile's group (`None`: a whole island).
    centre: Option<(i32, i32)>,
}

/// The tiles of one tick, one batch per colour: each centre tile with its neighbourhood. An
/// entity is in its root vehicle's tile, so riders tick with their vehicle.
fn tile_batches(list: &[Entity], players: &[&mut Player], island: &[usize], island_players: &[usize]) -> [Vec<Group>; 9] {
    let tile = |p: [f64; 3]| ((p[0] / TILE).floor() as i32, (p[2] / TILE).floor() as i32);
    let root = |mut i: usize| {
        for _ in 0..8 {
            let Some(v) = list[i].phys.as_deref().and_then(|p| p.vehicle) else { break };
            match list.binary_search_by_key(&v, |e| e.id) {
                Ok(j) if j != i => i = j,
                _ => break,
            }
        }
        i
    };
    let mut ents: crate::FastMap<(i32, i32), Vec<usize>> = Default::default();
    let mut at: crate::FastMap<usize, (i32, i32)> = Default::default();
    for &i in island {
        let t = tile(list[root(i)].pos);
        ents.entry(t).or_default().push(i);
        at.insert(i, t);
    }
    let mut pls: crate::FastMap<(i32, i32), Vec<usize>> = Default::default();
    for &j in island_players {
        pls.entry(tile(players[j].pos)).or_default().push(j);
    }
    let mut centres: Vec<(i32, i32)> = ents.keys().copied().collect();
    centres.sort_unstable();
    let mut batches: [Vec<Group>; 9] = Default::default();
    for (colour, batch) in batches.iter_mut().enumerate() {
        for &c in centres.iter().filter(|c| (c.0.rem_euclid(3) * 3 + c.1.rem_euclid(3)) as usize == colour) {
            let mut g = Group { ents: Vec::new(), players: Vec::new(), ticks: None, centre: Some(c) };
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let t = (c.0 + dx, c.1 + dz);
                    g.ents.extend(ents.get(&t).into_iter().flatten());
                    g.players.extend(pls.get(&t).into_iter().flatten());
                }
            }
            g.ents.sort_unstable();
            g.players.sort_unstable();
            g.ticks = Some(g.ents.iter().map(|&i| at[&i] == c).collect());
            batch.push(g);
        }
    }
    batches
}

/// Ticks the region's entities as islands, if the region qualifies; returns whether it did
/// (else the caller ticks them serially).
pub(super) fn tick_islands(sim: &mut SimLevel, ticking: &blocks::Ticking, any_player: bool, ctx: &Ctx<'_>) -> bool {
    if sim.list.len() < MIN_ENTITIES {
        return false;
    }
    let Some(region) = sim.level.region_ref() else { return false };
    let mode = region.env.entity_ticking;
    if mode == crate::EntityTicking::Serial {
        return false;
    }
    if !region.blocks.hearts.is_empty() || crate::sculk::listening(region) || sim.list.iter().any(|e| SERIAL.contains(&e.kind.name)) {
        return false;
    }
    let islands = partition(sim.list, sim.players);
    let n = sim.list.len();
    let largest = islands.iter().map(|g| g.0.len()).max().unwrap_or(0);
    let mut next = sim.next_placeholder;
    // Islands with more than their share of the entities tick in tiles (their own entities and
    // players only, so no tile reaches another island), the others whole, in the first batch.
    let batches: Vec<Vec<Group>> = if largest * ISLAND_SHARE <= n && islands.len() >= 2 {
        vec![islands.into_iter().map(|(ents, players)| Group { ents, players, ticks: None, centre: None }).collect()]
    } else if mode == crate::EntityTicking::Tiles {
        let mut batches: Vec<Vec<Group>> = (0..9).map(|_| Vec::new()).collect();
        for (ents, players) in islands {
            if ents.len() * ISLAND_SHARE > n {
                for (colour, groups) in tile_batches(sim.list, sim.players, &ents, &players).into_iter().enumerate() {
                    batches[colour].extend(groups);
                }
            } else {
                batches[0].push(Group { ents, players, ticks: None, centre: None });
            }
        }
        batches.retain(|b| !b.is_empty());
        batches
    } else if islands.len() >= 2 {
        vec![islands.into_iter().map(|(ents, players)| Group { ents, players, ticks: None, centre: None }).collect()]
    } else {
        return false;
    };
    // Which player each stand-in and view is.
    let player_at: crate::FastMap<i32, usize> = sim.players.iter().enumerate().map(|(j, p)| (p.entity_id, j)).collect();
    let mut taken: Vec<Option<Entity>> = std::mem::take(sim.list).into_iter().map(Some).collect();
    // The players' stand-ins go to their groups and come back, like the entities.
    let mut stand_ins: Vec<Option<Proxy>> = std::mem::take(&mut sim.proxies).into_iter().map(Some).collect();
    if batches.len() > 1 {
        // Tiles: each group as soon as the groups of earlier passes that share its tiles are done.
        let groups: Vec<(usize, Group)> = batches.into_iter().enumerate().flat_map(|(pass, b)| b.into_iter().map(move |g| (pass, g))).collect();
        run_tiles(sim, &mut taken, &mut stand_ins, groups, &player_at, ticking, any_player, ctx, &mut next);
    } else {
        for batch in batches {
            run_batch(sim, &mut taken, &mut stand_ins, batch, &player_at, ticking, any_player, ctx, &mut next);
        }
    }
    *sim.list = taken.into_iter().map(|e| e.expect("every entity back from its group")).collect();
    sim.proxies = stand_ins.into_iter().map(|e| e.expect("every stand-in back from its group")).collect();
    sim.next_placeholder = next;
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    true
}

/// Runs one batch of groups in parallel and merges what they left behind in list order.
#[allow(clippy::too_many_arguments)]
fn run_batch(
    sim: &mut SimLevel,
    taken: &mut [Option<Entity>],
    stand_ins: &mut [Option<Proxy>],
    groups: Vec<Group>,
    player_at: &crate::FastMap<i32, usize>,
    ticking: &blocks::Ticking,
    any_player: bool,
    ctx: &Ctx<'_>,
    next: &mut i32,
) {
    let mut jobs: Vec<Job> = Vec::with_capacity(groups.len());
    {
        let SimLevel { level, players, views, despawn, .. } = &mut *sim;
        let region = level.region_ref().expect("a region");
        let nearest = despawn.expect("the region's players");
        let shared = Shared { cells: &*region.cells, env: region.env, pistons: &region.blocks.data.pistons, ticking, nearest, any_player };
        let mut slots: Vec<Option<&mut Player>> = players.iter_mut().map(|p| Some(&mut **p)).collect();
        let mut job_of: Vec<usize> = vec![usize::MAX; slots.len()];
        for (k, g) in groups.iter().enumerate() {
            for &j in &g.players {
                job_of[j] = k;
            }
        }
        for g in groups {
            let group_players: Vec<&mut Player> = g.players.iter().map(|&j| slots[j].take().expect("a player in one group")).collect();
            *next -= PLACEHOLDERS;
            jobs.push(Job {
                list: g.ents.iter().map(|&i| taken[i].take().expect("an entity in one group")).collect(),
                global: g.ents,
                proxies: Vec::with_capacity(group_players.len()),
                views: Vec::with_capacity(group_players.len()),
                players: group_players,
                placeholder: *next + PLACEHOLDERS,
                spawns: Vec::new(),
                deaths: Vec::new(),
                events: Vec::new(),
                deferred: Vec::new(),
                packets: Vec::new(),
                marks: Vec::new(),
                ticks: g.ticks,
                proxy_slots: Vec::with_capacity(g.players.len()),
            });
        }
        let job = |id: i32| player_at.get(&id).map(|&j| job_of[j]).filter(|&k| k != usize::MAX);
        for (i, slot) in stand_ins.iter_mut().enumerate() {
            if let Some(k) = slot.as_ref().and_then(|e| job(e.id)) {
                jobs[k].proxies.push(slot.take().expect("a stand-in"));
                jobs[k].proxy_slots.push(i);
            }
        }
        for v in views.iter() {
            if let Some(k) = job(v.id) {
                jobs[k].views.push(*v);
            }
        }
        // Largest first, one group per chunk, so the long ones start early.
        jobs.sort_by_key(|j| std::cmp::Reverse(j.ticks.as_ref().map_or(j.list.len(), |t| t.iter().filter(|&&b| b).count())));
        ctx.map_mut_with(Window::new().chunk(1).strategy(Strategy::Parallel), &mut jobs, |_, job| run_island(job, &shared));
    }

    // Back in the region, in list order, with the groups' outputs in the order of the turns
    // that made them: each group's turns in order, the groups' turns interleaved by list index.
    let mut parts: Vec<(usize, usize, usize)> = Vec::new();
    let mut outs: Vec<Outputs> = Vec::with_capacity(jobs.len());
    for (k, job) in jobs.into_iter().enumerate() {
        for (g, e) in job.global.iter().zip(job.list) {
            taken[*g] = Some(e);
        }
        for (i, p) in job.proxy_slots.into_iter().zip(job.proxies) {
            stand_ins[i] = Some(p);
        }
        let first = job.global.first().copied().unwrap_or(0);
        parts.push((first, k, 0));
        parts.extend(job.marks.iter().enumerate().map(|(j, m)| (m.global, k, j + 1)));
        outs.push(Outputs {
            marks: job.marks,
            spawns: job.spawns.into_iter(),
            deaths: job.deaths.into_iter(),
            events: job.events.into_iter(),
            deferred: job.deferred.into_iter(),
            packets: job.packets.into_iter(),
        });
    }
    parts.sort_unstable();
    for (_, k, j) in parts {
        merge_turn(sim, &mut outs[k], j);
    }
}

/// Slots that the tile groups take their entities, players and stand-ins from and put them back
/// into, on whichever worker runs them.
struct Slots<T>(Vec<std::cell::UnsafeCell<Option<T>>>);

// SAFETY: a slot is only reached through `take` and `put`, whose callers hold it alone.
unsafe impl<T: Send> Sync for Slots<T> {}

impl<T> Slots<T> {
    fn new(items: impl Iterator<Item = Option<T>>) -> Self {
        Slots(items.map(std::cell::UnsafeCell::new).collect())
    }

    /// SAFETY: nothing else reaches slot `i` meanwhile.
    unsafe fn take(&self, i: usize) -> Option<T> {
        unsafe { (*self.0[i].get()).take() }
    }

    /// SAFETY: nothing else reaches slot `i` meanwhile.
    unsafe fn put(&self, i: usize, v: T) {
        unsafe { *self.0[i].get() = Some(v) }
    }

    fn into_inner(self) -> impl Iterator<Item = Option<T>> {
        self.0.into_iter().map(std::cell::UnsafeCell::into_inner)
    }
}

/// What a group left behind, for the merge.
struct Done {
    pass: usize,
    first: usize,
    out: Outputs,
}

/// The tiles' passes without a barrier between them: a group (a centre tile and the eight
/// around it) runs once every group of an earlier pass whose tiles overlap its own is done, so
/// it sees exactly what it would after its pass's predecessors, and groups that share no tile
/// run side by side. Which groups run when depends on the workers; what each sees does not.
/// Blocks the groups change reach the region after all the passes (a group reads its own at
/// once). What the groups left behind is merged pass by pass, in list order.
#[allow(clippy::too_many_arguments)]
fn run_tiles(
    sim: &mut SimLevel,
    taken: &mut [Option<Entity>],
    stand_ins: &mut [Option<Proxy>],
    groups: Vec<(usize, Group)>,
    player_at: &crate::FastMap<i32, usize>,
    ticking: &blocks::Ticking,
    any_player: bool,
    ctx: &Ctx<'_>,
    next: &mut i32,
) {
    let n = groups.len();
    // Placeholder ids in pass order, as the passes would hand them out.
    let placeholders: Vec<i32> = (0..n)
        .map(|_| {
            *next -= PLACEHOLDERS;
            *next + PLACEHOLDERS
        })
        .collect();
    // An earlier pass's group whose tiles overlap (centres at most two tiles apart) goes first.
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut preds: Vec<std::sync::atomic::AtomicUsize> = (0..n).map(|_| std::sync::atomic::AtomicUsize::new(0)).collect();
    for a in 0..n {
        let Some(ca) = groups[a].1.centre else { continue };
        for b in a + 1..n {
            let Some(cb) = groups[b].1.centre else { continue };
            // Only what they share matters: tiles overlapping without an entity or a player in
            // common leave each other alone.
            if groups[a].0 < groups[b].0
                && (ca.0 - cb.0).abs() <= 2
                && (ca.1 - cb.1).abs() <= 2
                && (sorted_meet(&groups[a].1.ents, &groups[b].1.ents) || sorted_meet(&groups[a].1.players, &groups[b].1.players))
            {
                succ[a].push(b);
                *preds[b].get_mut() += 1;
            }
        }
    }
    let ticking_count = |g: &Group| g.ticks.as_ref().map_or(g.ents.len(), |t| t.iter().filter(|&&b| b).count());
    let mut ready: Vec<usize> = (0..n).filter(|&k| *preds[k].get_mut() == 0).collect();
    // Largest first, so the long ones start early.
    ready.sort_by_key(|&k| std::cmp::Reverse(ticking_count(&groups[k].1)));
    let queue = std::sync::Mutex::new((std::collections::VecDeque::from(ready), n));
    let results: Slots<Done> = Slots::new((0..n).map(|_| None));
    {
        let SimLevel { level, players, views, despawn, .. } = &mut *sim;
        let region = level.region_ref().expect("a region");
        let nearest = despawn.expect("the region's players");
        let shared = Shared { cells: &*region.cells, env: region.env, pistons: &region.blocks.data.pistons, ticking, nearest, any_player };
        let stand_of: Vec<Option<usize>> = {
            let mut of = vec![None; players.len()];
            for (s, e) in stand_ins.iter().enumerate() {
                if let Some(&j) = e.as_ref().and_then(|e| player_at.get(&e.id)) {
                    of[j] = Some(s);
                }
            }
            of
        };
        let mut view_of: Vec<Option<usize>> = vec![None; players.len()];
        for (v, view) in views.iter().enumerate() {
            if let Some(&j) = player_at.get(&view.id) {
                view_of[j] = Some(v);
            }
        }
        let views = &*views;
        let ents = Slots::new(taken.iter_mut().map(Option::take));
        let pls = Slots::new(players.iter_mut().map(|p| Some(&mut **p)));
        let stands = Slots::new(stand_ins.iter_mut().map(Option::take));
        let run = |k: usize| {
            let (pass, g) = &groups[k];
            // SAFETY: the groups that share an entity, a player or a stand-in with this one are
            // ordered before or after it, and the queue hands each group out once.
            let mut job = unsafe {
                Job {
                    global: g.ents.clone(),
                    list: g.ents.iter().map(|&i| ents.take(i).expect("an entity in one group at a time")).collect(),
                    players: g.players.iter().map(|&j| pls.take(j).expect("a player in one group at a time")).collect(),
                    proxies: g.players.iter().filter_map(|&j| stand_of[j]).map(|s| stands.take(s).expect("a stand-in")).collect(),
                    proxy_slots: g.players.iter().filter_map(|&j| stand_of[j]).collect(),
                    views: g.players.iter().filter_map(|&j| view_of[j]).map(|v| views[v]).collect(),
                    placeholder: placeholders[k],
                    spawns: Vec::new(),
                    deaths: Vec::new(),
                    events: Vec::new(),
                    deferred: Vec::new(),
                    packets: Vec::new(),
                    marks: Vec::new(),
                    ticks: g.ticks.clone(),
                }
            };
            run_island(&mut job, &shared);
            let Job { global, list, players: back, proxies, proxy_slots, spawns, deaths, events, deferred, packets, marks, .. } = job;
            // SAFETY: as above.
            unsafe {
                for (&i, e) in global.iter().zip(list) {
                    ents.put(i, e);
                }
                for (&j, p) in g.players.iter().zip(back) {
                    pls.put(j, p);
                }
                for (s, p) in proxy_slots.into_iter().zip(proxies) {
                    stands.put(s, p);
                }
                let out = Outputs {
                    marks,
                    spawns: spawns.into_iter(),
                    deaths: deaths.into_iter(),
                    events: events.into_iter(),
                    deferred: deferred.into_iter(),
                    packets: packets.into_iter(),
                };
                results.put(k, Done { pass: *pass, first: global.first().copied().unwrap_or(0), out });
            }
        };
        let lanes: Vec<usize> = (0..ctx.workers().max(1)).collect();
        ctx.map_indexed_with(Window::new().chunk(1).strategy(Strategy::Parallel), &lanes, |_, _| {
            loop {
                let k = {
                    let mut q = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    match q.0.pop_front() {
                        Some(k) => k,
                        None if q.1 == 0 => break,
                        None => {
                            drop(q);
                            std::thread::yield_now();
                            continue;
                        }
                    }
                };
                run(k);
                let mut q = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                q.1 -= 1;
                for &b in &succ[k] {
                    if preds[b].fetch_sub(1, std::sync::atomic::Ordering::AcqRel) == 1 {
                        q.0.push_back(b);
                    }
                }
            }
        });
        for (slot, e) in taken.iter_mut().zip(ents.into_inner()) {
            *slot = e;
        }
        for (slot, e) in stand_ins.iter_mut().zip(stands.into_inner()) {
            *slot = e;
        }
    }
    // The merge, pass by pass, as the passes would have left it.
    let mut done: Vec<Done> = results.into_inner().map(|d| d.expect("every group ran")).collect();
    let mut parts: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (k, d) in done.iter().enumerate() {
        parts.push((d.pass, d.first, k, 0));
        parts.extend(d.out.marks.iter().enumerate().map(|(j, m)| (d.pass, m.global, k, j + 1)));
    }
    parts.sort_unstable();
    for (_, _, k, j) in parts {
        let o = &mut done[k].out;
        merge_turn(sim, o, j);
    }
}

/// Whether two ascending lists have an element in common.
fn sorted_meet(a: &[usize], b: &[usize]) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

/// Moves turn `j`'s outputs (0: before the first turn) of a group into the region.
fn merge_turn(sim: &mut SimLevel, o: &mut Outputs, j: usize) {
    let end = |f: fn(&Mark) -> usize, all: usize| o.marks.get(j).map_or(all, f);
    let start = |f: fn(&Mark) -> usize| if j == 0 { 0 } else { f(&o.marks[j - 1]) };
    let n = |f: fn(&Mark) -> usize, all: usize| end(f, all) - start(f);
    let (sp, de, ev, df, pk) = (
        n(|m| m.spawns, usize::MAX),
        n(|m| m.deaths, usize::MAX),
        n(|m| m.events, usize::MAX),
        n(|m| m.deferred, usize::MAX),
        n(|m| m.packets, usize::MAX),
    );
    sim.spawns.extend(o.spawns.by_ref().take(sp));
    sim.deaths.extend(o.deaths.by_ref().take(de));
    sim.events.extend(o.events.by_ref().take(ev));
    let region = sim.level.region().expect("a region");
    for f in o.deferred.by_ref().take(df) {
        f(region);
    }
    region.out.packets.extend(o.packets.by_ref().take(pk));
}

/// A group's outputs, taken turn by turn.
struct Outputs {
    marks: Vec<Mark>,
    spawns: std::vec::IntoIter<Spawn>,
    deaths: std::vec::IntoIter<health::Death>,
    events: std::vec::IntoIter<Event>,
    deferred: std::vec::IntoIter<Deferred>,
    packets: std::vec::IntoIter<([f64; 3], f64, Bytes)>,
}

/// One island's turns.
fn run_island(job: &mut Job, sh: &Shared) {
    let _enchanting = crate::enchant::install_enchanter(sh.env.loot.as_ref());
    let Job { global, list, players, proxies, views, placeholder, spawns, deaths, events, deferred, packets, marks, ticks, .. } = job;
    let mut sim = SimLevel {
        level: World::Island(IslandWorld::new(sh.cells, sh.env, sh.pistons)),
        list,
        players: players.as_mut_slice(),
        deaths,
        proxies: std::mem::take(proxies),
        views: std::mem::take(views),
        spawns,
        events: Vec::new(),
        next_placeholder: *placeholder,
        current: 0,
        seeds: 0,
        current_source: None,
        rng: LegacyRandom::new(0),
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: Some(sh.nearest),
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    tick_list(&mut sim, sh.ticking, sh.any_player, &mut |sim, i| {
        if ticks.as_ref().is_some_and(|t| !t[i]) {
            return false;
        }
        let World::Island(w) = &sim.level else { return true };
        marks.push(Mark {
            global: global[i],
            spawns: sim.spawns.len(),
            deaths: sim.deaths.len(),
            events: sim.events.len(),
            deferred: w.deferred.len(),
            packets: w.packets.len(),
        });
        true
    });
    let SimLevel { level, events: made, proxies: moved, .. } = sim;
    let World::Island(w) = level else { unreachable!("an island's level") };
    *events = made;
    *deferred = w.deferred;
    *packets = w.packets;
    *proxies = moved;
}
