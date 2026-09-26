//! The simulation is a function of its inputs: the same scripted players, block edits and
//! console commands give the same state hashes, tick for tick.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};

const PLAYERS: usize = 24;
const GROUPS: usize = 3;
const SURFACE_Y: f64 = -60.0;

fn run(ticks: usize) -> Vec<u64> {
    let mut sim = Sim::new(SimConfig {
        max_players: PLAYERS,
        view_distance: 4,
        simulation_distance: 4,
        world: None,
        online_mode: false,
    });
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let mut walkers = Vec::new();
    let mut inbox = Vec::new();
    let mut hashes = Vec::new();
    let mut placed = Vec::new();
    for tick in 0..ticks {
        if walkers.len() < PLAYERS {
            let i = walkers.len();
            let conn = i as u64 + 1;
            let name = format!("P{i}");
            let (msg, stats) = join(conn, &name, 2);
            let [ox, oz] = group_offset(i % GROUPS, GROUPS, 64.0);
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
        if tick % 100 == 99 {
            hashes.push(sim.state_hash());
        }
    }
    assert_eq!(sim.player_count(), PLAYERS);
    let stone_block = kiln_data::blocks::default_state::STONE;
    let built = placed.iter().filter(|p| sim.world().get_block(p[0], p[1], p[2]) == Some(stone_block)).count();
    assert!(built > 10, "only {built} of {} edits left stone", placed.len());
    assert!(walkers.iter().all(|w| !w.client.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed)));
    hashes
}

#[test]
fn same_inputs_give_the_same_states() {
    let a = run(400);
    let b = run(400);
    assert_eq!(a, b);
    assert!(a.windows(2).all(|w| w[0] != w[1]), "the state should keep changing: {a:x?}");
}
