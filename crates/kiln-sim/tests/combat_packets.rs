//! wp82 (packets wp80's gallery saw only on vanilla): `ClientboundPlayerCombatEnterPacket` / `…EndPacket` (`CombatTracker`: a hit from a living entity starts a
//! fight, rechecked every 20th tick of the player: 300 ticks after the last hit it ends and the packet carries its length) and `ClientboundProjectilePowerPacket`
//! (`ServerEntity.sendChanges`: a fireball's, skull's or wind charge's acceleration power rides in a bundle with its motion).

use kiln_data::packets::play::clientbound as cb;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct Lab {
    sim: Sim,
    client: Client,
    log: std::sync::Arc<kiln_sim::testing::SinkStats>,
}

impl Lab {
    fn new() -> Lab {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Fighter", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let log = stats.clone();
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamerule minecraft:natural_health_regeneration false".into())]));
        let mut lab = Lab { sim, client: Client::new(1, stats), log };
        lab.ticks(5);
        lab.console("gamemode survival Fighter");
        lab.console("difficulty normal");
        lab.ticks(40);
        lab
    }

    fn console(&mut self, c: &str) {
        assert!(self.sim.step([ToSim::Console(c.into())]));
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn clear(&self) {
        self.log.log.lock().unwrap().as_mut().unwrap().clear();
    }

    fn packets(&self) -> Vec<bytes::Bytes> {
        self.log.log.lock().unwrap().clone().unwrap()
    }

    fn with_id(&self, id: i32) -> Vec<bytes::Bytes> {
        self.packets().into_iter().filter(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(id)).collect()
    }
}

#[test]
fn a_hit_from_a_living_entity_starts_a_fight_and_three_hundred_ticks_later_it_ends() {
    let mut lab = Lab::new();
    let p = lab.client.pos;
    lab.console(&format!("summon minecraft:zombie {} {} {} {{NoAI:1b,Tags:[\"foe\"]}}", p[0] + 10.0, p[1], p[2]));
    lab.ticks(2);
    lab.clear();
    // A hit with no entity behind it starts no fight.
    lab.console("damage Fighter 1 minecraft:cactus");
    lab.ticks(30);
    assert!(lab.with_id(cb::PLAYER_COMBAT_ENTER).is_empty(), "cactus damage is no fight");
    lab.console("damage Fighter 1 minecraft:mob_attack by @e[tag=foe,limit=1]");
    lab.ticks(2);
    let enter = lab.with_id(cb::PLAYER_COMBAT_ENTER);
    assert_eq!(enter.len(), 1, "the zombie's hit starts a fight");
    assert_eq!(enter[0].len(), 1, "the packet has no fields");
    lab.ticks(10);
    // Another hit inside the fight does not start a second one.
    lab.console("damage Fighter 1 minecraft:mob_attack by @e[tag=foe,limit=1]");
    lab.ticks(290);
    assert_eq!(lab.with_id(cb::PLAYER_COMBAT_ENTER).len(), 1);
    let end = lab.with_id(cb::PLAYER_COMBAT_END);
    assert!(end.is_empty(), "300 ticks have not passed since the last hit yet: {end:?}");
    lab.ticks(40);
    let end = lab.with_id(cb::PLAYER_COMBAT_END);
    assert_eq!(end.len(), 1, "the fight ended");
    let mut r = kiln_proto::codec::Reader::new(&end[0]);
    r.varint().unwrap();
    let duration = r.varint().unwrap();
    // From the first hit to the check (every 20th tick) after the last hit's 300 ticks: the last hit was 12 ticks after the first.
    assert!((313..=332).contains(&duration), "{duration}");
}

#[test]
fn a_fireballs_acceleration_power_comes_with_its_motion_in_a_bundle() {
    let mut lab = Lab::new();
    let p = lab.client.pos;
    // An arrow moves without one.
    lab.console(&format!("summon minecraft:arrow {} {} {} {{Motion:[-1.0d,0.3d,0.0d]}}", p[0] + 12.0, p[1] + 3.0, p[2] + 6.0));
    lab.ticks(6);
    assert!(lab.with_id(cb::PROJECTILE_POWER).is_empty());
    lab.clear();
    lab.console(&format!("summon minecraft:small_fireball {} {} {} {{Motion:[-0.5d,0.0d,0.0d],power:[-0.5d,0.0d,0.0d]}}", p[0] + 12.0, p[1] + 3.0, p[2] + 3.0));
    lab.ticks(6);
    let all = lab.packets();
    let power: Vec<usize> = (0..all.len()).filter(|&i| kiln_proto::codec::Reader::new(&all[i]).varint().ok() == Some(cb::PROJECTILE_POWER)).collect();
    assert!(!power.is_empty(), "a projectile power packet");
    for &i in &power {
        let mut r = kiln_proto::codec::Reader::new(&all[i]);
        r.varint().unwrap();
        let _id = r.varint().unwrap();
        assert_eq!(r.f64().unwrap(), 0.1);
        // The bundle: delimiter, motion of the same entity, the power, delimiter.
        assert_eq!(kiln_proto::codec::Reader::new(&all[i - 1]).varint().ok(), Some(cb::SET_ENTITY_MOTION));
        assert_eq!(kiln_proto::codec::Reader::new(&all[i - 2]).varint().ok(), Some(cb::BUNDLE_DELIMITER));
        assert_eq!(kiln_proto::codec::Reader::new(&all[i + 1]).varint().ok(), Some(cb::BUNDLE_DELIMITER));
    }
}
