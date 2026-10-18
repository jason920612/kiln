//! Vanilla loads the chunks within the *server's* view distance of a player (the
//! `PLAYER_LOADING` tickets of `DistanceManager.PlayerTicketTracker`), whatever render distance
//! the client asked for; the client's own distance (clamped to 2..server view) only decides
//! which of the loaded chunks are sent. An AFK player with a render distance of 2 therefore
//! keeps everything within the simulation distance loaded and running.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::time::Duration;

const SERVER_VIEW: u8 = 6;

fn settle(sim: &mut Sim, clients: &mut [Client], ticks: usize) {
    for _ in 0..ticks {
        let mut inbox: Vec<ToSim> = Vec::new();
        for c in clients.iter_mut() {
            c.ack_batches = true;
            c.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn loaded_after_joining(client_view: u8) -> usize {
    let mut sim = Sim::new(SimConfig::new(2, SERVER_VIEW, None));
    let (msg, stats) = join(1, "Afk", client_view);
    assert!(sim.step([msg]));
    let mut clients = [Client::new(1, stats)];
    settle(&mut sim, &mut clients, 300);
    assert_eq!(sim.chunk_backlog(), 0, "everything asked for is loaded");
    sim.loaded_chunks()[0]
}

#[test]
fn a_small_render_distance_still_loads_the_servers_view() {
    let side = 2 * SERVER_VIEW as usize + 1;
    let small = loaded_after_joining(2);
    let full = loaded_after_joining(SERVER_VIEW);
    assert!(full >= side * side, "the full view loads {full} chunks, expected at least {}", side * side);
    assert_eq!(small, full, "a render distance of 2 loads the same chunks as the server's view");
}
