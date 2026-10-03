//! Riding stacks, leashes and open menus across cell and region boundaries (wp37).
//!
//! Regions are cells of 8x8 chunks that merge when occupied cells come within two cells of each
//! other and split again (lazily) when they drift apart. A vehicle with its passengers, a leashed
//! mob, a chest minecart whose menu a player has open: none of them may notice. Each scenario
//! runs on several topologies (one region for the level, one region per far-away group, several
//! workers under chaos scheduling) and every tick's digest, which covers all entities as saved
//! (positions, riding, leashes), the players' vehicles, open menus, positions and the packets
//! they were sent, must be the same. The scenarios drive the merges and splits themselves: a
//! second group of players far away comes close by teleport and goes again.
//!
//! Independent scheduling (`KILN_SCHEDULE=independent`) is not deterministic, so it gets
//! invariants instead of hashes (the last test).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::hash::{Hash, Hasher};

const SURFACE_Y: f64 = -60.0;

#[derive(Clone, Copy, Debug)]
struct Topo {
    workers: usize,
    unified: bool,
    chaos: Option<u64>,
}

/// One region for the level, one region per group, and parallel workers under chaos.
const TOPOS: [Topo; 3] = [
    Topo { workers: 1, unified: true, chaos: None },
    Topo { workers: 1, unified: false, chaos: None },
    Topo { workers: 4, unified: false, chaos: Some(7) },
];

/// What each tick's digest was made of, by part (for the message when two runs differ).
const PARTS: [&str; 5] = ["state hash", "entities as saved", "riding links", "players (vehicle, menu, position)", "packets received"];

#[derive(Default)]
struct Trace {
    digests: Vec<[u64; 5]>,
    regions: Vec<usize>,
    /// Packets received per packet id, per client, at every tick (to say what differs).
    by_id: Vec<Vec<std::collections::BTreeMap<i32, (u64, u64)>>>,
}

struct World {
    sim: Sim,
    clients: Vec<Client>,
    inbox: Vec<ToSim>,
    trace: Trace,
    tick: usize,
}

fn name(i: usize) -> String {
    format!("P{i}")
}

impl World {
    /// Players `P0`... at `spots` (x, z) on the surface, creative, in an empty world: nothing
    /// spawns, time and weather stand still.
    fn new(topo: Topo, spots: &[[f64; 2]]) -> World {
        Self::custom(topo, spots, |_| {})
    }

