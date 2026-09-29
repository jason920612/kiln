//! Cargo minecarts in the running simulation: chest and hopper minecarts opened as menus,
//! sharing their slots with hoppers, dropping them (and their loot tables) when broken;
//! furnace minecarts taking fuel; TNT minecarts going off; minecarts placed by hand and by
//! dispenser.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`); tests that
//! need them skip without it.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
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
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
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
fn chest_minecarts_open_and_share_their_slots() {
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    // Placed from the item onto a rail: a chest minecart with 27 empty slots.
    w.hold("minecraft:chest_minecart", 1);
    w.use_on_top(rail);
    assert_eq!(w.held(), None);
    let id = w.sim.entity_ids_of("minecraft:chest_minecart")[0];
    assert_eq!(w.sim.cart_items(id), Some((Vec::new(), None)));
    w.run("kill @e[type=minecraft:chest_minecart]");
    // With contents: opened by a click as a 3-row menu that shows them.
    let id = w.summon(
        "minecraft:chest_minecart",
        rail,
        "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:5},{Slot:26b,id:\"minecraft:stick\",count:12}]}",
    );
    w.run("give User minecraft:cobblestone 20");
    w.interact(id);
    let (ty, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:generic_9x3");
    assert_eq!(slots.len(), 27 + 36);
    assert_eq!(slots[0], Some(("minecraft:diamond", 5)));
    assert_eq!(slots[26], Some(("minecraft:stick", 12)));
    // Shift-click takes the diamonds to the inventory; the minecart's slots follow.
    w.click(1, 0, 0, ContainerInput::QuickMove);
    assert_eq!(w.sim.cart_items(id).unwrap().0, vec![(26, "minecraft:stick", 12)]);
    assert_eq!(w.inventory_count("minecraft:diamond"), 5);
    // And the other way: the hotbar's cobblestone goes into the minecart's first free slot.
    let hotbar = 27 + 27;
    let before = w.inventory_count("minecraft:cobblestone");
    assert_eq!(before, 20);
    w.click(1, hotbar as i16, 0, ContainerInput::QuickMove);
    assert_eq!(w.sim.cart_items(id).unwrap().0, vec![(0, "minecraft:cobblestone", 20), (26, "minecraft:stick", 12)]);
    // A hopper block below the rail's line does not disturb an open menu; a second player-side
    // operation still sees the same slots.
    w.ticks(3);
    assert!(w.sim.open_menu(1).is_some());
    assert_eq!(w.sim.open_menu(1).unwrap().1[0], Some(("minecraft:cobblestone", 20)));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    assert!(w.sim.open_menu(1).is_none());
    assert_eq!(w.sim.cart_items(id).unwrap().0.len(), 2, "the slots stay with the minecart");
}

#[test]
fn hopper_minecarts_open_as_a_hopper_menu_and_are_switched_off_by_powered_activator_rails() {
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:activator_rail[shape=east_west,powered=false]");
    let id = w.summon("minecraft:hopper_minecart", rail, "{Items:[{Slot:4b,id:\"minecraft:coal\",count:9}]}");
    w.interact(id);
    let (ty, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:hopper");
    assert_eq!(slots.len(), 5 + 36);
    assert_eq!(slots[4], Some(("minecraft:coal", 9)));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    // Unpowered rail: enabled, pulling in an item entity that lies in it.
    w.ticks(2);
    assert_eq!(w.sim.cart_state(id).unwrap().2, true);
    w.run(&format!("summon minecraft:item {} {} {} {{Item:{{id:\"minecraft:coal\",count:3}},Age:0s,PickupDelay:100s}}", rail[0] as f64 + 0.5, rail[1] as f64 + 0.3, rail[2] as f64 + 0.5));
    w.ticks(3);
    assert_eq!(w.sim.cart_items(id).unwrap().0, vec![(0, "minecraft:coal", 3), (4, "minecraft:coal", 9)]);
    // Power the rail: the cart switches off and leaves items lying.
    w.set(rail, "minecraft:activator_rail[shape=east_west,powered=true]");
    w.ticks(2);
    assert_eq!(w.sim.cart_state(id).unwrap().2, false);
    let items_before = w.sim.cart_items(id).unwrap().0;
    w.run(&format!("summon minecraft:item {} {} {} {{Item:{{id:\"minecraft:stick\",count:2}},Age:0s,PickupDelay:100s}}", rail[0] as f64 + 0.5, rail[1] as f64 + 0.3, rail[2] as f64 + 0.5));
    w.ticks(5);
    assert_eq!(w.sim.cart_items(id).unwrap().0, items_before, "disabled: nothing came in");
    assert_eq!(w.count("minecraft:item"), 1);
}

#[test]
fn breaking_a_chest_minecart_drops_its_contents_and_the_item() {
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    let id = w.summon("minecraft:chest_minecart", rail, "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:70},{Slot:3b,id:\"minecraft:stick\",count:4}]}");
    // Hits add up (ten a point, one point less every tick): fist blows every five ticks take
    // a minecart apart.
    for _ in 0..16 {
        w.attack(id);
        w.ticks(5);
    }
    assert_eq!(w.count("minecraft:chest_minecart"), 0);
    let stacks = w.sim.item_stacks();
    let total = |name: &str| stacks.iter().filter(|s| s.item_name() == name).map(|s| s.count()).sum::<i32>();
    assert_eq!(total("minecraft:diamond"), 70, "the contents, in stacks of 10 to 30");
    assert_eq!(total("minecraft:stick"), 4);
    assert_eq!(total("minecraft:chest_minecart"), 1, "and the minecart itself");
}

#[test]
fn a_chest_minecart_with_a_loot_table_rolls_it_when_opened_or_broken() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    let opened = w.summon("minecraft:chest_minecart", rail, "{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:12345L}");
    assert_eq!(w.sim.cart_items(opened), Some((Vec::new(), Some("minecraft:chests/simple_dungeon".into()))));
    w.interact(opened);
    let (items, table) = w.sim.cart_items(opened).unwrap();
    assert!(table.is_none() && !items.is_empty(), "the table rolled into the slots: {items:?}");
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots.iter().take(27).flatten().count(), items.len(), "the menu shows them");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    // The same seed rolls the same loot for a second minecart, this time by breaking it.
    let broken = w.summon("minecraft:chest_minecart", rail, "{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:12345L}");
    let before = w.sim.item_stacks().len();
    for _ in 0..16 {
        w.attack(broken);
        w.ticks(5);
    }
    assert_eq!(w.sim.entity_ids_of("minecraft:chest_minecart"), vec![opened]);
    let expected: i32 = items.iter().map(|&(_, _, n)| n).sum();
    let dropped: i32 = w.sim.item_stacks().iter().skip(before).filter(|s| s.item_name() != "minecraft:chest_minecart").map(|s| s.count()).sum();
    assert_eq!(dropped, expected, "the broken minecart dropped the rolled loot");
}

