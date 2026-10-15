//! wp65: end gateways with two players (found in a real two-client run: the second player, entering in the same
//! tick, stayed in the gateway for good while vanilla sent both).
//!
//! `TheEndGatewayBlock.entityInside` only starts a trip when the gateway is not cooling down; players that were already
//! inside when the first one left all go, and one that comes (or waits) inside during the cooldown goes as soon as it ends.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let mut clients = Vec::new();
        for (conn, name) in [(1, "Alpha"), (2, "Beta")] {
            let (msg, stats) = join(conn, name, 2);
            assert!(sim.step([msg, ToSim::Console(format!("gamemode survival {name}"))]));
            clients.push(Client::new(conn, stats));
        }
        let mut w = World { sim, clients };
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in self.clients.iter_mut() {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, commands: &[String]) {
        assert!(self.sim.step(commands.iter().map(|c| ToSim::Console(c.clone()))));
    }
}

fn gateway_world() -> (World, [i32; 3]) {
    let mut w = World::new();
    for n in ["Alpha", "Beta"] {
        w.run(&[format!("gamemode creative {n}"), format!("execute in minecraft:the_end run tp {n} 0.5 90 40.5")]);
    }
    w.ticks(45);
    w.run(&["execute in minecraft:the_end run kill @e[type=minecraft:ender_dragon]".to_owned()]);
    w.ticks(5);
    let g = kiln_worldgen::end::end_gateway_positions(0)[0];
    for n in ["Alpha", "Beta"] {
        w.run(&[format!("gamemode survival {n}"), format!("effect give {n} minecraft:slow_falling 600 0 true"), format!("execute in minecraft:the_end run tp {n} {} {} {}", g.x as f64 - 3.0, g.y as f64 + 1.0, g.z as f64 + 0.5)]);
    }
    w.ticks(10);
    (w, [g.x, g.y, g.z])
}

fn far_from(c: &Client, g: [i32; 3]) -> bool {
    (c.pos[0] - g[0] as f64).abs() > 100.0 || (c.pos[2] - g[2] as f64).abs() > 100.0
}

#[test]
fn two_players_entering_a_gateway_in_the_same_tick_both_go() {
    let (mut w, g) = gateway_world();
    let cmds: Vec<String> = ["Alpha", "Beta"].iter().map(|n| format!("execute in minecraft:the_end run tp {n} {} {} {}", g[0] as f64 + 0.5, g[1] as f64, g[2] as f64 + 0.5)).collect();
    w.run(&cmds);
    w.ticks(30);
    assert!(far_from(&w.clients[0], g), "Alpha went: {:?}", w.clients[0].pos);
    assert!(far_from(&w.clients[1], g), "Beta went too: {:?}", w.clients[1].pos);
}

#[test]
fn a_player_inside_a_cooling_gateway_goes_when_it_has_cooled() {
    let (mut w, g) = gateway_world();
    let at = |n: &str| format!("execute in minecraft:the_end run tp {n} {} {} {}", g[0] as f64 + 0.5, g[1] as f64, g[2] as f64 + 0.5);
    w.run(&[at("Alpha")]);
    w.ticks(6);
    assert!(far_from(&w.clients[0], g), "Alpha went: {:?}", w.clients[0].pos);
    // Beta comes while the gateway cools down (40 ticks): it waits inside, then goes.
    w.run(&[at("Beta")]);
    w.ticks(10);
    assert!(!far_from(&w.clients[1], g), "Beta is held back by the cooldown: {:?}", w.clients[1].pos);
    w.ticks(60);
    assert!(far_from(&w.clients[1], g), "Beta went when the gateway had cooled: {:?}", w.clients[1].pos);
}
