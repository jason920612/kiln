//! A brute-force reference model of the regionizer and a harness that drives both the
//! real regionizer (with parts holding entities, messages and counters) and the model.
#![allow(dead_code)]

use kiln_region::{
    CellHooks, CellPos, FusePin, FuseReason, Inbox, MaxCounters, MsgKey, RegionId, RegionPolicy, Regionizer, Regions,
    TickList, TopologyDelta, TopologyEvent,
};
use std::collections::{BTreeMap, BTreeSet};

/// Same semantics as the regionizer, different representation: a flat cell map, globally
/// unique label tokens instead of per-region label vectors, O(n²) component search.
pub struct Model {
    policy: RegionPolicy,
    next_id: u64,
    next_label: u64,
    cells: BTreeMap<CellPos, MCell>,
    since: BTreeMap<u64, u64>,
    groups: BTreeMap<u64, MGroup>,
}

#[derive(Clone, Copy)]
struct MCell {
    group: u64,
    label: Option<u64>,
}

#[derive(Default)]
struct MGroup {
    pins: BTreeMap<(CellPos, CellPos, FuseReason), u64>,
    dirty: bool,
}

impl Model {
    pub fn new(policy: RegionPolicy) -> Self {
        Self {
            policy,
            next_id: 1,
            next_label: 0,
            cells: BTreeMap::new(),
            since: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }

    pub fn owner(&self, p: CellPos) -> Option<RegionId> {
        self.cells.get(&p).map(|c| RegionId(c.group))
    }

    pub fn cells(&self) -> impl Iterator<Item = (CellPos, RegionId)> + '_ {
        self.cells.iter().map(|(&p, c)| (p, RegionId(c.group)))
    }

    pub fn groups(&self) -> Vec<RegionId> {
        self.groups.keys().map(|&g| RegionId(g)).collect()
    }

    pub fn pins(&self, g: RegionId) -> Vec<FusePin> {
        self.groups[&g.0].pins.iter().map(|(&(a, b, reason), &until)| FusePin { a, b, reason, until_tick: until }).collect()
    }

    fn cells_of(&self, g: u64) -> Vec<CellPos> {
        self.cells.iter().filter(|(_, c)| c.group == g).map(|(&p, _)| p).collect()
    }

    fn anchor(&self, g: u64) -> CellPos {
        *self.cells.iter().find(|(_, c)| c.group == g).unwrap().0
    }

