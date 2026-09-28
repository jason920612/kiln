//! Container blocks in the running simulation: chests opened from blocks (double chests,
//! shift-clicks, drops when broken), hoppers moving items along a chain into a chest, furnaces
//! smelting with fuel, comparators reading containers, droppers and ender chests.
//!
//! Recipes and fuel times come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`);
//! tests that need them skip without it.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

/// Whether the vanilla datapack is available (recipes, loot, enchantments). The repository's
/// `work/generated` is used when `KILN_DATAPACK` is not set, so the simulation (which looks
/// relative to the working directory, the crate here) finds it too.
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

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
    sequence: i32,
}

impl World {
    fn new(mode: &str) -> Self {
        have_datapack();
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Keeper"))]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground, sequence: 0 }
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

    fn setblock(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    fn hold(&mut self, slot: i16, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot, item: Some(stack) })]));
    }

    /// Right-clicks the top face of `pos` with the main hand.
    fn use_on(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn click(&mut self, container_id: i32, slot: i16, button: i8, input: ContainerInput) {
        let c = ContainerClick { container_id, state_id: 0, slot, button, input, changed: Vec::new(), carried: HashedStack::Empty };
        let mut body = bytes::BytesMut::new();
        c.write(&mut body);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() })]));
    }

    fn close(&mut self, container_id: i32) {
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id })]));
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn items(&self, p: [i32; 3]) -> Vec<(usize, &'static str, i32)> {
        self.sim.container_at(p).expect("container").0
    }
}

