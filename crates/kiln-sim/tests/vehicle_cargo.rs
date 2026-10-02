//! Vehicles with cargo in the running simulation: chest boats and rafts (menus, loot tables,
//! drops, hoppers) and arrows hitting carts and boats.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`); tests that
//! need them skip without it.

use kiln_blocks::state;
use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
    sequence: i32,
    mode: &'static str,
}

impl World {
    fn new(mode: &'static str) -> Self {
        have_datapack();
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
        let mut w = Self { sim, client, ground, sequence: 0, mode };
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

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
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
        self.run(&format!("gamemode {} User", self.mode));
    }

    fn held(&self) -> Option<(String, i32)> {
        let inv = self.sim.inventory(1).unwrap();
        inv[36].map(|(id, n)| (kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string(), n))
    }

    fn inventory_count(&self, item: &str) -> i32 {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.sim.inventory(1).unwrap().iter().flatten().filter(|&&(i, _)| i == id).map(|&(_, n)| n).sum()
    }

    fn use_on_top(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn interact(&mut self, id: i32) {
        self.interact_sneaking(id, false);
    }

    fn interact_sneaking(&mut self, id: i32, sneaking: bool) {
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn attack(&mut self, id: i32) {
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id })]));
    }

    fn click(&mut self, container_id: i32, slot: i16, button: i8, input: ContainerInput) {
        let c = ContainerClick { container_id, state_id: 0, slot, button, input, changed: Vec::new(), carried: HashedStack::Empty };
        let mut body = bytes::BytesMut::new();
        c.write(&mut body);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() })]));
    }

    fn count(&self, name: &str) -> usize {
        self.sim.entities().iter().filter(|(k, _)| *k == name).count()
    }

    /// The rail line: `n` rails east from `x0`, on the ground beside the player.
    fn rails(&mut self, x0: i32, z: i32, n: i32, rail: &str) {
        let y = self.ground[1];
        for i in 0..n {
            self.set([x0 + i, y + 1, z], rail);
        }
    }

    /// Summons `name` at the middle of block `pos` (a rail) with saved data.
    fn summon(&mut self, name: &str, pos: [i32; 3], nbt: &str) -> i32 {
        let before = self.sim.entity_ids_of(name);
        self.run(&format!("summon {name} {} {} {} {nbt}", pos[0] as f64 + 0.5, pos[1] as f64 + 0.0625, pos[2] as f64 + 0.5));
        // Commands' spawns join the level in the next tick.
        self.ticks(1);
        *self.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
    }
}

