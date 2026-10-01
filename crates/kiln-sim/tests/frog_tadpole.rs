//! Frogs and tadpoles end to end: a frog eats a small magma cube (froglights with the vanilla
//! datapack's loot tables: `KILN_DATAPACK`), frogspawn hatches into tadpoles.
//! Their brains are compared with vanilla by kiln-entity's `mob_parity` test.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Hunter", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, clients: vec![Client::new(1, stats)] };
        w.console("gamerule minecraft:spawn_mobs false");
        w.console("gamemode creative Hunter");
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in self.clients.iter_mut() {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn pos(&self) -> [f64; 3] {
        self.clients[0].pos
    }

    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.pos();
        self.console(format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]).trim());
        self.ticks(1);
    }

    fn mobs(&self, kind: &str) -> usize {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind && m.3 > 0.0).count()
    }
}

#[test]
fn a_frog_eats_a_small_magma_cube() {
    let mut w = World::new();
    w.summon("minecraft:frog", [-2.0, 0.0, 0.0], "{variant:\"minecraft:warm\",Brain:{memories:{\"minecraft:long_jump_cooling_down\":{value:2000}}}}");
    w.summon("minecraft:magma_cube", [2.0, 0.0, 0.0], "{Size:0,NoAI:1b}");
    assert_eq!(w.mobs("minecraft:magma_cube"), 1);
    w.ticks(300);
    assert_eq!(w.mobs("minecraft:magma_cube"), 0, "eaten");
    assert_eq!(w.mobs("minecraft:frog"), 1);
    // The frog's variant decides the froglight (`pearlescent_froglight` for a warm one).
    if std::env::var_os("KILN_DATAPACK").is_some() {
        assert!(!w.sim.entity_ids_of("minecraft:item").is_empty(), "a froglight dropped: {:?}", w.sim.entities());
    }
}

#[test]
fn a_big_slime_is_not_food() {
    let mut w = World::new();
    w.summon("minecraft:frog", [-2.0, 0.0, 0.0], "{Brain:{memories:{\"minecraft:long_jump_cooling_down\":{value:2000}}}}");
    w.summon("minecraft:slime", [2.0, 0.0, 0.0], "{Size:1,NoAI:1b}");
    w.ticks(300);
    assert_eq!(w.mobs("minecraft:slime"), 1);
}

#[test]
fn frogspawn_hatches_into_tadpoles() {
    let mut w = World::new();
    let p = w.pos();
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32 - 1, p[2].floor() as i32);
    // A pond one block deep with the spawn on top.
    w.console(&format!("fill {} {} {} {} {} {} minecraft:water", x, y, z, x + 1, y, z + 1));
    w.console(&format!("setblock {} {} {} minecraft:frogspawn", x, y + 1, z));
    w.ticks(2);
    assert_eq!(w.sim.block_at(x, y + 1, z), Some(kiln_data::blocks::default_state::FROGSPAWN), "the spawn was placed");
    // 3600 to 12000 ticks later.
    w.ticks(12050);
    assert_ne!(w.sim.block_at(x, y + 1, z), Some(kiln_data::blocks::default_state::FROGSPAWN), "the spawn hatched");
    let tadpoles = w.mobs("minecraft:tadpole");
    assert!((2..=5).contains(&tadpoles), "{tadpoles} tadpoles");
}
