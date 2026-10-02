//! Llamas and trader llamas: the chest and its screen (as many columns as the llama is strong),
//! the carpet in the body slot, what they save, what they drop, and the spit that hurts.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`).

use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
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
}

impl World {
    fn new() -> Self {
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
        let mut w = Self { sim, client };
        w.run("gamemode survival User");
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

    /// Puts `count` of `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        self.run("gamemode survival User");
    }

    fn held(&self) -> Option<(String, i32)> {
        let inv = self.sim.inventory(1).unwrap();
        inv[36].map(|(id, n)| (kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string(), n))
    }

    fn interact(&mut self, id: i32, sneaking: bool) {
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn click(&mut self, container_id: i32, slot: i16, button: i8, input: ContainerInput) {
        let c = ContainerClick { container_id, state_id: 0, slot, button, input, changed: Vec::new(), carried: HashedStack::Empty };
        let mut body = bytes::BytesMut::new();
        c.write(&mut body);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() })]));
    }

    /// Summons `name` next to the player with `extra` saved data; `still`: without AI.
    fn summon(&mut self, name: &str, nbt: &str, still: bool) -> i32 {
        let before = self.sim.entity_ids_of(name);
        let p = self.client.pos;
        let ai = if still { "NoAI:1b," } else { "" };
        self.run(&format!("summon {name} {} {} {} {{{ai}PersistenceRequired:1b{nbt}}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(1);
        *self.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
    }

    fn mob_nbt(&self, name: &str) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }

    fn stacks_total(&self, name: &str) -> i32 {
        self.sim.item_stacks().iter().filter(|s| s.item_name() == name).map(|s| s.count()).sum()
    }
}

#[test]
fn a_chest_goes_on_a_tame_llama_and_its_screen_has_three_slots_per_strength() {
    for strength in [1, 3, 5] {
        let mut w = World::new();
        let llama = w.summon("minecraft:llama", &format!(",Tame:1b,Strength:{strength}"), true);
        w.hold("minecraft:chest", 2);
        w.interact(llama, false);
        assert_eq!(w.held(), Some(("minecraft:chest".into(), 1)), "one chest went on");
        assert_eq!(w.mob_nbt("minecraft:llama").get("ChestedHorse").and_then(Tag::as_i64), Some(1));
        // A sneaking click shows the screen: saddle, carpet, 3 per strength and the player's 36.
        w.hold("minecraft:stone", 5);
        w.interact(llama, true);
        let (_, slots) = w.sim.open_menu(1).expect("the screen opened");
        assert_eq!(slots.len(), 2 + 3 * strength + 36, "strength {strength}");
        // Shift-click the stone: neither saddle nor carpet, so into the chest's first slot.
        let hotbar = 2 + 3 * strength as usize + 27;
        w.click(1, hotbar as i16, 0, ContainerInput::QuickMove);
        let (_, slots) = w.sim.open_menu(1).unwrap();
        assert_eq!(slots[2], Some(("minecraft:stone", 5)));
        let saved = w.mob_nbt("minecraft:llama");
        let Some(Tag::List(items)) = saved.get("Items") else { panic!("{saved:?}") };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("Slot").and_then(Tag::as_i64), Some(0));
    }
}

#[test]
fn a_wild_llama_takes_no_chest_and_a_baby_has_no_screen() {
    let mut w = World::new();
    let llama = w.summon("minecraft:llama", ",Strength:2", true);
    w.hold("minecraft:chest", 1);
    w.interact(llama, false);
    assert_eq!(w.held(), Some(("minecraft:chest".into(), 1)), "a wild llama is angered, not equipped");
    assert_eq!(w.mob_nbt("minecraft:llama").get("ChestedHorse").and_then(Tag::as_i64), Some(0));
    w.interact(llama, true);
    assert!(w.sim.open_menu(1).is_none(), "no screen for a wild llama");
    let baby = w.summon("minecraft:llama", ",Tame:1b,Strength:2,Age:-24000", true);
    w.hold("minecraft:stone", 1);
    w.interact(baby, true);
    assert!(w.sim.open_menu(1).is_none(), "no screen for a baby");
}