    fn custom(topo: Topo, spots: &[[f64; 2]], tweak: impl FnOnce(&mut SimConfig)) -> World {
        let mut config = SimConfig::new(spots.len() + 2, 4, None);
        tweak(&mut config);
        config.keep_alive = false;
        config.pool.workers = topo.workers;
        config.pool.chaos = topo.chaos;
        config.unified_regions = topo.unified;
        let mut w = World { sim: Sim::new(config), clients: Vec::new(), inbox: Vec::new(), trace: Trace::default(), tick: 0 };
        for c in ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:advance_weather false", "gamerule minecraft:advance_time false", "time set 18000"] {
            w.cmd(c);
        }
        for (i, spot) in spots.iter().enumerate() {
            let conn = i as u64 + 1;
            let (msg, stats) = join(conn, &name(i), 2);
            w.inbox.push(msg);
            w.cmd(format!("gamemode creative {}", name(i)));
            w.cmd(format!("tp {} {} {SURFACE_Y} {}", name(i), spot[0], spot[1]));
            stats.count_ids.store(true, std::sync::atomic::Ordering::Relaxed);
            w.clients.push(Client::new(conn, stats));
        }
        for _ in 0..20 {
            w.step();
        }
        assert!(w.clients.iter().all(Client::settled), "players did not settle");
        w
    }

    fn cmd(&mut self, c: impl Into<String>) {
        self.inbox.push(ToSim::Console(c.into()));
    }

    fn send(&mut self, player: usize, pkt: PlayIn) {
        self.inbox.push(ToSim::Packet(player as u64 + 1, pkt));
    }

    fn step(&mut self) {
        for c in &mut self.clients {
            c.tick(None, &mut self.inbox);
        }
        assert!(self.sim.step(self.inbox.drain(..)), "simulation stopped");
        self.tick += 1;
        let d = self.digest();
        self.trace.digests.push(d);
        self.trace.regions.push(self.sim.region_count());
        self.trace.by_id.push(self.clients.iter().map(|c| c.stats.by_id.lock().unwrap().clone()).collect());
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Everything a topology must not change.
    fn digest(&self) -> [u64; 5] {
        let hash = |f: &dyn Fn(&mut std::hash::DefaultHasher)| {
            let mut h = std::hash::DefaultHasher::new();
            f(&mut h);
            h.finish()
        };
        [
            self.sim.state_hash(),
            hash(&|h| self.sim.entity_nbt().iter().for_each(|t| format!("{t:?}").hash(h))),
            hash(&|h| self.sim.riding().hash(h)),
            hash(&|h| {
                for i in 0..self.clients.len() {
                    let conn = i as u64 + 1;
                    self.sim.vehicle_of(conn).hash(h);
                    format!("{:?}", self.sim.open_menu(conn)).hash(h);
                    self.sim.player_level(conn).map(|(l, p)| (l, p.map(f64::to_bits))).hash(h);
                }
            }),
            hash(&|h| {
                for c in &self.clients {
                    (c.stats.packets.load(std::sync::atomic::Ordering::Relaxed), c.stats.bytes.load(std::sync::atomic::Ordering::Relaxed)).hash(h);
                }
            }),
        ]
    }

    fn ids(&self, entity: &str) -> Vec<i32> {
        self.sim.entity_ids_of(entity)
    }

    fn id(&self, entity: &str) -> i32 {
        *self.ids(entity).first().unwrap_or_else(|| panic!("no {entity} among {:?}", self.sim.entities()))
    }

    fn pos(&self, entity: &str) -> [f64; 3] {
        self.sim.entities().into_iter().find(|(k, _)| *k == entity).unwrap_or_else(|| panic!("no {entity}")).1
    }

    fn give(&mut self, player: usize, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.send(player, PlayIn::SetCreativeSlot { slot: 36, item: Some(ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() }) });
        self.step();
    }

    fn interact(&mut self, player: usize, entity: i32) {
        self.send(player, PlayIn::Interact { entity_id: entity, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false });
        self.step();
    }

    /// The rider of `boat` (player `p`) drives it to `x`, keeping its height and z.
    fn steer(&mut self, p: usize, boat: &str, x: f64) {
        let at = self.pos(boat);
        self.send(p, PlayIn::MoveVehicle { pos: [x, at[1], at[2]], rot: [90.0, 0.0], on_ground: true });
    }

    fn nbt(&self, id: i32) -> Tag {
        let list = self.sim.entity_nbt();
        let ids = self.sim.riding();
        // `entity_nbt` and `riding` list the same entities in the same (id) order.
        let i = ids.iter().position(|r| r.0 == id).unwrap_or_else(|| panic!("entity {id} is gone"));
        list[i].clone()
    }
}

/// Runs `scenario` on every topology and demands the same digest at every tick. Returns each
/// topology's trace for the caller's own checks.
fn on_every_topology(scenario: impl Fn(Topo) -> Trace) -> Vec<Trace> {
    let traces: Vec<Trace> = TOPOS.iter().map(|&t| scenario(t)).collect();
    for (t, topo) in traces.iter().zip(TOPOS).skip(1) {
        let first_diff = t.digests.iter().zip(&traces[0].digests).position(|(a, b)| a != b);
        assert_eq!(t.digests.len(), traces[0].digests.len());
        if let Some(i) = first_diff {
            let parts: Vec<&str> = (0..5).filter(|&k| t.digests[i][k] != traces[0].digests[i][k]).map(|k| PARTS[k]).collect();
            let mut what = Vec::new();
            for (c, (a, b)) in t.by_id[i].iter().zip(&traces[0].by_id[i]).enumerate() {
                for id in a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>() {
                    if a.get(id) != b.get(id) {
                        what.push(format!("P{c} packet id {id}: {:?} vs {:?}", a.get(id), b.get(id)));
                    }
                }
            }
            panic!("{topo:?}: differs from one region's at tick {i}: {parts:?} {what:?}");

        }

    }
    traces
}

