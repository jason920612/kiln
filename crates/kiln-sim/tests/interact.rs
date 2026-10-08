//! Player interaction with items and blocks in the running simulation (wp44): editing, dyeing
//! and waxing signs, the editing lock between players, equipping armor with a right click,
//! books, and middle click. The vanilla comparison of the same behaviour is
//! `crates/kiln-sim/src/interact_parity.rs` (needs `tools/interact_vectors.py`).

use kiln_data::packets::play::clientbound as ids;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack as WireStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, SinkStats, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;

struct World {
    sim: Sim,
    client: Client,
    stats: Arc<SinkStats>,
    /// The block the player stands on.
    ground: [i32; 3],
    sequence: i32,
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats.clone());
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        let mut w = Self { sim, client, stats, ground, sequence: 0 };
        w.run(&format!("gamemode {mode} User"));
        *w.stats.log.lock().unwrap() = Some(Vec::new());
        w
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn send(&mut self, pkt: PlayIn) {
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    /// Gives `item` (with `count`) in hotbar slot `slot` through creative mode, then returns to
    /// the mode the player was in.
    fn give(&mut self, slot: i16, item: &str, count: i32) {
        let before = self.sim.game_mode(1).expect("player");
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.send(PlayIn::SetCreativeSlot { slot: 36 + slot, item: Some(WireStack { item: id, count, added: Vec::new(), removed: Vec::new() }) });
        self.run(&format!("gamemode {} User", ["survival", "creative", "adventure", "spectator"][before as usize]));
    }

    fn select(&mut self, slot: i16) {
        self.send(PlayIn::SetCarriedItem { slot });
    }

    fn use_on(&mut self, pos: [i32; 3], hand: i32) {
        self.sequence += 1;
        self.send(PlayIn::UseItemOn { hand, pos, face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: self.sequence });
    }

    fn use_item(&mut self, hand: Hand) {
        self.sequence += 1;
        self.send(PlayIn::UseItem { hand, sequence: self.sequence, yaw: 0.0, pitch: 0.0 });
    }

    fn sign_update(&mut self, pos: [i32; 3], lines: [&str; 4], front: bool) {
        self.send(PlayIn::SignUpdate { pos, lines: Box::new(lines.map(str::to_owned)), front });
    }

    /// The sign text of a side as plain strings, color and glow.
    fn side(&self, pos: [i32; 3], front: bool) -> (Vec<String>, String, bool) {
        let tag = self.sim.block_entity_saved(pos[0], pos[1], pos[2]).expect("sign block entity");
        let side = tag.get(if front { "front_text" } else { "back_text" }).expect("text");
        let lines = side
            .get("messages")
            .and_then(Tag::as_list)
            .expect("messages")
            .iter()
            .map(|m| m.unwrap_list_element().as_str().or_else(|| m.unwrap_list_element().get("text").and_then(Tag::as_str)).unwrap_or("?").to_owned())
            .collect();
        let color = side.get("color").and_then(Tag::as_str).unwrap().to_owned();
        let glow = side.get("has_glowing_text").and_then(Tag::as_i64).unwrap() != 0;
        (lines, color, glow)
    }

    fn waxed(&self, pos: [i32; 3]) -> bool {
        self.sim.block_entity_saved(pos[0], pos[1], pos[2]).and_then(|t| t.get("is_waxed").and_then(Tag::as_i64)).unwrap_or(0) != 0
    }

    /// Whether the sign editor was opened for this player since the last call (for which side).
    fn editor_opened(&mut self) -> Option<bool> {
        let log = std::mem::take(self.stats.log.lock().unwrap().as_mut().unwrap());
        *self.stats.log.lock().unwrap() = Some(Vec::new());
        log.iter().rev().find_map(|p| {
            let mut r = kiln_proto::codec::Reader::new(p);
            (r.varint().ok()? == ids::OPEN_SIGN_EDITOR).then(|| {
                kiln_proto::packets::read_position(&mut r).ok()?;
                Some(r.varint().ok()? != 0)
            })?
        })
    }

    fn inventory_item(&self, slot: usize) -> Option<(String, i32)> {
        let inv = self.sim.inventory(1).unwrap();
        inv[slot].map(|(id, n)| (kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string(), n))
    }
}

/// A standing sign south of the player, whose front faces north (the player).
fn place_sign(w: &mut World) -> [i32; 3] {
    let support = w.at(0, 0, 3);
    let sign = w.at(0, 1, 3);
    w.set(support, "minecraft:stone");
    w.set(sign, "minecraft:oak_sign[rotation=8]");
    sign
}