    fn fresh_label(&mut self, since: u64) -> u64 {
        let l = self.next_label;
        self.next_label += 1;
        self.since.insert(l, since);
        l
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn apply(&mut self, events: &[TopologyEvent], tick: u64) -> Vec<TopologyDelta> {
        let mut out = Vec::new();
        let mut net = BTreeMap::new();
        for e in events {
            match *e {
                TopologyEvent::Occupied(p) => net.insert(p, true),
                TopologyEvent::Vacated(p) => net.insert(p, false),
                _ => None,
            };
        }
        for (&p, _) in net.iter().filter(|e| !*e.1) {
            self.vacate(p, &mut out);
        }
        for (&p, _) in net.iter().filter(|e| *e.1) {
            self.occupy(p, &mut out);
        }
        for e in events {
            match *e {
                TopologyEvent::Fuse(pin) => self.fuse(pin, tick, &mut out),
                TopologyEvent::Expire(r) => self.drop_pins(|k, _| k.2 == r),
                _ => {}
            }
        }
        self.drop_pins(|_, until| until <= tick);
        if !self.policy.unified {
            let period = self.policy.split_period;
            let due: Vec<u64> =
                self.groups.iter().filter(|(id, g)| g.dirty && tick % period == *id % period).map(|(&id, _)| id).collect();
            for g in due {
                self.check(g, tick, &mut out);
            }
        }
        out
    }

    fn vacate(&mut self, p: CellPos, out: &mut Vec<TopologyDelta>) {
        let Some(c) = self.cells.remove(&p) else { return };
        let g = self.groups.get_mut(&c.group).unwrap();
        g.pins.retain(|k, _| k.0 != p && k.1 != p);
        g.dirty = true;
        if !self.cells.values().any(|x| x.group == c.group) {
            self.groups.remove(&c.group);
            out.push(TopologyDelta::Dead(RegionId(c.group)));
        }
    }

    fn occupy(&mut self, p: CellPos, out: &mut Vec<TopologyDelta>) {
        if self.cells.contains_key(&p) {
            return;
        }
        let link = self.policy.link_cheb as u32;
        let unified = self.policy.unified;
        let near: BTreeSet<u64> =
            self.cells.iter().filter(|(q, _)| unified || q.cheb(p) <= link).map(|(_, c)| c.group).collect();
        let g = match self.merge(near.into_iter().collect(), out) {
            Some(g) => g,
            None => {
                let id = self.new_id();
                self.groups.insert(id, MGroup::default());
                out.push(TopologyDelta::Created(RegionId(id)));
                id
            }
        };
        self.cells.insert(p, MCell { group: g, label: None });
    }

    fn merge(&mut self, mut gs: Vec<u64>, out: &mut Vec<TopologyDelta>) -> Option<u64> {
        gs.sort_by_key(|&g| self.anchor(g));
        let (&into, from) = gs.split_first()?;
        for &f in from {
            for c in self.cells.values_mut() {
                if c.group == f {
                    c.group = into;
                }
            }
            let fg = self.groups.remove(&f).unwrap();
            let tg = self.groups.get_mut(&into).unwrap();
            for (k, u) in fg.pins {
                let e = tg.pins.entry(k).or_insert(u);
                *e = (*e).max(u);
            }
            tg.dirty |= fg.dirty;
        }
        if !from.is_empty() {
            out.push(TopologyDelta::Merged { into: RegionId(into), from: from.iter().map(|&f| RegionId(f)).collect() });
        }
        Some(into)
    }

    fn fuse(&mut self, pin: FusePin, tick: u64, out: &mut Vec<TopologyDelta>) {
        let (a, b) = if pin.b < pin.a { (pin.b, pin.a) } else { (pin.a, pin.b) };
        if pin.until_tick <= tick {
            return;
        }
        let (Some(ca), Some(cb)) = (self.cells.get(&a).copied(), self.cells.get(&b).copied()) else { return };
        let mut gs = vec![ca.group];
        if cb.group != ca.group {
            gs.push(cb.group);
        }
        let g = self.merge(gs, out).unwrap();
        let e = self.groups.get_mut(&g).unwrap().pins.entry((a, b, pin.reason)).or_insert(pin.until_tick);
        *e = (*e).max(pin.until_tick);
    }

    fn drop_pins(&mut self, f: impl Fn(&(CellPos, CellPos, FuseReason), u64) -> bool) {
        for g in self.groups.values_mut() {
            let n = g.pins.len();
            g.pins.retain(|k, u| !f(k, *u));
            if g.pins.len() != n {
                g.dirty = true;
            }
        }
    }

    fn check(&mut self, g: u64, tick: u64, out: &mut Vec<TopologyDelta>) {
        let cells = self.cells_of(g);
        let pins: Vec<(CellPos, CellPos)> = self.groups[&g].pins.keys().map(|k| (k.0, k.1)).collect();
        let link = self.policy.link_cheb as u32;
        let linked = |a: CellPos, b: CellPos| a.cheb(b) <= link || pins.contains(&(a, b)) || pins.contains(&(b, a));
        let mut comp: BTreeMap<CellPos, usize> = BTreeMap::new();
        let mut m = 0;
        for &start in &cells {
            if comp.contains_key(&start) {
                continue;
            }
            comp.insert(start, m);
            let mut stack = vec![start];
            while let Some(x) = stack.pop() {
                for &y in &cells {
                    if !comp.contains_key(&y) && linked(x, y) {
                        comp.insert(y, m);
                        stack.push(y);
                    }
                }
            }
            m += 1;
        }
        if m == 1 {
            let l = self.fresh_label(tick);
            for p in &cells {
                self.cells.get_mut(p).unwrap().label = Some(l);
            }
            self.groups.get_mut(&g).unwrap().dirty = false;
            return;
        }
        let mut since_of = Vec::new();
        for k in 0..m {
            let labels: BTreeSet<u64> = cells.iter().filter(|p| comp[p] == k).filter_map(|p| self.cells[p].label).collect();
            let spanning =
                labels.iter().any(|&l| cells.iter().any(|q| comp[q] != k && self.cells[q].label == Some(l)));
            since_of.push(if spanning || labels.is_empty() {
                tick
            } else {
                labels.iter().map(|l| self.since[l]).max().unwrap()
            });
        }
        let leaving: Vec<usize> = (1..m).filter(|&k| tick - since_of[k] >= self.policy.split_hysteresis).collect();
        let staying: Vec<usize> = (0..m).filter(|k| !leaving.contains(k)).collect();
        for &k in &staying {
            let l = self.fresh_label(since_of[k]);
            for p in cells.iter().filter(|p| comp[p] == k) {
                self.cells.get_mut(p).unwrap().label = Some(l);
            }
        }
        self.groups.get_mut(&g).unwrap().dirty = staying.len() > 1;
        if leaving.is_empty() {
            return;
        }
        let mut new_ids = Vec::new();
        for &k in &leaving {
            let id = self.new_id();
            new_ids.push(RegionId(id));
            let l = self.fresh_label(tick);
            for p in cells.iter().filter(|p| comp[p] == k) {
                *self.cells.get_mut(p).unwrap() = MCell { group: id, label: Some(l) };
            }
            self.groups.insert(id, MGroup::default());
        }
        let pins = std::mem::take(&mut self.groups.get_mut(&g).unwrap().pins);
        for (k, u) in pins {
            let owner = self.cells[&k.0].group;
            self.groups.get_mut(&owner).unwrap().pins.insert(k, u);
        }
        out.push(TopologyDelta::Split { from: RegionId(g), into: new_ids.into_iter().collect() });
    }
}

pub struct CellData {
    pub pos: CellPos,
}

/// Tick list values are `seq * 31` so a value separated from its entry is detected.
pub type Parts = (TickList<u64>, Inbox<u64>, MaxCounters<1>);

#[derive(Default)]
pub struct Hooks {
    pub created: u64,
    pub retired: u64,
}

impl CellHooks<CellData> for Hooks {
    fn create(&mut self, pos: CellPos) -> Box<CellData> {
        self.created += 1;
        Box::new(CellData { pos })
    }
    fn retire(&mut self, pos: CellPos, cell: Box<CellData>) {
        assert_eq!(cell.pos, pos, "retired payload belongs to another cell");
        self.retired += 1;
    }
}

pub const REASONS: [FuseReason; 3] = [FuseReason::Explosion, FuseReason::GlobalConflict, FuseReason::Operator];

/// Plays the sim: queues events, keeps entities and messages in the region parts
/// (honouring the vacate contract) and checks everything after every apply.
pub struct Sim {
    pub rz: Regionizer,
    pub regions: Regions<CellData, Parts>,
    pub hooks: Hooks,
    pub model: Option<Model>,
    pub tick: u64,
    queued: Vec<TopologyEvent>,
    next_seq: u64,
    entities: BTreeMap<u64, CellPos>,
    letters: BTreeMap<MsgKey, CellPos>,
    next_msg: u32,
    counters: BTreeMap<RegionId, u64>,
    vacating: BTreeSet<CellPos>,
    pub log: Vec<(u64, TopologyDelta)>,
}

impl Sim {
    pub fn new(policy: RegionPolicy, hash_seed: u64, with_model: bool) -> Self {
        Self {
            rz: Regionizer::new(policy),
            regions: Regions::with_hash_seed(hash_seed),
            hooks: Hooks::default(),
            model: with_model.then(|| Model::new(policy)),
            tick: 0,
            queued: Vec::new(),
            next_seq: 0,
            entities: BTreeMap::new(),
            letters: BTreeMap::new(),
            next_msg: 0,
            counters: BTreeMap::new(),
            vacating: BTreeSet::new(),
            log: Vec::new(),
        }
    }