// ---------------------------------------------------------------------------------------------

/// The scene of the first scenarios: a group at x = 100 (a cell boundary at 128 runs through its
/// walks), a lone player 560 blocks east in a region of his own that teleports in and out.
const HOME: f64 = 100.5;
const FAR: f64 = HOME + 560.0;
const NEAR: f64 = HOME + 230.0;

/// Whether a region split and merged during the run: the counts go from several to one and back.
fn merged_then_split(t: &Trace) -> bool {
    let first_many = t.regions.iter().position(|&r| r >= 2);
    let Some(a) = first_many else { return false };
    let Some(b) = t.regions[a..].iter().position(|&r| r == 1).map(|i| a + i) else { return false };
    t.regions[b..].iter().any(|&r| r >= 2)
}

fn stack_scene(topo: Topo) -> (Trace, Vec<(i32, Option<i32>, Vec<i32>)>) {
    // P0 rides a boat with a villager, P1 has a chest minecart's menu open and a cow on a lead,
    // P2 is far away.
    let mut w = World::new(topo, &[[HOME, 8.5], [HOME + 4.0, 8.5], [FAR, 8.5]]);
    w.cmd(format!("summon minecraft:oak_boat {} {SURFACE_Y} {} {{Passengers:[{{id:\"minecraft:villager\",NoAI:1b}}]}}", HOME + 1.0, 8.5));
    w.cmd(format!(
        "summon minecraft:chest_minecart {} {SURFACE_Y} {} {{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3}},{{Slot:5b,id:\"minecraft:stick\",count:9}}]}}",
        HOME + 6.0,
        8.5
    ));
    w.cmd(format!("summon minecraft:horse {} {SURFACE_Y} {} {{Tame:1b,Passengers:[{{id:\"minecraft:skeleton\",Passengers:[{{id:\"minecraft:parrot\"}}]}}]}}", HOME + 8.0, 12.5));
    w.cmd(format!("summon minecraft:cow {} {SURFACE_Y} {} {{PersistenceRequired:1b}}", HOME + 3.0, 10.5));
    w.steps(3);
    let boat = w.id("minecraft:oak_boat");
    w.cmd(format!("ride {} mount @e[type=minecraft:oak_boat,limit=1]", name(0)));
    w.steps(2);
    assert_eq!(w.sim.vehicle_of(1), Some(boat));
    let (cart, cow) = (w.id("minecraft:chest_minecart"), w.id("minecraft:cow"));
    w.interact(1, cart);
    assert!(w.sim.open_menu(2).is_some(), "the cart's menu opened");
    w.give(1, "minecraft:lead", 1);
    w.interact(1, cow);
    assert!(w.nbt(cow).get("leash").is_some(), "the cow is on the lead");
    let links = w.sim.riding();
    // The boat goes east over the cell boundary at x = 128, back again, over it again, while
    // the far player comes close (the regions merge), goes away (they split).
    let (mut x, mut dir) = (HOME + 1.0, 0.4);
    for t in 0..520 {
        if t == 40 {
            w.cmd(format!("tp {} {NEAR} {SURFACE_Y} 8.5", name(2)));
        }
        if t == 260 {
            w.cmd(format!("tp {} {FAR} {SURFACE_Y} 8.5", name(2)));
        }
        x += dir;
        if !(HOME + 1.0..HOME + 61.0).contains(&x) {
            dir = -dir;
            x += 2.0 * dir;
        }
        w.steer(0, "minecraft:oak_boat", x);
        w.step();
        assert!(w.sim.open_menu(2).is_some(), "tick {t}: the minecart's menu closed (regions {})", w.sim.region_count());
        assert_eq!(w.sim.vehicle_of(1), Some(boat), "tick {t}: P0 left the boat");
        assert!(w.nbt(cow).get("leash").is_some(), "tick {t}: the cow's lead broke");
    }
    assert_eq!(w.sim.riding(), links, "the same riding links as at the start");
    (w.trace, links)
}

