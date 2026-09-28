//! The three levels: a nether portal lit with flint and steel takes a player to the nether,
//! where an exit portal is built at an eighth of the coordinates and back again; an end
//! portal leads to the End's obsidian platform, the End's exit portal to the credits and a
//! respawn in the overworld; `/execute in` and `/tp` cross levels.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::ClientCommand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const NETHER: &str = "minecraft:the_nether";
const END: &str = "minecraft:the_end";
const OVERWORLD: &str = "minecraft:overworld";

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Traveller", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Traveller"))]));
        let mut w = Self { sim, client: Client::new(1, stats) };
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

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn level(&self) -> (&'static str, [f64; 3]) {
        self.sim.player_level(1).expect("player")
    }

    /// Ticks until the player is in `level` (at most `n` ticks); returns the ticks it took.
    fn until_in(&mut self, level: &str, n: usize) -> usize {
        for i in 0..n {
            if self.level().0 == level {
                return i;
            }
            self.ticks(1);
        }
        panic!("still in {:?} after {n} ticks", self.level());
    }

    fn hold(&mut self, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }
}

/// A 2×3 obsidian frame along x at z, its bottom row at y, left inner column at x.
fn frame(w: &mut World, x: i32, y: i32, z: i32) {
    w.run(&format!("fill {} {y} {z} {} {} {z} minecraft:obsidian", x - 1, x + 2, y + 4));
    w.run(&format!("fill {x} {} {z} {} {} {z} minecraft:air", y + 1, x + 1, y + 3));
}

#[test]
fn nether_portal_round_trip() {
    let mut w = World::new("creative");
    // Flat overworld: grass at -61, the frame's bottom replaces it.
    frame(&mut w, 11, -61, 12);
    w.run("tp Traveller 11.5 -60 10.5");
    w.ticks(3);
    w.hold("minecraft:flint_and_steel");
    let pkt = PlayIn::UseItemOn { hand: 0, pos: [11, -61, 12], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
    assert!(w.sim.step([ToSim::Packet(1, pkt)]));
    for (x, y) in [(11, -60), (12, -60), (11, -58), (12, -58)] {
        let s = w.sim.block_at(x, y, 12).unwrap();
        assert!(state::is(s, d::NETHER_PORTAL), "portal at {x},{y}: {}", kiln_blocks::state::state_string(s));
        assert_eq!(state::get(s, "axis"), Some("x"));
    }

    // Creative players go through at once (players_nether_portal_creative_delay 0).
    w.run("tp Traveller 12.0 -60 12.5");
    let took = w.until_in(NETHER, 20);
    assert!(took <= 5, "creative travel took {took} ticks");
    let (_, pos) = w.level();
    // 12/8 = 1.5: the exit portal is built within 16 blocks of (1, -60, 1), on the nether's
    // flat ground (grass at 3).
    assert!((pos[0] - 1.5).abs() <= 17.0 && (pos[2] - 1.5).abs() <= 17.0, "{pos:?}");
    assert_eq!(pos[1], 4.0, "standing on the new portal's frame");
    let feet = pos.map(|c| c.floor() as i32);
    let s = w.sim.block_in(NETHER, feet[0], feet[1], feet[2]).unwrap();
    assert!(state::is(s, d::NETHER_PORTAL), "arrived in the exit portal");
    assert!(state::is(w.sim.block_in(NETHER, feet[0], feet[1] - 1, feet[2]).unwrap(), d::OBSIDIAN));

    // Standing in the exit portal keeps the cooldown up: no trip back.
    w.ticks(40);
    assert_eq!(w.level().0, NETHER);

    // Step out and back in: back to the overworld portal (found, not built).
    w.run(&format!("execute in {NETHER} run tp Traveller {} {} {}", pos[0], pos[1], pos[2] + 3.0));
    w.ticks(20);
    w.run(&format!("execute in {NETHER} run tp Traveller {} {} {}", pos[0], pos[1], pos[2]));
    w.until_in(OVERWORLD, 20);
    let (_, back) = w.level();
    assert!((11.0..13.0).contains(&back[0]) && back[1] == -60.0 && (12.0..13.0).contains(&back[2]), "{back:?}");

    // Survival players wait players_nether_portal_default_delay ticks.
    w.run("gamemode survival Traveller");
    w.run("gamerule minecraft:players_nether_portal_default_delay 30");
    w.run("tp Traveller 11.5 -60 14.5");
    w.ticks(20);
    w.run("tp Traveller 12.0 -60 12.5");
    let took = w.until_in(NETHER, 60);
    assert!((30..=36).contains(&took), "survival travel took {took} ticks");
}

#[test]
fn thrown_items_go_through_portals() {
    let mut w = World::new("creative");
    frame(&mut w, 11, -61, 12);
    w.run("setblock 11 -60 12 minecraft:fire");
    assert!(state::is(w.sim.block_at(12, -59, 12).unwrap(), d::NETHER_PORTAL));
    // Facing south (+z) in front of the portal, the dropped stack flies into it.
    w.run("tp Traveller 11.5 -60 10.8 0 0");
    w.ticks(3);
    w.hold("minecraft:diamond");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 4, pos: [0, 0, 0], face: 0, sequence: 0 })]));
    let mut arrived = None;
    for _ in 0..40 {
        w.ticks(1);
        if let Some(&(_, pos)) = w.sim.entities_in(NETHER).iter().find(|(k, _)| *k == "minecraft:item") {
            arrived = Some(pos);
            break;
        }
    }
    let pos = arrived.expect("the item reached the nether");
    assert!(w.sim.entities_in(OVERWORLD).iter().all(|(k, _)| *k != "minecraft:item"));
    // It came out of the exit portal Kiln built.
    let near = |dx: i32, dz: i32| {
        let (x, y, z) = (pos[0].floor() as i32 + dx, pos[1].floor() as i32, pos[2].floor() as i32 + dz);
        w.sim.block_in(NETHER, x, y, z).is_some_and(|s| state::is(s, d::NETHER_PORTAL))
    };
    assert!((-1..=1).any(|dx| (-1..=1).any(|dz| near(dx, dz))), "{pos:?}");
}

