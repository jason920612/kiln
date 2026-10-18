//! `BreakDoorGoal` (wp77): a zombie that can break doors, held back by a wooden door in a glass wall between it and a player, hacks the door away
//! in 240 ticks on hard difficulty and leaves it alone on normal; a vindicator in a raid opens doors (`RaiderOpenDoorGoal`) instead of waiting
//! in front of them. Kiln's zombies walked into closed doors and stayed there.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn door_after(difficulty: &str, ticks_to_run: usize) -> (bool, Vec<u16>) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Victim", 2);
    assert!(sim.step([
        msg,
        ToSim::Console("gamemode survival Victim".into()),
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console(format!("difficulty {difficulty}")),
    ]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, &mut client, 5);
    // Night, so that the zombie does not burn.
    assert!(sim.step([ToSim::Console("time set 18000".into())]));
    let p = client.pos;
    let [x, y, z] = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    // A stone wall across the way with a door in it; the zombie is angry at the player (`HurtByTargetGoal` needs no sight).
    run(&mut sim, format!("fill {} {} {} {} {} {} minecraft:stone", x + 3, y, z - 24, x + 3, y + 3, z + 24));
    run(&mut sim, format!("setblock {} {y} {z} minecraft:oak_door[facing=east,half=lower,open=false]", x + 3));
    run(&mut sim, format!("setblock {} {} {z} minecraft:oak_door[facing=east,half=upper,open=false]", x + 3, y + 1));
    run(&mut sim, format!("summon minecraft:zombie {} {y} {} {{CanBreakDoors:1b,PersistenceRequired:1b}}", x + 6, z as f64 + 0.5));
    tick(&mut sim, &mut client, 3);
    run(&mut sim, "damage @e[type=minecraft:zombie,limit=1] 0.5 minecraft:player_attack by Victim".into());
    for k in 0..ticks_to_run / 50 {
        tick(&mut sim, &mut client, 50);
        if std::env::var_os("KILN_DOOR_TRACE").is_some() {
            let z = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:zombie"));
            eprintln!("{} {:?} player {:?}", k * 50, z.as_ref().and_then(|t| t.get("Pos")).map(|p| format!("{p:?}")), client.pos);
        }
    }
    let door = sim.block_at(x + 3, y, z).expect("loaded");
    let air = kiln_data::blocks_types::block_by_name("minecraft:air").map(|b| b.default).expect("air");
    let states = (y..y + 2).map(|yy| sim.block_at(x + 3, yy, z).unwrap()).collect();
    (door == air, states)
}

#[test]
fn a_zombie_breaks_a_door_on_hard_only() {
    let (broken, states) = door_after("hard", 900);
    assert!(broken, "the door is gone on hard: {states:?}");
    let (broken, _) = door_after("normal", 900);
    assert!(!broken, "and stays on normal");
}

/// `MoveThroughVillageGoal` of a zombie (wp77): at night a zombie with no target, 25 blocks from a village whose beds are claimed, walks to
/// a point of interest of it; far from any village it only strolls. Kiln's goal never started.
fn closest_approach(village: bool) -> f64 {
    let mut sim = Sim::new(SimConfig::new(8, 4, None));
    let (msg, stats) = join(1, "Victim", 2);
    assert!(sim.step([
        msg,
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console("difficulty normal".into()),
        ToSim::Console("gamemode creative Victim".into()),
    ]));
    let mut client = Client::new(1, stats);
    let tick = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, &mut client, 5);
    assert!(sim.step([ToSim::Console("time set 18000".into())]));
    let p = client.pos;
    let [x, y, z] = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    if village {
        for i in 0..3 {
            run(&mut sim, format!("setblock {} {y} {} minecraft:red_bed[part=head]", x + 3 + 2 * i, z + 4));
            run(&mut sim, format!("summon minecraft:villager {} {y} {}", x + 3 + 2 * i, z + 6));
        }
        run(&mut sim, format!("setblock {} {y} {} minecraft:bell", x - 3, z + 4));
        tick(&mut sim, &mut client, 60);
    }
    // A zombie that sees nothing (follow range 2) 25 blocks to the north of the village.
    run(&mut sim, format!("summon minecraft:zombie {}.5 {y} {}.5 {{PersistenceRequired:1b,attributes:[{{id:\"minecraft:follow_range\",base:2.0d}}]}}", x + 5, z - 21));
    let mut closest = f64::MAX;
    for _ in 0..40 {
        tick(&mut sim, &mut client, 20);
        let zombie = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:zombie"));
        let Some(zombie) = zombie else { continue };
        let kiln_proto::nbt::Tag::List(pos) = zombie.get("Pos").unwrap() else { panic!("Pos") };
        let v: Vec<f64> = pos.iter().map(|t| if let kiln_proto::nbt::Tag::Double(d) = t { *d } else { 0.0 }).collect();
        // The village's middle: the beds and the bell.
        let (cx, cz) = (x as f64 + 3.0, z as f64 + 5.0);
        closest = closest.min(((v[0] - cx).powi(2) + (v[2] - cz).powi(2)).sqrt());
    }
    closest
}

#[test]
fn a_zombie_at_night_walks_to_the_village_it_is_close_to() {
    let near = closest_approach(true);
    let far = closest_approach(false);
    assert!(near < 10.0, "it reached the village: closest {near:.1}");
    assert!(far > near + 4.0, "and not just by strolling: {far:.1} without a village");
}