#[test]
fn stacks_leashes_and_menus_survive_merges_and_splits() {
    let traces = on_every_topology(|t| stack_scene(t).0);
    assert!(!traces[0].regions.iter().any(|&r| r != 1), "the unified run has one region");
    for t in &traces[1..] {
        assert!(merged_then_split(t), "the scenario merges and splits regions: {:?}", t.regions.iter().step_by(20).collect::<Vec<_>>());
    }
}


// ---------------------------------------------------------------------------------------------
// Teleports: stacks move to another region as a whole, a rider that teleports gets off.

/// Every kind of stack gets teleported to the far player's region: a chest minecart whose menu is
/// open (the player goes too), a cow on a lead (moves with its holder), a horse with a skeleton
/// that carries a parrot, a boat with a player and a villager. Then the boat's player teleports
/// by itself.
fn teleport_scene(topo: Topo) -> Trace {
    let mut w = World::new(topo, &[[HOME, 8.5], [HOME + 4.0, 8.5], [FAR, 8.5]]);
    w.cmd(format!("summon minecraft:oak_boat {} {SURFACE_Y} 8.5 {{Passengers:[{{id:\"minecraft:villager\",NoAI:1b}}]}}", HOME + 1.0));
    w.cmd(format!("summon minecraft:chest_minecart {} {SURFACE_Y} 8.5 {{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3}}]}}", HOME + 6.0));
    w.cmd(format!("summon minecraft:horse {} {SURFACE_Y} 12.5 {{Tame:1b,Passengers:[{{id:\"minecraft:skeleton\",Passengers:[{{id:\"minecraft:parrot\"}}]}}]}}", HOME + 8.0));
    w.cmd(format!("summon minecraft:cow {} {SURFACE_Y} 10.5 {{PersistenceRequired:1b,NoAI:1b}}", HOME + 3.0));
    w.steps(3);
    w.cmd("ride P0 mount @e[type=minecraft:oak_boat,limit=1]");
    w.steps(2);
    let (boat, cart, cow, horse) = (w.id("minecraft:oak_boat"), w.id("minecraft:chest_minecart"), w.id("minecraft:cow"), w.id("minecraft:horse"));
    w.interact(1, cart);
    assert!(w.sim.open_menu(2).is_some());
    w.give(1, "minecraft:lead", 1);
    w.interact(1, cow);
    assert!(w.nbt(cow).get("leash").is_some());
    let links = w.sim.riding();
    let at = |w: &World, e: &str| w.pos(e);
    let near = |a: [f64; 3], x: f64, z: f64| (a[0] - x).abs() < 6.0 && (a[2] - z).abs() < 6.0;

    // The menu's player, the cart and the cow on its lead go to the far region together.
    w.cmd(format!("tp {} {} {SURFACE_Y} 12.5", name(1), FAR + 2.0));
    w.cmd(format!("tp @e[type=minecraft:chest_minecart] {} {SURFACE_Y} 12.5", FAR + 4.0));
    w.cmd(format!("tp @e[type=minecraft:cow] {} {SURFACE_Y} 14.5", FAR + 3.0));
    w.steps(2);
    assert!(near(at(&w, "minecraft:chest_minecart"), FAR + 4.0, 12.5), "the cart is there: {:?}", at(&w, "minecraft:chest_minecart"));
    assert_eq!(w.id("minecraft:chest_minecart"), cart, "the same entity (same id)");
    assert!(w.sim.open_menu(2).is_some(), "the cart's menu stays open when the cart and its player change region");
    let (items, _) = w.sim.cart_items(cart).unwrap();
    assert_eq!(items, vec![(0, "minecraft:diamond", 3)]);
    assert!(w.nbt(cow).get("leash").is_some(), "the cow keeps its lead");
    w.steps(40);
    assert!(w.sim.open_menu(2).is_some(), "still open");
    assert!(w.nbt(cow).get("leash").is_some());

    // The horse with its riders.
    w.cmd(format!("tp @e[type=minecraft:horse] {} {SURFACE_Y} 8.5", HOME + 250.0));
    w.steps(2);
    let riding = w.sim.riding();
    for (id, vehicle, riders) in &links {
        let now = riding.iter().find(|r| r.0 == *id).unwrap_or_else(|| panic!("entity {id} is gone"));
        assert_eq!((&now.1, &now.2), (vehicle, riders), "entity {id}: the links are as before");
    }
    assert!(near(at(&w, "minecraft:skeleton"), HOME + 250.0, 8.5) && near(at(&w, "minecraft:parrot"), HOME + 250.0, 8.5), "the riders went with the horse");
    let _ = horse;

    // The boat with its player and villager.
    w.cmd(format!("tp @e[type=minecraft:oak_boat] {} {SURFACE_Y} 8.5", HOME + 300.0));
    w.steps(3);
    assert_eq!(w.sim.vehicle_of(1), Some(boat), "the player is still in the boat");
    let (_, p) = w.sim.player_level(1).unwrap();
    assert!(near(p, HOME + 300.0, 8.5), "the player went with the boat: {p:?}");
    assert!(w.sim.riding().iter().find(|r| r.0 == boat).unwrap().2.contains(&w.sim.entity_id(1).unwrap()), "the boat still carries the player's id");
    assert!(near(at(&w, "minecraft:villager"), HOME + 300.0, 8.5));

    // The player teleports away by itself: it gets off the boat, which stays.
    w.cmd(format!("tp {} {FAR} {SURFACE_Y} 30.5", name(0)));
    w.steps(3);
    assert_eq!(w.sim.vehicle_of(1), None);
    let boat_riders = w.sim.riding().into_iter().find(|r| r.0 == boat).unwrap().2;
    assert!(!boat_riders.contains(&w.sim.entity_id(1).unwrap()), "the boat let go of the player: {boat_riders:?}");
    assert!(near(at(&w, "minecraft:oak_boat"), HOME + 300.0, 8.5));
    w.steps(40);
    w.trace
}


