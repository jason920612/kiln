//! Player item uses in the running simulation: buckets (filling, emptying, waterlogging,
//! cauldrons).

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
    sequence: i32,
    creative: bool,
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
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
        let mut w = Self { sim, client, ground, sequence: 0, creative: mode == "creative" };
        w.run(&format!("gamemode {mode} User"));
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
        let mode = self.mode();
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        self.run(&format!("gamemode {mode} User"));
    }

    fn mode(&self) -> &'static str {
        // The tests only switch between these two.
        if self.creative { "creative" } else { "survival" }
    }

    /// The item and count in the first hotbar slot.
    fn held(&self) -> Option<(String, i32)> {
        let inv = self.sim.inventory(1).unwrap();
        inv[36].map(|(id, n)| (kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string(), n))
    }

    /// Uses the held item looking at `pitch` (90: straight down).
    fn use_item(&mut self, pitch: f32) {
        self.sequence += 1;
        let pkt = PlayIn::UseItem { hand: Hand::Main, sequence: self.sequence, yaw: 0.0, pitch };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    /// Right-clicks the top face of `pos`.
    fn use_on_top(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }
}

#[test]
fn buckets_fill_and_empty() {
    let mut w = World::new("survival");
    // The player stands on the ground block: looking straight down aims at it.
    let below = w.at(0, 0, 0);
    let feet = w.at(0, 1, 0);
    w.set(below, "minecraft:water");
    w.hold("minecraft:bucket", 2);
    w.use_item(90.0);
    assert!(state::is(w.block(below), d::AIR), "the source was scooped up");
    assert_eq!(w.held(), Some(("minecraft:bucket".into(), 1)));
    let water_bucket = kiln_data::builtin_id("minecraft:item", "minecraft:water_bucket").unwrap();
    let inv = w.sim.inventory(1).unwrap();
    assert!(inv.iter().flatten().any(|&(id, n)| id == water_bucket && n == 1));

    // A filled bucket pours in front of the face it hits.
    w.set(below, "minecraft:stone");
    w.hold("minecraft:lava_bucket", 1);
    w.use_item(90.0);
    assert!(state::is(w.block(feet), d::LAVA), "lava on top of the ground: {}", state::state_string(w.block(feet)));
    assert_eq!(w.held(), Some(("minecraft:bucket".into(), 1)));
    w.use_item(90.0);
    assert!(state::is(w.block(feet), d::AIR), "the lava went back into the bucket");
    assert_eq!(w.held(), Some(("minecraft:lava_bucket".into(), 1)));

    // Water into a waterloggable block; the bucket takes it back out.
    w.set(below, "minecraft:oak_slab[type=bottom]");
    w.hold("minecraft:water_bucket", 1);
    w.use_item(90.0);
    let slab = w.block(below);
    assert!(state::get_bool(slab, "waterlogged"), "{}", state::state_string(slab));
    w.use_item(90.0);
    assert!(!state::get_bool(w.block(below), "waterlogged"));
    assert_eq!(w.held(), Some(("minecraft:water_bucket".into(), 1)));

    // Nothing to pick up: the bucket stays empty.
    w.set(below, "minecraft:stone");
    w.hold("minecraft:bucket", 1);
    w.use_item(90.0);
    assert_eq!(w.held(), Some(("minecraft:bucket".into(), 1)));
    // Adventure players cannot use buckets on blocks.
    w.set(below, "minecraft:water");
    w.run("gamemode adventure User");
    w.use_item(90.0);
    assert!(state::is(w.block(below), d::WATER));
}

#[test]
fn creative_buckets_stay_full() {
    let mut w = World::new("creative");
    let feet = w.at(0, 1, 0);
    w.hold("minecraft:water_bucket", 1);
    w.use_item(90.0);
    assert!(state::is(w.block(feet), d::WATER));
    assert_eq!(w.held(), Some(("minecraft:water_bucket".into(), 1)), "creative players keep the full bucket");
}

#[test]
fn cauldrons_take_and_give() {
    let mut w = World::new("survival");
    let c = w.at(1, 1, 0);
    w.set(c, "minecraft:cauldron");
    w.hold("minecraft:water_bucket", 1);
    w.use_on_top(c);
    let s = w.block(c);
    assert!(state::is(s, state::set_int(d::WATER_CAULDRON, "level", 3)), "{}", state::state_string(s));
    assert_eq!(w.held(), Some(("minecraft:bucket".into(), 1)));
    // Bottles take a level each.
    w.hold("minecraft:glass_bottle", 2);
    w.use_on_top(c);
    assert_eq!(state::get_int(w.block(c), "level"), 2);
    assert_eq!(w.held(), Some(("minecraft:glass_bottle".into(), 1)));
    w.hold("minecraft:glass_bottle", 2);
    w.use_on_top(c);
    w.use_on_top(c);
    assert!(state::is(w.block(c), d::CAULDRON), "three bottles empty it: {}", state::state_string(w.block(c)));
    // Lava in and out.
    w.hold("minecraft:lava_bucket", 1);
    w.use_on_top(c);
    assert!(state::is(w.block(c), d::LAVA_CAULDRON));
    w.use_on_top(c);
    assert!(state::is(w.block(c), d::CAULDRON));
    assert_eq!(w.held(), Some(("minecraft:lava_bucket".into(), 1)));
    // Powder snow placed from its bucket leaves an empty bucket.
    w.hold("minecraft:powder_snow_bucket", 1);
    let top = w.at(2, 1, 0);
    w.use_on_top(w.at(2, 0, 0));
    assert!(state::is(w.block(top), d::POWDER_SNOW));
    assert_eq!(w.held(), Some(("minecraft:bucket".into(), 1)));
    w.ticks(1);
}
