//! Block behaviour in the running simulation: placing with shapes, levers powering lamps,
//! water spreading through scheduled ticks, falling sand, commands with neighbour updates,
//! and breaking blocks in survival.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Builder", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Builder"))]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground }
    }

    /// Runs `n` ticks with the client idle.
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

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    fn hold(&mut self, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 64, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    /// Right-clicks the top face of `pos`.
    fn use_on_top(&mut self, pos: [i32; 3], sequence: i32) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }
}

#[test]
fn placed_fences_connect_and_levers_light_lamps() {
    let mut w = World::new("creative");
    w.hold("minecraft:oak_fence");
    let (a, b) = (w.at(2, 0, 0), w.at(3, 0, 0));
    w.use_on_top(a, 1);
    w.use_on_top(b, 2);
    let (fa, fb) = (w.block(w.at(2, 1, 0)), w.block(w.at(3, 1, 0)));
    assert!(state::is(fa, d::OAK_FENCE) && state::is(fb, d::OAK_FENCE));
    assert!(state::get_bool(fa, "east") && state::get_bool(fb, "west"), "neighbouring fences connect");

    w.hold("minecraft:lever");
    let lever = w.at(0, 1, 2);
    w.use_on_top(w.at(0, 0, 2), 3);
    assert!(state::is(w.block(lever), d::LEVER));
    w.run(&format!("setblock {} {} {} minecraft:redstone_lamp", lever[0] + 1, lever[1], lever[2]));
    let lamp = [lever[0] + 1, lever[1], lever[2]];
    assert!(!state::get_bool(w.block(lamp), "lit"));
    // Clicking the lever (not sneaking) pulls it rather than placing another.
    w.use_on_top(lever, 4);
    assert!(state::get_bool(w.block(lever), "powered"));
    assert!(state::get_bool(w.block(lamp), "lit"), "a powered lamp lights at once");
    w.use_on_top(lever, 5);
    assert!(state::get_bool(w.block(lamp), "lit"), "lamps turn off after a delay");
    w.ticks(5);
    assert!(!state::get_bool(w.block(lamp), "lit"));
}

#[test]
fn water_spreads_and_sand_falls() {
    let mut w = World::new("creative");
    let source = w.at(4, 1, 4);
    w.run(&format!("setblock {} {} {} minecraft:water", source[0], source[1], source[2]));
    // Water's tick delay is 5: the first ring appears after the first scheduled tick.
    w.ticks(7);
    let next = w.block([source[0] + 1, source[1], source[2]]);
    assert!(state::is(next, d::WATER) && state::get_int(next, "level") == 1, "flowing water beside the source");

    let high = w.at(-4, 6, -4);
    w.run(&format!("setblock {} {} {} minecraft:sand", high[0], high[1], high[2]));
    // A falling block entity: five blocks take about 17 ticks.
    w.ticks(3);
    assert!(state::is(w.block(high), d::AIR));
    assert!(w.sim.entities().iter().any(|(kind, _)| *kind == "minecraft:falling_block"), "the sand is falling");
    w.ticks(30);
    assert!(w.sim.entities().is_empty(), "{:?}", w.sim.entities());
    assert!(state::is(w.block(w.at(-4, 1, -4)), d::SAND), "the sand landed on the ground");
    assert!(state::is(w.block(high), d::AIR));
}

#[test]
fn powered_tnt_primes_and_explodes() {
    let mut w = World::new("creative");
    let tnt = w.at(6, 1, 6);
    let below = w.at(6, 0, 6);
    let before = w.block(below);
    assert!(!state::is(before, d::AIR));
    w.run(&format!("setblock {} {} {} minecraft:tnt", tnt[0], tnt[1], tnt[2]));
    w.run(&format!("setblock {} {} {} minecraft:redstone_block", tnt[0] + 1, tnt[1], tnt[2]));
    w.ticks(2);
    assert!(state::is(w.block(tnt), d::AIR), "the TNT block was primed");
    assert!(w.sim.entities().iter().any(|(kind, _)| *kind == "minecraft:tnt"), "{:?}", w.sim.entities());
    // The default fuse is 80 ticks.
    w.ticks(85);
    assert!(!w.sim.entities().iter().any(|(kind, _)| *kind == "minecraft:tnt"), "{:?}", w.sim.entities());
    assert!(state::is(w.block(below), d::AIR), "the explosion blew a crater");
    assert_eq!(w.sim.health(1), Some((20.0, false)), "creative players take no damage");
}