    fn push(&mut self, e: TopologyEvent) {
        self.rz.push(e);
        self.queued.push(e);
    }

    /// Owned cells that may receive elements, in cell order.
    pub fn live_cells(&self) -> Vec<CellPos> {
        let mut v: Vec<CellPos> = self
            .regions
            .iter()
            .flat_map(|r| r.cells().positions())
            .filter(|p| !self.vacating.contains(p))
            .collect();
        v.sort();
        v
    }

    pub fn region_ids(&self) -> Vec<RegionId> {
        self.regions.iter().map(|r| r.id()).collect()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn entity_seqs(&self) -> Vec<u64> {
        self.entities.keys().copied().collect()
    }

    pub fn occupy(&mut self, p: CellPos) {
        self.push(TopologyEvent::Occupied(p));
    }

    /// Removes the cell's elements (the sim's unload path), then queues the event.
    pub fn vacate(&mut self, p: CellPos) {
        if let Some(r) = self.regions.at_mut(p) {
            let (tl, inbox, _) = r.part_mut();
            tl.retain(|e| e.cell != p);
            inbox.retain(|l| l.target != p);
            self.entities.retain(|_, c| *c != p);
            self.letters.retain(|_, t| *t != p);
        }
        self.vacating.insert(p);
        self.push(TopologyEvent::Vacated(p));
    }

    pub fn spawn(&mut self, p: CellPos) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let r = self.regions.at_mut(p).expect("spawn into an owned cell");
        assert!(r.part_mut().0.insert(seq, p, seq * 31));
        self.entities.insert(seq, p);
    }