#[test]
fn chests_open_take_items_and_drop_them_when_broken() {
    let mut w = World::new("survival");
    let chest = w.at(2, 1, 0);
    w.setblock(chest, "minecraft:chest");
    w.run("give Keeper minecraft:cobblestone 20");
    w.use_on(chest);
    let (ty, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:generic_9x3");
    assert_eq!(slots.len(), 27 + 36);
    // Shift-click the hotbar's first slot (menu slot 27 + 27) into the chest.
    w.click(1, 54, 0, ContainerInput::QuickMove);
    assert_eq!(w.items(chest), vec![(0, "minecraft:cobblestone", 20)]);
    w.close(1);
    assert!(w.sim.open_menu(1).is_none());
    // Breaking the chest drops its contents (and the chest itself).
    w.run(&format!("setblock {} {} {} minecraft:air destroy", chest[0], chest[1], chest[2]));
    let items = w.sim.entities().iter().filter(|(k, _)| *k == "minecraft:item").count();
    assert!(items >= 1, "{:?}", w.sim.entities());
    assert!(w.sim.container_at(chest).is_none());
}

#[test]
fn chests_beside_each_other_open_as_a_double_chest() {
    let mut w = World::new("creative");
    // Facing east, a left half joins the right half south of it.
    let (a, b) = (w.at(2, 1, 1), w.at(2, 1, 0));
    w.setblock(a, "minecraft:chest[facing=east,type=right]");
    w.setblock(b, "minecraft:chest[facing=east,type=left]");
    w.run(&format!("setblock {} {} {} minecraft:chest[facing=east,type=left]{{Items:[{{Slot:0b,id:\"minecraft:stone\",count:5}}]}}", b[0], b[1], b[2]));
    w.use_on(a);
    let (ty, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:generic_9x6");
    // The right half comes first: the left half's slot 0 is menu slot 27.
    assert_eq!(slots[27], Some(("minecraft:stone", 5)));
    // A solid block on top keeps it shut.
    w.close(1);
    w.setblock(w.at(2, 2, 1), "minecraft:stone");
    w.use_on(a);
    assert!(w.sim.open_menu(1).is_none());
}

#[test]
fn a_hopper_chain_feeds_a_chest() {
    let mut w = World::new("creative");
    let top = w.at(3, 4, 3);
    let (h1, h2, bottom) = (w.at(3, 3, 3), w.at(3, 2, 3), w.at(3, 1, 3));
    w.setblock(bottom, "minecraft:chest");
    w.setblock(h2, "minecraft:hopper[facing=down]");
    w.setblock(h1, "minecraft:hopper[facing=down]");
    w.run(&format!("setblock {} {} {} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:oak_log\",count:3}}]}}", top[0], top[1], top[2]));
    w.ticks(60);
    assert_eq!(w.items(bottom), vec![(0, "minecraft:oak_log", 3)]);
    assert!(w.items(top).is_empty());
    // A powered hopper is locked.
    w.setblock([h1[0] + 1, h1[1], h1[2]], "minecraft:redstone_block");
    assert!(!state::get_bool(w.block(h1), "enabled"));
    w.run(&format!("setblock {} {} {} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:oak_log\",count:3}}]}}", top[0], top[1], top[2]));
    w.ticks(40);
    assert_eq!(w.items(top), vec![(0, "minecraft:oak_log", 3)]);
}

#[test]
fn hoppers_pick_up_items_and_comparators_read_them() {
    let mut w = World::new("creative");
    let hopper = w.at(3, 1, -3);
    w.setblock(hopper, "minecraft:hopper[facing=down]");
    let comparator = [hopper[0] + 1, hopper[1], hopper[2]];
    w.setblock(comparator, "minecraft:comparator[facing=west]");
    // A broken block's drop lands in the hopper's pickup area.
    let above = [hopper[0], hopper[1] + 1, hopper[2]];
    w.setblock(above, "minecraft:dirt");
    w.run(&format!("setblock {} {} {} minecraft:air destroy", above[0], above[1], above[2]));
    w.ticks(10);
    assert_eq!(w.items(hopper), vec![(0, "minecraft:dirt", 1)]);
    assert!(w.sim.entities().iter().all(|(k, _)| *k != "minecraft:item"), "the item entity is gone");
    w.ticks(4);
    // 1/64 in 1 of 5 slots: floor(0.003 * 14) + 1 = 1.
    assert!(state::get_bool(w.block(comparator), "powered"));
}

#[test]
fn furnaces_smelt_with_fuel() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("creative");
    let furnace = w.at(-3, 1, 3);
    w.run(&format!(
        "setblock {} {} {} minecraft:furnace[facing=north]{{Items:[{{Slot:0b,id:\"minecraft:raw_iron\",count:2}},{{Slot:1b,id:\"minecraft:coal\",count:1}}]}}",
        furnace[0], furnace[1], furnace[2]
    ));
    w.ticks(5);
    assert!(state::get_bool(w.block(furnace), "lit"), "the furnace lights");
    let (_, data) = w.sim.container_at(furnace).unwrap();
    assert_eq!(data[1], 1600, "coal burns for 1600 ticks");
    assert_eq!(data[3], 200, "iron takes 200 ticks");
    w.ticks(400);
    let (items, _) = w.sim.container_at(furnace).unwrap();
    assert_eq!(items, vec![(2, "minecraft:iron_ingot", 2)]);
}

/// A beacon on a one-level iron pyramid lights (`construct_beacon`); an iron ingot pays for
/// speed, which reaches the player on the next 80-tick pulse.
#[test]
fn beacons_light_and_give_their_power() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let b = w.at(3, 2, 3);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:iron_block", b[0] - 1, b[1] - 1, b[2] - 1, b[0] + 1, b[1] - 1, b[2] + 1));
    w.setblock(b, "minecraft:beacon");
    w.ticks(170);
    if let Some(done) = w.sim.criterion_done(1, "minecraft:nether/create_beacon", "beacon") {
        assert!(done, "construct_beacon when it lights");
    }
    // Open it, put the payment in, choose speed.
    w.run("gamemode creative Keeper");
    w.hold(36, "minecraft:iron_ingot", 1);
    w.use_on(b);
    w.ticks(1);
    w.click(1, 28, 0, ContainerInput::QuickMove);
    let speed = kiln_data::synced_id("minecraft:mob_effect", "minecraft:speed").or_else(|| kiln_data::builtin_id("minecraft:mob_effect", "minecraft:speed"));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetBeacon { primary: speed, secondary: None })]));
    w.close(1);
    w.ticks(90);
    let effects = w.sim.effects(1).unwrap();
    assert!(effects.iter().any(|e| e.0 == "minecraft:speed" && e.1 == 0), "the beacon's speed: {effects:?}");
    assert_eq!(w.sim.inventory(1).unwrap()[36], None, "the payment was used");
}

