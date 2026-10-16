//! An explosion's knockback reaches a player in the explosion packet, which the client adds to its own motion; Kiln sent a motion packet
//! instead, which replaces it: found with a wind charge thrown at the feet of a player beside a vanilla server, where the lift differed by
//! the player's own motion (wp68). The sound and particle come in the same packet, and not as packets of their own.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

type Blast = ([f64; 3], f32, Option<[f64; 3]>, usize);

/// A primed TNT at `dx` east of a player who stands still: every explosion packet (center, radius, knockback) and the number of motion packets
/// the player is sent about himself.
fn blast(dx: f64, dy: f64) -> (Vec<Blast>, usize) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let log = stats.clone();
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival User".into())]));
    let mut client = Client::new(1, stats);
    fn tick(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    tick(&mut sim, &mut client, 5);
    let me = sim.entity_id(1).expect("the player");
    let p = client.pos;
    log.log.lock().unwrap().as_mut().unwrap().clear();
    assert!(sim.step([ToSim::Console(format!("summon minecraft:tnt {} {} {} {{fuse:1}}", p[0] + dx, p[1] + dy, p[2]))]));
    tick(&mut sim, &mut client, 10);
    let packets = log.log.lock().unwrap().clone().unwrap();
    let mut explosions = Vec::new();
    let mut motions = 0;
    for pkt in &packets {
        let mut r = kiln_proto::codec::Reader::new(pkt);
        match r.varint().unwrap() {
            id if id == kiln_data::packets::play::clientbound::EXPLODE => {
                let center = [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()];
                let radius = r.f32().unwrap();
                let blocks = r.i32().unwrap() as usize;
                let knockback = r.bool().unwrap().then(|| [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()]);
                explosions.push((center, radius, knockback, blocks));
            }
            id if id == kiln_data::packets::play::clientbound::SET_ENTITY_MOTION => {
                if r.varint().unwrap() == me {
                    motions += 1;
                }
            }
            _ => {}
        }
    }
    (explosions, motions)
}

#[test]
fn a_player_pushed_by_a_blast_gets_the_knockback_in_the_explosion_packet() {
    let (explosions, _) = blast(1.5, 0.0);
    assert_eq!(explosions.len(), 1, "one explosion packet: {explosions:?}");
    let (center, radius, knockback, _) = explosions[0];
    assert_eq!(radius, 4.0);
    let k = knockback.expect("the player 1.5 blocks from a primed TNT is pushed");
    // away from the blast: it was east of the player, so the push goes west (and up: the eyes are above the TNT's feet)
    assert!(k[0] < -0.3 && k[1] > 0.0, "{k:?} from {center:?}");
}

#[test]
fn a_blast_at_the_feet_lifts_the_player() {
    let (explosions, _) = blast(0.0, 0.0);
    let (_, _, knockback, _) = explosions[0];
    let k = knockback.expect("pushed");
    assert!(k[1] > 0.8, "straight up and hard: {k:?}");
}

#[test]
fn the_push_is_not_sent_a_second_time_as_a_motion_packet_that_would_replace_the_players_own() {
    // (A hurt player is told his server-side motion, which the blast's push is part of: that one is the same as vanilla's.)
    let (_, motions) = blast(1.5, 0.0);
    assert!(motions <= 1, "{motions}");
}
