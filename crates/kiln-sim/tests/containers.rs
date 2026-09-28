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

fn have_datapack() -> bool {
    let dir = std::env::var_os("KILN_DATAPACK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated"));
    dir.join("data/minecraft/recipe").is_dir()
}

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
    sequence: i32,
}

impl World {
    fn new(mode: &str) -> Self {
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
