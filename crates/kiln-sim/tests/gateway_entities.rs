//! wp80 (found comparing a pig and an item standing in an end gateway in both servers: vanilla sent them to the gateway's exit, Kiln left them there; only players
//! went): `TheEndGatewayBlock.entityInside` takes every entity that can use portals, mobs and items too.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn pos_of(sim: &Sim, id: &str) -> Option<[f64; 3]> {
    let t = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(id))?;
    match t.get("Pos") {
        Some(Tag::List(l)) if l.len() == 3 => Some([l[0].as_f64()?, l[1].as_f64()?, l[2].as_f64()?]),
        _ => None,
    }
}

#[test]
fn a_pig_and_an_item_in_an_end_gateway_go_to_its_exit() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Watcher", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode creative Watcher".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 5);
    let (gx, gy, gz) = (30, 100, 30);
    for c in [
        format!("fill {} {} {} {} {} {} minecraft:air", gx - 4, gy - 3, gz - 4, gx + 25, gy + 6, gz + 4),
        format!("setblock {gx} {gy} {gz} minecraft:end_gateway{{exit_portal:[I;{},{},{}],ExactTeleport:1b}}", gx + 20, gy + 2, gz),
        format!("summon minecraft:pig {gx}.5 {gy} {gz}.5 {{NoAI:1b,PersistenceRequired:1b}}"),
        format!("summon minecraft:item {gx}.5 {gy} {gz}.5 {{Item:{{id:\"minecraft:stone\",count:1}},PickupDelay:32767s}}"),
    ] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, 40);
    for id in ["minecraft:pig", "minecraft:item"] {
        let p = pos_of(&sim, id).unwrap_or_else(|| panic!("{id} exists"));
        assert!((p[0] - (gx + 20) as f64 - 0.5).abs() < 1.5 && (p[1] - (gy + 2) as f64).abs() < 1.5, "{id} went to the exit: {p:?}");
    }
}
