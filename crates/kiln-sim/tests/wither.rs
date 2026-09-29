//! The wither end to end: building it from soul sand and wither skeleton skulls, its
//! invulnerable start and the explosion that ends it. Guardians swim in a pool. The behaviour
//! itself is checked tick by tick against vanilla by kiln-entity's `mob_parity` test.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Builder", 2);
        assert!(sim.step([
            msg,
            ToSim::Console("gamemode creative Builder".into()),
            ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
            ToSim::Console("difficulty normal".into())
        ]));
        let mut w = World { sim, client: Client::new(1, stats), ground: [0; 3] };
        w.ticks(5);
        let p = w.client.pos;
        w.ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        w
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

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn name(&self, p: [i32; 3]) -> &'static str {
        kiln_entity::blocks::block_name(self.sim.block_at(p[0], p[1], p[2]).expect("loaded"))
    }

    fn hold(&mut self, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 64, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn use_on_top(&mut self, pos: [i32; 3]) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }
}

#[test]
fn wither_is_built_from_soul_sand_and_skulls() {
    let mut w = World::new();
    // The T of soul sand along z, two skulls already on it.
    let (x, z) = (4, 0);
    for p in [w.at(x, 1, z), w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 2, z + 1)] {
        w.set(p, "minecraft:soul_sand");
    }
    w.set(w.at(x, 3, z - 1), "minecraft:wither_skeleton_skull");
    w.set(w.at(x, 3, z + 1), "minecraft:wither_skeleton_skull");
    w.ticks(1);
    assert!(w.mobs("minecraft:wither").is_empty());
    // The third skull completes it.
    w.hold("minecraft:wither_skeleton_skull");
    w.use_on_top(w.at(x, 2, z));
    w.ticks(1);
    let withers = w.mobs("minecraft:wither");
    assert_eq!(withers.len(), 1, "a wither appears");
    let (_, pos, health) = withers[0];
    assert!(health == 100.0 || health == 110.0, "a third of its health, maybe healed once ({health})");
    let base = w.at(x, 1, z);
    assert_eq!([pos[0], pos[2]], [base[0] as f64 + 0.5, base[2] as f64 + 0.5]);
    for p in [w.at(x, 1, z), w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 3, z), w.at(x, 3, z + 1)] {
        assert_eq!(w.name(p), "minecraft:air", "the pattern is cleared");
    }
    // Invulnerable, it heals 10 every 10 ticks; after 220 ticks it explodes and is free.
    w.ticks(100);
    let h = w.mobs("minecraft:wither")[0].2;
    assert!(h > 180.0, "healing while invulnerable ({h})");
    let ground_before = w.name(w.at(x, 0, z));
    w.ticks(130);
    assert_eq!(w.mobs("minecraft:wither")[0].2, 300.0);
    assert_ne!(ground_before, "minecraft:air");
    assert_eq!(w.name(w.at(x, 0, z)), "minecraft:air", "the explosion broke the ground under it");
}

#[test]
fn an_incomplete_pattern_or_peaceful_builds_nothing() {
    let mut w = World::new();
    let (x, z) = (4, 0);
    for p in [w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 2, z + 1)] {
        w.set(p, "minecraft:soul_soil");
    }
    w.set(w.at(x, 3, z - 1), "minecraft:wither_skeleton_skull");
    w.set(w.at(x, 3, z + 1), "minecraft:wither_skeleton_skull");
    w.hold("minecraft:wither_skeleton_skull");
    // No stem under the middle: not a T.
    w.use_on_top(w.at(x, 2, z));
    w.ticks(2);
    assert!(w.mobs("minecraft:wither").is_empty());
    // With the stem, but peaceful.
    w.set(w.at(x, 3, z), "minecraft:air");
    w.set(w.at(x, 1, z), "minecraft:soul_soil");
    w.run("difficulty peaceful");
    w.use_on_top(w.at(x, 2, z));
    w.ticks(2);
    assert!(w.mobs("minecraft:wither").is_empty());
}

#[test]
fn guardians_swim_in_water() {
    let mut w = World::new();
    let [gx, gy, gz] = w.at(0, 1, 6);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:stone", gx - 4, gy, gz - 4, gx + 4, gy + 4, gz + 4));
    w.run(&format!("fill {} {} {} {} {} {} minecraft:water", gx - 3, gy, gz - 3, gx + 3, gy + 4, gz + 3));
    w.run(&format!("summon minecraft:guardian {} {} {}", gx as f64 + 0.5, gy as f64 + 1.0, gz as f64 + 0.5));
    w.ticks(1);
    let start = w.mobs("minecraft:guardian")[0].1;
    let mut moved = 0.0f64;
    for _ in 0..20 {
        w.ticks(20);
        let g = w.mobs("minecraft:guardian");
        assert_eq!(g.len(), 1, "the guardian stays");
        let p = g[0].1;
        moved = moved.max((p[0] - start[0]).abs() + (p[2] - start[2]).abs());
        assert!(p[1] >= gy as f64 && p[1] < (gy + 6) as f64, "in the pool or at its surface ({p:?})");
    }
    assert!(moved > 0.5, "it swam about ({moved})");
}
