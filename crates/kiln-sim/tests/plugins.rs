//! WASM plugins in the running simulation (design §11): the example plugins built for
//! wasm32-wasip2 and loaded from a plugin directory. Spawn protection denies breaking and
//! placing near spawn (the client gets its blocks back and a message), the chat formatter
//! rewrites chat, and the counter counts broken blocks per player and in total (`/broken`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

/// A call budget no call reaches: tests run next to builds, and a preempted call that runs
/// out of its 500 µs would be denied (fail-closed) for reasons the test is not about.
fn settings(dir: impl Into<std::path::PathBuf>) -> kiln_sim::PluginSettings {
    kiln_sim::PluginSettings { call_budget: std::time::Duration::from_millis(500), ..kiln_sim::PluginSettings::new(dir) }
}

struct World {
    sim: Sim,
    client: Client,
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

impl World {
    fn new() -> Self {
        let mut config = SimConfig::new(4, 4, None);
        config.plugins = Some(settings(kiln_plugin_host::examples::custom_dir("sim-plugins", &["chat-format", "counter", "petting", "spawn-protection"], &[]).expect("example plugins")));
        let mut sim = Sim::new(config);
        let (msg, stats) = join(1, "Builder", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg, ToSim::Console("gamemode creative Builder".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut w = World { sim, client: Client::new(1, stats) };
        w.ticks(5);
        let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
        let stack = ItemStack { item: stone, count: 64, added: Vec::new(), removed: Vec::new() };
        assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn send(&mut self, pkt: PlayIn) {
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn ground(&self) -> [i32; 3] {
        let p = self.client.pos;
        [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32]
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    /// Packets received since the last call.
    fn received(&mut self) -> Vec<bytes::Bytes> {
        std::mem::take(self.client.stats.log.lock().unwrap().as_mut().unwrap())
    }

    fn got_text(&mut self, text: &str) -> bool {
        self.received().iter().any(|p| contains(p, text))
    }
}

#[test]
fn spawn_protection_chat_format_and_counter() {
    let mut w = World::new();
    let g = w.ground();
    assert!(g[0].abs() < 16 && g[2].abs() < 16, "joined at the spawn ({g:?})");
    let near = [g[0] + 2, g[1], g[2]];
    let before = w.block(near);
    w.received();

    // Breaking near spawn is denied: the block stays and the client hears why.
    w.send(PlayIn::PlayerAction { action: 0, pos: near, face: 1, sequence: 1 });
    assert_eq!(w.block(near), before, "protected block not broken");
    assert!(w.got_text("This area is protected"));
    // So is placing.
    w.send(PlayIn::UseItemOn { hand: 0, pos: near, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 2 });
    assert_eq!(w.block([near[0], near[1] + 1, near[2]]), 0, "nothing placed");

    // Chat comes out formatted.
    w.received();
    w.send(PlayIn::Chat { message: "hello plugins".into() });
    let chat = w.received();
    assert!(chat.iter().any(|p| contains(p, "] \u{bb} ") && contains(p, "hello plugins")), "formatted chat");

    // Far from spawn the player builds and breaks; the counter sees the breaks.
    assert!(w.sim.step([ToSim::Console("tp Builder 300 -60 300".into())]));
    w.ticks(10);
    let g = w.ground();
    assert!(g[0] > 250, "teleported ({g:?})");
    let far = [g[0] + 2, g[1], g[2]];
    w.send(PlayIn::PlayerAction { action: 0, pos: far, face: 1, sequence: 3 });
    assert_eq!(w.block(far), 0, "broken far from spawn");
    w.send(PlayIn::UseItemOn { hand: 0, pos: [far[0], far[1] - 1, far[2]], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 4 });
    assert_ne!(w.block(far), 0, "placed far from spawn");
    w.send(PlayIn::PlayerAction { action: 0, pos: far, face: 1, sequence: 5 });
    w.ticks(2);
    w.received();
    w.send(PlayIn::ChatCommand { command: "broken".into() });
    assert!(w.got_text("You broke "), "the counter replied");
    let uuid = uuid::Uuid::from_u64_pair(0x6b69_6c6e, 1);
    let st = w.sim.plugin_player_value(uuid, "counter", "broken");
    assert_eq!(st, Some(2i64.to_le_bytes().to_vec()), "two breaks counted");
}

fn stack(name: &str, count: i32) -> ItemStack {
    let item = kiln_data::builtin_id("minecraft:item", name).unwrap();
    ItemStack { item, count, added: Vec::new(), removed: Vec::new() }
}

#[test]
fn a_water_bucket_near_spawn_is_denied() {
    let mut w = World::new();
    w.send(PlayIn::SetCreativeSlot { slot: 36, item: Some(stack("minecraft:water_bucket", 1)) });
    w.ticks(2);
    w.received();
    // Looking straight down at the protected ground.
    w.send(PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 7, yaw: 0.0, pitch: 90.0 });
    assert!(w.got_text("This area is protected"), "the bucket goes through the protection check");
}

/// Entity-scoped data: petting a cow twice counts twice, the count lives in the cow's NBT
/// (`kiln:plugin`), and other entity types never reach the plugin.
#[test]
fn petting_keeps_its_count_in_the_entity() {
    let mut w = World::new();
    let p = w.client.pos;
    assert!(w.sim.step([ToSim::Console(format!("summon minecraft:cow {} {} {} {{NoAI:1b}}", p[0] + 1.5, p[1], p[2]))]));
    w.ticks(2);
    let cow = w.sim.mobs().into_iter().find(|m| m.1 == "minecraft:cow").expect("a cow").0;
    w.received();
    let pet = PlayIn::Interact { entity_id: cow, hand: kiln_proto::packets::serverbound::Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
    w.send(pet.clone());
    w.send(pet);
    w.ticks(1);
    assert!(w.got_text("2 times."), "the second pet counted twice");
    let is_cow = |t: &kiln_proto::nbt::Tag| t.get("id").and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:cow");
    let nbt = w.sim.entity_nbt().into_iter().find(is_cow).expect("the cow saves");
    let pets = nbt.get("kiln:plugin").and_then(|t| t.get("petting")).and_then(|t| t.get("pets")).cloned();
    assert_eq!(pets, Some(kiln_proto::nbt::Tag::ByteArray(2i64.to_le_bytes().map(|b| b as i8).to_vec())), "saved with the entity");
}

/// A survival break the client finishes early completes later on the server clock; the
/// counter still sees it (observed in the block phase that completes it).
#[test]
fn delayed_survival_breaks_are_observed() {
    let mut w = World::new();
    assert!(w.sim.step([ToSim::Console("tp Builder 300 -60 300".into()), ToSim::Console("gamemode survival Builder".into())]));
    w.ticks(10);
    let g = w.ground();
    let at = [g[0] + 1, g[1], g[2]];
    let before = w.block(at);
    w.send(PlayIn::PlayerAction { action: 0, pos: at, face: 1, sequence: 1 });
    w.ticks(2);
    // Stop destroying (3), far too early.
    w.send(PlayIn::PlayerAction { action: 3, pos: at, face: 1, sequence: 2 });
    assert_eq!(w.block(at), before, "not broken yet: the client was early");
    // Grass by hand takes 18 ticks, five times as long off the ground.
    w.ticks(120);
    assert_eq!(w.block(at), 0, "broken once the server's clock agreed");
    let uuid = uuid::Uuid::from_u64_pair(0x6b69_6c6e, 1);
    assert_eq!(w.sim.plugin_player_value(uuid, "counter", "broken"), Some(1i64.to_le_bytes().to_vec()), "observed");
}

/// `/kiln plugins reload chat-format` after its manifest changed: the next chat line comes
/// out in the new format.
#[test]
fn reload_by_command_changes_the_chat_format() {
    let dir = kiln_plugin_host::examples::custom_dir("reload-sim", &["chat-format", "counter"], &[]).expect("example plugins");
    let mut config = SimConfig::new(4, 4, None);
    config.plugins = Some(settings(&dir));
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Talker", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    assert!(sim.step([msg]));
    let mut w = World { sim, client: Client::new(1, stats) };
    w.ticks(3);
    w.received();
    w.send(PlayIn::Chat { message: "before".into() });
    assert!(w.got_text("] \u{bb} "), "v1 format");
    let manifest = dir.join("chat-format").join("plugin.toml");
    let text = std::fs::read_to_string(&manifest).unwrap().replace("name_color = \"gold\"", "name_color = \"aqua\"\nprefix = \"(v2) \"");
    std::fs::write(&manifest, text).unwrap();
    assert!(w.sim.step([ToSim::Console("kiln plugins reload chat-format".into())]));
    for _ in 0..500 {
        if w.sim.plugin_generation("chat-format") == Some(1) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        w.ticks(1);
    }
    assert_eq!(w.sim.plugin_generation("chat-format"), Some(1), "reloaded");
    w.received();
    w.send(PlayIn::Chat { message: "after".into() });
    let chat = w.received();
    assert!(chat.iter().any(|p| contains(p, "(v2) ") && contains(p, "after") && contains(p, "aqua")), "v2 format");
    assert_eq!(w.sim.plugin_stat("reloads"), 1);
}
