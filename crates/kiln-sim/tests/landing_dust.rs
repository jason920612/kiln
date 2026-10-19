//! wp80 (found by comparing the packets two real clients got while a cow fell five blocks and while a breeze long-jumped): `LivingEntity.checkFallDamage` sends a
//! `Level Particles` packet of block dust (`150 * min(0.2 + power / 15, 2.5)` of them) every time a living entity lands from above its safe fall distance, whether or not
//! the fall hurt. Kiln sent none, so mobs landing never kicked up dust.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn dust_packets_after_dropping(kind: &str, height: f64) -> usize {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Watcher", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let log = stats.clone();
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode creative Watcher".into())]));
    let mut client = Client::new(1, stats);
    fn tick(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    tick(&mut sim, &mut client, 10);
    let p = client.pos;
    // (The floor under the player is the surface; the mob starts above it in the open.)
    log.log.lock().unwrap().as_mut().unwrap().clear();
    assert!(sim.step([ToSim::Console(format!("summon {kind} {} {} {} {{NoAI:0b,PersistenceRequired:1b}}", p[0] + 3.0, p[1] + height, p[2]))]));
    tick(&mut sim, &mut client, 60);
    let log = log.log.lock().unwrap().clone().unwrap();
    log.iter()
        .filter(|pk| {
            let mut r = kiln_proto::codec::Reader::new(pk);
            r.varint().ok() == Some(kiln_data::packets::play::clientbound::LEVEL_PARTICLES)
        })
        .count()
}

#[test]
fn a_cow_landing_from_five_blocks_kicks_up_dust_but_one_stepping_down_a_block_does_not() {
    assert_eq!(dust_packets_after_dropping("minecraft:cow", 5.0), 1, "five blocks: power 1");
    assert_eq!(dust_packets_after_dropping("minecraft:cow", 1.0), 0, "one block: inside the safe fall distance");
}