#[test]
fn a_sign_is_edited_by_the_player_it_opened_the_editor_for() {
    let mut w = World::new("survival");
    let sign = place_sign(&mut w);
    // Nothing happens to a sign nobody opened.
    w.sign_update(sign, ["sneaky", "", "", ""], true);
    assert_eq!(w.side(sign, true).0, ["", "", "", ""]);
    // Right click with an empty hand opens the editor on the side facing the player.
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    w.sign_update(sign, ["Hello", "\u{a7}cRed", "caf\u{e9}", ""], true);
    assert_eq!(w.side(sign, true).0, ["Hello", "Red", "caf\u{e9}", ""], "formatting codes are stripped");
    // The lock was used up.
    w.sign_update(sign, ["again", "", "", ""], true);
    assert_eq!(w.side(sign, true).0[0], "Hello");
    // The text can be edited again, and the back is another text.
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    w.sign_update(sign, ["Edited", "", "", ""], false);
    assert_eq!(w.side(sign, false).0[0], "Edited");
    assert_eq!(w.side(sign, true).0[0], "Hello");
}

#[test]
fn dyes_glow_ink_and_honeycomb_change_the_side_the_player_faces() {
    let mut w = World::new("survival");
    let sign = place_sign(&mut w);
    // An empty sign takes no dye: the editor opens instead.
    w.give(0, "minecraft:red_dye", 3);
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    assert_eq!(w.side(sign, true).1, "black");
    w.sign_update(sign, ["text", "", "", ""], true);
    // Dye with text on it.
    w.use_on(sign, 0);
    assert_eq!(w.side(sign, true).1, "red");
    assert_eq!(w.inventory_item(36), Some(("minecraft:red_dye".into(), 2)), "one dye is used up");
    assert_eq!(w.editor_opened(), None);
    // The same color again changes nothing: the editor opens.
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    assert_eq!(w.inventory_item(36), Some(("minecraft:red_dye".into(), 2)));
    w.sign_update(sign, ["text", "", "", ""], true);
    // Glow ink makes the text glow, an ink sac takes it back.
    w.give(1, "minecraft:glow_ink_sac", 2);
    w.select(1);
    w.use_on(sign, 0);
    assert!(w.side(sign, true).2);
    w.give(2, "minecraft:ink_sac", 2);
    w.select(2);
    w.use_on(sign, 0);
    assert!(!w.side(sign, true).2);
    // Honeycomb waxes the sign; a waxed sign takes no more dye and no text.
    w.give(3, "minecraft:honeycomb", 2);
    w.select(3);
    w.use_on(sign, 0);
    assert!(w.waxed(sign));
    w.select(0);
    w.use_on(sign, 0);
    assert_eq!(w.side(sign, true).1, "red");
    assert_eq!(w.editor_opened(), None, "waxed signs do not open the editor");
    w.sign_update(sign, ["waxed", "", "", ""], true);
    assert_eq!(w.side(sign, true).0[0], "text");
}

#[test]
fn creative_players_keep_their_dye_and_adventure_players_change_nothing() {
    let mut w = World::new("creative");
    let sign = place_sign(&mut w);
    w.sign_update(sign, ["x", "", "", ""], true);
    w.use_on(sign, 0);
    w.sign_update(sign, ["text", "", "", ""], true);
    w.give(0, "minecraft:blue_dye", 2);
    w.use_on(sign, 0);
    assert_eq!(w.side(sign, true).1, "blue");
    assert_eq!(w.inventory_item(36), Some(("minecraft:blue_dye".into(), 2)));
    assert_eq!(w.editor_opened(), Some(true), "creative players edit");
    w.run("gamemode adventure User");
    w.give(1, "minecraft:red_dye", 2);
    w.select(1);
    w.use_on(sign, 0);
    assert_eq!(w.side(sign, true).1, "blue", "adventure players cannot dye");
    assert_eq!(w.editor_opened(), None, "nor edit");
}

#[test]
fn a_sign_in_use_by_another_player_cannot_be_edited() {
    let mut w = World::new("survival");
    let sign = place_sign(&mut w);
    let (msg, stats2) = join(2, "Other", 2);
    assert!(w.sim.step([msg]));
    let mut other = Client::new(2, stats2.clone());
    for _ in 0..5 {
        let mut inbox = Vec::new();
        other.tick(None, &mut inbox);
        assert!(w.sim.step(inbox));
    }
    *stats2.log.lock().unwrap() = Some(Vec::new());
    // The first player opens the editor; the second is turned away.
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    let pkt = PlayIn::UseItemOn { hand: 0, pos: sign, face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: 1 };
    assert!(w.sim.step([ToSim::Packet(2, pkt)]));
    let opened = stats2.log.lock().unwrap().as_ref().unwrap().iter().any(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(ids::OPEN_SIGN_EDITOR));
    assert!(!opened, "the editor does not open for a second player");
    let line = PlayIn::SignUpdate { pos: sign, lines: Box::new(["theirs".to_owned(), String::new(), String::new(), String::new()]), front: true };
    assert!(w.sim.step([ToSim::Packet(2, line)]));
    assert_eq!(w.side(sign, true).0[0], "", "only the player holding the lock edits");
    w.sign_update(sign, ["mine", "", "", ""], true);
    assert_eq!(w.side(sign, true).0[0], "mine");
    // The lock goes when its holder walks away: leave without sending the text.
    w.use_on(sign, 0);
    assert_eq!(w.editor_opened(), Some(true));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: Some([w.client.pos[0] + 40.0, w.client.pos[1], w.client.pos[2]]), rot: None, on_ground: true, horizontal_collision: false })]));
    w.ticks(3);
}

