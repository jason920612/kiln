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
    // A position with nothing around it, high up.
    sim.console_now("time set noon");
    assert_eq!(sim.sky_light_in("minecraft:overworld", 8, 100, 8), Some(15), "open sky in the overworld");
    assert_eq!(sim.sky_light_in("minecraft:the_nether", 8, 200, 8), Some(0), "no sky light in the nether, not even above its roof");
    assert_eq!(sim.sky_light_in("minecraft:the_nether", 8, 100, 8), Some(0), "nor below it");
}