    /// Moves an entity to `to`, across regions if needed, keeping its sequence number.
    pub fn move_entity(&mut self, seq: u64, to: CellPos) {
        let from = self.entities[&seq];
        let (src, dst) = (self.regions.owner(from).unwrap(), self.regions.owner(to).expect("move into an owned cell"));
        if src == dst {
            self.regions.get_mut(src).unwrap().part_mut().0.get_mut(seq).unwrap().cell = to;
        } else {
            let e = self.regions.get_mut(src).unwrap().part_mut().0.remove(seq).unwrap();
            assert!(self.regions.get_mut(dst).unwrap().part_mut().0.insert(seq, to, e.value));
        }
        self.entities.insert(seq, to);
    }

    pub fn despawn(&mut self, seq: u64) {
        let cell = self.entities.remove(&seq).unwrap();
        self.regions.at_mut(cell).unwrap().part_mut().0.remove(seq).unwrap();
    }

    pub fn send(&mut self, src: CellPos, target: CellPos) {
        let key = MsgKey { src_tick: self.tick, src_cell: src, seq: self.next_msg };
        self.next_msg += 1;
        let r = self.regions.at_mut(target).expect("send to an owned cell");
        assert!(r.part_mut().1.push(key, target, key.seq as u64 * 7));
        self.letters.insert(key, target);
    }

    /// Delivers a region's inbox: every letter must target one of its cells, in key order.
    pub fn deliver(&mut self, id: RegionId) {
        let r = self.regions.get_mut(id).unwrap();
        let mut last = None;
        let letters: Vec<_> = r.part_mut().1.drain().collect();
        for l in letters {
            assert!(last < Some(l.key), "inbox out of key order");
            last = Some(l.key);
            assert_eq!(self.regions.owner(l.target), Some(id), "letter delivered by a region not owning its target");
            assert_eq!(self.letters.remove(&l.key), Some(l.target), "unknown or duplicated letter");
            assert_eq!(l.body, l.key.seq as u64 * 7);
        }
    }

    pub fn bump(&mut self, id: RegionId) {
        let c = &mut self.regions.get_mut(id).unwrap().part_mut().2.0[0];
        *c += 1;
        self.counters.insert(id, *c);
    }

