//! Scheduled block and fluid ticks run one chunk further than entities and random ticks (`ServerLevel.shouldTickBlocksAt` = `inBlockTickingRange`, simulation level
//! 32, against `inEntityTickingRange`, 31). Measured on vanilla 26.3 with `tools/wp79/simradius79.py` (simulation distance 6: a baby cow's age rises up to 6 chunks from
//! the player, a water source spreads up to 7). Kiln used the entity range for everything.

use kiln_data::blocks::default_state as d;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_water_source_spreads_up_to_one_chunk_beyond_the_simulation_distance() {
    let mut config = SimConfig::new(2, 8, None);
    config.simulation_distance = 3;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Watcher".into()), ToSim::Console("tp Watcher 8 120 8".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 60);
    // A sealed glass box at y 200 in the middle of the chunks 3, 4 and 5 east of the player's (distances 3, 4, 5 with a simulation distance of 3).
    for chunk in 3..=5 {
        let x = chunk * 16 + 8;
        let cmds = [
            format!("fill {} 199 5 {} 199 11 minecraft:stone", x - 2, x + 3),
            format!("fill {} 200 5 {} 203 11 minecraft:glass", x - 2, x + 3),
            format!("fill {} 200 6 {} 200 10 minecraft:air", x - 1, x + 2),
            format!("setblock {x} 200 8 minecraft:water"),
        ];
        assert!(sim.step(cmds.map(ToSim::Console)));
    }
    tick(&mut sim, 60);
    let spread = |chunk: i32| sim.block_at(chunk * 16 + 8 + 1, 200, 8) == Some(d::WATER);
    assert!(spread(3), "inside the simulation distance");
    assert!(spread(4), "one chunk beyond it: blocks still tick");
    assert!(!spread(5), "two chunks beyond it nothing ticks");
}