#[test]
fn the_body_slot_takes_carpets_and_the_saddle_slot_nothing() {
    let mut w = World::new();
    let llama = w.summon("minecraft:llama", ",Tame:1b,Strength:3,ChestedHorse:1b", true);
    // A carpet in the hand goes on; no saddle does.
    w.hold("minecraft:red_carpet", 1);
    w.interact(llama, false);
    assert_eq!(w.held(), None, "the carpet went on");
    assert!(w.mob_nbt("minecraft:llama").get("equipment").and_then(|e| e.get("body")).is_some());
    // The screen: a saddle and horse armor stay in the player's inventory when shift-clicked.
    w.run("give User minecraft:saddle 1");
    w.run("give User minecraft:iron_horse_armor 1");
    w.run("give User minecraft:white_carpet 1");
    w.ticks(1);
    w.interact(llama, true);
    let (_, slots) = w.sim.open_menu(1).expect("the screen opened");
    assert_eq!(slots[1], Some(("minecraft:red_carpet", 1)));
    let columns = 3 * 3;
    for item in ["minecraft:saddle", "minecraft:iron_horse_armor"] {
        let slot = (2 + columns..2 + columns + 36).find(|&i| w.sim.open_menu(1).unwrap().1[i].is_some_and(|(n, _)| n == item)).expect(item);
        w.click(1, slot as i16, 0, ContainerInput::QuickMove);
        let (_, slots) = w.sim.open_menu(1).unwrap();
        assert_eq!(slots[0], None, "no saddle on a llama");
        // (what is shift-clicked goes into the chest's slots instead)
        assert!(slots[2..2 + columns].iter().any(|s| s.is_some_and(|(n, _)| n == item)), "{item} went in the chest");
    }
    // The carpet slot is taken: a second carpet shift-clicked goes into the chest too.
    assert_eq!(w.sim.open_menu(1).unwrap().1[1], Some(("minecraft:red_carpet", 1)));
}

#[test]
fn a_llama_keeps_strength_coat_chest_items_and_carpet_across_a_save() {
    let mut w = World::new();
    w.summon(
        "minecraft:llama",
        ",Tame:1b,Strength:4,Variant:2,ChestedHorse:1b,Items:[{Slot:0b,id:\"minecraft:diamond\",count:3},{Slot:11b,id:\"minecraft:stick\",count:9}],equipment:{body:{id:\"minecraft:red_carpet\",count:1}}",
        true,
    );
    let saved = w.mob_nbt("minecraft:llama");
    assert_eq!(saved.get("Strength").and_then(Tag::as_i64), Some(4));
    assert_eq!(saved.get("Variant").and_then(Tag::as_i64), Some(2));
    assert_eq!(saved.get("ChestedHorse").and_then(Tag::as_i64), Some(1));
    let Some(Tag::List(items)) = saved.get("Items") else { panic!("{saved:?}") };
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].get("Slot").and_then(Tag::as_i64), Some(11));
    assert_eq!(saved.get("equipment").and_then(|e| e.get("body")).and_then(|b| b.get("id")).and_then(Tag::as_str), Some("minecraft:red_carpet"));
    // The trader llama keeps its despawn delay, and its coat and strength.
    w.summon("minecraft:trader_llama", ",Strength:2,Variant:3,DespawnDelay:1234", true);
    let trader = w.mob_nbt("minecraft:trader_llama");
    assert_eq!(trader.get("DespawnDelay").and_then(Tag::as_i64), Some(1234), "(persistent: it does not count down)");
    assert_eq!(trader.get("Strength").and_then(Tag::as_i64), Some(2));
    assert_eq!(trader.get("Variant").and_then(Tag::as_i64), Some(3));
    // A slot beyond the screen (strength 1: 3 slots) is dropped on load, and a strength of 0 or
    // 9 is brought into 1 to 5.
    w.summon("minecraft:llama", ",Strength:9,ChestedHorse:1b,Items:[{Slot:7b,id:\"minecraft:stick\",count:1}]", true);
    let ids = w.sim.entity_ids_of("minecraft:llama");
    assert_eq!(ids.len(), 2);
    let strengths: Vec<i64> = w.sim.entity_nbt().iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:llama")).filter_map(|t| t.get("Strength").and_then(Tag::as_i64)).collect();
    assert!(strengths.contains(&5), "{strengths:?}");
}

