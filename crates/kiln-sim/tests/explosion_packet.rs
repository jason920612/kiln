//! An explosion's knockback reaches a player in the explosion packet, which the client adds to its own motion; Kiln sent a motion packet
//! instead, which replaces it: found with a wind charge thrown at the feet of a player beside a vanilla server, where the lift differed by
//! the player's own motion (wp68). The sound and particle come in the same packet, and not as packets of their own.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_player_pushed_by_a_blast_gets_the_knockback_in_the_explosion_packet_and_no_motion_packet() {
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
    assert!(sim.step([ToSim::Console(format!("summon minecraft:tnt {} {} {} {{fuse:1}}", p[0] + 1.5, p[1], p[2]))]));
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
                let blocks = r.i32().unwrap();
                let knockback = r.bool().unwrap().then(|| [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()]);
                explosions.push((center, radius, blocks, knockback));
            }
            id if id == kiln_data::packets::play::clientbound::SET_ENTITY_MOTION => {
                if r.varint().unwrap() == me {
                    motions += 1;
                }
            }
            _ => {}
        }
    }
    assert_eq!(explosions.len(), 1, "one explosion packet: {explosions:?}");
    let (center, radius, _, knockback) = explosions[0];
    assert_eq!(radius, 4.0);
    let k = knockback.expect("the player 1.5 blocks from a primed TNT is pushed");
    // away from the blast: it was east of the player, so the push goes west (and up: the eyes are above the TNT's feet)
    assert!(k[0] < -0.3 && k[1] > 0.0, "{k:?} from {center:?}");
    assert_eq!(motions, 0, "the push is not sent a second time as a motion packet");
}
