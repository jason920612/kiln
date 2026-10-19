mod support;
use kiln_link::ToSim;
use support::Game;

#[test]
fn scratch() {
    let mut g = Game::new("scratch", &["kiln_wp83"], "Digger");
    let [x, y, z] = g.spot(2);
    g.console("give Digger minecraft:diamond_shovel");
    g.console(&format!("setblock {x} {y} {z} minecraft:dirt"));
    g.ticks(2);
    eprintln!("spot {x} {y} {z}  client {:?}", g.client.pos);
    assert!(g.sim.step([ToSim::Packet(1, kiln_link::PlayIn::PlayerAction { action: 0, pos: [x, y, z], face: 1, sequence: 1 })]));
    for i in 0..60 {
        g.ticks(1);
        let s = kiln_blocks::state::state_string(g.sim.block_at(x, y, z).unwrap());
        if s != "minecraft:dirt" {
            eprintln!("broken after {i} ticks: {s}");
            break;
        }
    }
    eprintln!("inventory {:?}", g.sim.inventory(1).map(|v| v.iter().take(10).cloned().collect::<Vec<_>>()));
}
