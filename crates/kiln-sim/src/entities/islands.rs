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
//! neighbouring tiles; what they leave behind is merged per pass in list order.

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
const SERIAL: [&str; 9] = [
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
const TILE: f64 = 24.0;

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
    proxies: Vec<kiln_entity::Entity>,
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
/// cell of half the link distance are always close; nodes in neighbouring cells are compared
/// until one close pair (with an entity in it) joins the cells' sets.
fn partition(list: &[Entity], players: &[&mut Player]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let (n, m) = (list.len(), players.len());
    let pos = |k: usize| if k < n { list[k].pos } else { players[k - n].pos };
    let mut sets = Sets((0..(n + m) as u32).collect());
    const CELL: f64 = LINK / 2.0;
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
        for dx in -2..=2 {
            for dy in -2..=2 {
                for dz in -2..=2 {
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
}

/// The tiles of one tick, one batch per colour: each centre tile with its neighbourhood. An
/// entity is in its root vehicle's tile, so riders tick with their vehicle.
fn tile_batches(list: &[Entity], players: &[&mut Player]) -> Vec<Vec<Group>> {
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
    let mut at: Vec<(i32, i32)> = Vec::with_capacity(list.len());
    for i in 0..list.len() {
        let t = tile(list[root(i)].pos);
        ents.entry(t).or_default().push(i);
        at.push(t);
    }
    let mut pls: crate::FastMap<(i32, i32), Vec<usize>> = Default::default();
    for (j, p) in players.iter().enumerate() {
        pls.entry(tile(p.pos)).or_default().push(j);
    }
    let mut centres: Vec<(i32, i32)> = ents.keys().copied().collect();
    centres.sort_unstable();
    let mut batches: Vec<Vec<Group>> = Vec::new();
    for colour in 0..9 {
        let mut batch = Vec::new();
        for &c in centres.iter().filter(|c| c.0.rem_euclid(3) * 3 + c.1.rem_euclid(3) == colour) {
            let mut g = Group { ents: Vec::new(), players: Vec::new(), ticks: None };
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let t = (c.0 + dx, c.1 + dz);
                    g.ents.extend(ents.get(&t).into_iter().flatten());
                    g.players.extend(pls.get(&t).into_iter().flatten());
                }
            }
            g.ents.sort_unstable();
            g.players.sort_unstable();
            g.ticks = Some(g.ents.iter().map(|&i| at[i] == c).collect());
            batch.push(g);
        }
        if !batch.is_empty() {
            batches.push(batch);
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
    if !region.blocks.hearts.is_empty() || crate::sculk::listening(region) || sim.list.iter().any(|e| SERIAL.contains(&e.kind.name)) {
        return false;
    }
    let islands = partition(sim.list, sim.players);
    let n = sim.list.len();
    let largest = islands.iter().map(|g| g.0.len()).max().unwrap_or(0);
    let mut next = sim.next_placeholder;
    let batches = if islands.len() >= 2 && largest * ISLAND_SHARE <= n {
        vec![islands.into_iter().map(|(ents, players)| Group { ents, players, ticks: None }).collect()]
    } else {
        tile_batches(sim.list, sim.players)
    };
    // Which player each stand-in and view is.
    let player_at: crate::FastMap<i32, usize> = sim.players.iter().enumerate().map(|(j, p)| (p.entity_id, j)).collect();
    let mut taken: Vec<Option<Entity>> = std::mem::take(sim.list).into_iter().map(Some).collect();
    // The players' stand-ins go to their groups and come back, like the entities.
    let mut stand_ins: Vec<Option<kiln_entity::Entity>> = std::mem::take(&mut sim.proxies).into_iter().map(Some).collect();
    for batch in batches {
        run_batch(sim, &mut taken, &mut stand_ins, batch, &player_at, ticking, any_player, ctx, &mut next);
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
    stand_ins: &mut [Option<kiln_entity::Entity>],
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
        let shared = Shared { cells: &*region.cells, env: region.env, ticking, nearest, any_player };
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
        let o = &mut outs[k];
        // Turn `j` (0: before the first) ends where the next one starts.
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
        level: World::Island(IslandWorld::new(sh.cells, sh.env)),
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