#[test]
fn setblock_updates_neighbours_unless_strict() {
    let mut w = World::new("creative");
    let torch = w.at(1, 1, -3);
    w.run(&format!("setblock {} {} {} minecraft:torch", torch[0], torch[1], torch[2]));
    assert!(state::is(w.block(torch), d::TORCH));
    // Removing the support pops the torch off.
    let below = w.at(1, 0, -3);
    w.run(&format!("setblock {} {} {} minecraft:air", below[0], below[1], below[2]));
    assert!(state::is(w.block(torch), d::AIR));
    assert!(w.sim.entities().iter().any(|(kind, _)| *kind == "minecraft:item"), "the torch dropped");
    // Strict mode leaves a floating torch.
    let torch2 = w.at(3, 1, -3);
    w.run(&format!("setblock {} {} {} minecraft:torch", torch2[0], torch2[1], torch2[2]));
    let below2 = w.at(3, 0, -3);
    w.run(&format!("setblock {} {} {} minecraft:air strict", below2[0], below2[1], below2[2]));
    assert!(state::is(w.block(torch2), d::TORCH));
}

#[test]
fn survival_digging_takes_time_and_drops_the_block() {
    let mut w = World::new("survival");
    let dirt = w.at(1, 0, 0);
    let start = PlayIn::PlayerAction { action: 0, pos: dirt, face: 1, sequence: 1 };
    assert!(w.sim.step([ToSim::Packet(1, start)]));
    // Dirt needs no tool: by hand it breaks 1 / 0.5 / 30 per tick, in 15 ticks.
    w.ticks(3);
    let early = PlayIn::PlayerAction { action: 3, pos: dirt, face: 1, sequence: 2 };
    assert!(w.sim.step([ToSim::Packet(1, early)]));
    assert!(state::is(w.block(dirt), d::DIRT) || state::is(w.block(dirt), d::GRASS_BLOCK), "too early: not broken yet");
    // The server finishes the break on its own clock.
    w.ticks(15);
    assert!(state::is(w.block(dirt), d::AIR));
    w.ticks(2);
    assert!(w.sim.entities().iter().any(|(kind, _)| *kind == "minecraft:item"), "the block dropped");
}

#[test]
fn creative_players_break_at_once_without_drops() {
    let mut w = World::new("creative");
    let block = w.at(-1, 0, 1);
    let start = PlayIn::PlayerAction { action: 0, pos: block, face: 1, sequence: 1 };
    assert!(w.sim.step([ToSim::Packet(1, start)]));
    assert!(state::is(w.block(block), d::AIR));
    w.ticks(2);
    assert!(w.sim.entities().is_empty());
}

#[test]
fn a_lever_set_powered_by_command_lights_a_wire_line() {
    let mut w = World::new("creative");
    let y = w.ground[1] + 1;
    w.run(&format!("fill 3 {y} 12 10 {y} 12 redstone_wire"));
    w.run(&format!("setblock 11 {y} 12 redstone_lamp"));
    w.run(&format!("setblock 2 {y} 12 lever[face=floor,facing=east]"));
    w.run(&format!("setblock 2 {y} 12 lever[face=floor,facing=east,powered=true]"));
    let wire = w.block([3, y, 12]);
    assert_eq!(state::get_int(wire, "power"), 15);
    assert!(state::get_bool(w.block([11, y, 12]), "lit"));
}

#[test]
fn powering_by_command_reaches_the_client() {
    let mut w = World::new("creative");
    w.client.stats.count_ids.store(true, std::sync::atomic::Ordering::Relaxed);
    for c in ["fill 3 -60 12 10 -60 12 redstone_wire", "setblock 11 -60 12 redstone_lamp", "setblock 2 -60 12 lever[face=floor,facing=east]"] {
        w.run(c);
    }
    w.ticks(2);
    let before = w.client.stats.by_id.lock().unwrap().clone();
    w.run("setblock 2 -60 12 lever[face=floor,facing=east,powered=true]");
    w.ticks(2);
    let after = w.client.stats.by_id.lock().unwrap().clone();
    let sent = |id: i32| after.get(&id).map_or(0, |a| a.0) - before.get(&id).map_or(0, |b| b.0);
    // The lever as a Block Update; the eight wires and the lamp in one Section Blocks Update.
    assert_eq!(sent(kiln_data::packets::play::clientbound::BLOCK_UPDATE), 1);
    assert_eq!(sent(kiln_data::packets::play::clientbound::SECTION_BLOCKS_UPDATE), 1);
}
