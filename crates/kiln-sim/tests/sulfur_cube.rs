//! A sulfur cube in a running simulation: it hops about, a ball made of the item inside it flies off
//! when a player hits it, and one with an explosive inside blows up when lit.

use kiln_link::{PlayIn, ToSim};
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
        let (msg, stats) = join(1, "User", 2);
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

    fn nbt(&self, name: &str) -> Option<Tag> {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name))
    }

    fn pos(&self, name: &str) -> [f64; 3] {
        let t = self.nbt(name).expect("the entity");
        let Some(Tag::List(p)) = t.get("Pos") else { panic!("Pos") };
        [p[0].as_f64().unwrap(), p[1].as_f64().unwrap(), p[2].as_f64().unwrap()]
    }
}

#[test]
fn a_sulfur_cube_hops_about() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!("summon minecraft:sulfur_cube {} {} {} {{PersistenceRequired:1b,Size:1}}", p[0] + 4.0, p[1], p[2]));
    w.ticks(2);
    let start = w.pos("minecraft:sulfur_cube");
    w.ticks(300);
    let now = w.pos("minecraft:sulfur_cube");
    assert!((0..3).any(|i| (start[i] - now[i]).abs() > 0.5), "it moved: {start:?} -> {now:?}");
    let cube = w.nbt("minecraft:sulfur_cube").unwrap();
    assert_eq!(cube.get("Size").and_then(Tag::as_i64), Some(1));
}

#[test]
fn a_ball_flies_off_when_hit() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run("gamemode survival User");
    w.run(&format!(
        "summon minecraft:sulfur_cube {} {} {} {{PersistenceRequired:1b,Size:1,equipment:{{body:{{id:\"minecraft:oak_planks\",count:1}}}}}}",
        p[0] + 1.5,
        p[1],
        p[2]
    ));
    w.ticks(5);
    let id = *w.sim.entity_ids_of("minecraft:sulfur_cube").first().expect("cube");
    let before = w.pos("minecraft:sulfur_cube");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id })]));
    w.ticks(10);
    let after = w.pos("minecraft:sulfur_cube");
    let moved = ((after[0] - before[0]).powi(2) + (after[2] - before[2]).powi(2)).sqrt();
    assert!(moved > 0.3, "the ball rolled off: {before:?} -> {after:?}");
}

#[test]
fn a_lit_explosive_ball_blows_up() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!(
        "summon minecraft:sulfur_cube {} {} {} {{PersistenceRequired:1b,Size:1,equipment:{{body:{{id:\"minecraft:tnt\",count:1}}}}}}",
        p[0] + 8.0,
        p[1],
        p[2]
    ));
    w.ticks(3);
    assert!(w.nbt("minecraft:sulfur_cube").is_some());
    // Fire lights the fuse.
    w.run("execute as @e[type=minecraft:sulfur_cube] run damage @s 1 minecraft:on_fire");
    w.ticks(150);
    assert!(w.nbt("minecraft:sulfur_cube").is_none(), "it went off");
}