/// A brewing stand with a water bottle, nether wart and blaze powder: the bottle shows in the
/// block state, and 400 ticks later the potion is awkward; taking it fires `brewed_potion`.
#[test]
fn brewing_stands_brew_with_blaze_powder() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let stand = w.at(2, 1, 0);
    let water = "{Slot:0b,id:\"minecraft:potion\",count:1,components:{\"minecraft:potion_contents\":{potion:\"minecraft:water\"}}}";
    w.run(&format!(
        "setblock {} {} {} minecraft:brewing_stand{{Items:[{water},{{Slot:3b,id:\"minecraft:nether_wart\",count:1}},{{Slot:4b,id:\"minecraft:blaze_powder\",count:1}}]}}",
        stand[0], stand[1], stand[2]
    ));
    w.ticks(3);
    assert!(state::get_bool(w.block(stand), "has_bottle_0") && !state::get_bool(w.block(stand), "has_bottle_1"));
    let (_, data) = w.sim.container_at(stand).unwrap();
    // (Fuel, total fuel, brew time, total brew time.)
    assert_eq!((data[0], data[1], data[3]), (19, 20, 400), "blaze powder gives 20 brews, one under way");
    w.ticks(410);
    let (items, _) = w.sim.container_at(stand).unwrap();
    assert_eq!(items.iter().map(|i| (i.0, i.1)).collect::<Vec<_>>(), vec![(0, "minecraft:potion")], "the wart is used up");
    // Open it and shift-click the potion out.
    w.use_on(stand);
    w.ticks(1);
    w.click(1, 0, 0, ContainerInput::QuickMove);
    w.ticks(1);
    let (items, _) = w.sim.container_at(stand).unwrap();
    assert!(items.is_empty(), "the potion was taken: {items:?}");
    if let Some(done) = w.sim.criterion_done(1, "minecraft:nether/brew_potion", "potion") {
        assert!(done, "brewed_potion");
    }
}

#[test]
fn droppers_drop_and_feed_containers() {
    let mut w = World::new("creative");
    let dropper = w.at(-3, 1, -3);
    let chest = [dropper[0] + 1, dropper[1], dropper[2]];
    w.setblock(chest, "minecraft:chest");
    w.run(&format!(
        "setblock {} {} {} minecraft:dropper[facing=east]{{Items:[{{Slot:4b,id:\"minecraft:dirt\",count:2}}]}}",
        dropper[0], dropper[1], dropper[2]
    ));
    // A redstone block beside it fires it once.
    w.setblock([dropper[0], dropper[1], dropper[2] - 1], "minecraft:redstone_block");
    w.ticks(6);
    assert_eq!(w.items(chest), vec![(0, "minecraft:dirt", 1)]);
    assert_eq!(w.items(dropper), vec![(4, "minecraft:dirt", 1)]);
}

#[test]
fn ender_chests_show_the_players_own_items() {
    let mut w = World::new("survival");
    let ender = w.at(2, 1, -2);
    w.setblock(ender, "minecraft:ender_chest");
    w.run("give Keeper minecraft:diamond 3");
    w.use_on(ender);
    let (ty, _) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(ty, "minecraft:generic_9x3");
    w.click(1, 54, 0, ContainerInput::QuickMove);
    w.close(1);
    let other = w.at(-2, 1, 2);
    w.setblock(other, "minecraft:ender_chest");
    w.use_on(other);
    let (_, slots) = w.sim.open_menu(1).expect("a menu opened");
    assert_eq!(slots[0], Some(("minecraft:diamond", 3)));
    let _ = d::AIR;
}

