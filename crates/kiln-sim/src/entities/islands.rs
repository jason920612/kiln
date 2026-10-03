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

use super::*;
use crate::entity_world::{Deferred, IslandWorld, World};
use kiln_sched::{Ctx, Strategy, Window};

/// Entities closer than this (blocks, on each axis) to each other or to a player share an
/// island: more than the reach of the entities' area queries (a 16-block sensor box reaches
/// 23 blocks along a diagonal).
const LINK: f64 = 24.0;
/// Regions with fewer entities tick them serially.
const MIN_ENTITIES: usize = 32;
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
/// Placeholder ids each island may hand out.
const PLACEHOLDERS: i32 = 100_000;

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
}

/// What every island reads.
struct Shared<'s> {
    cells: &'s kiln_region::CellSet<kiln_world::Cell>,
    env: &'s blocks::BlockEnv,
    ticking: &'s blocks::Ticking,
    /// The region's players, for `Mob.checkDespawn` (the nearest player decides).
    views: &'s [PlayerView],
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
        let Some(phys) = e.phys.as_ref() else { continue };
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

/// Splits `v` at the marks' offsets: what came before the first mark, then each turn's part.
fn split<T>(mut v: Vec<T>, offsets: impl DoubleEndedIterator<Item = usize>) -> Vec<Vec<T>> {
    let mut parts: Vec<Vec<T>> = offsets.rev().map(|o| v.split_off(o.min(v.len()))).collect();
    parts.push(v);
    parts.reverse();
    parts
}

/// Ticks the region's entities as islands, if the region qualifies; returns whether it did
/// (else the caller ticks them serially).
pub(super) fn tick_islands(sim: &mut SimLevel, ticking: &blocks::Ticking, any_player: bool, ctx: &Ctx<'_>) -> bool {
    if ctx.workers() < 2 || sim.list.len() < MIN_ENTITIES {
        return false;
    }
    let Some(region) = sim.level.region_ref() else { return false };
    if !region.blocks.hearts.is_empty() || crate::sculk::listening(region) || sim.list.iter().any(|e| SERIAL.contains(&e.kind.name)) {
        return false;
    }
    let islands = partition(sim.list, sim.players);
    if islands.len() < 2 {
        return false;
    }
    let n = sim.list.len();
    let base = sim.next_placeholder;
    let mut taken: Vec<Option<Entity>> = std::mem::take(sim.list).into_iter().map(Some).collect();
    let mut jobs: Vec<Job> = Vec::with_capacity(islands.len());
    {
        let SimLevel { level, players, proxies, views, .. } = &mut *sim;
        let region = level.region_ref().expect("a region");
        let shared = Shared {
            cells: &*region.cells,
            env: region.env,
            ticking,
            views: views.as_slice(),
            any_player,
        };
        let mut slots: Vec<Option<&mut Player>> = players.iter_mut().map(|p| Some(&mut **p)).collect();
        for (k, (ents, pls)) in islands.into_iter().enumerate() {
            let island_players: Vec<&mut Player> = pls.iter().map(|&j| slots[j].take().expect("a player in one island")).collect();
            let ids: Vec<i32> = island_players.iter().map(|p| p.entity_id).collect();
            jobs.push(Job {
                list: ents.iter().map(|&i| taken[i].take().expect("an entity in one island")).collect(),
                global: ents,
                proxies: proxies.iter().filter(|e| ids.contains(&e.id)).cloned().collect(),
                views: views.iter().filter(|v| ids.contains(&v.id)).copied().collect(),
                players: island_players,
                placeholder: base - k as i32 * PLACEHOLDERS,
                spawns: Vec::new(),
                deaths: Vec::new(),
                events: Vec::new(),
                deferred: Vec::new(),
                packets: Vec::new(),
                marks: Vec::new(),
            });
        }
        // Largest first, one island per chunk, so the long ones start early.
        jobs.sort_by_key(|j| std::cmp::Reverse(j.list.len()));
        ctx.map_mut_with(Window::new().chunk(1).strategy(Strategy::Parallel), &mut jobs, |_, job| run_island(job, &shared));
    }
    // Back in the region, in list order, with the islands' outputs in the order of the turns
    // that made them.
    let mut back: Vec<Option<Entity>> = (0..n).map(|_| None).collect();
    let mut parts: Vec<(usize, usize, usize)> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut outputs: Vec<(Vec<Vec<Spawn>>, Vec<Vec<health::Death>>, Vec<Vec<Event>>, Vec<Vec<Deferred>>, Vec<Vec<([f64; 3], f64, Bytes)>>)> = Vec::new();
    let mut proxies: Vec<kiln_entity::Entity> = Vec::new();
    for (k, job) in jobs.into_iter().enumerate() {
        for (g, e) in job.global.iter().zip(job.list) {
            back[*g] = Some(e);
        }
        proxies.extend(job.proxies);
        let marks = &job.marks;
        let first = job.global.first().copied().unwrap_or(0);
        parts.push((first, k, 0));
        parts.extend(marks.iter().enumerate().map(|(j, m)| (m.global, k, j + 1)));
        outputs.push((
            split(job.spawns, marks.iter().map(|m| m.spawns)),
            split(job.deaths, marks.iter().map(|m| m.deaths)),
            split(job.events, marks.iter().map(|m| m.events)),
            split(job.deferred, marks.iter().map(|m| m.deferred)),
            split(job.packets, marks.iter().map(|m| m.packets)),
        ));
    }
    *sim.list = back.into_iter().map(|e| e.expect("every entity ticked in an island")).collect();
    parts.sort_by_key(|&(g, k, j)| (g, k, j));
    for (_, k, j) in parts {
        let o = &mut outputs[k];
        sim.spawns.append(&mut o.0[j]);
        sim.deaths.append(&mut o.1[j]);
        sim.events.append(&mut o.2[j]);
        let region = sim.level.region().expect("a region");
        for f in std::mem::take(&mut o.3[j]) {
            f(region);
        }
        region.out.packets.append(&mut o.4[j]);
    }
    // Explosions push the players' stand-ins; the region's copies take what happened to them.
    for p in proxies {
        if let Some(i) = sim.proxies.iter().position(|q| q.id == p.id) {
            sim.proxies[i] = p;
        }
    }
    sim.next_placeholder = base - outputs.len() as i32 * PLACEHOLDERS;
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    true
}

/// One island's turns.
fn run_island(job: &mut Job, sh: &Shared) {
    let _enchanting = crate::enchant::install_enchanter(sh.env.loot.as_ref());
    let Job { global, list, players, proxies, views, placeholder, spawns, deaths, events, deferred, packets, marks } = job;
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
        despawn_views: Some(sh.views),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    tick_list(&mut sim, sh.ticking, sh.any_player, &mut |sim, i| {
        let World::Island(w) = &sim.level else { return };
        marks.push(Mark {
            global: global[i],
            spawns: sim.spawns.len(),
            deaths: sim.deaths.len(),
            events: sim.events.len(),
            deferred: w.deferred.len(),
            packets: w.packets.len(),
        });
    });
    let SimLevel { level, events: made, proxies: moved, .. } = sim;
    let World::Island(w) = level else { unreachable!("an island's level") };
    *events = made;
    *deferred = w.deferred;
    *packets = w.packets;
    *proxies = moved;
}
