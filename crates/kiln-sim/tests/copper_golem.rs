//! A copper golem in a running simulation: it carries items from a copper chest to a chest, and a fully
//! oxidized one stiffens into a statue.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 3);
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        Self { sim, client }
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn count_items(&self, x: i32, y: i32, z: i32) -> i64 {
        let Some(be) = self.sim.block_entity_nbt(x, y, z) else { return -1 };
        match be.get("Items") {
            Some(Tag::List(items)) => items.iter().map(|i| i.get("count").and_then(Tag::as_i64).unwrap_or(0)).sum(),
            _ => 0,
        }
    }
}

#[test]
fn a_golem_carries_items_from_a_copper_chest_to_a_chest() {
    let mut w = World::new();
    let p = w.client.pos;
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("setblock {} {} {} minecraft:copper_chest[facing=west]{{Items:[{{Slot:0b,id:\"minecraft:iron_ingot\",count:40}}]}}", x + 6, y, z));
    w.run(&format!("setblock {} {} {} minecraft:chest[facing=east]", x - 6, y, z));
    w.run(&format!("summon minecraft:copper_golem {} {} {}", x as f64 + 0.5, y, z as f64 + 0.5));
    w.ticks(2);
    assert_eq!(w.count_items(x + 6, y, z), 40);
    assert_eq!(w.count_items(x - 6, y, z), 0);
    // The first transport cooldown is 60 to 99 ticks; the walk and the two openings take a while.
    w.ticks(900);
    let (source, dest) = (w.count_items(x + 6, y, z), w.count_items(x - 6, y, z));
    assert_eq!(source + dest, 40, "nothing is lost: {source} + {dest}");
    assert!(dest > 0, "something reached the chest: {source} + {dest}");
}

#[test]
fn an_oxidized_golem_becomes_a_statue() {
    let mut w = World::new();
    let p = w.client.pos;
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("summon minecraft:copper_golem {} {} {} {{next_weather_age:0L,weather_state:\"oxidized\",CustomName:\"Rusty\"}}", x as f64 + 3.5, y, z as f64 + 0.5));
    w.ticks(3000);
    assert!(w.sim.entity_ids_of("minecraft:copper_golem").is_empty(), "the golem is gone");
    // A statue stands where it was (somewhere near: it walked about meanwhile).
    let mut found = false;
    for dx in -20..=20 {
        for dz in -20..=20 {
            if let Some(s) = w.sim.block_at(x + dx, y, z + dz)
                && kiln_data::blocks_types::block_of(s).name.contains("copper_golem_statue")
            {
                found = true;
            }
        }
    }
    assert!(found, "a statue block stands somewhere");
}
