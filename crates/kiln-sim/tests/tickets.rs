//! wp81: chunk tickets other than the players' (`TicketType`): `/forceload` loads the chunk, ticks entities in it and blocks one ring around it and
//! loads the ring after that; a player's ender pearl keeps the chunks around it loaded and ticking wherever its player is, lands for its owner in
//! another region, and comes back with its owner after a logout.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const CELL: i32 = 16;

fn loaded(sim: &Sim, cx: i32, cz: i32) -> bool {
    sim.block_at(cx * CELL + 8, 80, cz * CELL + 8).is_some()
}

#[test]
fn a_forced_chunk_loads_two_rings_around_it() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    sim.console_now("forceload add 4008 4008");
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    let (cx, cz) = (4008 >> 4, 4008 >> 4);
    for d in -2..=2 {
        for e in -2..=2 {
            assert!(loaded(&sim, cx + d, cz + e), "chunk {d},{e} around the forced one is loaded");
        }
    }
    for (d, e) in [(3, 0), (0, -3), (3, 3), (-3, 1)] {
        assert!(!loaded(&sim, cx + d, cz + e), "chunk {d},{e} is beyond the rings");
    }
    sim.console_now("forceload remove 4008 4008");
    for _ in 0..60 {
        assert!(sim.step([]));
    }
    assert!(!loaded(&sim, cx, cz) && !loaded(&sim, cx + 2, cz + 2), "all of it goes when the chunk is let go");
}

fn age_of(sim: &Sim, name: &str) -> Option<i64> {
    sim.entity_nbt()
        .into_iter()
        .find(|t| t.get("Tags").and_then(Tag::as_list).is_some_and(|l| l.iter().any(|x| x.as_str() == Some(name))))
        .and_then(|t| t.get("Age").and_then(Tag::as_i64))
}

/// Entities tick in the forced chunk only; the ring around it ticks blocks (a water source spreads) and the ring after that only loads
/// (`ChunkLevel`: the ticket's level 31 at the chunk, 32 and 33 further).
#[test]
fn a_forced_chunk_ticks_entities_in_itself_and_blocks_one_ring_out() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    sim.console_now("gamerule minecraft:spawn_mobs false");
    sim.console_now("forceload add 4008 4008");
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    // A platform over the forced chunk and the two rings along x, a baby cow on each of the three chunks.
    sim.console_now("fill 4000 99 4000 4063 99 4015 minecraft:stone");
    let base = 4008;
    for (name, dx) in [("centre", 0), ("ring1", 16), ("ring2", 32)] {
        sim.console_now(&format!("summon minecraft:cow {} 100 {} {{Age:-2000,Tags:[\"{name}\"]}}", base + dx, base));
        // Water on the platform, 3 blocks from the cow.
        sim.console_now(&format!("setblock {} 100 {} minecraft:water", base + dx + 3, base + 3));
    }
    for _ in 0..30 {
        assert!(sim.step([]));
    }
    assert!(age_of(&sim, "centre").unwrap() > -2000 + 20, "the cow in the forced chunk ages: {:?}", age_of(&sim, "centre"));
    assert_eq!(age_of(&sim, "ring1"), Some(-2000), "the cow in the ring next to it does not tick");
    assert_eq!(age_of(&sim, "ring2"), Some(-2000), "nor does the one in the ring after");
    let water = |dx: i32| sim.block_at(base + dx + 4, 100, base + 3).map(|b| kiln_entity::blocks::block_name(b));
    assert_eq!(water(0), Some("minecraft:water"), "water spreads in the forced chunk");
    assert_eq!(water(16), Some("minecraft:water"), "and in the ring around it (block ticking)");
    assert_eq!(water(32), Some("minecraft:air"), "but not in the ring after that, which is only loaded");
}

