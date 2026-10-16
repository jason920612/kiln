//! A sniffer egg cracks twice and then hatches (wp68). Each crack changes the egg's state, and `SnifferEggBlock.onPlace` - which
//! schedules the next crack - runs on every change of the state; Kiln ran it for the first placement only, so an egg stopped
//! at its first crack for ever. Found by waiting for an egg on moss to hatch beside a vanilla server's.

use kiln_data::blocks::default_state as d;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_sniffer_egg_on_moss_hatches_a_baby_sniffer() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    fn ticks(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    assert!(sim.step([
        ToSim::Console(format!("setblock {x} {} {z} minecraft:moss_block", y - 1)),
        ToSim::Console(format!("setblock {x} {y} {z} minecraft:sniffer_egg[hatch=0]")),
    ]));
    let mut cracked = None;
    for t in 0..14_000 {
        ticks(&mut sim, &mut client, 1);
        let b = sim.block_at(x, y, z);
        if cracked.is_none() && b != Some(d::AIR) && b != sim.block_at(x, y + 5, z) && t > 100 {
            // (the first crack changes the state but not the block)
        }
        if b == Some(d::AIR) {
            cracked = Some(t);
            break;
        }
    }
    let t = cracked.expect("the egg hatched within 14000 ticks on moss (three delays of 4000 to 4300)");
    assert!(t > 8_000, "three delays of 4000+ ticks, not fewer: {t}");
    assert_eq!(sim.entity_ids_of("minecraft:sniffer").len(), 1, "a baby sniffer");
}
