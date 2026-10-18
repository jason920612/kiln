//! `/data merge entity` on a vehicle keeps its riders (wp68): `Entity.load` does not touch the passengers, but Kiln replaced
//! the entity by a freshly loaded one without its links, so a player on a happy ghast (or a horse, a boat) was thrown off by
//! any change of the mount's data. Found while steering a happy ghast: `NoAI:0b` dropped both riders.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn merging_data_into_a_vehicle_does_not_dismount_its_riders() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    assert!(sim.step([ToSim::Console(format!("summon minecraft:pig {} {} {} {{PersistenceRequired:1b,NoAI:1b}}", p[0] + 2.0, p[1], p[2]))]));
    ticks(&mut sim, &mut client, 3);
    assert!(sim.step([ToSim::Console("ride User mount @e[type=pig,limit=1]".into())]));
    ticks(&mut sim, &mut client, 3);
    assert!(sim.vehicle_of(1).is_some(), "mounted");
    assert!(sim.step([ToSim::Console("data merge entity @e[type=pig,limit=1] {Silent:1b}".into())]));
    ticks(&mut sim, &mut client, 5);
    assert!(sim.vehicle_of(1).is_some(), "still on the pig after its data changed");
    assert_eq!(sim.riding().len(), 1, "one entity with a rider");
}
