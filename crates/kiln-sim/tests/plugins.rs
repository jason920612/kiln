//! WASM plugins in the running simulation (design §11): the example plugins built for
//! wasm32-wasip2 and loaded from a plugin directory. Spawn protection denies breaking and
//! placing near spawn (the client gets its blocks back and a message), the chat formatter
//! rewrites chat, and the counter counts broken blocks per player and in total (`/broken`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

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
        config.plugins = Some(kiln_sim::PluginSettings::new(kiln_plugin_host::examples::build().expect("example plugins")));
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
