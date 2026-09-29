//! The dragon fight: a new world's End gets an inactive exit portal and an ender dragon when a
//! player first comes near; the dragon's death opens the portal, leaves the egg and a gateway;
//! four end crystals on the portal's rim bring a new dragon back.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const END: &str = "minecraft:the_end";

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Slayer", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Slayer"))]));
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

    fn dragons(&self) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == "minecraft:ender_dragon").map(|m| (m.0, m.2, m.3)).collect()
    }

    fn crystals(&self) -> Vec<[f64; 3]> {
        self.sim.entities_in(END).into_iter().filter(|e| e.0 == "minecraft:end_crystal").map(|e| e.1).collect()
    }

    fn block(&self, x: i32, y: i32, z: i32) -> u16 {
        self.sim.block_in(END, x, y, z).unwrap_or(0)
    }

    /// The podium's pillar bottom: the lowest of four bedrock blocks up the origin's column.
    fn podium(&self) -> Option<i32> {
        (0..200).find(|&y| (0..4).all(|dy| state::is(self.block(0, y + dy, 0), d::BEDROCK)) && state::is(self.block(3, y, 0), d::BEDROCK))
    }

    fn portal_blocks(&self, y: i32) -> usize {
        let mut n = 0;
        for x in -2..=2 {
            for z in -2..=2 {
                if state::is(self.block(x, y, z), d::END_PORTAL) {
                    n += 1;
                }
            }
        }
        n
    }
}

#[test]
fn first_visit_starts_the_fight_and_the_kill_opens_the_portal() {
    let mut w = World::new("creative");
    w.run("execute in minecraft:the_end run tp Slayer 0.5 90 40.5");
    w.ticks(45);
    let dragons = w.dragons();
    assert_eq!(dragons.len(), 1, "one dragon: {dragons:?}");
    let y = w.podium().expect("the exit portal's podium");
    assert_eq!(w.portal_blocks(y), 0, "the exit portal is closed while the dragon lives");
    // The dragon flies (the holding pattern).
    let before = dragons[0].1;
    w.ticks(20);
    let after = w.dragons()[0].1;
    assert!((0..3).map(|i| (after[i] - before[i]).powi(2)).sum::<f64>() > 1.0, "{before:?} -> {after:?}");
    // It stays the only one.
    w.ticks(60);
    assert_eq!(w.dragons().len(), 1);

    w.run("execute in minecraft:the_end run kill @e[type=minecraft:ender_dragon]");
    w.ticks(5);
    assert!(w.dragons().is_empty(), "the dragon is gone");
    assert_eq!(w.portal_blocks(y), 20, "the exit portal opened (21 minus the pillar)");
    assert!(state::is(w.block(0, y + 4, 0), d::DRAGON_EGG), "the egg on the pillar");
    // The first gateway of the world's list (seed 0).
    let g = kiln_worldgen::end::end_gateway_positions(0)[0];
    assert!(state::is(w.block(g.x, g.y, g.z), d::END_GATEWAY), "gateway at {g:?}");
    // No new dragon comes by itself.
    w.ticks(40);
    assert!(w.dragons().is_empty());
}

#[test]
fn four_crystals_on_the_portal_respawn_the_dragon() {
    let mut w = World::new("creative");
    w.run("execute in minecraft:the_end run tp Slayer 0.5 90 40.5");
    w.ticks(45);
    w.run("execute in minecraft:the_end run kill @e[type=minecraft:ender_dragon]");
    w.ticks(5);
    let y = w.podium().expect("podium");
    assert_eq!(w.portal_blocks(y), 20);
    // A crystal on each side of the rim, from just outside it.
    let id = kiln_data::builtin_id("minecraft:item", "minecraft:end_crystal").unwrap();
    let stack = ItemStack { item: id, count: 4, added: Vec::new(), removed: Vec::new() };
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    for (i, (x, z)) in [(0, -3), (3, 0), (0, 3), (-3, 0)].into_iter().enumerate() {
        let (sx, sz) = (x as f64 * 1.5 + 0.5, z as f64 * 1.5 + 0.5);
        w.run(&format!("execute in minecraft:the_end run tp Slayer {sx} {} {sz}", y + 1));
        w.ticks(3);
        let pkt = PlayIn::UseItemOn { hand: 0, pos: [x, y, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 10 + i as i32 };
        assert!(w.sim.step([ToSim::Packet(1, pkt)]));
    }
    w.ticks(2);
    assert_eq!(w.crystals().iter().filter(|c| c[1] == (y + 1) as f64).count(), 4, "{:?}", w.crystals());
    // The ritual: the portal closes, then the pillars and after them a new dragon.
    w.ticks(10);
    assert_eq!(w.portal_blocks(y), 0, "the portal closed for the ritual");
    let mut ticks = 0;
    while w.dragons().is_empty() {
        w.ticks(20);
        ticks += 20;
        assert!(ticks < 1000, "no dragon after {ticks} ticks");
    }
    // The ritual's crystals are gone.
    assert_eq!(w.crystals().iter().filter(|c| c[1] == (y + 1) as f64).count(), 0);
}
