//! A block that cannot stand any more drops its item in the tick a command changed its support.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_crop_over_a_powered_dispenser_pops_off_with_its_drop() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator User".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, client: &mut Client, extra: Vec<ToSim>| {
        let mut inbox = extra;
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    };
    for _ in 0..5 {
        tick(&mut sim, &mut client, Vec::new());
    }
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    tick(
        &mut sim,
        &mut client,
        vec![
            ToSim::Console(format!("setblock {x} {y} {z} minecraft:dispenser[facing=up]")),
            ToSim::Console(format!("setblock {x} {} {z} minecraft:torchflower_crop[age=0]", y + 1)),
        ],
    );
    tick(&mut sim, &mut client, Vec::new());
    tick(&mut sim, &mut client, vec![ToSim::Console(format!("setblock {} {y} {z} minecraft:redstone_block", x + 1))]);
    let mut seen = Vec::new();
    let mut blocks = Vec::new();
    for _ in 0..4 {
        blocks.push(sim.block_at(x, y + 1, z));
        seen.push(sim.entity_nbt().iter().filter(|t| t.get("id").and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:item")).count());
        tick(&mut sim, &mut client, Vec::new());
    }
    eprintln!("items per tick after the redstone block: {seen:?}, the crop: {blocks:?} (air is {})", kiln_data::blocks::default_state::AIR);
    assert!(seen.iter().any(|&n| n > 0), "the seeds dropped: {seen:?}");
}