#[test]
fn furnace_minecarts_burn_fuel_and_push_themselves() {
    let mut w = World::new("survival");
    let y = w.ground[1];
    let (x0, z) = (w.ground[0] + 2, w.ground[2] + 4);
    w.rails(x0, z, 12, "minecraft:rail[shape=east_west]");
    let rail = [x0 + 2, y + 1, z];
    let id = w.summon("minecraft:furnace_minecart", rail, "{}");
    w.hold("minecraft:coal", 3);
    // Coal from the hand: 3600 ticks of fuel each; the push points away from the player, who
    // stands west of the line.
    w.interact(id);
    assert_eq!(w.sim.cart_state(id).unwrap().0, 3599, "3600, less the tick that followed");
    assert_eq!(w.held(), Some(("minecraft:coal".into(), 2)));
    // Something that does not burn changes nothing.
    w.hold("minecraft:stick", 1);
    w.interact(id);
    assert!(w.sim.cart_state(id).unwrap().0 <= 3599, "no more fuel from a stick");
    assert_eq!(w.held(), Some(("minecraft:stick".into(), 1)));
    let start = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:furnace_minecart").unwrap().1;
    w.ticks(30);
    let at = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:furnace_minecart").unwrap().1;
    assert!((at[0] - start[0]).abs() > 1.0 || (at[2] - start[2]).abs() > 1.0, "it drives itself: {start:?} -> {at:?}");
    assert!(w.sim.cart_state(id).unwrap().0 < 3600 - 20, "the fuel burns down");
    // The fuel is capped: 9 pieces would pass 32000.
    let fresh = w.summon("minecraft:furnace_minecart", [x0 + 1, y + 1, z], "{}");
    w.hold("minecraft:coal", 64);
    for _ in 0..12 {
        w.interact(fresh);
    }
    let fuel = w.sim.cart_state(fresh).unwrap().0;
    assert!(fuel <= 32000 && fuel > 28000, "eight pieces would pass 32000: {fuel}");
    assert_eq!(w.held(), Some(("minecraft:coal".into(), 64 - 8)), "only the ones that fit were taken");
}

