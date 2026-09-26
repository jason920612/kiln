//! Per-function volume cost on the overworld corner grid, to see where router time goes.
//!
//! usage: cargo run --release -p kiln-worldgen --example profile [-- <datapack dir>]

use kiln_worldgen::{Datapack, RandomState, Scratch, Volume};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("KILN_WORK").expect("pass a datapack dir or set KILN_WORK")).join("generated")
    });
    let pack = Datapack::load(&dir).expect("load datapack");
    let names = [
        "minecraft:overworld/final_density",
        "minecraft:overworld/base_3d_noise",
        "minecraft:overworld/sloped_cheese",
        "minecraft:overworld/caves/entrances",
        "minecraft:overworld/caves/noodle",
        "minecraft:overworld/caves/pillars",
        "minecraft:overworld/caves/spaghetti_2d",
        "minecraft:overworld/caves/spaghetti_roughness_function",
        "minecraft:overworld/offset",
        "minecraft:overworld/factor",
        "minecraft:overworld/jaggedness",
    ];
    let mut state = RandomState::new(0, false);
    let roots: Vec<_> = names.iter().map(|n| pack.graph.function(n).unwrap()).collect();
    let samplers = state.compile(&pack.graph, &roots).unwrap();
    let mut scratch = Scratch::default();
    let mut buf = vec![0f32; 5 * 49 * 5];
    for (name, s) in names.iter().zip(&samplers) {
        let start = Instant::now();
        let mut n = 0;
        for cx in 0..16 {
            for cz in 0..16 {
                let vol = Volume::new([5, 49, 5], [cx * 16, -64, cz * 16], [4, 8, 4]);
                s.fill(&mut scratch, &vol, &mut buf);
                n += 1;
            }
        }
        println!("{name:60} {:8.1} us/chunk", start.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
}
