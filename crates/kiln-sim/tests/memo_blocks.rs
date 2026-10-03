//! Results the entities keep while they stand still (collisions, supporting block, in-wall,
//! inside blocks) are dropped when a block of the chunk changes: a mob in a shaft falls when
//! its floor is dug out, is held by a block placed under it, and burns in a fire put in its
//! box, however long it stood there before. (`KILN_MEMO_CHECK=1` makes every reuse compare itself
//! with a fresh scan.)

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(6, 4, None));
        let (msg, stats) = join(1, "Bait", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        w.console("gamerule minecraft:spawn_mobs false");
        w.console("gamemode creative Bait");
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    /// The pig's position and health.
    fn pig(&self) -> ([f64; 3], f64) {
        let found: Vec<Tag> = self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(|i| i.as_str()) == Some("minecraft:pig")).collect();
        assert_eq!(found.len(), 1, "one pig");
        let pos: Vec<f64> = match found[0].get("Pos") {
            Some(Tag::List(v)) => v.iter().filter_map(|t| t.as_f64()).collect(),
            other => panic!("{other:?}"),
        };
        ([pos[0], pos[1], pos[2]], found[0].get("Health").and_then(|h| h.as_f64()).unwrap_or(0.0))
    }
}

/// A pig in a one-block shaft of stone: it cannot walk anywhere, so every tick repeats the last.
fn shaft() -> (World, i32, i32, i32) {
    let mut w = World::new();
    let p = w.client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 6, p[1].floor() as i32, p[2].floor() as i32 + 6);
    w.console(&format!("fill {} {} {} {} {} {} minecraft:stone", x - 1, y - 3, z - 1, x + 1, y + 6, z + 1));
    w.console(&format!("fill {x} {y} {z} {x} {} {z} minecraft:air", y + 3));
    w.ticks(3);
    w.console(&format!("summon minecraft:pig {} {} {} {{PersistenceRequired:1b,Health:10.0f}}", x as f64 + 0.5, y, z as f64 + 0.5));
    w.ticks(80);
    (w, x, y, z)
}

#[test]
fn a_mob_that_stood_still_falls_when_its_floor_is_dug_out() {
    let (mut w, x, y, z) = shaft();
    let (pos, _) = w.pig();
    assert!((pos[1] - y as f64).abs() < 0.01, "stands on the floor: {pos:?}");
    w.console(&format!("fill {x} {} {z} {x} {} {z} minecraft:air", y - 2, y - 1));
    w.ticks(30);
    let (pos, _) = w.pig();
    assert!(pos[1] <= (y - 2) as f64 + 0.01, "fell to the new floor: {pos:?}");
}

#[test]
fn a_block_placed_under_a_falling_mob_holds_it() {
    let (mut w, x, y, z) = shaft();
    w.console(&format!("fill {x} {} {z} {x} {} {z} minecraft:air", y - 2, y - 1));
    w.ticks(1);
    // Mid-fall a block appears (the chunk changed again after the mob's last scan).
    w.console(&format!("setblock {x} {} {z} minecraft:stone", y - 2));
    w.ticks(30);
    let (pos, _) = w.pig();
    assert!((pos[1] - (y - 1) as f64).abs() < 0.01, "stopped on the placed block: {pos:?}");
}

#[test]
fn fire_placed_in_a_standing_mobs_box_burns_it() {
    let (mut w, x, y, z) = shaft();
    let (pos, before) = w.pig();
    assert_eq!(before, 10.0, "{pos:?}");
    // Fire in the pig's own block (it stands on stone, which fire may sit on).
    w.console(&format!("setblock {x} {y} {z} minecraft:fire"));
    w.ticks(40);
    let (_, after) = w.pig();
    assert!(after < before, "the pig burned: {after}");
}