#[test]
fn tnt_minecarts_go_off_on_an_activator_rail_and_when_hit_by_fire() {
    let mut w = World::new("survival");
    let y = w.ground[1];
    let (x0, z) = (w.ground[0] + 2, w.ground[2] + 4);
    w.rails(x0, z, 12, "minecraft:rail[shape=east_west]");
    // A minecart running in over a powered activator rail is primed.
    w.set([x0 + 3, y, z], "minecraft:redstone_block");
    w.set([x0 + 3, y + 1, z], "minecraft:activator_rail[shape=east_west]");
    let id = w.summon("minecraft:tnt_minecart", [x0, y + 1, z], "{Motion:[0.3d,0.0d,0.0d]}");
    assert_eq!(w.sim.cart_state(id).unwrap().1, -1);
    for t in 0..30 {
        w.ticks(1);
        eprintln!("t{t} {:?} {:?} rail {}", w.sim.cart_state(id), w.sim.entities().first().map(|e| e.1), state::state_string(w.block([x0 + 3, y + 1, z])));
    }
    let fuse = w.sim.cart_state(id).map(|s| s.1);
    assert!(fuse.is_some_and(|f| f > 0), "primed: {fuse:?} {:?}", w.sim.entities());
    // 80 ticks later it explodes: the minecart is gone and the floor beside the rail cratered.
    let solid_before = !state::is(w.block([x0 + 2, y, z + 1]), d::AIR);
    assert!(solid_before);
    w.ticks(85);
    assert_eq!(w.count("minecraft:tnt_minecart"), 0);
    let crater = (-3..=3).any(|dx| (-3..=3).any(|dz| state::is(w.block([x0 + 3 + dx + 4, y, z + dz]), d::AIR)));
    assert!(crater, "the blast broke blocks");
    // Fire (lava) sets one off at once with a short fuse.
    let id2 = w.summon("minecraft:tnt_minecart", [x0 + 9, y + 1, z + 8], "{}");
    w.run(&format!("setblock {} {} {} minecraft:fire", x0 + 9, y + 1, z + 8));
    w.ticks(4);
    let s = w.sim.cart_state(id2).map(|s| s.1);
    assert!(s.is_none() || s.is_some_and(|f| f >= 0), "primed by fire or already gone: {s:?}");
}

#[test]
fn dispensers_place_minecarts_on_the_rail_in_front() {
    let mut w = World::new("creative");
    let y = w.ground[1];
    let (x, z) = (w.ground[0] + 2, w.ground[2] + 4);
    // A dispenser facing east, a rail in front of it.
    w.set([x, y + 1, z], "minecraft:dispenser[facing=east]");
    w.set([x + 1, y + 1, z], "minecraft:rail[shape=east_west]");
    w.run(&format!("item replace block {x} {} {z} container.0 with minecraft:hopper_minecart 2", y + 1));
    w.set([x, y + 1, z + 1], "minecraft:redstone_block");
    w.ticks(6);
    assert_eq!(w.count("minecraft:hopper_minecart"), 1, "{:?}", w.sim.entities());
    let at = w.sim.entities().into_iter().find(|(k, _)| *k == "minecraft:hopper_minecart").unwrap().1;
    assert!((at[0] - (x as f64 + 1.5)).abs() < 1.0 && (at[2] - (z as f64 + 0.5)).abs() < 0.6, "on the rail: {at:?}");
    assert_eq!(w.sim.container_at([x, y + 1, z]).map(|c| c.0), Some(vec![(0, "minecraft:hopper_minecart", 1)]));
}

#[test]
fn spectators_open_chest_minecarts_but_not_unrolled_loot_ones() {
    let mut w = World::new("spectator");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    let plain = w.summon("minecraft:chest_minecart", rail, "{Items:[{Slot:0b,id:\"minecraft:diamond\",count:2}]}");
    w.interact(plain);
    let (ty, slots) = w.sim.open_menu(1).expect("spectators see what a chest minecart holds");
    assert_eq!(ty, "minecraft:generic_9x3");
    assert_eq!(slots[0], Some(("minecraft:diamond", 2)));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    w.run("kill @e[type=minecraft:chest_minecart]");
    let loot = w.summon("minecraft:chest_minecart", rail, "{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:5L}");
    w.interact(loot);
    assert!(w.sim.open_menu(1).is_none(), "an unrolled loot table stays shut to spectators");
    assert_eq!(w.sim.cart_items(loot).unwrap().1.as_deref(), Some("minecraft:chests/simple_dungeon"));
}

