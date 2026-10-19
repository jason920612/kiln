mod support;
use support::Game;

#[test]
fn scratch() {
    let _x = kiln_data::live::exclusive();
    let mut g = Game::new("scratch2", &["kiln_wp83"], "Farmer");
    g.console("loot give Farmer loot kiln83:till/gravel");
    g.ticks(2);
    eprintln!("inv {:?}", g.sim.inventory(1).map(|v| v.iter().filter_map(|x| *x).collect::<Vec<_>>()));
    let ids = kiln_data::builtin_entries("minecraft:item").unwrap();
    eprintln!("flint id {:?}", ids.iter().position(|n| *n == "minecraft:flint"));
    g.cleanup();
}
