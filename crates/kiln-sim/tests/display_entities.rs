//! Display entities, interactions and markers in a running simulation: they can be summoned, are kept, and a text
//! display's selectors and scores are resolved once it ticks; a player's hit and click on an interaction are
//! remembered.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
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
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        Self { sim, client }
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

    fn nbt(&self, name: &str) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }

    fn count(&self, name: &str) -> usize {
        self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some(name)).count()
    }
}

#[test]
fn the_data_entities_can_be_summoned() {
    let mut w = World::new();
    let p = w.client.pos;
    for (name, nbt) in [
        ("block_display", "{block_state:\"minecraft:stone\"}"),
        ("item_display", "{item:{id:\"minecraft:diamond\",count:1}}"),
        ("text_display", "{text:\"hello\"}"),
        ("interaction", "{width:2f,height:3f}"),
        ("marker", "{data:{a:1b}}"),
    ] {
        w.run(&format!("summon minecraft:{name} {} {} {} {nbt}", p[0] + 2.0, p[1], p[2]));
        w.ticks(2);
        assert_eq!(w.count(&format!("minecraft:{name}")), 1, "{name}");
    }
    assert_eq!(w.nbt("minecraft:block_display").get("block_state").and_then(Tag::as_str), Some("minecraft:stone"));
    assert_eq!(w.nbt("minecraft:text_display").get("text").and_then(Tag::as_str), Some("hello"));
    assert_eq!(w.nbt("minecraft:interaction").get("width").and_then(Tag::as_f64), Some(2.0));
    assert_eq!(w.nbt("minecraft:marker").get("data").and_then(|d| d.get("a")).and_then(Tag::as_i64), Some(1));
}

#[test]
fn a_text_display_resolves_its_selectors() {
    let mut w = World::new();
    let p = w.client.pos;
    // The display names what the selector finds when it ticks: the player.
    w.run(&format!("summon minecraft:text_display {} {} {} {{text:{{selector:\"@a\"}}}}", p[0] + 2.0, p[1], p[2]));
    w.ticks(3);
    let text = w.nbt("minecraft:text_display").get("text").cloned().expect("text");
    let shown = format!("{text:?}");
    assert!(shown.contains("User"), "the selector was not resolved: {shown}");
    assert!(!shown.contains("selector"), "{shown}");
}

#[test]
fn a_text_display_resolves_its_scores() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run("scoreboard objectives add o dummy");
    w.run("scoreboard players set User o 42");
    w.run(&format!("summon minecraft:text_display {} {} {} {{text:{{score:{{name:\"User\",objective:\"o\"}}}}}}", p[0] + 2.0, p[1], p[2]));
    w.ticks(3);
    let text = w.nbt("minecraft:text_display").get("text").cloned().expect("text");
    let shown = format!("{text:?}");
    assert!(shown.contains("42"), "{shown}");
}

#[test]
fn an_interaction_remembers_the_hit_and_the_click() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run("gamemode survival User");
    w.run(&format!("summon minecraft:interaction {} {} {} {{width:2f,height:2f,response:0b}}", p[0] + 1.0, p[1], p[2]));
    w.ticks(2);
    let id = *w.sim.entity_ids_of("minecraft:interaction").first().expect("interaction");
    assert!(w.nbt("minecraft:interaction").get("attack").is_none());
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id })]));
    let after_hit = w.nbt("minecraft:interaction");
    assert!(after_hit.get("attack").is_some(), "{after_hit:?}");
    assert!(after_hit.get("interaction").is_none());
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false })]));
    let after_click = w.nbt("minecraft:interaction");
    assert!(after_click.get("interaction").is_some(), "{after_click:?}");
}