#[test]
fn chest_boats_open_by_sneaking_and_otherwise_take_the_rider() {
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    let id = w.summon(
        "minecraft:oak_chest_boat",
        at,
        "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:5},{Slot:26b,id:\"minecraft:stick\",count:12}]}",
    );
    assert_eq!(w.sim.cart_items(id).unwrap().0.len(), 2);
    // A plain click gets the player aboard; no menu.
    w.interact(id);
    assert!(w.sim.open_menu(1).is_none());
    assert_eq!(w.sim.vehicle_of(1), Some(id));
    // With the boat taken, a plain click opens the menu too (a rider fits no more); a click
    // from the saddle, as from the ground, shows the 3-row menu.
    w.interact(id);
    let (ty, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:generic_9x3");
    assert_eq!(slots[0], Some(("minecraft:diamond", 5)));
    assert_eq!(slots[26], Some(("minecraft:stick", 12)));
    w.click(1, 0, 0, ContainerInput::QuickMove);
    assert_eq!(w.sim.cart_items(id).unwrap().0, vec![(26, "minecraft:stick", 12)]);
    assert_eq!(w.inventory_count("minecraft:diamond"), 5);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    assert!(w.sim.open_menu(1).is_none());
}

#[test]
fn a_sneaking_click_opens_a_chest_boat_instead_of_boarding() {
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    let id = w.summon("minecraft:oak_chest_boat", at, "{Items:[{Slot:1b,id:\"minecraft:stick\",count:2}]}");
    w.interact_sneaking(id, true);
    assert_eq!(w.sim.vehicle_of(1), None);
    assert!(w.sim.open_menu(1).is_some());
}

#[test]
fn plain_boats_do_not_open_and_rafts_carry_chests_too() {
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    let boat = w.summon("minecraft:oak_boat", at, "{}");
    assert_eq!(w.sim.cart_items(boat), None);
    w.interact_sneaking(boat, true);
    assert!(w.sim.open_menu(1).is_none(), "a sneaking click does nothing to a boat");
    let raft = w.summon("minecraft:bamboo_chest_raft", at, "{Items:[{Slot:3b,id:\"minecraft:apple\",count:2}]}");
    w.interact_sneaking(raft, true);
    let (ty, slots) = w.sim.open_menu(1).expect("a chest raft opens");
    assert_eq!(ty, "minecraft:generic_9x3");
    assert_eq!(slots[3], Some(("minecraft:apple", 2)));
}

#[test]
fn breaking_a_chest_boat_drops_its_contents_and_the_item() {
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    let id = w.summon("minecraft:cherry_chest_boat", at, "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:70},{Slot:3b,id:\"minecraft:stick\",count:4}]}");
    for _ in 0..16 {
        w.attack(id);
        w.ticks(5);
    }
    assert_eq!(w.count("minecraft:cherry_chest_boat"), 0);
    let stacks = w.sim.item_stacks();
    let total = |name: &str| stacks.iter().filter(|s| s.item_name() == name).map(|s| s.count()).sum::<i32>();
    assert_eq!(total("minecraft:diamond"), 70);
    assert_eq!(total("minecraft:stick"), 4);
    assert_eq!(total("minecraft:cherry_chest_boat"), 1);
}

#[test]
fn a_creative_player_breaks_a_chest_boat_at_once_and_the_contents_still_drop() {
    let mut w = World::new("creative");
    let at = w.at(3, 1, 0);
    let id = w.summon("minecraft:oak_chest_boat", at, "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:7}]}");
    w.attack(id);
    w.ticks(2);
    assert_eq!(w.count("minecraft:oak_chest_boat"), 0);
    let stacks = w.sim.item_stacks();
    assert_eq!(stacks.iter().filter(|s| s.item_name() == "minecraft:diamond").map(|s| s.count()).sum::<i32>(), 7);
    assert_eq!(stacks.iter().filter(|s| s.item_name() == "minecraft:oak_chest_boat").count(), 0, "creative breaking drops no boat");
}

#[test]
fn a_chest_boat_with_a_loot_table_rolls_it_when_opened() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    let id = w.summon("minecraft:oak_chest_boat", at, "{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:12345L}");
    assert_eq!(w.sim.cart_items(id), Some((Vec::new(), Some("minecraft:chests/simple_dungeon".into()))));
    w.interact_sneaking(id, true);
    let (items, table) = w.sim.cart_items(id).unwrap();
    assert!(table.is_none() && !items.is_empty(), "rolled: {items:?}");
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots.iter().take(27).flatten().count(), items.len());
}

#[test]
fn chest_boats_save_their_slots() {
    use kiln_proto::nbt::Tag;
    let mut w = World::new("survival");
    let at = w.at(3, 1, 0);
    w.summon("minecraft:oak_chest_boat", at, "{Items:[{Slot:5b,id:\"minecraft:diamond\",count:3}]}");
    let nbt = w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:oak_chest_boat")).expect("saved");
    let Some(Tag::List(items)) = nbt.get("Items") else { panic!("{nbt:?}") };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("Slot").and_then(Tag::as_i64), Some(5));
}