#[test]
fn shulker_boxes_keep_their_contents_when_broken() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let shulker = w.at(2, 1, 2);
    w.run(&format!(
        "setblock {} {} {} minecraft:red_shulker_box{{Items:[{{Slot:3b,id:\"minecraft:emerald\",count:7}}]}}",
        shulker[0], shulker[1], shulker[2]
    ));
    w.run(&format!("setblock {} {} {} minecraft:air destroy", shulker[0], shulker[1], shulker[2]));
    w.ticks(1);
    let stacks = w.sim.item_stacks();
    assert_eq!(stacks.len(), 1, "only the box drops, not its contents: {stacks:?}");
    let contents = stacks[0].get(kiln_item::keys::CONTAINER).expect("the box keeps its contents");
    assert_eq!(stacks[0].item_name(), "minecraft:red_shulker_box");
    let emerald = contents.0.get(3).and_then(|s| s.as_ref()).expect("slot 3");
    assert_eq!(emerald.create().count(), 7);
}

#[test]
fn a_comparator_on_a_filled_chest_lights_a_lamp() {
    let mut w = World::new("creative");
    let chest = w.at(5, 1, -5);
    let comparator = [chest[0] + 1, chest[1], chest[2]];
    let lamp = [chest[0] + 2, chest[1], chest[2]];
    w.setblock(chest, "minecraft:chest");
    w.setblock(comparator, "minecraft:comparator[facing=west]");
    w.setblock(lamp, "minecraft:redstone_lamp");
    let hopper = [chest[0], chest[1] + 1, chest[2]];
    w.run(&format!("setblock {} {} {} minecraft:hopper[facing=down]{{Items:[{{Slot:0b,id:\"minecraft:stone\",count:2}}]}}", hopper[0], hopper[1], hopper[2]));
    w.ticks(10);
    assert_eq!(w.items(chest), vec![(0, "minecraft:stone", 2)]);
    assert!(state::get_bool(w.block(comparator), "powered"), "{}", state::state_string(w.block(comparator)));
    assert!(state::get_bool(w.block(lamp), "lit"), "{}", state::state_string(w.block(lamp)));
}

#[test]
fn workstations_open_their_menus_and_the_anvil_renames() {
    let mut w = World::new("creative");
    let stations = [
        ("minecraft:crafting_table", "minecraft:crafting"),
        ("minecraft:stonecutter", "minecraft:stonecutter"),
        ("minecraft:smithing_table", "minecraft:smithing"),
        ("minecraft:grindstone[face=floor]", "minecraft:grindstone"),
        ("minecraft:loom", "minecraft:loom"),
        ("minecraft:cartography_table", "minecraft:cartography_table"),
        ("minecraft:enchanting_table", "minecraft:enchantment"),
        ("minecraft:anvil", "minecraft:anvil"),
    ];
    for (i, (block, menu)) in stations.iter().enumerate() {
        let pos = w.at(-4 + i as i32, 1, 2);
        w.setblock(pos, block);
        w.use_on(pos);
        assert_eq!(w.sim.open_menu(1).map(|m| m.0), Some(*menu), "{block}");
    }
    // The anvil is open: an iron pickaxe gets a new name.
    w.hold(36, "minecraft:iron_pickaxe", 1);
    w.click(8, 30, 0, ContainerInput::QuickMove);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::RenameItem { name: "Digger".into() })]));
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[2], Some(("minecraft:iron_pickaxe", 1)));
    // Breaking the anvil closes the menu (its block is gone).
    let anvil = w.at(3, 1, 2);
    w.setblock(anvil, "minecraft:air");
    w.ticks(1);
    assert!(w.sim.open_menu(1).is_none());
}

#[test]
fn enchanting_tables_enchant_for_creative_players() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("creative");
    let table = w.at(3, 1, -3);
    w.setblock(table, "minecraft:enchanting_table");
    // A ring of bookshelves two blocks out.
    for (dx, dz) in [(-2, -2), (-2, 0), (-2, 2), (0, -2), (0, 2), (2, -2), (2, 0), (2, 2)] {
        w.setblock([table[0] + dx, table[1], table[2] + dz], "minecraft:bookshelf");
    }
    w.hold(36, "minecraft:book", 1);
    w.use_on(table);
    assert_eq!(w.sim.open_menu(1).map(|m| m.0), Some("minecraft:enchantment"));
    w.click(1, 29, 0, ContainerInput::QuickMove);
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[0], Some(("minecraft:book", 1)));
    // The third option (creative players need no lapis or levels).
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerButtonClick { container_id: 1, button_id: 2 })]));
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[0], Some(("minecraft:enchanted_book", 1)), "the book got enchanted");
}

