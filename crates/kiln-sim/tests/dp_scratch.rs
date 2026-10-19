mod support;
use kiln_link::ToSim;
use support::Game;

#[test]
fn scratch() {
    let _x = kiln_data::live::exclusive();
    let mut g = Game::new("scratch3", &["kiln_wp83"], "Farmer");
    let [x, y, z] = g.spot(2);
    g.console(&format!("fill {x} {} {z} {} {} {z} minecraft:stone", y - 1, x + 3, y - 1));
    g.console(&format!("setblock {x} {y} {z} minecraft:oak_log"));
    g.console("give Farmer minecraft:diamond_axe");
    g.ticks(2);
    eprintln!("sel {:?} inv {:?}", g.sim.selected_slot(1), g.sim.inventory(1).map(|v| v.iter().enumerate().filter_map(|(i, x)| x.map(|x| (i, x))).collect::<Vec<_>>()));
    let pkt = kiln_link::PlayIn::UseItemOn { hand: 0, pos: [x, y, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
    assert!(g.sim.step([ToSim::Packet(1, pkt)]));
    eprintln!("block {}", kiln_blocks::state::state_string(g.sim.block_at(x, y, z).unwrap()));
    g.cleanup();
}