#[test]
fn a_llama_drops_its_chest_items_and_carpet() {
    let mut w = World::new();
    let llama = w.summon(
        "minecraft:llama",
        ",Health:2f,Tame:1b,Strength:3,ChestedHorse:1b,Items:[{Slot:0b,id:\"minecraft:diamond\",count:3},{Slot:8b,id:\"minecraft:stick\",count:9}],equipment:{body:{id:\"minecraft:red_carpet\",count:1}},drop_chances:{body:2.0f}",
        true,
    );
    w.hold("minecraft:diamond_sword", 1);
    w.ticks(25);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: llama })]));
    w.ticks(3);
    assert_eq!(w.stacks_total("minecraft:diamond"), 3);
    assert_eq!(w.stacks_total("minecraft:stick"), 9);
    assert_eq!(w.stacks_total("minecraft:chest"), 1, "the chest");
    assert_eq!(w.stacks_total("minecraft:red_carpet"), 1, "the carpet, guaranteed by its drop chance");
    // The loot table: leather (a trader llama's too).
    let trader = w.summon("minecraft:trader_llama", ",Health:2f,Tame:1b,Strength:2", true);
    w.ticks(25);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: trader })]));
    w.ticks(30);
    assert!(w.sim.entity_ids_of("minecraft:trader_llama").is_empty(), "dead, and gone after its death animation");
}

#[test]
fn a_llama_spits_at_the_player_who_hurt_it() {
    let mut w = World::new();
    let p = w.client.pos;
    let before = w.sim.entity_ids_of("minecraft:llama");
    w.run(&format!("summon minecraft:llama {} {} {} {{Strength:3,Health:50f,PersistenceRequired:1b}}", p[0] + 2.0, p[1], p[2]));
    w.ticks(2);
    let llama = *w.sim.entity_ids_of("minecraft:llama").iter().find(|id| !before.contains(id)).expect("summoned");
    let mut min_health = 20.0f32;
    let mut spit_seen = false;
    for round in 0..4 {
        w.ticks(25);
        assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: llama })]));
        for _ in 0..40 {
            w.ticks(1);
            spit_seen |= w.sim.entities().iter().any(|(n, _)| *n == "minecraft:llama_spit");
            min_health = min_health.min(w.sim.health(1).unwrap().0);
        }
        if round == 0 {
            assert!(spit_seen, "the first grievance is answered with a spit");
        }
    }
    assert!(min_health < 20.0, "a spit hit the player (health went down to {min_health})");
    assert!(min_health >= 16.0, "each spit does 1 damage (health went down to {min_health})");
}

#[test]
fn a_summoned_llama_comes_with_a_strength_a_coat_and_a_health_of_its_own() {
    let mut w = World::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut coats = std::collections::BTreeSet::new();
    for _ in 0..12 {
        w.run("summon minecraft:llama ~ ~ ~ {NoAI:1b}");
        w.ticks(1);
    }
    for t in w.sim.entity_nbt().iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:llama")) {
        let strength = t.get("Strength").and_then(Tag::as_i64).unwrap();
        let coat = t.get("Variant").and_then(Tag::as_i64).unwrap();
        assert!((1..=5).contains(&strength), "strength {strength}");
        assert!((0..=3).contains(&coat), "coat {coat}");
        let health = t.get("Health").and_then(Tag::as_f64).unwrap();
        assert!((15.0..=30.0).contains(&health), "health {health}");
        seen.insert(strength);
        coats.insert(coat);
    }
    assert!(seen.len() >= 2 && coats.len() >= 2, "strengths {seen:?} and coats {coats:?} vary");
}
