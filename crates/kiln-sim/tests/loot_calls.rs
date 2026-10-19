//! wp83: loot tables the simulation rolls where vanilla does, so a data pack that changes the table changes the game. Each test installs a small data
//! pack of its own (tables written here, replacing the vanilla one of the same id when the vanilla data is present) and checks what the game
//! drops: harvesting sweet berries and glow berries, carving a pumpkin, shearing a beehive, hoeing rooted dirt.

use kiln_blocks::state;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
    sequence: i32,
    dir: PathBuf,
}

impl World {
    /// A survival player in a world with the data pack `p83` holding `tables` (ids relative to `data/minecraft/loot_table`, and their JSON).
    fn new(name: &str, tables: &[(&str, &str)]) -> World {
        let dir = std::env::temp_dir().join(format!("kiln-loot-calls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("datapacks/p83");
        for (id, json) in tables {
            let file = root.join("data/minecraft/loot_table").join(format!("{id}.json"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, json).unwrap();
        }
        std::fs::write(root.join("pack.mcmeta"), r#"{"pack":{"description":"p83","min_format":121,"max_format":121}}"#).unwrap();
        let mut sim = Sim::new(SimConfig::new(4, 3, Some(dir.clone())));
        assert!(sim.step([]));
        sim.console_now("reload");
        sim.console_now("datapack enable \"file/p83\"");
        for _ in 0..3 {
            assert!(sim.step([]));
        }
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        World { sim, client, ground, sequence: 0, dir }
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

    /// The block `(dx, dy, dz)` from the one the player stands on.
    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    /// Puts `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        self.run("gamemode survival User");
    }

    fn use_on(&mut self, pos: [i32; 3], face: i32) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face, cursor: [0.5, 0.5, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
        self.ticks(2);
    }

    /// The items lying about, by name, with their counts added up.
    fn items(&self) -> std::collections::BTreeMap<String, i32> {
        let mut out = std::collections::BTreeMap::new();
        for s in self.sim.item_stacks() {
            *out.entry(s.item_name().to_owned()).or_insert(0) += s.count();
        }
        out
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn item(name: &str, count: i32) -> String {
    format!(r#"{{"type":"minecraft:item","name":"minecraft:{name}","modifier":{{"type":"minecraft:set_count","count":{count}}}}}"#)
}

fn table(kind: &str, pools: &[String]) -> String {
    format!(r#"{{"type":"minecraft:{kind}","pools":[{}]}}"#, pools.join(","))
}

fn pool(entries: &[String]) -> String {
    format!(r#"{{"rolls":1,"entries":[{}]}}"#, entries.join(","))
}

#[test]
fn carving_a_pumpkin_rolls_carve_pumpkin_with_the_shears() {
    // Three diamonds, and an emerald only if the tool in the context is the shears.
    let t = table(
        "block_interact",
        &[
            pool(&[item("diamond", 3)]),
            format!(
                r#"{{"rolls":1,"entries":[{{"type":"minecraft:item","name":"minecraft:emerald","condition":{{"type":"minecraft:match_tool","predicate":{{"items":"minecraft:shears"}}}}}}]}}"#
            ),
        ],
    );
    let mut w = World::new("pumpkin", &[("carve/pumpkin", &t)]);
    let pumpkin = w.at(1, 1, 0);
    w.set(pumpkin, "minecraft:pumpkin");
    w.hold("minecraft:shears", 1);
    w.use_on(pumpkin, 5);
    assert!(state::is(w.block(pumpkin), state::set_dir(kiln_data::blocks::default_state::CARVED_PUMPKIN, "facing", kiln_blocks::Direction::East)), "carved");
    let items = w.items();
    assert_eq!(items.get("minecraft:diamond"), Some(&3), "the pack's table: {items:?}");
    assert_eq!(items.get("minecraft:emerald"), Some(&1), "with the shears as the tool: {items:?}");
    assert!(!items.contains_key("minecraft:pumpkin_seeds"), "{items:?}");
}

#[test]
fn picking_sweet_berries_rolls_harvest_sweet_berry_bush_for_the_bush_as_it_was() {
    // A diamond from a ripe bush (the `block_state` of the context), an emerald from any.
    let ripe = r#"{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","condition":{"type":"minecraft:match_block","blocks":"minecraft:sweet_berry_bush","state":{"age":"3"}}}]}"#;
    let t = table("block_interact", &[ripe.to_owned(), pool(&[item("emerald", 2)])]);
    let mut w = World::new("berries", &[("harvest/sweet_berry_bush", &t)]);
    let bush = w.at(1, 1, 0);
    w.set(bush, "minecraft:sweet_berry_bush[age=3]");
    w.use_on(bush, 1);
    assert_eq!(state::get_int(w.block(bush), "age"), 1, "the bush goes back to its first stage");
    let items = w.items();
    assert_eq!(items.get("minecraft:diamond"), Some(&1), "{items:?}");
    assert_eq!(items.get("minecraft:emerald"), Some(&2), "{items:?}");
    assert!(!items.contains_key("minecraft:sweet_berries"), "{items:?}");

    // An unripe one (stage 2) is picked too, without the ripe pool's diamond; stage 1 is left alone.
    w.set(bush, "minecraft:sweet_berry_bush[age=2]");
    w.use_on(bush, 1);
    let items = w.items();
    assert_eq!(items.get("minecraft:diamond"), Some(&1), "no second diamond: {items:?}");
    assert_eq!(items.get("minecraft:emerald"), Some(&4), "{items:?}");
    w.use_on(bush, 1);
    assert_eq!(w.items().get("minecraft:emerald"), Some(&4), "stage 1 gives nothing");
}

#[test]
fn picking_glow_berries_rolls_harvest_cave_vine() {
    let t = table("block_interact", &[pool(&[item("gold_nugget", 7)])]);
    let mut w = World::new("vines", &[("harvest/cave_vine", &t)]);
    let head = w.at(1, 3, 0);
    let plant = w.at(1, 2, 0);
    w.set(head, "minecraft:stone");
    w.set(plant, "minecraft:cave_vines_plant[berries=true]");
    w.set(w.at(1, 1, 0), "minecraft:cave_vines[age=3,berries=true]");
    w.use_on(w.at(1, 1, 0), 2);
    assert!(!state::get_bool(w.block(w.at(1, 1, 0)), "berries"), "the head is bare");
    assert_eq!(w.items().get("minecraft:gold_nugget"), Some(&7), "{:?}", w.items());
    // The plant part gives its berries as well.
    w.use_on(plant, 2);
    assert!(!state::get_bool(w.block(plant), "berries"));
    assert_eq!(w.items().get("minecraft:gold_nugget"), Some(&14), "{:?}", w.items());
}

#[test]
fn shearing_a_full_beehive_rolls_harvest_beehive() {
    let t = table("block_interact", &[pool(&[item("gold_ingot", 2)])]);
    let mut w = World::new("hive", &[("harvest/beehive", &t)]);
    let hive = w.at(1, 1, 0);
    w.set(hive, "minecraft:beehive[honey_level=5,facing=west]");
    w.hold("minecraft:shears", 1);
    w.use_on(hive, 1);
    assert_eq!(state::get_int(w.block(hive), "honey_level"), 0, "the honey is gone");
    let items = w.items();
    assert_eq!(items.get("minecraft:gold_ingot"), Some(&2), "{items:?}");
    assert!(!items.contains_key("minecraft:honeycomb"), "{items:?}");
}

#[test]
fn hoeing_rooted_dirt_rolls_till_rooted_dirt() {
    let t = table("block_interact", &[pool(&[item("emerald", 3)])]);
    let mut w = World::new("hoe", &[("till/rooted_dirt", &t)]);
    let dirt = w.at(1, 0, 0);
    w.set(dirt, "minecraft:rooted_dirt");
    w.hold("minecraft:iron_hoe", 1);
    w.use_on(dirt, 5);
    assert!(state::same_block(w.block(dirt), kiln_data::blocks::default_state::DIRT), "rooted dirt turns into dirt");
    let items = w.items();
    assert_eq!(items.get("minecraft:emerald"), Some(&3), "{items:?}");
    assert!(!items.contains_key("minecraft:hanging_roots"), "{items:?}");
}

/// The blast of a creeper in a pen of two zombies (day is kept out with the night); `charged` or not.
fn creeper_blast(w: &mut World, charged: bool) {
    let [x, y, z] = w.at(0, 1, 0);
    w.run("gamemode creative User");
    w.run("time set 18000");
    for dz in 0..2 {
        w.run(&format!("summon minecraft:zombie {} {y} {} {{NoAI:1b,Health:1f}}", x + 4, z + dz));
    }
    w.run(&format!("summon minecraft:creeper {} {y} {} {{powered:{}b,ignited:1b,Fuse:2}}", x + 5, z, charged as i32));
    w.ticks(6);
}

#[test]
fn a_charged_creeper_s_victim_drops_what_charged_creeper_root_rolls_once() {
    // Whatever dies in the blast drops a diamond if the creeper is charged, and only the first of its victims does.
    let t = table("entity", &[pool(&[item("diamond", 1)])]);
    let mut w = World::new("charged", &[("charged_creeper/root", &t)]);
    creeper_blast(&mut w, true);
    assert_eq!(w.items().get("minecraft:diamond"), Some(&1), "one head for the creeper: {:?}", w.items());
}

#[test]
fn an_uncharged_creeper_s_victims_drop_no_head() {
    let t = table("entity", &[pool(&[item("diamond", 1)])]);
    let mut w = World::new("uncharged", &[("charged_creeper/root", &t)]);
    creeper_blast(&mut w, false);
    assert!(!w.items().contains_key("minecraft:diamond"), "{:?}", w.items());
}
