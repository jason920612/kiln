//! Independent scheduling (design §4.3 `Independent`, REG-02): a region too slow for the tick
//! leaves the lockstep and ticks on its own thread, so the other regions keep 20 TPS.
//!
//! # The model
//! In lockstep every region ticks once per server tick and the tick lasts as long as the
//! slowest region. In independent mode a region whose tick time (EMA) goes over
//! [`DETACH_MS`] is *detached*: at the L phase of a tick it is **lent out**, its cells, part
//! and players move to a thread of their own and it runs its L tick there while the server
//! goes on. At the start of every server tick, lent regions that finished come back, and a
//! detached region spends that tick at home like any other (B0, its packets in P, PX, G)
//! before it is lent again. It keeps ticking detached until its EMA drops under
//! [`REJOIN_MS`]. A detached region ticks as fast as it can, at most once per server tick;
//! the other regions do not wait for it.
//!
//! **Clock (REG-02).** A lent region misses the server ticks that pass while it is away. When
//! it comes back its scheduled block and fluid ticks move later by the ticks it missed, so
//! what was due in n of its own ticks is still due in n of its ticks; everything else in the
//! region reads the server's time again (day time, game time stamps jump forward).
//!
//! **Rendezvous.** Whatever needs the whole server waits until every lent region is back
//! (and then runs as in lockstep): joins and leaves, console commands, chat and commands
//! from any player (their broadcasts and selectors reach everyone), datapack functions due,
//! autosave and shutdown, topology changes in a level with a lent region, and portal trips.
//! Broadcasts sent while a region is away (time, deaths, the tab list) are kept for its
//! players. A server running `#minecraft:tick` functions or plugins never detaches regions
//! (they would need a rendezvous every tick, Q27).
//!
//! **Not deterministic.** How many ticks a lent region misses depends on wall time, so
//! independent mode is not reproducible and state hashes vary; lockstep stays the default
//! and the determinism tests run in lockstep. Inspecting the world (`state_hash`,
//! `block_at`, ...) sees lent regions as absent: call [`Sim::rendezvous`] first.

use crate::entities::Entities;
use crate::portal::Travel;
use crate::region::{Env, RegionOut, RegionWork};
use crate::{DimId, Player, Sim, blocks::RegionBlocks};
use bytes::Bytes;
use kiln_link::{ConnId, PlayIn};
use kiln_region::{CellSet, RegionId};
use kiln_world::{Cell, ChunkPos};
use std::collections::HashMap;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// How regions are scheduled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScheduleMode {
    /// Every region ticks once per server tick; deterministic.
    #[default]
    Lockstep,
    /// Regions too slow for the tick tick on their own; not deterministic.
    Independent,
}

/// Test hook: a region holding the column `x`, `z` of level `dimension` sleeps `delay` at
/// the start of each of its ticks.
#[derive(Clone, Debug)]
pub struct InjectedDelay {
    /// Level key, e.g. `minecraft:overworld`.
    pub dimension: String,
    pub x: i32,
    pub z: i32,
    pub delay: Duration,
}

impl InjectedDelay {
    pub(crate) fn delay_for(&self, dim: DimId, cells: &CellSet<Cell>) -> Duration {
        let here = crate::dim_id(&self.dimension) == Some(dim) && cells.contains(ChunkPos::of_block(self.x, self.z).cell());
        if here { self.delay } else { Duration::ZERO }
    }
}

/// A region whose tick time EMA goes over this leaves the lockstep...
pub const DETACH_MS: f64 = 30.0;
/// ...and rejoins once it is back under this.
pub const REJOIN_MS: f64 = 15.0;

#[derive(Default)]
struct Clock {
    /// Tick time EMA in milliseconds.
    ema_ms: f64,
    detached: bool,
    /// The region's own tick count.
    local_tick: u64,
}

/// What a lent region brings back.
struct Returned {
    cells: CellSet<Cell>,
    part: (Entities, RegionBlocks),
    players: Vec<Player>,
    out: RegionOut,
    elapsed: Duration,
}

