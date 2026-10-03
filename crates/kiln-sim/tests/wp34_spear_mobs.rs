//! wp34: mobs with spears in the whole simulation: a zombie with a spear charges a survival
//! player and stabs it, a rider on a zombie horse does too, and what it touches along the way
//! (a villager) is hurt. The goal itself is compared tick by tick with vanilla by kiln-entity's
//! `mob_parity` test (the `spear_` scenarios of tools/MobVectors.java).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Target", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        for c in ["gamerule minecraft:natural_health_regeneration false", "gamerule minecraft:spawn_mobs false", "difficulty hard", "time set 18000", "gamerule minecraft:advance_time false"] {
            w.console(c);
        }
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

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.client.pos;
        self.console(&format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]));
        self.ticks(1);
    }

    fn health(&self) -> f32 {
        self.sim.health(1).unwrap().0
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }
}

const SPEAR: &str = r#"equipment:{mainhand:{id:"minecraft:iron_spear",count:1}}"#;

#[test]
fn a_zombie_with_a_spear_charges_and_stabs_a_player() {
    let mut w = World::new();
    w.console("gamemode survival Target");
    w.summon("minecraft:zombie", [14.0, 0.0, 0.0], &format!("{{PersistenceRequired:1b,{SPEAR}}}"));
    let mut hit_at = None;
    for t in 0..400 {
        w.ticks(1);
        if w.health() < 20.0 {
            hit_at = Some(t);
            break;
        }
    }
    let t = hit_at.expect("the spear reached the player");
    assert!(t > 20, "it had to walk up first ({t})");
    // An iron spear's stab at a walking speed: the damage condition needs speed, so the amount is
    // the zombie's attack damage (3 on hard: 4.5) or more, not a fist's.
    assert!(w.health() <= 17.0, "a real hit: health {}", w.health());
}

#[test]
fn a_zombie_horse_rider_with_a_spear_stabs_a_player() {
    let mut w = World::new();
    w.console("gamemode survival Target");
    w.summon(
        "minecraft:zombie_horse",
        [16.0, 0.0, 0.0],
        &format!(r#"{{PersistenceRequired:1b,Passengers:[{{id:"minecraft:zombie",PersistenceRequired:1b,{SPEAR}}}]}}"#),
    );
    assert_eq!(w.mobs("minecraft:zombie").len(), 1, "the rider is there");
    let mut hit = false;
    for _ in 0..500 {
        w.ticks(1);
        if w.health() < 20.0 {
            hit = true;
            break;
        }
    }
    assert!(hit, "the rider charged and hit");
    // The rider still sits on its horse.
    assert_eq!(w.mobs("minecraft:zombie_horse").len(), 1);
}

#[test]
fn a_zombie_with_a_spear_hurts_a_villager_it_charges() {
    let mut w = World::new();
    w.console("gamemode creative Target");
    w.summon("minecraft:zombie", [0.0, 0.0, -12.0], &format!("{{PersistenceRequired:1b,{SPEAR}}}"));
    w.summon("minecraft:villager", [0.0, 0.0, 0.0], r#"{NoAI:1b,PersistenceRequired:1b}"#);
    // (The villager is next to the player, who is in creative: the zombie goes for the villager.)
    let before = w.mobs("minecraft:villager")[0].2;
    for _ in 0..300 {
        w.ticks(1);
        let v = w.mobs("minecraft:villager");
        if v.is_empty() || v[0].2 < before {
            break;
        }
    }
    let v = w.mobs("minecraft:villager");
    assert!(v.is_empty() || v[0].2 < before, "the villager was stabbed");
}