#[test]
fn teleported_stacks_keep_ids_links_leads_and_menus() {
    on_every_topology(teleport_scene);
}

/// A boat with a player and a villager enters a nether portal: all of them arrive in the
/// nether, the player sitting in the new boat, the villager behind.
fn portal_scene(topo: Topo) -> Trace {
    let mut w = World::new(topo, &[[8.5, 8.5], [FAR, 8.5]]);
    w.cmd("fill 30 -61 8 33 -57 8 minecraft:obsidian");
    w.cmd("fill 31 -60 8 32 -58 8 minecraft:air");
    w.cmd("setblock 31 -60 8 minecraft:fire");
    w.cmd(format!("summon minecraft:oak_boat 9.5 {SURFACE_Y} 8.5 {{Passengers:[{{id:\"minecraft:villager\",NoAI:1b}}]}}"));
    w.steps(3);
    w.cmd("ride P0 mount @e[type=minecraft:oak_boat,limit=1]");
    w.steps(3);
    let villager_uuid = w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:villager")).unwrap().get("UUID").cloned();
    let mut x = 9.5;
    let mut arrived = None;
    for t in 0..120 {
        if x < 31.4 {
            x += 0.5;
            w.steer(0, "minecraft:oak_boat", x);
        }
        w.step();
        if arrived.is_none() && w.sim.player_level(1).is_some_and(|(l, _)| l == "minecraft:the_nether") {
            arrived = Some(t);
        }
    }
    assert!(arrived.is_some(), "the player came to the nether");
    w.steps(5);
    let boat = w.sim.entity_ids_of("minecraft:oak_boat");
    assert_eq!(boat.len(), 1, "one boat: {:?}", w.sim.entities());
    assert_eq!(w.sim.vehicle_of(1), Some(boat[0]), "the player sits in the boat that came with it");
    let riders = w.sim.riding().into_iter().find(|r| r.0 == boat[0]).unwrap().2;
    assert_eq!(riders.len(), 2, "the player and the villager: {riders:?}");
    assert_eq!(riders[0], w.sim.entity_id(1).unwrap(), "the player first");
    let villager = w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:villager")).unwrap();
    assert_eq!(villager.get("UUID").cloned(), villager_uuid, "the villager is the same one");
    assert_eq!(w.sim.entities_in("minecraft:overworld").len(), 0, "nothing left behind in the overworld: {:?}", w.sim.entities_in("minecraft:overworld"));
    w.trace
}