#[test]
fn hoppers_take_from_chest_boats() {
    let mut w = World::new("creative");
    let y = w.ground[1];
    let (x, z) = (w.ground[0] + 2, w.ground[2] + 4);
    // A hopper block under a chest boat pulls from it into the chest beside the hopper.
    w.set([x, y + 1, z], "minecraft:hopper[facing=east]");
    w.set([x + 1, y + 1, z], "minecraft:chest");
    let boat = w.summon("minecraft:oak_chest_boat", [x, y + 2, z], "{NoGravity:1b,Items:[{Slot:0b,id:\"minecraft:apple\",count:3}]}");
    w.ticks(60);
    assert!(w.sim.cart_items(boat).is_none_or(|c| c.0.is_empty()), "emptied: {:?}", w.sim.cart_items(boat));
    assert_eq!(w.sim.container_at([x + 1, y + 1, z]).unwrap().0, vec![(0, "minecraft:apple", 3)]);
}

#[test]
fn arrows_break_minecarts_and_burning_ones_set_off_tnt_minecarts() {
    let mut w = World::new("survival");
    let y = w.ground[1];
    let (x0, z) = (w.ground[0] + 2, w.ground[2] + 4);
    w.rails(x0, z, 14, "minecraft:rail[shape=east_west]");
    let cart = w.summon("minecraft:chest_minecart", [x0 + 6, y + 1, z], "{Items:[{Slot:0b,id:\"minecraft:apple\",count:3}]}");
    // A fast arrow from the west along the rail: 3 blocks a tick, 6 damage, ten times that.
    w.run(&format!("summon minecraft:arrow {} {} {} {{Motion:[3.0d,0.0d,0.0d]}}", x0 as f64 + 0.5, y as f64 + 1.35, z as f64 + 0.5));
    w.ticks(8);
    assert_eq!(w.sim.cart_items(cart), None, "broken by the arrow");
    let stacks = w.sim.item_stacks();
    assert_eq!(stacks.iter().filter(|s| s.item_name() == "minecraft:apple").map(|s| s.count()).sum::<i32>(), 3);
    assert_eq!(stacks.iter().filter(|s| s.item_name() == "minecraft:chest_minecart").count(), 1);
    // A burning arrow explodes a TNT minecart on the spot.
    let tnt = w.summon("minecraft:tnt_minecart", [x0 + 10, y + 1, z], "{}");
    w.run(&format!("summon minecraft:arrow {} {} {} {{Motion:[1.0d,0.0d,0.0d],Fire:400s}}", x0 as f64 + 6.5, y as f64 + 1.35, z as f64 + 0.5));
    w.ticks(10);
    assert!(!w.sim.entity_ids_of("minecraft:tnt_minecart").contains(&tnt), "the TNT minecart exploded");
}

#[test]
fn a_tnt_minecart_a_player_set_off_credits_the_player_with_what_it_kills() {
    let mut w = World::new("survival");
    w.run("effect give User minecraft:resistance 1000 4 true");
    let y = w.ground[1];
    let (x0, z) = (w.ground[0] - 1, w.ground[2] + 2);
    w.rails(x0, z, 16, "minecraft:rail[shape=east_west]");
    // A pig with no mind of its own beside the track, a little way on.
    w.run(&format!("summon minecraft:pig {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", x0 as f64 + 6.5, y as f64 + 1.0, z as f64 + 1.5));
    w.hold("minecraft:diamond_sword", 1);
    let cart = w.summon("minecraft:tnt_minecart", [x0 + 2, y + 1, z], "{Motion:[0.14d,0.0d,0.0d]}");
    for _ in 0..10 {
        w.attack(cart);
        w.ticks(1);
        if w.sim.cart_state(cart).is_some_and(|s| s.1 >= 0) {
            break;
        }
    }
    assert!(w.sim.cart_state(cart).is_some_and(|s| s.1 >= 0), "the hits primed it: {:?}", w.sim.cart_state(cart));
    w.ticks(120);
    assert_eq!(w.count("minecraft:tnt_minecart"), 0, "it blew up");
    assert!(w.sim.mobs().iter().all(|m| m.1 != "minecraft:pig"), "the pig died in the blast");
    w.ticks(40);
    let (_, _, xp) = w.sim.experience(1).unwrap();
    assert!(xp > 0, "the player that lit the cart gets the kill's experience (got {xp})");
}

