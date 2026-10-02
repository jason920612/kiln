//! The screens of horses, donkeys and mules (`HorseInventoryMenu`): chests, saddle and armor
//! slots, what the animal drops and saves.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`).

use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_proto::nbt::Tag;
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

fn interact_sneaking(w: &mut World, id: i32, sneaking: bool) {
    let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking };
    assert!(w.sim.step([ToSim::Packet(1, pkt)]));
}

fn mob_nbt(w: &World, name: &str) -> Tag {
    w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
}

fn summon_tame(w: &mut World, name: &str, extra: &str) -> i32 {
    let before = w.sim.entity_ids_of(name);
    let p = w.client.pos;
    w.run(&format!("summon {name} {} {} {} {{NoAI:1b,Tame:1b,PersistenceRequired:1b{extra}}}", p[0] + 1.5, p[1], p[2]));
    w.ticks(1);
    *w.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
}

#[test]
fn a_chest_goes_on_a_tame_donkey_and_its_screen_holds_15_slots() {
    let mut w = World::new("survival");
    let donkey = summon_tame(&mut w, "minecraft:donkey", "");
    w.hold("minecraft:chest", 2);
    interact_sneaking(&mut w, donkey, false);
    assert_eq!(w.held(), Some(("minecraft:chest".into(), 1)), "one chest went on");
    assert_eq!(mob_nbt(&w, "minecraft:donkey").get("ChestedHorse").and_then(Tag::as_i64), Some(1));
    // A sneaking click shows the screen: saddle, armor, 15 chest slots and the player's 36.
    w.hold("minecraft:stone", 5);
    interact_sneaking(&mut w, donkey, true);
    let (_, slots) = w.sim.open_menu(1).expect("the screen opened");
    assert_eq!(slots.len(), 2 + 15 + 36);
    // Shift-click the stone: not a saddle, not armor, so into the chest's first slot.
    let hotbar = 2 + 15 + 27;
    w.click(1, hotbar as i16, 0, ContainerInput::QuickMove);
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[2], Some(("minecraft:stone", 5)));
    // The animal keeps it, saved with its slot number.
    let saved = mob_nbt(&w, "minecraft:donkey");
    let Some(Tag::List(items)) = saved.get("Items") else { panic!("{saved:?}") };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("Slot").and_then(Tag::as_i64), Some(0));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    assert!(w.sim.open_menu(1).is_none());
}

#[test]
fn the_saddle_and_armor_slots_take_what_the_animal_can_wear() {
    let mut w = World::new("survival");
    let horse = summon_tame(&mut w, "minecraft:horse", "");
    let donkey = summon_tame(&mut w, "minecraft:donkey", "");
    w.hold("minecraft:saddle", 1);
    interact_sneaking(&mut w, horse, true);
    assert!(w.sim.open_menu(1).is_some());
    // Shift-click: the saddle goes to the saddle slot.
    let hotbar = 2 + 27;
    w.click(1, hotbar as i16, 0, ContainerInput::QuickMove);
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[0], Some(("minecraft:saddle", 1)));
    w.run("give User minecraft:diamond_horse_armor 1");
    w.ticks(1);
    let armor_slot = (2 + 27..2 + 36).chain(2..2 + 27).find(|&i| w.sim.open_menu(1).unwrap().1[i].is_some_and(|(n, _)| n == "minecraft:diamond_horse_armor")).expect("the armor in the menu");
    w.click(1, armor_slot as i16, 0, ContainerInput::QuickMove);
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[1], Some(("minecraft:diamond_horse_armor", 1)), "horse armor goes on a horse");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    // A donkey wears no horse armor: shift-click leaves it in the inventory.
    w.run("give User minecraft:iron_horse_armor 1");
    w.ticks(1);
    interact_sneaking(&mut w, donkey, true);
    assert!(w.sim.open_menu(1).is_some());
    let hotbar = 2 + 27;
    let slot = (hotbar..hotbar + 9).chain(2..2 + 27).find(|&i| w.sim.open_menu(1).unwrap().1[i].is_some_and(|(n, _)| n == "minecraft:iron_horse_armor")).expect("armor");
    w.click(1, slot as i16, 0, ContainerInput::QuickMove);
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert_eq!(slots[1], None, "no horse armor on a donkey");
}

#[test]
fn horse_armor_in_the_hand_goes_on_a_horse_and_the_animal_drops_all_it_wears_and_carries() {
    let mut w = World::new("survival");
    let horse = summon_tame(&mut w, "minecraft:horse", "");
    w.hold("minecraft:iron_horse_armor", 1);
    interact_sneaking(&mut w, horse, false);
    assert_eq!(w.held(), None, "put on");
    assert!(mob_nbt(&w, "minecraft:horse").get("equipment").and_then(|e| e.get("body")).is_some());
    let donkey = summon_tame(
        &mut w,
        "minecraft:donkey",
        ",Health:2f,ChestedHorse:1b,Items:[{Slot:0b,id:\"minecraft:diamond\",count:3},{Slot:14b,id:\"minecraft:stick\",count:9}],equipment:{saddle:{id:\"minecraft:saddle\",count:1}},drop_chances:{saddle:2.0f}",
    );
    w.hold("minecraft:diamond_sword", 1);
    w.ticks(25);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: donkey })]));
    w.ticks(3);
    let stacks = w.sim.item_stacks();
    let total = |name: &str| stacks.iter().filter(|s| s.item_name() == name).map(|s| s.count()).sum::<i32>();
    assert_eq!(total("minecraft:diamond"), 3);
    assert_eq!(total("minecraft:stick"), 9);
    assert_eq!(total("minecraft:chest"), 1, "the chest");
    assert_eq!(total("minecraft:saddle"), 1, "the saddle, guaranteed by its drop chance");
}

#[test]
fn a_tame_donkey_keeps_its_chest_across_a_save() {
    let mut w = World::new("survival");
    summon_tame(&mut w, "minecraft:mule", ",ChestedHorse:1b,Items:[{Slot:3b,id:\"minecraft:apple\",count:4}]");
    let saved = mob_nbt(&w, "minecraft:mule");
    let Some(Tag::List(items)) = saved.get("Items") else { panic!("{saved:?}") };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("Slot").and_then(Tag::as_i64), Some(3));
    // Without a chest the slots are not saved at all.
    summon_tame(&mut w, "minecraft:donkey", ",Items:[{Slot:3b,id:\"minecraft:apple\",count:4}]");
    assert!(mob_nbt(&w, "minecraft:donkey").get("Items").is_none());
}