#[test]
fn a_boat_takes_its_rider_through_a_nether_portal() {
    on_every_topology(portal_scene);
}

/// A chest minecart runs along powered rails over a cell boundary (x = 128) while its player
/// has the menu open beside the rails and the regions around merge and split.
fn rails_scene(topo: Topo) -> Trace {
    let mut w = World::new(topo, &[[HOME, 8.5], [128.0, 12.5], [FAR, 8.5]]);
    let (x0, z, y) = (122, 8, -61);
    w.cmd(format!("setblock {} {} {z} minecraft:stone", x0 - 1, y + 1));
    w.cmd(format!("setblock {x0} {y} {z} minecraft:redstone_block"));
    w.cmd(format!("setblock {} {y} {z} minecraft:redstone_block", x0 + 8));
    for i in 0..14 {
        w.cmd(format!("setblock {} {} {z} minecraft:powered_rail[shape=east_west]", x0 + i, y + 1));
    }
    w.steps(3);
    w.cmd(format!("summon minecraft:chest_minecart {} {} {} {{Items:[{{Slot:2b,id:\"minecraft:stick\",count:7}}]}}", x0 as f64 + 0.5, y as f64 + 1.0625, z as f64 + 0.5));
    w.steps(2);
    let cart = w.id("minecraft:chest_minecart");
    w.interact(1, cart);
    assert!(w.sim.open_menu(2).is_some());
    let start = w.pos("minecraft:chest_minecart")[0];
    let mut crossed = false;
    for t in 0..260 {
        if t == 20 {
            w.cmd(format!("tp {} {NEAR} {SURFACE_Y} 8.5", name(2)));
        }
        if t == 140 {
            w.cmd(format!("tp {} {FAR} {SURFACE_Y} 8.5", name(2)));
        }
        w.step();
        assert!(w.sim.open_menu(2).is_some(), "tick {t}: the minecart's menu closed");
        crossed |= w.pos("minecraft:chest_minecart")[0] >= 128.0;
    }
    assert!(crossed || w.pos("minecraft:chest_minecart")[0] > start + 2.0, "the cart ran along the rails: {start} -> {:?}", w.pos("minecraft:chest_minecart"));
    assert_eq!(w.id("minecraft:chest_minecart"), cart);
    w.trace
}

#[test]
fn a_cart_crosses_a_cell_boundary_with_its_menu_open() {
    let traces = on_every_topology(rails_scene);
    for t in &traces[1..] {
        assert!(merged_then_split(t), "regions merged and split: {:?}", t.regions.iter().step_by(20).collect::<Vec<_>>());
    }
}

