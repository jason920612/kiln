//! Player item uses in the running simulation: buckets (filling, emptying, waterlogging,
//! cauldrons), thrown and shot projectiles, tools on blocks, composters and golems.

use kiln_blocks::state;

#[test]
fn a_pumpkin_on_iron_blocks_builds_a_golem() {
    let mut w = World::new("creative");
    let base = w.at(3, 1, 0);
    w.set(base, "minecraft:iron_block");
    let body = [base[0], base[1] + 1, base[2]];
    w.set(body, "minecraft:iron_block");
    w.set([body[0] + 1, body[1], body[2]], "minecraft:iron_block");
    w.set([body[0] - 1, body[1], body[2]], "minecraft:iron_block");
    w.hold("minecraft:carved_pumpkin", 1);
    w.use_on_top(body);
    assert_eq!(w.count("minecraft:iron_golem"), 1, "{:?}", w.sim.entities());
    assert!(state::is(w.block(body), d::AIR) && state::is(w.block(base), d::AIR), "the pattern is used up");
}
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

impl World {
    fn release(&mut self) {
        self.sequence += 1;
        let pkt = PlayIn::PlayerAction { action: 6, pos: [0, 0, 0], face: 0, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn count(&self, kind: &str) -> usize {
        self.sim.entities().iter().filter(|(k, _)| *k == kind).count()
    }

    fn inventory_count(&self, item: &str) -> i32 {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.sim.inventory(1).unwrap().iter().flatten().filter(|(i, _)| *i == id).map(|(_, n)| n).sum()
    }

    /// Puts `count` of `item` in inventory menu slot `slot` (9-35 main).
    fn stash(&mut self, slot: i16, item: &str, count: i32) {
        let mode = self.mode();
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot, item: Some(stack) })]));
        self.run(&format!("gamemode {mode} User"));
    }
}

#[test]
fn snowballs_and_pearls_fly() {
    let mut w = World::new("survival");
    w.hold("minecraft:snowball", 4);
    w.use_item(30.0);
    assert_eq!(w.count("minecraft:snowball"), 1, "{:?}", w.sim.entities());
    assert_eq!(w.held(), Some(("minecraft:snowball".into(), 3)));
    w.ticks(60);
    assert_eq!(w.count("minecraft:snowball"), 0, "it hit the ground");

    // An ender pearl lobbed forward takes the player where it lands, for 5 damage.
    w.run("gamerule minecraft:natural_health_regeneration false");
    let before = w.sim.player_level(1).unwrap().1;
    w.hold("minecraft:ender_pearl", 2);
    w.use_item(-20.0);
    assert_eq!(w.held(), Some(("minecraft:ender_pearl".into(), 1)));
    // The pearl cools down for a second.
    w.use_item(-20.0);
    assert_eq!(w.held(), Some(("minecraft:ender_pearl".into(), 1)));
    w.ticks(60);
    let after = w.sim.player_level(1).unwrap().1;
    let moved = ((after[0] - before[0]).powi(2) + (after[2] - before[2]).powi(2)).sqrt();
    assert!(moved > 5.0, "teleported {moved} blocks: {before:?} -> {after:?}");
    let (health, _) = w.sim.health(1).unwrap();
    assert_eq!(health, 15.0, "the pearl's fall damage");
}

#[test]
fn bows_draw_and_shoot_arrows() {
    let mut w = World::new("survival");
    w.hold("minecraft:bow", 1);
    // Without arrows the bow does not draw.
    w.use_item(0.0);
    w.ticks(20);
    w.release();
    assert_eq!(w.count("minecraft:arrow"), 0);
    w.stash(9, "minecraft:arrow", 5);
    w.use_item(0.0);
    // A tap is too weak to shoot.
    w.release();
    assert_eq!(w.count("minecraft:arrow"), 0);
    assert_eq!(w.inventory_count("minecraft:arrow"), 5);
    w.use_item(10.0);
    w.ticks(25);
    w.release();
    assert_eq!(w.count("minecraft:arrow"), 1, "{:?}", w.sim.entities());
    assert_eq!(w.inventory_count("minecraft:arrow"), 4);
    assert_eq!(w.sim.item_damage(1, 36), Some(1), "one durability per arrow");
    // Shot down a little, the arrow sticks in the ground some blocks away; walking over it picks
    // it up.
    w.ticks(20);
    let (_, at) = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:arrow").unwrap();
    assert!((at[1] - (w.ground[1] as f64 + 1.0)).abs() < 0.5, "stuck in the ground: {at:?}");
    let to = [at[0], w.ground[1] as f64 + 1.0, at[2]];
    for _ in 0..5 {
        let mut inbox = Vec::new();
        w.client.tick(Some(to), &mut inbox);
        assert!(w.sim.step(inbox));
    }
    assert_eq!(w.count("minecraft:arrow"), 0, "picked up");
    assert_eq!(w.inventory_count("minecraft:arrow"), 5);
}