#[test]
fn closing_a_chest_minecart_or_boat_menu_is_heard_as_container_close() {
    let mut w = World::new("survival");
    let y = w.ground[1];
    let sensor = [w.ground[0] + 4, y + 1, w.ground[2] + 4];
    w.set(sensor, "minecraft:sculk_sensor");
    let phase = |w: &World| state::get(w.block(sensor), "sculk_sensor_phase").unwrap_or("?");
    let rail = w.at(2, 1, 3);
    w.set(rail, "minecraft:rail[shape=east_west]");
    let cart = w.summon("minecraft:chest_minecart", rail, "{}");
    w.ticks(60);
    assert_eq!(phase(&w), "inactive");
    // Opening it is heard (`container_open`); after the sensor has rested, closing it is too.
    w.interact(cart);
    assert!(w.sim.open_menu(1).is_some());
    w.ticks(2);
    assert_eq!(phase(&w), "active", "container_open");
    w.ticks(100);
    assert_eq!(phase(&w), "inactive");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    w.ticks(3);
    assert_eq!(phase(&w), "active", "container_close from stopOpen");
    // A chest boat does the same.
    w.ticks(100);
    let boat = w.summon("minecraft:oak_chest_boat", w.at(3, 1, 2), "{NoGravity:1b}");
    w.interact_sneaking(boat, true);
    w.ticks(100);
    assert_eq!(phase(&w), "inactive");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 2 })]));
    w.ticks(3);
    assert_eq!(phase(&w), "active", "a chest boat's menu closing is heard too");
}

/// Whether any piglin in the level has `minecraft:angry_at` in its brain.
fn piglins_angry(w: &World) -> usize {
    use kiln_proto::nbt::Tag;
    w.sim
        .entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:piglin"))
        .filter(|t| match t.get("Brain").and_then(|b| b.get("memories")).and_then(|m| m.get("minecraft:angry_at")) {
            Some(_) => true,
            None => false,
        })
        .count()
}

#[test]
fn opening_or_breaking_a_chest_minecart_angers_nearby_piglins() {
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    w.run("item replace entity User armor.feet with minecraft:golden_boots");
    w.run(&format!("summon minecraft:piglin {} {} {} {{PersistenceRequired:1b,IsImmuneToZombification:1b}}", w.ground[0] as f64 + 3.5, w.ground[1] as f64 + 1.0, w.ground[2] as f64 + 4.5));
    w.ticks(80);
    assert_eq!(piglins_angry(&w), 0);
    let cart = w.summon("minecraft:chest_minecart", rail, "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:2}]}");
    w.interact(cart);
    assert!(w.sim.open_menu(1).is_some());
    w.ticks(2);
    assert_eq!(piglins_angry(&w), 1, "opening the chest minecart angers the piglin");
}

#[test]
fn breaking_a_chest_boat_angers_nearby_piglins_but_a_hopper_minecart_opening_does_not() {
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    w.run("item replace entity User armor.feet with minecraft:golden_boots");
    w.run(&format!("summon minecraft:piglin {} {} {} {{PersistenceRequired:1b,IsImmuneToZombification:1b}}", w.ground[0] as f64 + 3.5, w.ground[1] as f64 + 1.0, w.ground[2] as f64 + 4.5));
    w.ticks(80);
    let hopper = w.summon("minecraft:hopper_minecart", rail, "{}");
    w.interact(hopper);
    w.ticks(2);
    assert_eq!(piglins_angry(&w), 0, "a hopper minecart's menu is not a chest's");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    let boat = w.summon("minecraft:oak_chest_boat", w.at(1, 1, 1), "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:2}]}");
    w.hold("minecraft:diamond_sword", 1);
    w.ticks(30);
    w.attack(boat);
    w.ticks(2);
    assert_eq!(w.count("minecraft:oak_chest_boat"), 0);
    assert_eq!(piglins_angry(&w), 1, "breaking the chest boat angers the piglin");
}
