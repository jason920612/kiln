//! The eye of ender in the running simulation: used on an end portal frame (the eye goes in,
//! a complete ring opens an end portal) and thrown (it flies toward the nearest stronghold of a
//! generated world and is gone after 80 ticks). The flight itself and the ring pattern are
//! checked against vanilla bit for bit elsewhere (`tools/entity_parity.py`,
//! `tools/block_vectors.py`, `tools/command_diff.py --structures`); this checks the wiring.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
    sequence: i32,
}

impl World {
    fn with(config: SimConfig, mode: &str) -> Self {
        let mut sim = Sim::new(config);
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        let mut w = Self { sim, client, ground, sequence: 0 };
        w.run(&format!("gamemode {mode} User"));
        w
    }

    fn new(mode: &str) -> Self {
        Self::with(SimConfig::new(4, 4, None), mode)
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

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    /// Puts `count` of `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        self.run("gamemode survival User");
    }

    /// The item and count in the first hotbar slot.
    fn held(&self) -> Option<(String, i32)> {
        let inv = self.sim.inventory(1).unwrap();
        inv[36].map(|(id, n)| (kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string(), n))
    }

    fn use_item(&mut self, pitch: f32) {
        self.sequence += 1;
        let pkt = PlayIn::UseItem { hand: Hand::Main, sequence: self.sequence, yaw: 0.0, pitch };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn use_on_top(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn count(&self, kind: &str) -> usize {
        self.sim.entities().iter().filter(|(k, _)| *k == kind).count()
    }
}

/// The twelve frames of a ring around `c` (the middle of the interior), each facing the middle,
/// all with their eye except `empty`.
fn ring(w: &mut World, c: [i32; 3], empty: [i32; 3]) {
    for i in -1..=1 {
        for (dx, dz, facing) in [(i, -2, "south"), (i, 2, "north"), (-2, i, "east"), (2, i, "west")] {
            let p = [c[0] + dx, c[1], c[2] + dz];
            let eye = p != empty;
            w.set(p, &format!("minecraft:end_portal_frame[facing={facing},eye={eye}]"));
        }
    }
}

#[test]
fn the_last_eye_opens_the_portal() {
    let mut w = World::new("survival");
    // A ring in front of the player, the frame at the west edge empty and within reach.
    let c = w.at(4, 0, 0);
    let empty = [c[0] - 2, c[1], c[2]];
    ring(&mut w, c, empty);
    // The inside holds stone and grass: both break.
    w.set([c[0], c[1], c[2]], "minecraft:stone");
    w.set([c[0] + 1, c[1], c[2] + 1], "minecraft:short_grass");
    w.hold("minecraft:ender_eye", 3);
    w.use_on_top(empty);
    assert!(state::get_bool(w.block(empty), "eye"), "the eye is in the frame");
    assert_eq!(w.held(), Some(("minecraft:ender_eye".into(), 2)), "one used up");
    for dx in -1..=1 {
        for dz in -1..=1 {
            let p = [c[0] + dx, c[1], c[2] + dz];
            assert!(state::is(w.block(p), d::END_PORTAL), "{p:?} is end portal: {}", state::state_string(w.block(p)));
        }
    }
    // Using it on a filled frame does nothing.
    w.use_on_top(empty);
    assert_eq!(w.held(), Some(("minecraft:ender_eye".into(), 2)));
}

#[test]
fn a_broken_ring_stays_shut_and_creative_keeps_its_eyes() {
    let mut w = World::new("survival");
    let c = w.at(4, 0, 0);
    let empty = [c[0] - 2, c[1], c[2]];
    ring(&mut w, c, empty);
    // One frame faces away.
    w.set([c[0] + 2, c[1], c[2] + 1], "minecraft:end_portal_frame[facing=east,eye=true]");
    w.hold("minecraft:ender_eye", 1);
    w.use_on_top(empty);
    assert!(state::get_bool(w.block(empty), "eye"));
    assert_eq!(w.held(), None, "the last eye was used up");
    assert!(!state::is(w.block(c), d::END_PORTAL), "no portal: {}", state::state_string(w.block(c)));
    // Creative players keep the item.
    w.set(empty, "minecraft:end_portal_frame[facing=east,eye=false]");
    w.run("gamemode creative User");
    w.hold("minecraft:ender_eye", 1);
    w.run("gamemode creative User");
    w.use_on_top(empty);
    assert!(state::get_bool(w.block(empty), "eye"));
    assert_eq!(w.held(), Some(("minecraft:ender_eye".into(), 1)));
}

#[test]
fn thrown_without_a_stronghold_nothing_happens() {
    // A flat world has no structures: the item is not used.
    let mut w = World::new("survival");
    w.hold("minecraft:ender_eye", 2);
    w.use_item(0.0);
    assert_eq!(w.count("minecraft:eye_of_ender"), 0);
    assert_eq!(w.held(), Some(("minecraft:ender_eye".into(), 2)));
}

fn datapack() -> Option<std::path::PathBuf> {
    let work = std::env::var_os("KILN_WORK").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let dir = std::env::var_os("KILN_DATAPACK").map(std::path::PathBuf::from).unwrap_or_else(|| work.join("generated"));
    dir.join("data").is_dir().then_some(dir)
}

#[test]
fn a_thrown_eye_flies_toward_the_stronghold_and_is_gone_after_80_ticks() {
    let Some(datapack) = datapack() else {
        eprintln!("skipping: no generated data pack (KILN_DATAPACK / KILN_WORK)");
        return;
    };
    // Seed 1: the nearest stronghold to the origin is at x -1136, z 848 (vanilla's /locate).
    let mut config = SimConfig::new(4, 4, None);
    config.noise = Some(NoiseConfig { seed: 1, datapack, threads: 2 });
    let mut w = World::with(config, "survival");
    // Wait for the terrain under the player.
    for _ in 0..600 {
        if w.sim.block_at(w.client.pos[0].floor() as i32, w.client.pos[1].floor() as i32 - 1, w.client.pos[2].floor() as i32).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        w.ticks(1);
    }
    w.hold("minecraft:ender_eye", 2);
    let start = w.client.pos;
    w.use_item(0.0);
    assert_eq!(w.count("minecraft:eye_of_ender"), 1, "the eye is out");
    assert_eq!(w.held(), Some(("minecraft:ender_eye".into(), 1)));
    w.ticks(40);
    let (_, at) = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:eye_of_ender").expect("still flying");
    // Toward -x, +z of where it started, and rising on the way.
    assert!(at[0] < start[0] - 1.0 && at[2] > start[2] + 0.5, "flew toward the stronghold: {start:?} -> {at:?}");
    assert!(at[1] > start[1] + 1.0, "rising: {start:?} -> {at:?}");
    w.ticks(45);
    assert_eq!(w.count("minecraft:eye_of_ender"), 0, "gone after 80 ticks");
    // It dropped as an item (four times in five) or shattered.
    assert!(w.count("minecraft:item") <= 1);
}