#[test]
fn broken_frame_takes_the_portal_down() {
    let mut w = World::new("creative");
    frame(&mut w, 3, -61, 3);
    w.run("setblock 3 -60 3 minecraft:fire");
    assert!(state::is(w.sim.block_at(4, -59, 3).unwrap(), d::NETHER_PORTAL));
    w.run("setblock 2 -59 3 minecraft:air");
    for y in -60..=-58 {
        assert!(!state::is(w.sim.block_at(3, y, 3).unwrap(), d::NETHER_PORTAL), "portal gone at y={y}");
    }
}

#[test]
fn end_portal_platform_credits_and_return() {
    let mut w = World::new("creative");
    w.run("setblock 20 -60 20 minecraft:end_portal");
    w.run("tp Traveller 20.5 -60 20.5");
    w.until_in(END, 10);
    let (_, pos) = w.level();
    assert_eq!(pos, [100.5, 49.0, 0.5]);
    for (x, z) in [(98, -2), (100, 0), (102, 2)] {
        assert!(state::is(w.sim.block_in(END, x, 48, z).unwrap(), d::OBSIDIAN), "platform at {x},{z}");
        assert!(state::is(w.sim.block_in(END, x, 49, z).unwrap(), d::AIR));
    }

    // The exit portal before the credits were seen: credits, then a respawn keeping
    // everything, in the overworld.
    w.run("execute in minecraft:the_end run setblock 103 49 0 minecraft:end_portal");
    w.run("execute in minecraft:the_end run tp Traveller 103.5 49 0.5");
    w.ticks(3);
    assert_eq!(w.level().0, END, "credits roll first");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ClientCommand(ClientCommand::PerformRespawn))]));
    w.ticks(2);
    assert_eq!(w.level().0, OVERWORLD);
    // Past the portal cooldown (stepping into a portal while it runs keeps it running).
    w.ticks(20);

    // The second trip through the exit portal skips the credits.
    w.run("setblock 20 -60 20 minecraft:end_portal");
    w.run("tp Traveller 20.5 -60 20.5");
    w.until_in(END, 10);
    w.ticks(20);
    w.run("execute in minecraft:the_end run tp Traveller 103.5 49 0.5");
    w.until_in(OVERWORLD, 20);
}

#[test]
fn tp_and_execute_in_cross_levels() {
    let mut w = World::new("creative");
    w.run("execute in minecraft:the_nether run tp Traveller 0.5 10 0.5");
    w.ticks(2);
    assert_eq!(w.level(), (NETHER, [0.5, 10.0, 0.5]));
    // Blocks go into the level the command runs in.
    w.run("execute in minecraft:the_nether run setblock 0 5 0 minecraft:glowstone");
    assert!(state::is(w.sim.block_in(NETHER, 0, 5, 0).unwrap(), d::GLOWSTONE));
    assert!(!w.sim.block_at(0, 5, 0).is_some_and(|s| state::is(s, d::GLOWSTONE)));
    w.run("execute in minecraft:the_end run tp Traveller 5.5 20 5.5");
    w.ticks(2);
    assert_eq!(w.level().0, END);
    w.run("tp Traveller 8.5 20 8.5");
    w.ticks(2);
    assert_eq!(w.level(), (OVERWORLD, [8.5, 20.0, 8.5]), "the console runs in the overworld");
    w.run("execute in minecraft:overworld run tp Traveller 8.5 -60 8.5");
    w.ticks(2);
    assert_eq!(w.level(), (OVERWORLD, [8.5, -60.0, 8.5]));
    // Chunks of each level stay apart.
    let loaded = w.sim.loaded_chunks();
    assert_eq!(loaded.len(), 3);
    assert!(loaded.iter().all(|&n| n > 0), "{loaded:?}");
}