#[test]
fn creative_bows_need_no_arrows() {
    let mut w = World::new("creative");
    w.hold("minecraft:bow", 1);
    w.use_item(0.0);
    w.ticks(25);
    w.release();
    assert_eq!(w.count("minecraft:arrow"), 1);
    assert_eq!(w.sim.item_damage(1, 36), Some(0), "creative bows do not wear");
}

#[test]
fn crossbows_load_then_shoot() {
    let mut w = World::new("survival");
    w.hold("minecraft:crossbow", 1);
    w.stash(9, "minecraft:arrow", 3);
    w.use_item(0.0);
    // 1.25 seconds to load.
    w.ticks(26);
    w.release();
    assert_eq!(w.inventory_count("minecraft:arrow"), 2, "one arrow loaded");
    assert_eq!(w.count("minecraft:arrow"), 0);
    w.use_item(0.0);
    assert_eq!(w.count("minecraft:arrow"), 1, "the loaded arrow flies");
    // An empty crossbow loads again rather than shooting.
    w.use_item(0.0);
    w.ticks(5);
    w.release();
    assert_eq!(w.inventory_count("minecraft:arrow"), 2, "released too early to load");
}

#[test]
fn tridents_throw() {
    let mut w = World::new("survival");
    w.hold("minecraft:trident", 1);
    w.use_item(30.0);
    w.ticks(12);
    w.release();
    assert_eq!(w.count("minecraft:trident"), 1);
    assert_eq!(w.held(), None, "thrown out of the hand");
}

/// Whether the vanilla datapack is available (context providers for composting). The
/// repository's `work/generated` is used when `KILN_DATAPACK` is not set.
fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            // Set once, before any simulation in this test binary reads it.
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

#[test]
fn tools_till_flatten_strip_and_wax() {
    let mut w = World::new("survival");
    let a = w.at(2, 0, 0);
    w.set(a, "minecraft:grass_block");
    w.hold("minecraft:wooden_hoe", 1);
    w.use_on_top(a);
    assert!(state::is(w.block(a), d::FARMLAND), "{}", state::state_string(w.block(a)));
    assert_eq!(w.sim.item_damage(1, 36), Some(1));

    w.set(a, "minecraft:dirt");
    w.hold("minecraft:stone_shovel", 1);
    w.use_on_top(a);
    assert!(state::is(w.block(a), d::DIRT_PATH));

    w.set(a, "minecraft:oak_log[axis=x]");
    w.hold("minecraft:iron_axe", 1);
    w.use_on_top(a);
    let log = w.block(a);
    assert!(state::same_block(log, d::STRIPPED_OAK_LOG) && state::get(log, "axis") == Some("x"), "{}", state::state_string(log));

    w.set(a, "minecraft:weathered_cut_copper");
    w.use_on_top(a);
    assert!(state::same_block(w.block(a), d::EXPOSED_CUT_COPPER), "scraped: {}", state::state_string(w.block(a)));

    w.hold("minecraft:honeycomb", 2);
    w.use_on_top(a);
    assert!(state::same_block(w.block(a), d::WAXED_EXPOSED_CUT_COPPER));
    assert_eq!(w.held(), Some(("minecraft:honeycomb".into(), 1)));
    w.hold("minecraft:iron_axe", 1);
    w.use_on_top(a);
    assert!(state::same_block(w.block(a), d::EXPOSED_CUT_COPPER), "wax off");

    // Shears carve a pumpkin toward the player for four seeds.
    w.set(a, "minecraft:pumpkin");
    w.hold("minecraft:shears", 1);
    w.use_on_top(a);
    assert!(state::same_block(w.block(a), d::CARVED_PUMPKIN));
    w.ticks(1);
    assert_eq!(w.count("minecraft:item"), 1);
}