fn saved_world(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl World {
    fn in_world(dir: &std::path::Path) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, Some(dir.to_owned())));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival Keeper".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        World { sim, client, ground, sequence: 0 }
    }
}

#[test]
fn containers_and_ender_items_survive_unloading_and_a_restart() {
    let dir = saved_world("containers-persist");
    let mut w = World::in_world(&dir);
    let chest = w.at(2, 1, 0);
    let ender = w.at(-2, 1, 0);
    w.setblock(chest, "minecraft:chest");
    w.setblock(ender, "minecraft:ender_chest");
    // Items that got into the chest through the menu (live state only, not yet in the chunk).
    w.run("give Keeper minecraft:cobblestone 20");
    w.use_on(chest);
    w.click(1, 54, 0, ContainerInput::QuickMove);
    w.close(1);
    w.run("give Keeper minecraft:emerald 4");
    w.use_on(ender);
    w.click(2, 54, 0, ContainerInput::QuickMove);
    w.close(2);
    assert_eq!(w.items(chest), vec![(0, "minecraft:cobblestone", 20)]);
    // Away far enough for the chunk to unload, then back.
    let home = w.client.pos;
    w.run(&format!("tp Keeper {} {} {}", home[0] + 5000.0, home[1], home[2]));
    w.ticks(60);
    assert!(w.sim.container_at(chest).is_none(), "the chunk unloaded");
    w.run(&format!("tp Keeper {} {} {}", home[0], home[1], home[2]));
    w.ticks(5);
    assert_eq!(w.items(chest), vec![(0, "minecraft:cobblestone", 20)]);
    // A restart.
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!w.sim.step([ToSim::Shutdown { done }]));
    drop(w);
    let mut w = World::in_world(&dir);
    assert_eq!(w.items(chest), vec![(0, "minecraft:cobblestone", 20)]);
    w.use_on(ender);
    let (_, slots) = w.sim.open_menu(1).expect("the ender chest opens");
    assert_eq!(slots[0], Some(("minecraft:emerald", 4)), "the player's ender items came back");
}

#[test]
fn locked_chests_open_only_with_the_key() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("survival");
    let chest = w.at(2, 1, 0);
    w.run(&format!("setblock {} {} {} minecraft:chest{{lock:{{items:\"minecraft:diamond\"}}}}", chest[0], chest[1], chest[2]));
    w.use_on(chest);
    assert!(w.sim.open_menu(1).is_none(), "locked without the key");
    w.run("give Keeper minecraft:diamond 1");
    w.use_on(chest);
    assert_eq!(w.sim.open_menu(1).map(|m| m.0), Some("minecraft:generic_9x3"));
}

#[test]
fn dispensers_shoot_arrows_and_place_water() {
    let mut w = World::new("creative");
    let arrows = w.at(-4, 1, 4);
    w.run(&format!("setblock {} {} {} minecraft:dispenser[facing=up]{{Items:[{{Slot:0b,id:\"minecraft:arrow\",count:5}}]}}", arrows[0], arrows[1], arrows[2]));
    let water = w.at(4, 1, 4);
    w.run(&format!("setblock {} {} {} minecraft:dispenser[facing=north]{{Items:[{{Slot:0b,id:\"minecraft:water_bucket\",count:1}}]}}", water[0], water[1], water[2]));
    w.setblock([arrows[0] + 1, arrows[1], arrows[2]], "minecraft:redstone_block");
    w.setblock([water[0] + 1, water[1], water[2]], "minecraft:redstone_block");
    w.ticks(6);
    assert!(w.sim.entities().iter().any(|(k, _)| *k == "minecraft:arrow"), "{:?}", w.sim.entities());
    assert_eq!(w.items(arrows), vec![(0, "minecraft:arrow", 4)]);
    assert!(state::is(w.block([water[0], water[1], water[2] - 1]), d::WATER));
    assert_eq!(w.items(water), vec![(0, "minecraft:bucket", 1)]);
}