/// wp81 (lock step against vanilla, `tools/wp81/simradius81.py`): water at the east edge of the last block-ticking chunk flows on into the chunk after it,
/// which only has to be loaded (level 33). Here around a forced chunk (ring 1 ticks blocks, ring 2 is loaded).
#[test]
fn water_flows_from_the_block_ticking_ring_into_the_loaded_one() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    sim.console_now("gamerule minecraft:spawn_mobs false");
    sim.console_now("forceload add 4008 4008");
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    // Ring 1 is the chunk x 4016..4031; its east edge is x = 4031, the next chunk (ring 2) starts at 4032. A stone floor with a hole at x = 4033.
    sim.console_now("fill 4022 99 4000 4040 99 4016 minecraft:stone");
    sim.console_now("setblock 4033 99 4008 minecraft:air");
    sim.console_now("setblock 4031 100 4008 minecraft:water");
    for _ in 0..40 {
        assert!(sim.step([]));
    }
    let block = |x| sim.block_at(x, 100, 4008).map(|b| kiln_entity::blocks::block_name(b));
    assert_eq!(block(4032), Some("minecraft:water"), "the water reached the chunk that is only loaded");
}

/// A player's loading tickets reach two chunks past the server's view distance (level 31 at `V`, 33 at `V + 2`): those chunks are loaded, none of them
/// is sent, and the one after is not loaded.
#[test]
fn the_server_loads_two_chunks_past_its_view_distance() {
    let mut sim = Sim::new(SimConfig::new(4, 3, None));
    let (msg, stats) = join(1, "Watcher", 3);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for _ in 0..30 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let (px, pz) = ((client.pos[0].floor() as i32) >> 4, (client.pos[2].floor() as i32) >> 4);
    for d in [0, 3, 4, 5] {
        assert!(loaded(&sim, px + d, pz), "{d} chunks east is loaded");
        assert!(loaded(&sim, px, pz - d), "{d} chunks north is loaded");
    }
    for d in [6, 7] {
        assert!(!loaded(&sim, px + d, pz), "{d} chunks east is not");
    }
}

/// `execute if loaded` is true for a chunk at the entity-ticking *loading* level: within the server's view distance of a player (lock step: `simradius81.py`,
/// with a view distance of 5 and a simulation distance of 4 vanilla says yes up to 5 chunks, and the chunks to 7 are loaded all the same), and for the chunk of a
/// forced chunk but not the rings around it.
#[test]
fn execute_if_loaded_follows_the_loading_level_not_just_loaded() {
    // (`SimConfig::new(max players, view distance, …)`; the simulation distance is set to 2 by the console.)
    let mut config = SimConfig::new(4, 3, None);
    config.simulation_distance = 2;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Watcher", 3);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for _ in 0..30 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let (px, pz) = ((client.pos[0].floor() as i32) >> 4, (client.pos[2].floor() as i32) >> 4);
    for d in 0..=5 {
        let (x, z) = ((px + d) * 16 + 8, pz * 16 + 8);
        sim.console_now(&format!("execute if loaded {x} 100 {z} run setblock {x} 100 {z} minecraft:diamond_block"));
    }
    let diamond = |d: i32| sim.block_at((px + d) * 16 + 8, 100, pz * 16 + 8) == Some(kiln_data::blocks::default_state::DIAMOND_BLOCK);
    assert!((0..=3).all(diamond), "within the view distance (the simulation distance is 2: it does not matter): {:?}", (0..=5).map(diamond).collect::<Vec<_>>());
    assert!(!diamond(4) && !diamond(5), "the rings that are only loaded do not count");
    assert!(loaded(&sim, px + 5, pz), "although they are loaded");
    sim.console_now("forceload add 4008 4008");
    for _ in 0..3 {
        assert!(sim.step([]));
    }
    for (dx, want) in [(0, true), (1, false), (2, false)] {
        let x = (4008 >> 4) * 16 + 8 + 16 * dx;
        sim.console_now(&format!("execute if loaded {x} 100 4008 run setblock {x} 100 4008 minecraft:gold_block"));
        let gold = sim.block_at(x, 100, 4008) == Some(kiln_data::blocks::default_state::GOLD_BLOCK);
        assert_eq!(gold, want, "forced chunk ring {dx}");
    }
}

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new(dir: Option<std::path::PathBuf>) -> World {
        let mut sim = Sim::new(SimConfig::new(2, 2, dir));
        let (msg, stats) = join(1, "Thrower", 2);
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival Thrower".into())]));
        let mut w = World { sim, client: Client::new(1, stats) };
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn throw_pearl_up(&mut self) {
        let id = kiln_data::builtin_id("minecraft:item", "minecraft:ender_pearl").unwrap();
        let stack = ItemStack { item: id, count: 16, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Console("gamemode creative Thrower".into())]));
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        assert!(self.sim.step([ToSim::Console("gamemode survival Thrower".into())]));
        self.ticks(2);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: Hand::Main, sequence: 1, yaw: 0.0, pitch: -90.0 })]));
    }

    fn pearls(&self) -> usize {
        self.sim.entities().iter().filter(|(k, _)| *k == "minecraft:ender_pearl").count()
    }
}