#[test]
fn armor_is_worn_with_a_right_click() {
    let mut w = World::new("survival");
    w.give(0, "minecraft:iron_helmet", 1);
    w.use_item(Hand::Main);
    let helmet = kiln_data::builtin_id("minecraft:item", "minecraft:iron_helmet").unwrap();
    // The head slot is inventory slot 5 of the player's menu.
    assert_eq!(w.sim.inventory(1).unwrap()[5], Some((helmet, 1)), "the helmet is worn");
    assert_eq!(w.inventory_item(36), None, "and gone from the hand");
    // An identical helmet changes nothing.
    w.give(0, "minecraft:iron_helmet", 1);
    w.use_item(Hand::Main);
    assert_eq!(w.sim.inventory(1).unwrap()[5], Some((helmet, 1)));
    assert_eq!(w.inventory_item(36), Some(("minecraft:iron_helmet".into(), 1)));
    // Another one swaps with the worn one.
    w.give(1, "minecraft:diamond_helmet", 1);
    w.select(1);
    w.use_item(Hand::Main);
    let diamond = kiln_data::builtin_id("minecraft:item", "minecraft:diamond_helmet").unwrap();
    assert_eq!(w.sim.inventory(1).unwrap()[5], Some((diamond, 1)));
    assert_eq!(w.inventory_item(37), Some(("minecraft:iron_helmet".into(), 1)), "the worn helmet is in the hand");
    // A carved pumpkin is not swappable.
    w.give(2, "minecraft:carved_pumpkin", 1);
    w.select(2);
    w.use_item(Hand::Main);
    assert_eq!(w.inventory_item(38), Some(("minecraft:carved_pumpkin".into(), 1)));
}

#[test]
fn writable_books_are_edited_and_signed() {
    let mut w = World::new("survival");
    w.give(0, "minecraft:writable_book", 1);
    w.send(PlayIn::EditBook { slot: 0, pages: vec!["one".into(), "two".into()], title: None });
    let inv = w.sim.inventory(1).unwrap();
    let writable = kiln_data::builtin_id("minecraft:item", "minecraft:writable_book").unwrap();
    assert_eq!(inv[36].map(|(id, _)| id), Some(writable), "still a book and quill");
    w.send(PlayIn::EditBook { slot: 0, pages: vec!["one".into(), "two".into()], title: Some("Title".into()) });
    assert_eq!(w.inventory_item(36), Some(("minecraft:written_book".into(), 1)), "signed");
    // A written book cannot be edited again, and slots outside the hotbar are ignored.
    w.send(PlayIn::EditBook { slot: 0, pages: vec!["no".into()], title: Some("Other".into()) });
    assert_eq!(w.inventory_item(36), Some(("minecraft:written_book".into(), 1)));
    w.send(PlayIn::EditBook { slot: 9, pages: vec![], title: Some("x".into()) });
}

#[test]
fn middle_click_picks_the_block_item() {
    let mut w = World::new("survival");
    let stone = w.at(1, 1, 0);
    w.set(stone, "minecraft:stone");
    w.give(3, "minecraft:stone", 5);
    w.send(PlayIn::PickItemFromBlock { pos: stone, include_data: false });
    assert_eq!(w.sim.selected_slot(1), Some(3), "the stone in the hotbar is selected");
    // Survival players are not given what they do not have; creative players are.
    let dirt = w.at(1, 2, 0);
    w.set(dirt, "minecraft:dirt");
    w.send(PlayIn::PickItemFromBlock { pos: dirt, include_data: false });
    assert_eq!(w.sim.selected_slot(1), Some(3));
    w.run("gamemode creative User");
    w.send(PlayIn::PickItemFromBlock { pos: dirt, include_data: false });
    let slot = w.sim.selected_slot(1).unwrap() as usize;
    assert_eq!(w.inventory_item(36 + slot), Some(("minecraft:dirt".into(), 1)));
    // A wheat crop gives its seeds, a wall sign its sign item.
    w.set(w.at(2, 0, 0), "minecraft:farmland");
    w.set(w.at(2, 1, 0), "minecraft:wheat[age=7]");
    w.send(PlayIn::PickItemFromBlock { pos: w.at(2, 1, 0), include_data: false });
    let slot = w.sim.selected_slot(1).unwrap() as usize;
    assert_eq!(w.inventory_item(36 + slot), Some(("minecraft:wheat_seeds".into(), 1)));
}
