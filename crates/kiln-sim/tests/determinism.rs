//! The simulation is a function of its inputs: the same scripted players, block edits and
//! console commands give the same state hashes, tick for tick, however the world is split
//! into regions and however many workers tick them in whatever order (design DT-R1).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};

const PLAYERS: usize = 24;
const GROUPS: usize = 3;
/// Far enough apart that each group gets its own region.
const GROUP_SPACING: f64 = 1024.0;
const SURFACE_Y: f64 = -60.0;

struct Run {
    hashes: Vec<u64>,
    max_regions: usize,
}

fn run(ticks: usize, workers: usize, unified: bool, chaos: Option<u64>) -> Run {
    let mut config = SimConfig::new(PLAYERS, 4, None);
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    config.unified_regions = unified;
    let mut sim = Sim::new(config);
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let mut walkers = Vec::new();
    let mut inbox = Vec::new();
    let mut hashes = Vec::new();
    let mut placed = Vec::new();
    let mut max_regions = 0;
    for tick in 0..ticks {
        if walkers.len() < PLAYERS {
            let i = walkers.len();
            let conn = i as u64 + 1;
            let name = format!("P{i}");
            let (msg, stats) = join(conn, &name, 2);
            let [ox, oz] = group_offset(i % GROUPS, GROUPS, GROUP_SPACING);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(msg);
            inbox.push(ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
            let item = ItemStack { item: stone, count: 64, added: Vec::new(), removed: Vec::new() };
            inbox.push(ToSim::Packet(conn, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }));
            walkers.push(Walker::new(Client::new(conn, stats), center, conn));
        }
        for (i, w) in walkers.iter_mut().enumerate() {
            w.tick(5.0, i % 2 == 0, &mut inbox);
            // Every so often a player builds next to itself, and later breaks what it built.
            if w.client.settled() && (tick + i) % 40 == 0 {
                let [x, y, z] = w.client.pos.map(|c| c.floor() as i32);
                let (conn, sequence) = (w.client.conn, tick as i32);
                let ground = [x + 2, y - 1, z];
                placed.push([x + 2, y, z]);
                inbox.push(ToSim::Packet(conn, if (tick / 40) % 3 == 2 {
                    PlayIn::PlayerAction { action: 0, pos: [x + 2, y, z], face: 1, sequence }
                } else {
                    PlayIn::UseItemOn { hand: 0, pos: ground, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence }
                }));
            }
        }
        if tick == 150 {
            inbox.push(ToSim::Console("time set 13000".into()));
        }
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        max_regions = max_regions.max(sim.region_count());
        if tick % 100 == 99 {
            hashes.push(sim.state_hash());
        }
    }
    assert_eq!(sim.player_count(), PLAYERS);
    let stone_block = kiln_data::blocks::default_state::STONE;
    let built = placed.iter().filter(|p| sim.block_at(p[0], p[1], p[2]) == Some(stone_block)).count();
    assert!(built > 10, "only {built} of {} edits left stone", placed.len());
    assert!(walkers.iter().all(|w| !w.client.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed)));
    Run { hashes, max_regions }
}

#[test]
fn same_inputs_give_the_same_states() {
    let a = run(400, 1, false, None);
    let b = run(400, 1, false, None);
    assert_eq!(a.hashes, b.hashes);
    assert!(a.hashes.windows(2).all(|w| w[0] != w[1]), "the state should keep changing: {:x?}", a.hashes);
}

#[test]
fn regions_and_workers_do_not_change_the_result() {
    let unified = run(400, 1, true, None);
    assert_eq!(unified.max_regions, 1);
    let split = run(400, 1, false, None);
    assert!(split.max_regions >= GROUPS, "groups should tick as separate regions ({} regions)", split.max_regions);
    assert_eq!(split.hashes, unified.hashes, "one region vs one per group");
    for (workers, seed) in [(4, 1), (7, 99)] {
        let parallel = run(400, workers, false, Some(seed));
        assert_eq!(parallel.hashes, unified.hashes, "{workers} workers, chaos seed {seed}");
    }
}