struct Lent {
    dim: DimId,
    id: RegionId,
    /// Server tick number it left at.
    left_at: u64,
    /// Broadcasts for its players while away.
    mail: Vec<Bytes>,
    handle: JoinHandle<Returned>,
}

#[derive(Default)]
pub(crate) struct Independent {
    lent: Vec<Lent>,
    clocks: HashMap<(DimId, RegionId), Clock>,
    /// Players of lent regions, for packet routing.
    away: HashMap<ConnId, (DimId, RegionId)>,
    /// Packets of players that were away when they arrived, in arrival order.
    deferred: Vec<(ConnId, PlayIn)>,
    /// Portal trips of regions that came back, run once everyone is back.
    travels: Vec<Travel>,
    /// Server ticks so far.
    steps: u64,
    /// Rendezvous so far and the time spent waiting in them.
    rendezvous: u64,
    waited: Duration,
    /// Regions lent out so far.
    lends: u64,
    fallback_logged: bool,
    /// Wall time of each region in the last lockstep fork (filled by `run_regions`).
    pub last_fork: Vec<(DimId, RegionId, u64)>,
}

impl Independent {
    /// Keeps a broadcast for the players of every lent region.
    pub fn mail(&mut self, pkt: &Bytes) {
        for l in &mut self.lent {
            l.mail.push(pkt.clone());
        }
    }

    pub fn players_away(&self) -> usize {
        self.away.len()
    }

    fn note(&mut self, key: (DimId, RegionId), ms: f64, independent: bool) {
        let c = self.clocks.entry(key).or_default();
        c.local_tick += 1;
        c.ema_ms = if c.local_tick == 1 { ms } else { 0.5 * c.ema_ms + 0.5 * ms };
        c.detached = independent && if c.detached { c.ema_ms > REJOIN_MS } else { c.ema_ms > DETACH_MS };
    }
}

impl Sim {
    /// Whether regions may leave the lockstep now: independent mode, and no consumer that
    /// needs the whole server every tick (plugins, `#minecraft:tick` functions).
    fn may_detach(&mut self) -> bool {
        if self.config.schedule != ScheduleMode::Independent {
            return false;
        }
        let blocker = if self.plugins.is_some() {
            Some("plugins are loaded")
        } else if self.has_tick_functions() {
            Some("#minecraft:tick functions run every tick")
        } else {
            None
        };
        if let Some(why) = blocker {
            if !std::mem::replace(&mut self.independent.fallback_logged, true) {
                info!("independent scheduling falls back to lockstep: {why}");
            }
            return false;
        }
        true
    }

    /// B0 in independent mode: brings back lent regions that finished; waits for all of them
    /// if this tick needs the whole server; returns the tick's packets for players at home
    /// (with those deferred while they were away first) and defers the rest.
    pub(crate) fn independent_b0(&mut self, packets: Vec<(ConnId, PlayIn)>, server_events: bool) -> Vec<(ConnId, PlayIn)> {
        self.independent.steps += 1;
        if self.independent.lent.is_empty() && self.independent.deferred.is_empty() {
            return packets;
        }
        self.collect_lent(false);
        let exclusive = packets.iter().chain(&self.independent.deferred).any(|(_, p)| crate::region::is_exclusive(p));
        let autosave = (self.game_time + 1) % crate::AUTOSAVE_TICKS == 0;
        let travels = !self.independent.travels.is_empty();
        if server_events || exclusive || autosave || travels || self.functions_due(self.game_time + 1) {
            self.rendezvous();
        }
        let mut all = std::mem::take(&mut self.independent.deferred);
        all.extend(packets);
        let (home, away): (Vec<_>, Vec<_>) = all.into_iter().partition(|(c, _)| !self.independent.away.contains_key(c));
        self.independent.deferred = away;
        home
    }

