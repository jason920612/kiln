//! wp87: a level without sky light (the nether: `has_skylight` false in its dimension type) has none of it anywhere, above its bedrock roof too. Kiln stored
//! sky light for every level as if it were open to the sky, so up on the nether roof the light was 15 by day and night, and monsters would not spawn there
//! (`isValidPositionForMob` reads the light: a zombified piglin farm on the roof gave nothing: wp82's "5 to 10 % more MONSTER tries" was this, a farm that
//! spawned fewer and so tried more often to reach its cap).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn up_on_the_nether_roof_it_is_dark() {
    let _x = kiln_data::live::exclusive();
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, 10);
    for dim in ["minecraft:overworld", "minecraft:the_nether"] {
        sim.console_now(&format!("execute in {dim} run forceload add 0 0 15 15"));
    }
    ticks(&mut sim, 20);
    // A position with nothing around it, high up: a gold block in the same level says that the predicate (a light of at least 8) held.
    let probe = "{condition:\"minecraft:location_check\",predicate:{light:{light:{min:8}}}}";
    for (dim, y, marker) in [("minecraft:overworld", 100, 70), ("minecraft:the_nether", 200, 70)] {
        sim.console_now("time set noon");
        sim.console_now(&format!("execute in {dim} positioned 8 {y} 8 if predicate {probe} run setblock 4 {marker} 4 minecraft:gold_block"));
    }
    ticks(&mut sim, 2);
    let gold = kiln_data::blocks::default_state::GOLD_BLOCK;
    assert_eq!(sim.block_in("minecraft:overworld", 4, 70, 4), Some(gold), "daylight on a world in the overworld has light");
    assert_eq!(sim.block_in("minecraft:the_nether", 4, 70, 4), Some(kiln_data::blocks::default_state::AIR), "no sky light in the nether, not even above its roof");
}