    pub fn fuse(&mut self, a: CellPos, b: CellPos, reason: FuseReason, until_tick: u64) {
        self.push(TopologyEvent::Fuse(FusePin { a, b, reason, until_tick }));
    }

    pub fn expire(&mut self, reason: FuseReason) {
        self.push(TopologyEvent::Expire(reason));
    }

    pub fn step(&mut self) {
        self.tick += 1;
        let deltas = self.rz.apply(&mut self.regions, self.tick, &mut self.hooks);
        if let Some(model) = &mut self.model {
            let expected = model.apply(&self.queued, self.tick);
            assert_eq!(deltas.as_slice(), expected.as_slice(), "deltas differ from the model at tick {}", self.tick);
        }
        self.queued.clear();
        self.vacating.clear();
        for d in &deltas {
            match d {
                TopologyDelta::Created(id) => {
                    self.counters.insert(*id, 0);
                }
                TopologyDelta::Merged { into, from } => {
                    let m = from.iter().map(|f| self.counters.remove(f).unwrap()).max().unwrap();
                    let c = self.counters.get_mut(into).unwrap();
                    *c = (*c).max(m);
                }
                TopologyDelta::Split { from, into } => {
                    let c = self.counters[from];
                    into.iter().for_each(|i| {
                        self.counters.insert(*i, c);
                    });
                }
                TopologyDelta::Dead(id) => {
                    self.counters.remove(id);
                }
            }
            self.log.push((self.tick, d.clone()));
        }
        self.verify();
    }

    pub fn verify(&self) {
        let t = self.tick;
        if let Err(e) = self.rz.check_invariants(&self.regions) {
            panic!("tick {t}: {e}");
        }
        if let Some(model) = &self.model {
            assert_eq!(self.region_ids(), model.groups(), "tick {t}: region sets differ");
            assert_eq!(self.regions.table().len(), model.cells().count(), "tick {t}: cell count differs");
            for (p, g) in model.cells() {
                assert_eq!(self.regions.owner(p), Some(g), "tick {t}: owner of {p:?} differs");
            }
            for r in self.regions.iter() {
                assert_eq!(r.pins(), model.pins(r.id()).as_slice(), "tick {t}: pins of {:?} differ", r.id());
            }
        }
        let mut entities = BTreeMap::new();
        let mut letters = BTreeMap::new();
        for r in self.regions.iter() {
            for (p, d) in r.cells().iter() {
                assert_eq!(d.pos, p, "tick {t}: payload of {p:?} is in the wrong slot");
            }
            let (tl, inbox, counters) = r.part();
            let mut last = None;
            for e in tl.iter() {
                assert!(last < Some(e.seq), "tick {t}: tick list of {:?} out of seq order", r.id());
                last = Some(e.seq);
                assert_eq!(self.regions.owner(e.cell), Some(r.id()), "tick {t}: entity {} in the wrong region", e.seq);
                assert_eq!(e.value, e.seq * 31);
                assert!(entities.insert(e.seq, e.cell).is_none(), "tick {t}: entity {} duplicated", e.seq);
            }
            let mut last = None;
            for l in inbox.iter() {
                assert!(last < Some(l.key), "tick {t}: inbox of {:?} out of key order", r.id());
                last = Some(l.key);
                assert_eq!(self.regions.owner(l.target), Some(r.id()), "tick {t}: letter routed to the wrong region");
                assert!(letters.insert(l.key, l.target).is_none(), "tick {t}: letter duplicated");
            }
            assert_eq!(counters.0[0], self.counters[&r.id()], "tick {t}: counter of {:?}", r.id());
        }
        assert_eq!(entities, self.entities, "tick {t}: entities lost or invented");
        assert_eq!(letters, self.letters, "tick {t}: letters lost or invented");
        assert_eq!(self.counters.len(), self.regions.len());
    }