/// Independent scheduling: the region of the stacks is slowed down until it leaves the lockstep
/// and ticks on its own thread. Not reproducible (wall time decides how many server ticks it
/// misses), so only what must hold whatever the schedule: the boat's rider stays aboard while
/// the boat is driven over a cell boundary, the menu stays open, the lead holds, the horse
/// carries its riders; a teleport (which needs the whole server) that merges the regions meets
/// the slow one first and changes nothing of that.
#[test]
fn independent_scheduling_keeps_stacks_leads_and_menus() {
    use kiln_sim::{InjectedDelay, ScheduleMode};
    let slow = std::time::Duration::from_millis(80);
    let mut w = World::custom(TOPOS[2], &[[HOME, 8.5], [HOME + 4.0, 8.5], [FAR, 8.5]], |c| {
        c.schedule = ScheduleMode::Independent;
        c.inject_delay = Some(InjectedDelay { dimension: "minecraft:overworld".into(), x: HOME as i32, z: 8, delay: slow });
    });
    w.cmd(format!("summon minecraft:oak_boat {} {SURFACE_Y} 8.5 {{Passengers:[{{id:\"minecraft:villager\",NoAI:1b}}]}}", HOME + 1.0));
    w.cmd(format!("summon minecraft:chest_minecart {} {SURFACE_Y} 8.5 {{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3}}]}}", HOME + 6.0));
    w.cmd(format!("summon minecraft:horse {} {SURFACE_Y} 12.5 {{Tame:1b,Passengers:[{{id:\"minecraft:skeleton\",Passengers:[{{id:\"minecraft:parrot\"}}]}}]}}", HOME + 8.0));
    w.cmd(format!("summon minecraft:cow {} {SURFACE_Y} 10.5 {{PersistenceRequired:1b,NoAI:1b}}", HOME + 3.0));
    w.steps(3);
    w.cmd("ride P0 mount @e[type=minecraft:oak_boat,limit=1]");
    w.steps(2);
    w.sim.rendezvous();
    let (boat, cart, cow) = (w.id("minecraft:oak_boat"), w.id("minecraft:chest_minecart"), w.id("minecraft:cow"));
    let boat_at = w.pos("minecraft:oak_boat");
    w.interact(1, cart);
    w.give(1, "minecraft:lead", 1);
    w.interact(1, cow);
    w.sim.rendezvous();
    let links = w.sim.riding();
    let check = |w: &mut World, what: &str| {
        w.sim.rendezvous();
        assert!(w.sim.open_menu(2).is_some(), "{what}: the menu is open");
        assert_eq!(w.sim.vehicle_of(1), Some(boat), "{what}: the rider is aboard");
        assert_eq!(w.sim.riding(), links, "{what}: the riding links");
        assert!(w.nbt(cow).get("leash").is_some(), "{what}: the lead holds");
    };
    let mut away = 0;
    let mut x = HOME + 1.0;
    for t in 0..120 {
        x += 0.5;
        w.send(0, PlayIn::MoveVehicle { pos: [x, boat_at[1], boat_at[2]], rot: [90.0, 0.0], on_ground: true });
        w.step();
        away = away.max(w.sim.regions_away());
        if t % 40 == 39 {
            check(&mut w, &format!("tick {t}"));
        }
    }
    assert!(away >= 1, "the slow region left the lockstep");
    // Merging with the far player needs the whole server: the slow region is waited for.
    w.cmd(format!("tp {} {NEAR} {SURFACE_Y} 8.5", name(2)));
    for t in 0..60 {
        w.send(0, PlayIn::MoveVehicle { pos: [x, boat_at[1], boat_at[2]], rot: [90.0, 0.0], on_ground: true });
        w.step();

        if t % 20 == 19 {
            check(&mut w, &format!("after the merge, tick {t}"));
        }
    }
    assert!(w.sim.region_count() >= 1);
}
