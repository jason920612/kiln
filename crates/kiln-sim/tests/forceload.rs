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