    /// Waits for lent regions if a level with one has topology changes queued.
    pub(crate) fn rendezvous_for_topology(&mut self) {
        let waiting = self.dims.iter().any(|d| !d.lent.is_empty() && (!d.regionizer.pending().is_empty() || !d.emptied.is_empty()));
        if waiting {
            self.rendezvous();
        }
    }

    /// Brings every lent region back (waiting for those still ticking) and runs the portal
    /// trips they brought. Afterwards the server is as in lockstep.
    pub fn rendezvous(&mut self) {
        if self.independent.lent.is_empty() && self.independent.travels.is_empty() {
            // Lockstep: nothing is away.
            return;
        }
        if !self.independent.lent.is_empty() {
            let start = Instant::now();
            self.collect_lent(true);
            self.independent.rendezvous += 1;
            self.independent.waited += start.elapsed();
            self.stats.phase("rendezvous", start.elapsed());
        }
        let mut travels = std::mem::take(&mut self.independent.travels);
        travels.sort_unstable_by_key(|t| t.conn);
        for t in travels {
            self.travel(t);
        }
        self.materialize_spawns();
    }

    /// Regions ticking away right now.
    pub fn regions_away(&self) -> usize {
        self.independent.lent.len()
    }

    /// (rendezvous, time waited in them, regions lent out) so far.
    pub fn independent_stats(&self) -> (u64, Duration, u64) {
        (self.independent.rendezvous, self.independent.waited, self.independent.lends)
    }

    /// The own tick count of the region holding column `x`, `z` of `dimension` (lockstep:
    /// the server's ticks since the region formed).
    pub fn local_tick_at(&self, dimension: &str, x: i32, z: i32) -> Option<u64> {
        let dim = crate::dim_id(dimension)?;
        let id = self.dims[dim].regions.owner(ChunkPos::of_block(x, z).cell())?;
        self.independent.clocks.get(&(dim, id)).map(|c| c.local_tick)
    }

