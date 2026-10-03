//! Leads: a lead on a mob or a boat (`Entity.interact`), a lead on a fence (`LeadItem`, the
//! knot), what the lead does (pulls, snaps when too far, drops as an item), shears, and the
//! `leash` tag that a save keeps.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    sequence: i32,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let mut w = Self { sim, client, sequence: 100 };
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

    fn click_block(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn summon(&mut self, name: &str, nbt: &str) -> i32 {
        let before = self.sim.entity_ids_of(name);
        let p = self.client.pos;
        self.run(&format!("summon {name} {} {} {} {{PersistenceRequired:1b{nbt}}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(1);
        *self.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
    }

    fn nbt(&self, name: &str) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }

    fn leads(&self) -> i32 {
        self.sim.item_stacks().iter().filter(|s| s.item_name() == "minecraft:lead").map(|s| s.count()).sum()
    }
}

fn pos_of(t: &Tag) -> [f64; 3] {
    let l = t.get("Pos").and_then(Tag::as_list).unwrap();
    [0, 1, 2].map(|i| l[i].as_f64().unwrap())
}

#[test]
fn a_lead_ties_a_cow_to_the_player_and_a_click_takes_it_off() {
    let mut w = World::new();
    let cow = w.summon("minecraft:cow", ",NoAI:1b");
    w.hold("minecraft:lead", 3);
    w.interact(cow, false);
    assert_eq!(w.held(), Some(("minecraft:lead".into(), 2)), "a lead went on");
    let saved = w.nbt("minecraft:cow");
    let leash = saved.get("leash").expect("a leash is saved");
    assert!(leash.get("UUID").is_some(), "led by the player: its UUID ({leash:?})");
    // A click with the lead on an already led cow does nothing (the lead is the player's).
    w.interact(cow, false);
    assert_eq!(w.held(), Some(("minecraft:lead".into(), 2)));
    // An empty hand: the lead comes off, as an item.
    w.run("clear User");
    w.interact(cow, false);
    w.ticks(2);
    assert!(w.nbt("minecraft:cow").get("leash").is_none(), "unleashed");
    assert_eq!(w.leads(), 1, "the lead dropped");
}

#[test]
fn a_lead_that_is_stretched_too_far_snaps_and_drops() {
    let mut w = World::new();
    let cow = w.summon("minecraft:cow", ",NoAI:1b");
    w.hold("minecraft:lead", 1);
    w.interact(cow, false);
    w.ticks(2);
    assert!(w.nbt("minecraft:cow").get("leash").is_some());
    // The player is 20 blocks away in one go: more than 12, it snaps at once.
    let p = w.client.pos;
    w.run(&format!("tp User {} {} {}", p[0] - 20.0, p[1], p[2]));
    w.ticks(3);
    assert!(w.nbt("minecraft:cow").get("leash").is_none(), "the lead snapped");
    assert_eq!(w.leads(), 1);
}

#[test]
fn a_lead_pulls_a_mob_in_when_it_is_stretched_a_little() {
    let mut w = World::new();
    // (A mob without AI does not move at all, pulled or not.)
    let cow = w.summon("minecraft:cow", "");
    w.hold("minecraft:lead", 1);
    w.interact(cow, false);
    w.ticks(2);
    // 9 blocks away: past the elastic distance, within the snap distance.
    let p = w.client.pos;
    w.run(&format!("tp User {} {} {}", p[0] - 9.0, p[1], p[2]));
    let player = [p[0] - 9.0, p[1], p[2]];
    let start = (pos_of(&w.nbt("minecraft:cow"))[0] - player[0]).abs();
    w.ticks(40);
    let end = (pos_of(&w.nbt("minecraft:cow"))[0] - player[0]).abs();
    assert!(w.nbt("minecraft:cow").get("leash").is_some(), "still led");
    assert!(start > 5.0 && end < start - 2.0, "the lead pulled the cow in ({start} -> {end})");
}

#[test]
fn shears_cut_a_lead_and_it_drops() {
    let mut w = World::new();
    let cow = w.summon("minecraft:cow", ",NoAI:1b");
    w.hold("minecraft:lead", 1);
    w.interact(cow, false);
    w.hold("minecraft:shears", 1);
    w.interact(cow, false);
    w.ticks(2);
    assert!(w.nbt("minecraft:cow").get("leash").is_none(), "cut");
    assert_eq!(w.leads(), 1);
    // The shears took the damage.
    assert_eq!(w.held().map(|h| h.0), Some("minecraft:shears".into()));
}

#[test]
fn enemies_and_villagers_cannot_be_led() {
    let mut w = World::new();
    let zombie = w.summon("minecraft:zombie", ",NoAI:1b");
    let villager = w.summon("minecraft:villager", ",NoAI:1b");
    w.hold("minecraft:lead", 2);
    w.interact(zombie, false);
    w.interact(villager, false);
    assert_eq!(w.held(), Some(("minecraft:lead".into(), 2)), "neither took a lead");
    assert!(w.nbt("minecraft:zombie").get("leash").is_none());
}

#[test]
fn a_lead_on_a_fence_makes_a_knot_and_the_mob_stays_led() {
    let mut w = World::new();
    let cow = w.summon("minecraft:cow", ",NoAI:1b");
    let p = w.client.pos;
    let (fx, fy, fz) = (p[0].floor() as i32 + 2, p[1].floor() as i32, p[2].floor() as i32 + 2);
    w.run(&format!("setblock {fx} {fy} {fz} minecraft:oak_fence"));
    w.hold("minecraft:lead", 1);
    w.interact(cow, false);
    w.ticks(1);
    // Click on the fence: the lead goes to a knot on it.
    w.click_block([fx, fy, fz]);
    w.ticks(3);
    assert_eq!(w.sim.entity_ids_of("minecraft:leash_knot").len(), 1, "a knot appeared");
    let cow_nbt = w.nbt("minecraft:cow");
    assert_eq!(cow_nbt.get("leash"), Some(&Tag::IntArray(vec![fx, fy, fz])), "led by the knot on the fence");
    let knot = w.nbt("minecraft:leash_knot");
    assert_eq!(knot.get("block_pos"), Some(&Tag::IntArray(vec![fx, fy, fz])));
    // Taking the lead off the knot (a click on the cow with an empty hand is the player's: the
    // lead goes to the player; the knot goes away with its last lead).
    w.run("clear User");
    let knot_id = w.sim.entity_ids_of("minecraft:leash_knot")[0];
    w.interact(knot_id, false);
    w.ticks(3);
    let cow_nbt = w.nbt("minecraft:cow");
    assert!(cow_nbt.get("leash").is_some_and(|l| l.get("UUID").is_some()), "back on the player: {cow_nbt:?}");
    assert!(w.sim.entity_ids_of("minecraft:leash_knot").is_empty(), "the knot is gone with its lead");
}

#[test]
fn breaking_the_fence_drops_the_lead() {
    let mut w = World::new();
    let cow = w.summon("minecraft:cow", ",NoAI:1b");
    let p = w.client.pos;
    let (fx, fy, fz) = (p[0].floor() as i32 + 2, p[1].floor() as i32, p[2].floor() as i32 + 2);
    w.run(&format!("setblock {fx} {fy} {fz} minecraft:oak_fence"));
    w.hold("minecraft:lead", 1);
    w.interact(cow, false);
    w.click_block([fx, fy, fz]);
    w.ticks(3);
    assert!(w.nbt("minecraft:cow").get("leash").is_some_and(|l| matches!(l, Tag::IntArray(_))));
    // The fence goes: the knot checks every 100 ticks, then the lead falls.
    w.run(&format!("setblock {fx} {fy} {fz} minecraft:air"));
    w.ticks(120);
    assert!(w.nbt("minecraft:cow").get("leash").is_none());
    assert!(w.sim.entity_ids_of("minecraft:leash_knot").is_empty());
    assert_eq!(w.leads(), 1);
}

#[test]
fn a_boat_can_be_led_and_a_saved_lead_finds_its_holder_again() {
    let mut w = World::new();
    let boat = w.summon("minecraft:oak_boat", "");
    w.hold("minecraft:lead", 1);
    w.interact(boat, false);
    w.ticks(1);
    let saved = w.nbt("minecraft:oak_boat");
    assert!(saved.get("leash").is_some_and(|l| l.get("UUID").is_some()), "{saved:?}");
    // Dragged away it is pulled toward the player (boats are Leashable too).
    let p = w.client.pos;
    w.run(&format!("tp User {} {} {}", p[0] - 9.0, p[1], p[2]));
    w.ticks(2);
    let player = w.client.pos;
    let start = (pos_of(&w.nbt("minecraft:oak_boat"))[0] - player[0]).abs();
    w.ticks(20);
    let end = (pos_of(&w.nbt("minecraft:oak_boat"))[0] - player[0]).abs();
    assert!(end < start, "the lead pulled the boat in ({start} -> {end})");
}

/// The client is told, for every mob led by a knot, the knot's real id (the last Set Entity Link
/// each mob got), however many mobs share it and whatever order they and the knot appear in.
#[test]
fn every_mob_on_a_knot_is_sent_the_knots_id() {
    let mut w = World::new();
    *w.client.stats.log.lock().unwrap() = Some(Vec::new());
    let p = w.client.pos;
    let (fx, fy, fz) = (p[0].floor() as i32 + 4, p[1].floor() as i32, p[2].floor() as i32 + 4);
    w.run(&format!("setblock {fx} {fy} {fz} minecraft:oak_fence"));
    let sheep: Vec<i32> = (0..3).map(|i| w.summon("minecraft:sheep", &format!(",leash:[I;{fx},{fy},{fz}],Tags:[\"s{i}\"]"))).collect();
    w.ticks(10);
    let knots = w.sim.entity_ids_of("minecraft:leash_knot");
    assert_eq!(knots.len(), 1, "one knot for all three: {knots:?}");
    let mut last = std::collections::BTreeMap::new();
    let log: Vec<bytes::Bytes> = w.client.stats.log.lock().unwrap().clone().unwrap();
    for pkt in &log {
        let mut r = kiln_proto::Reader::new(pkt);
        if r.varint().ok() == Some(kiln_data::packets::play::clientbound::SET_ENTITY_LINK) {
            let (source, dest) = (r.i32().unwrap(), r.i32().unwrap());
            last.insert(source, dest);
        }
    }
    for id in sheep {
        assert_eq!(last.get(&id), Some(&knots[0]), "sheep {id} is shown on the knot {} ({last:?})", knots[0]);
    }
}