#[test]
fn bone_meal_grows_crops() {
    let mut w = World::new("survival");
    let farm = w.at(2, 0, 0);
    let crop = w.at(2, 1, 0);
    w.set(farm, "minecraft:farmland");
    w.set(crop, "minecraft:wheat[age=0]");
    w.hold("minecraft:bone_meal", 3);
    w.use_on_top(farm);
    // The click on the farmland's top does nothing; on the wheat it grows 2 to 5 stages.
    assert_eq!(state::get_int(w.block(crop), "age"), 0);
    w.use_on_top(crop);
    let age = state::get_int(w.block(crop), "age");
    assert!((2..=5).contains(&age), "age {age}");
    assert_eq!(w.held(), Some(("minecraft:bone_meal".into(), 2)));
    // Grass spreads short grass around.
    let g = w.at(-3, 0, -3);
    for dx in -3..=3 {
        for dz in -3..=3 {
            w.set([g[0] + dx, g[1], g[2] + dz], "minecraft:grass_block");
        }
    }
    w.use_on_top(g);
    let grass = (-3..=3).flat_map(|dx| (-3..=3).map(move |dz| (dx, dz))).filter(|&(dx, dz)| {
        let s = w.block([g[0] + dx, g[1] + 1, g[2] + dz]);
        state::same_block(s, d::SHORT_GRASS) || state::same_block(s, d::TALL_GRASS)
    });
    assert!(grass.count() > 3);
}

#[test]
fn composters_fill_and_give_bone_meal() {
    if !have_datapack() {
        eprintln!("skipped: no vanilla datapack (set KILN_DATAPACK)");
        return;
    }
    let mut w = World::new("survival");
    let c = w.at(1, 1, 0);
    w.set(c, "minecraft:composter");
    // Pumpkin pies always add a layer.
    w.hold("minecraft:pumpkin_pie", 16);
    for _ in 0..7 {
        w.use_on_top(c);
    }
    assert_eq!(state::get_int(w.block(c), "level"), 7);
    assert_eq!(w.held(), Some(("minecraft:pumpkin_pie".into(), 9)));
    // A second later the bone meal is ready; an empty hand takes it.
    w.ticks(25);
    assert_eq!(state::get_int(w.block(c), "level"), 8);
    w.run("gamemode creative User");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: None })]));
    w.run("gamemode survival User");
    w.use_on_top(c);
    assert_eq!(state::get_int(w.block(c), "level"), 0);
    w.ticks(1);
    assert_eq!(w.count("minecraft:item"), 1, "the bone meal");
}

#[test]
fn elytra_glide_wears_the_wings() {
    let mut w = World::new("survival");
    w.run("gamemode creative User");
    let id = kiln_data::builtin_id("minecraft:item", "minecraft:elytra").unwrap();
    let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 6, item: Some(stack) })]));
    w.run("gamemode survival User");
    let mut pos = w.client.pos;
    pos[1] += 5.0;
    let air = |w: &mut World, on_ground: bool, n: usize| {
        for _ in 0..n {
            let pkt = PlayIn::Move { pos: Some(pos), rot: None, on_ground };
            assert!(w.sim.step([ToSim::Packet(1, pkt), ToSim::Packet(1, PlayIn::ClientTickEnd)]));
        }
    };
    // On the ground the command does nothing: no wear.
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 8 })]));
    air(&mut w, true, 25);
    assert_eq!(w.sim.item_damage(1, 6), Some(0));
    // In the air: one durability every 20 ticks of gliding.
    air(&mut w, false, 1);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 8 })]));
    air(&mut w, false, 60);
    assert_eq!(w.sim.item_damage(1, 6), Some(3));
    // Landing ends the flight.
    air(&mut w, true, 25);
    assert_eq!(w.sim.item_damage(1, 6), Some(3));
}

#[test]
fn firework_rockets_launch_and_burst() {
    let mut w = World::new("survival");
    w.hold("minecraft:firework_rocket", 3);
    let top = w.at(0, 0, 3);
    w.use_on_top(top);
    assert_eq!(w.count("minecraft:firework_rocket"), 1, "{:?}", w.sim.entities());
    assert_eq!(w.held(), Some(("minecraft:firework_rocket".into(), 2)));
    w.ticks(5);
    let (_, at) = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:firework_rocket").unwrap();
    assert!(at[1] > top[1] as f64 + 1.5, "rising: {at:?}");
    // Default rockets fly one duration: gone within 10 + 5 + 6 ticks.
    w.ticks(20);
    assert_eq!(w.count("minecraft:firework_rocket"), 0);
    // Used in the air without wings nothing happens.
    w.use_item(0.0);
    assert_eq!(w.count("minecraft:firework_rocket"), 0);
    assert_eq!(w.held(), Some(("minecraft:firework_rocket".into(), 2)));
}