    /// A digest of the whole state, for determinism comparisons.
    pub fn digest(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        for r in self.regions.iter() {
            let (tl, inbox, counters) = r.part();
            write!(s, "{:?} anchor {:?} cells {:?} pins {:?} counters {:?}", r.id(), r.anchor(), r.cells().positions().collect::<Vec<_>>(), r.pins(), counters.0).unwrap();
            write!(s, " entities {:?}", tl.iter().map(|e| (e.seq, e.cell)).collect::<Vec<_>>()).unwrap();
            writeln!(s, " letters {:?}", inbox.iter().map(|l| (l.key, l.target)).collect::<Vec<_>>()).unwrap();
        }
        s
    }
}

/// xorshift64*: deterministic randomness for scripted scenarios.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }

    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
}

/// Players wandering over a bounded area; each keeps the cells within `radius` occupied.
/// Entities spawn near players and wander; messages fly between cells; pins come and go.
pub fn walkers(sim: &mut Sim, seed: u64, players: usize, ticks: u64, area: i32, radius: i32) {
    let mut rng = Rng::new(seed);
    let mut pos: Vec<(f64, f64)> =
        (0..players).map(|_| (rng.range(-area, area) as f64, rng.range(-area, area) as f64)).collect();
    let mut vel: Vec<(f64, f64)> = vec![(0.0, 0.0); players];
    let mut occupied: BTreeSet<CellPos> = BTreeSet::new();
    for _ in 0..ticks {
        for i in 0..players {
            if rng.chance(0.02) {
                let speed = [0.0, 0.02, 0.05, 0.2][rng.below(4) as usize];
                let a = rng.below(360) as f64 * std::f64::consts::PI / 180.0;
                vel[i] = (a.cos() * speed, a.sin() * speed);
            }
            if rng.chance(0.001) {
                pos[i] = (rng.range(-area, area) as f64, rng.range(-area, area) as f64);
            }
            let lim = area as f64;
            pos[i].0 = (pos[i].0 + vel[i].0).clamp(-lim, lim);
            pos[i].1 = (pos[i].1 + vel[i].1).clamp(-lim, lim);
        }
        let mut want = BTreeSet::new();
        for &(x, z) in &pos {
            let c = CellPos::new(x.floor() as i32, z.floor() as i32);
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    want.insert(c.offset(dx, dz));
                }
            }
        }
        for &p in occupied.difference(&want) {
            sim.vacate(p);
        }
        for &p in want.difference(&occupied) {
            sim.occupy(p);
        }
        occupied = want;

        let live = sim.live_cells();
        if !live.is_empty() {
            for _ in 0..rng.below(3) {
                let p = live[rng.below(live.len() as u64) as usize];
                sim.spawn(p);
            }
            let seqs = sim.entity_seqs();
            if !seqs.is_empty() && rng.chance(0.7) {
                let seq = seqs[rng.below(seqs.len() as u64) as usize];
                if rng.chance(0.1) {
                    sim.despawn(seq);
                } else {
                    sim.move_entity(seq, live[rng.below(live.len() as u64) as usize]);
                }
            }
            if rng.chance(0.3) {
                let (a, b) = (live[rng.below(live.len() as u64) as usize], live[rng.below(live.len() as u64) as usize]);
                sim.send(a, b);
            }
            if rng.chance(0.05) {
                let ids = sim.region_ids();
                sim.deliver(ids[rng.below(ids.len() as u64) as usize]);
            }
            if rng.chance(0.05) {
                let ids = sim.region_ids();
                sim.bump(ids[rng.below(ids.len() as u64) as usize]);
            }
            if rng.chance(0.01) {
                let (a, b) = (live[rng.below(live.len() as u64) as usize], live[rng.below(live.len() as u64) as usize]);
                let reason = REASONS[rng.below(3) as usize];
                sim.fuse(a, b, reason, sim.tick + 1 + rng.below(300));
            }
            if rng.chance(0.002) {
                sim.expire(REASONS[rng.below(3) as usize]);
            }
        }
        sim.step();
    }
}