    /// Takes back lent regions: those that finished, or all of them (waiting) if `all`.
    fn collect_lent(&mut self, all: bool) {
        let lent = std::mem::take(&mut self.independent.lent);
        let (done, still): (Vec<Lent>, Vec<Lent>) = lent.into_iter().partition(|l| all || l.handle.is_finished());
        self.independent.lent = still;
        let mut done = done;
        // Back in a fixed order, so ids of what they spawned do not depend on thread timing
        // within one collection.
        done.sort_unstable_by_key(|l| (l.dim, l.id));
        for l in done {
            let r = match l.handle.join() {
                Ok(r) => r,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            self.bring_back(l.dim, l.id, l.left_at, l.mail, r);
        }
    }

    fn bring_back(&mut self, dim: DimId, id: RegionId, left_at: u64, mail: Vec<Bytes>, r: Returned) {
        let Returned { cells, mut part, players, out, elapsed } = r;
        // It ticked once (in the server tick it left at) and missed the ticks since.
        let missed = self.independent.steps.saturating_sub(left_at + 1) as i64;
        part.1.shift_time(missed);
        let d = &mut self.dims[dim];
        d.regions.get_mut(id).expect("a lent region keeps its id").restore(cells, part);
        d.lent.remove(&id);
        // Chunks loaded for its cells while it was away.
        let arrived: Vec<ChunkPos> = d.pending.keys().copied().filter(|p| d.regions.owner(p.cell()) == Some(id)).collect();
        for pos in arrived {
            if let Some(chunk) = d.pending.remove(&pos) {
                d.install(pos, chunk);
            }
        }
        d.requests.extend(out.wanted);
        d.unloads.extend(out.unload);
        d.spawns.extend(out.spawns);
        for mut p in players {
            self.independent.away.remove(&p.conn);
            p.outbox.extend(mail.iter().cloned());
            self.players.insert(p.conn, p);
        }
        self.independent.travels.extend(out.portals);
        self.announce_deaths(out.deaths);
        let independent = self.may_detach();
        self.independent.note((dim, id), elapsed.as_secs_f64() * 1e3, independent);
        self.stats.phase("away", elapsed);
    }

    /// L in independent mode: lends out the detached regions at home; they tick on their
    /// own threads while the lockstep regions tick in the fork.
    pub(crate) fn lend_slow_regions(&mut self) {
        if !self.may_detach() {
            // Back to lockstep (mode switched or a blocker appeared): clear detached flags.
            for c in self.independent.clocks.values_mut() {
                c.detached = false;
            }
            return;
        }
        let detached: Vec<(DimId, RegionId)> =
            self.independent.clocks.iter().filter(|(_, c)| c.detached).map(|(&k, _)| k).collect();
        let mut detached: Vec<_> =
            detached.into_iter().filter(|&(dim, id)| self.dims[dim].regions.get(id).is_some() && !self.dims[dim].lent.contains(&id)).collect();
        detached.sort_unstable();
        for (dim, id) in detached {
            self.lend(dim, id);
        }
    }

    fn lend(&mut self, dim: DimId, id: RegionId) {
        let env: Env = self.env(dim);
        let conns: Vec<ConnId> = self.players.values().filter(|p| p.dim == dim && p.region == id).map(|p| p.conn).collect();
        let mut players: Vec<Player> = conns.iter().filter_map(|c| self.players.remove(c)).collect();
        players.sort_unstable_by_key(|p| p.conn);
        for p in &players {
            self.independent.away.insert(p.conn, (dim, id));
        }
        let d = &mut self.dims[dim];
        let (cells, part) = d.regions.get_mut(id).expect("region").lend();
        d.lent.insert(id);
        let delay = self.config.inject_delay.as_ref().map_or(Duration::ZERO, |i| i.delay_for(dim, &cells));
        let spawned = std::thread::Builder::new().name("kiln-region-away".into()).spawn(move || {
            let start = Instant::now();
            let (mut cells, mut part, mut players) = (cells, part, players);
            let mut pool = kiln_sched::TickPool::new(1);
            let out = {
                let refs: Vec<&mut Player> = players.iter_mut().collect();
                let (entities, blocks) = (&mut part.0, &mut part.1);
                let mut work = RegionWork {
                    dim,
                    region: id,
                    cells: &mut cells,
                    entities,
                    blocks,
                    players: refs,
                    packets: Vec::new(),
                    plugins: None,
                    delay,
                    out: RegionOut::default(),
                };
                pool.serial(|ctx| work.tick(&env, ctx));
                work.out
            };
            Returned { cells, part, players, out, elapsed: start.elapsed() }
        });
        match spawned {
            Ok(handle) => {
                self.independent.lends += 1;
                let left_at = self.independent.steps;
                self.independent.lent.push(Lent { dim, id, left_at, mail: Vec::new(), handle });
            }
            Err(e) => {
                // No thread: the region is gone with it; nothing can recover that.
                panic!("cannot start a thread for a region ticking away: {e}");
            }
        }
    }

    /// After the lockstep fork: each region's tick time and own tick count.
    pub(crate) fn note_region_ticks(&mut self) {
        let independent = self.config.schedule == ScheduleMode::Independent;
        for (dim, id, ns) in std::mem::take(&mut self.independent.last_fork) {
            self.independent.note((dim, id), ns as f64 / 1e6, independent);
        }
        if self.independent.clocks.len() > 4 * self.region_count() + 64 {
            // Forget regions that no longer exist.
            let live: std::collections::HashSet<(DimId, RegionId)> =
                self.dims.iter().enumerate().flat_map(|(d, dim)| dim.regions.iter().map(move |r| (d, r.id()))).collect();
            self.independent.clocks.retain(|k, _| live.contains(k));
        }
        if let Some(l) = self.independent.lent.first()
            && self.independent.steps.saturating_sub(l.left_at) == 200
        {
            warn!("a region has been ticking away for 200 server ticks");
        }
    }
}
