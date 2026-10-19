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

fn run(summon: &str, id: &str) -> [f64; 3] {
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
        summon.replace("GX", &gx.to_string()).replace("GY", &gy.to_string()).replace("GZ", &gz.to_string()),
    ] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, 40);
    pos_of(&sim, id).unwrap_or_else(|| panic!("{id} exists"))
}

fn at_exit(p: [f64; 3]) -> bool {
    (p[0] - 50.5).abs() < 1.5 && (p[2] - 30.5).abs() < 1.5 && p[1] < 105.0
}

#[test]
fn a_pig_in_an_end_gateway_goes_to_its_exit() {
    let p = run("summon minecraft:pig GX.5 GY GZ.5 {NoAI:1b,PersistenceRequired:1b}", "minecraft:pig");
    assert!(at_exit(p), "the pig went to the exit: {p:?}");
}

#[test]
fn an_item_in_an_end_gateway_goes_to_its_exit() {
    let p = run("summon minecraft:item GX.5 GY GZ.5 {Item:{id:\"minecraft:stone\",count:1},PickupDelay:32767s}", "minecraft:item");
    assert!(at_exit(p), "the item went to the exit: {p:?}");
}

fn motion_of(sim: &Sim, id: &str) -> Option<[f64; 3]> {
    let t = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(id))?;
    match t.get("Motion") {
        Some(Tag::List(l)) if l.len() == 3 => Some([l[0].as_f64()?, l[1].as_f64()?, l[2].as_f64()?]),
        _ => None,
    }
}

/// wp82: `EndGatewayBlock.getPortalDestination` keeps the motion of what goes through (`Relative.DELTA`) except an ender pearl, which leaves with none.
#[test]
fn a_gateway_keeps_the_motion_of_what_goes_through_but_not_of_a_pearl() {
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
        // an arrow in the gateway moving north-east and a pearl with the same motion (their exit is the same block; the first one through starts the cooldown, so each has its own gateway below)
        format!("summon minecraft:snowball {gx}.5 {gy}.5 {gz}.5 {{Motion:[0.0d,0.0d,0.0d]}}"),
    ] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, 3);
    // A boat-less test of the motion: a minecart-free item thrown through with a motion the tick after it arrives still has most of it.
    assert!(sim.step([ToSim::Console(format!("kill @e[type=minecraft:snowball]"))]));
    assert!(sim.step([ToSim::Console(format!("summon minecraft:item {gx}.5 {gy} {gz}.5 {{Item:{{id:\"minecraft:stone\",count:1}},PickupDelay:32767s,Motion:[0.0d,0.2d,0.0d]}}"))]));
    tick(&mut sim, 3);
    let m = motion_of(&sim, "minecraft:item").expect("the item");
    let p = pos_of(&sim, "minecraft:item").expect("the item");
    assert!((p[0] - 50.5).abs() < 2.0, "it went through: {p:?}");
    assert!(m[1].abs() > 0.05 || p[1] > 102.0, "it kept going up: motion {m:?} at {p:?}");
}
