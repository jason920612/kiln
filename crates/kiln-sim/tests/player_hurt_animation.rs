//! wp80 (found with two real clients side by side: a survival player in a guardian's beam, hit by an arrow, a small fireball, a trident or a wind charge):
//! vanilla's `LivingEntity.hurtServer` pushes the victim (`dealDefaultKnockback`, from `getSourcePosition`) and a player's own client is told which way the blow
//! came from (`ServerPlayer.indicateDamage`, the `Hurt Animation` packet that tilts the camera) for EVERY damage type outside `#minecraft:no_knockback`.
//! Kiln did both only for melee hits and thorns, so a projectile, a guardian's beam or a falling anvil neither tilted the camera nor pushed.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn hurt_animations_after(console: &[&str], ticks: usize) -> usize {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Tilt", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let log = stats.clone();
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamerule minecraft:natural_health_regeneration false".into())]));
    let mut client = Client::new(1, stats);
    fn tick(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    tick(&mut sim, &mut client, 5);
    assert!(sim.step([ToSim::Console("gamemode survival Tilt".into()), ToSim::Console("difficulty normal".into())]));
    tick(&mut sim, &mut client, 40);
    let p = client.pos;
    log.log.lock().unwrap().as_mut().unwrap().clear();
    for c in console {
        let c = c.replace("{x}", &format!("{}", p[0] + 4.0)).replace("{y}", &format!("{}", p[1] + 1.0)).replace("{z}", &format!("{}", p[2]));
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, &mut client, ticks);
    let log = log.log.lock().unwrap().clone().unwrap();
    log.iter()
        .filter(|pk| {
            let mut r = kiln_proto::codec::Reader::new(pk);
            r.varint().ok() == Some(kiln_data::packets::play::clientbound::HURT_ANIMATION)
        })
        .count()
}

#[test]
fn an_arrow_a_fireball_and_a_wind_charge_tilt_the_players_camera() {
    assert_eq!(hurt_animations_after(&["summon minecraft:arrow {x} {y} {z} {Motion:[-3.0d,0.0d,0.0d],damage:2.0d}"], 8), 1, "arrow");
    assert_eq!(hurt_animations_after(&["summon minecraft:small_fireball {x} {y} {z} {Motion:[-0.5d,0.0d,0.0d],power:[-0.5d,0.0d,0.0d]}"], 8), 1, "small fireball");
    assert_eq!(hurt_animations_after(&["summon minecraft:wind_charge {x} {y} {z} {Motion:[-0.5d,0.0d,0.0d],acceleration_power:0.1d}"], 8), 1, "wind charge");
}

#[test]
fn damage_without_a_position_still_tilts_and_hops_but_a_no_knockback_type_does_not() {
    // `/damage` without `by`: no source position, the push is a random wisp but `indicateDamage(0, 0)` is still sent.
    assert_eq!(hurt_animations_after(&["damage Tilt 1 minecraft:mob_attack"], 3), 1, "mob_attack without a position");
    assert_eq!(hurt_animations_after(&["damage Tilt 1 minecraft:generic"], 3), 0, "generic is in #no_knockback");
    assert_eq!(hurt_animations_after(&["damage Tilt 1 minecraft:cactus"], 3), 0, "cactus is in #no_knockback");
}
