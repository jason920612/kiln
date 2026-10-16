//! A fully hydrated dried ghast hatches a ghastling (`DriedGhastBlock.spawnGhastling`) (wp68). Kiln removed the block and
//! spawned nothing ("the happy ghast is not a mob Kiln has yet"), found by pouring water on a dried ghast in a real client.

use kiln_data::blocks::default_state as d;
use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_hydrated_dried_ghast_hatches_a_baby_happy_ghast_facing_the_way_it_faced() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    fn tick(sim: &mut Sim, client: &mut Client, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    tick(&mut sim, &mut client, 5);
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    assert!(sim.step([
        ToSim::Console("gamerule minecraft:random_tick_speed 4096".into()),
        ToSim::Console(format!("setblock {x} {y} {z} minecraft:dried_ghast[facing=east,hydration=3,waterlogged=true]")),
    ]));
    assert_ne!(sim.block_at(x, y, z), Some(d::AIR));
    // A random tick schedules the next step 5000 ticks ahead; the step at hydration 3 hatches it.
    for _ in 0..70 {
        tick(&mut sim, &mut client, 100);
        if sim.block_at(x, y, z) != sim.block_at(x, y + 10, z) && sim.entity_ids_of("minecraft:happy_ghast").len() == 1 {
            break;
        }
    }
    let ghasts: Vec<Tag> = sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:happy_ghast")).collect();
    assert_eq!(ghasts.len(), 1, "one ghastling hatched");
    let g = &ghasts[0];
    let age = g.get("Age").and_then(Tag::as_i64).unwrap();
    assert!((-24000..-22000).contains(&age), "a baby that has just hatched: {age}");
    let Some(Tag::List(at)) = g.get("Pos") else { panic!("Pos") };
    // (It starts in the block's column and flies about while the loop above waits for the next hundred ticks.)
    assert!((at[0].as_f64().unwrap() - x as f64).abs() < 12.0 && (at[2].as_f64().unwrap() - z as f64).abs() < 12.0, "near the block: {at:?}");
    assert_ne!(sim.block_at(x, y, z), Some(d::DRIED_GHAST), "the dried ghast is gone");
}