/// A pearl thrown straight up and left alone while its player goes 4000 blocks away: its ticket keeps the chunk loaded and ticking, so it comes down and
/// takes its player back (the player is in another region of the level: the serial phase carries the teleport out).
#[test]
fn a_pearl_keeps_its_chunk_ticking_and_lands_for_a_player_far_away() {
    let mut w = World::new(None);
    let start = w.client.pos;
    let (cx, cz) = ((start[0].floor() as i32) >> 4, (start[2].floor() as i32) >> 4);
    w.throw_pearl_up();
    w.ticks(3);
    assert_eq!(w.pearls(), 1, "the pearl is in the air");
    assert!(w.sim.step([ToSim::Console("tp Thrower 4000 80 4000".into())]));
    w.ticks(10);
    assert!(w.client.pos[0] > 3000.0, "the player is away: {:?}", w.client.pos);
    // The chunk the pearl is in is out of every player's reach but loaded and ticking all the time it is in the air.
    for _ in 0..40 {
        w.ticks(1);
        assert!(loaded(&w.sim, cx, cz), "the pearl's chunk is loaded while it flies");
        assert_eq!(w.pearls(), 1, "the pearl is still flying");
    }
    w.ticks(160);
    assert_eq!(w.pearls(), 0, "the pearl came down");
    assert!(w.client.pos[0].abs() < 40.0 && w.client.pos[2].abs() < 40.0, "its owner went to where it landed: {:?} (from {:?})", w.client.pos, start);
    // The ticket runs out 40 ticks after the pearl's last renewal and the chunk goes with it.
    w.ticks(10);
}

/// `ServerPlayer.saveEnderPearls` and `loadAndSpawnEnderPearls`: a pearl in the air is saved with its owner when it logs out (and is gone from the
/// level), and back in the air when it logs in again.
#[test]
fn a_players_pearl_leaves_with_it_and_comes_back() {
    let dir = std::env::temp_dir().join(format!("kiln-pearl-login-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut w = World::new(Some(dir.clone()));
        w.throw_pearl_up();
        w.ticks(5);
        assert_eq!(w.pearls(), 1);
        let saved = w.sim.saved_player_nbt(1).expect("the player");
        let list = saved.get("ender_pearls").and_then(Tag::as_list).expect("the pearls it has in flight");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("ender_pearl_dimension").and_then(Tag::as_str), Some("minecraft:overworld"));
        assert!(w.sim.step([ToSim::Leave(1)]));
        w.ticks(1);
        assert_eq!(w.pearls(), 0, "the pearl went with its player");
    }
    {
        let mut sim = Sim::new(SimConfig::new(2, 2, Some(dir.clone())));
        let (msg, stats) = join(1, "Thrower", 2);
        assert!(sim.step([msg]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let pearls = sim.entities().iter().filter(|(k, _)| *k == "minecraft:ender_pearl").count();
        assert_eq!(pearls, 1, "the pearl is back in the air");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
