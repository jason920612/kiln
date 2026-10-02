//! Parrots: tamed with seeds (one in ten tries), poisoned and killed by cookies, and what they
//! save (variant, owner) and show their viewers.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
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

    fn interact(&mut self, id: i32) {
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    /// Summons a parrot next to the player (without AI, so that it stays put).
    fn summon(&mut self, nbt: &str) -> i32 {
        let before = self.sim.entity_ids_of("minecraft:parrot");
        let p = self.client.pos;
        self.run(&format!("summon minecraft:parrot {} {} {} {{NoAI:1b,PersistenceRequired:1b{nbt}}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(1);
        *self.sim.entity_ids_of("minecraft:parrot").iter().find(|id| !before.contains(id)).expect("summoned")
    }

    fn parrot_nbt(&self) -> Option<Tag> {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:parrot"))
    }
}

#[test]
fn seeds_tame_a_wild_parrot_one_try_in_ten_and_are_eaten_each_time() {
    let mut w = World::new();
    let parrot = w.summon(",Variant:3");
    assert!(w.parrot_nbt().unwrap().get("Owner").is_none());
    w.hold("minecraft:wheat_seeds", 64);
    let mut tries = 0;
    while w.parrot_nbt().unwrap().get("Owner").is_none() {
        tries += 1;
        assert!(tries <= 200, "one in ten: not tame after {tries} seeds");
        w.interact(parrot);
    }
    assert_eq!(w.held(), Some(("minecraft:wheat_seeds".into(), 64 - tries)), "a seed is eaten per try");
    let nbt = w.parrot_nbt().unwrap();
    assert_eq!(nbt.get("Variant").and_then(Tag::as_i64), Some(3));
    assert_eq!(nbt.get("Sitting").and_then(Tag::as_i64), Some(0), "a tamed parrot does not sit down by itself");
}

#[test]
fn a_cookie_poisons_a_parrot_to_death() {
    let mut w = World::new();
    let parrot = w.summon("");
    w.hold("minecraft:cookie", 3);
    w.interact(parrot);
    assert_eq!(w.held(), Some(("minecraft:cookie".into(), 2)));
    // The death animation takes 20 ticks, then the parrot is gone and a feather may drop.
    assert!(w.parrot_nbt().is_some() || w.sim.entity_ids_of("minecraft:parrot").is_empty());
    w.ticks(30);
    assert!(w.sim.entity_ids_of("minecraft:parrot").is_empty(), "the parrot died");
}
