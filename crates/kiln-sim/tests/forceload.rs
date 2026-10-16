//! wp66: `/forceload add` loads the chunk before it returns (`ServerLevel.setChunkForced` calls `getChunk`, which waits for the
//! chunk), so the commands after it in the same tick (a function, a console batch) can edit it. Here the chunk waited in `pending`
//! until the next tick and then installed within a wall-clock budget: a farm's setup that began with `forceload add` lost its
//! first commands to "That position isn't loaded", and under load lost more.

use kiln_data::blocks::default_state as d;
use kiln_link::ToSim;
use kiln_sim::{Sim, SimConfig};

#[test]
fn commands_after_a_forceload_in_the_same_tick_can_edit_the_chunk() {
    let mut sim = Sim::new(SimConfig::new(4, 2, None));
    // Far from anything loaded: nobody is near, the cell has no region yet.
    assert!(sim.step([
        ToSim::Console("forceload add 4000 4000".into()),
        ToSim::Console("setblock 4000 80 4000 minecraft:diamond_block".into()),
        ToSim::Console("fill 4001 80 4001 4002 80 4002 minecraft:gold_block".into()),
        ToSim::Console("forceload add 4100 4100 4131 4131".into()),
        ToSim::Console("setblock 4131 80 4131 minecraft:emerald_block".into()),
    ]));
    assert_eq!(sim.block_at(4000, 80, 4000), Some(d::DIAMOND_BLOCK), "the first chunk");
    assert_eq!(sim.block_at(4002, 80, 4002), Some(d::GOLD_BLOCK), "a fill in it");
    assert_eq!(sim.block_at(4131, 80, 4131), Some(d::EMERALD_BLOCK), "a range of chunks, the last one");
}

/// The vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`).
fn datapack() -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    if dir.join("reports/biome_parameters").is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: no datapack at {} (KILN_DATAPACK or `cargo xtask data`)", dir.display());
        None
    }
}

/// wp73: with generated terrain the forced chunks are made side by side on the generation threads
/// while the command waits (not one after another on the tick thread), and when it returns they
/// are all there and lit (none waits for its region's next run to get the light across its
/// borders).
#[test]
fn generated_forced_chunks_are_loaded_and_lit_when_the_command_returns() {
    let Some(datapack) = datapack() else { return };
    let mut config = SimConfig::new(4, 3, None);
    config.noise = Some(kiln_sim::NoiseConfig { seed: 73, datapack, threads: 4 });
    let mut sim = Sim::new(config);
    assert!(sim.step([]));
    sim.console_now("forceload add 1600 1600 1663 1631");
    sim.console_now("setblock 1663 300 1631 minecraft:diamond_block");
    assert_eq!(sim.block_at(1663, 300, 1631), Some(d::DIAMOND_BLOCK), "the far corner, right after");
    for cx in 100..104 {
        for cz in 100..102 {
            assert!(sim.block_at(cx * 16 + 8, 0, cz * 16 + 8).is_some(), "chunk {cx},{cz} is loaded");
        }
    }
    assert_eq!(sim.unlit_chunks(), 0, "a forced chunk still waits for its light");
}
