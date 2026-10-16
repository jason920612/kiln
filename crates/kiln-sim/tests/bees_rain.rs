//! Bees stay in the hive while there is any rain (`WeatherAttributes` lays `bees_stay_in_hive: true` over the level's value as soon as the
//! rain level is above 0), and come out once the sky is clear (wp68). Kiln only knew the timeline's night; a persistence run with
//! `weather rain` on both servers showed vanilla's bee still inside the hive after minutes and Kiln's gone.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn bees_in(sim: &Sim, x: i32, y: i32, z: i32) -> usize {
    match sim.block_entity_nbt(x, y, z).as_ref().and_then(|t| t.get("bees")) {
        Some(Tag::List(l)) => l.len(),
        _ => 0,
    }
}

#[test]
fn a_bee_that_is_ready_stays_in_its_hive_in_the_rain_and_leaves_when_it_clears() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("time set 1000".into())]));
    let mut client = Client::new(1, stats);
    fn tick(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    tick(&mut sim, &mut client, 5);
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    assert!(sim.step([
        ToSim::Console("weather rain 100000".into()),
        ToSim::Console(format!(
            "setblock {x} {y} {z} minecraft:bee_nest[facing=south]{{bees:[{{entity_data:{{id:'minecraft:bee',Health:10f}},min_ticks_in_hive:100,ticks_in_hive:5000}}]}}"
        )),
    ]));
    tick(&mut sim, &mut client, 40);
    assert_eq!(bees_in(&sim, x, y, z), 1, "the bee is ready to leave but it rains");
    assert!(sim.entity_ids_of("minecraft:bee").is_empty());
    assert!(sim.step([ToSim::Console("weather clear".into())]));
    // the rain level fades by 0.01 a tick
    tick(&mut sim, &mut client, 150);
    assert_eq!(bees_in(&sim, x, y, z), 0, "the sky is clear: the bee has come out");
    assert_eq!(sim.entity_ids_of("minecraft:bee").len(), 1);
}