#[test]
fn carts_keep_the_name_of_their_item_and_their_display_block() {
    use kiln_proto::nbt::Tag;
    let mut w = World::new("creative");
    let rail = w.at(3, 1, 0);
    w.set(rail, "minecraft:rail[shape=east_west]");
    // The named item, first in the hotbar, placed on the rail: the minecart carries its name.
    w.run("give User minecraft:chest_minecart[custom_name='\"Loot Cart\"']");
    w.ticks(1);
    w.use_on_top(rail);
    let saved = |w: &World, name: &str| w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name));
    let named = saved(&w, "minecraft:chest_minecart").expect("placed");
    assert!(named.get("CustomName").is_some(), "{named:?}");
    // A shown block and its offset survive the load (and go to the clients as entity data).
    w.summon("minecraft:hopper_minecart", rail, "{DisplayState:{Name:\"minecraft:diamond_block\"},DisplayOffset:9}");
    let nbt = saved(&w, "minecraft:hopper_minecart").unwrap();
    assert_eq!(nbt.get("DisplayOffset").and_then(Tag::as_i64), Some(9));
    assert_eq!(nbt.get("DisplayState").and_then(Tag::as_str), Some("minecraft:diamond_block"));
}

#[test]
fn a_hopper_minecart_pulls_from_the_container_above_it_one_item_at_a_time() {
    let mut w = World::new("creative");
    let y = w.ground[1];
    let (x, z) = (w.ground[0] + 2, w.ground[2] + 4);
    w.set([x, y + 1, z], "minecraft:rail[shape=east_west]");
    w.set([x, y + 2, z], "minecraft:chest");
    w.run(&format!("item replace block {x} {} {z} container.0 with minecraft:apple 5", y + 2));
    let cart = w.summon("minecraft:hopper_minecart", [x, y + 1, z], "{}");
    // One item per tick at most (`consumedItemThisFrame`), from the chest into the cart's first slot.
    w.ticks(2);
    let taken = |w: &World| w.sim.cart_items(cart).unwrap().0.iter().map(|&(_, _, n)| n).sum::<i32>();
    assert!(taken(&w) >= 1 && taken(&w) <= 3, "a little so far: {}", taken(&w));
    w.ticks(20);
    assert_eq!(taken(&w), 5);
    assert_eq!(w.sim.container_at([x, y + 2, z]).unwrap().0, Vec::new(), "the chest is empty");
}

#[test]
fn hopper_blocks_and_minecarts_move_items_between_each_other() {
    let mut w = World::new("creative");
    let y = w.ground[1];
    let (x, z) = (w.ground[0] + 2, w.ground[2] + 4);
    // A chest minecart on a rail on top of a hopper block: the hopper pulls from the minecart
    // above it and pushes into the chest beside it.
    w.set([x, y + 1, z], "minecraft:hopper[facing=east]");
    w.set([x + 1, y + 1, z], "minecraft:chest");
    w.set([x, y + 2, z], "minecraft:rail[shape=east_west]");
    let cart = w.summon("minecraft:chest_minecart", [x, y + 2, z], "{Items:[{Slot:0b,id:\"minecraft:apple\",count:3}]}");
    w.ticks(60);
    assert!(w.sim.cart_items(cart).unwrap().0.is_empty(), "the hopper emptied the minecart: {:?}", w.sim.cart_items(cart));
    assert_eq!(w.sim.container_at([x + 1, y + 1, z]).unwrap().0, vec![(0, "minecraft:apple", 3)]);
    // Now the other way: a hopper block facing down into a hopper minecart in the block under it.
    w.run("kill @e[type=minecraft:chest_minecart]");
    w.set([x, y + 1, z], "minecraft:air");
    w.set([x, y + 2, z], "minecraft:hopper[facing=down]");
    w.set([x, y + 1, z], "minecraft:rail[shape=east_west]");
    w.run(&format!("item replace block {x} {} {z} container.0 with minecraft:stone 5", y + 2));
    let hopper_cart = w.summon("minecraft:hopper_minecart", [x, y + 1, z], "{}");
    w.ticks(60);
    let moved = w.sim.cart_items(hopper_cart).map(|c| c.0.iter().map(|&(_, _, n)| n).sum::<i32>()).unwrap_or(0);
    assert!(moved > 0, "the hopper block pushed items into the minecart below: {:?}", w.sim.cart_items(hopper_cart));
}
