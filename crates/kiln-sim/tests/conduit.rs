//! Conduits (`ConduitBlockEntity`, wp72): Kiln had the block and its effect but nothing that ticked it, so a finished conduit never
//! gave Conduit Power and never hurt a hostile mob in its water. A real client at a monument-style frame (42 prismarine, water) got
//! no effect in Kiln and the effect (and the zombie's damage) on vanilla.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const C: [i32; 3] = [4, 110, 4];

fn ticks(sim: &mut Sim, client: &mut Client, n: usize) {
    for _ in 0..n {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

fn run(sim: &mut Sim, commands: &[String]) {
    assert!(sim.step(commands.iter().map(|c| ToSim::Console(c.clone()))));
}

/// A water cube around the conduit's 5x5x5 frame, a player in the water and a zombie that cannot move.
fn world(frame: bool) -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Diver", 2);
    let setup = ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:random_tick_speed 0", "gamemode survival Diver", "forceload add 0 0 15 15"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats);
    ticks(&mut sim, &mut client, 10);
    let [x, y, z] = C;
    let mut cmds = vec![
        format!("fill {} {} {} {} {} {} minecraft:water", x - 7, y - 7, z - 7, x + 7, y + 7, z + 7),
        format!("fill {} {} {} {} {} {} minecraft:prismarine outline", x - 2, y - 2, z - 2, x + 2, y + 2, z + 2),
        format!("setblock {x} {y} {z} minecraft:conduit[waterlogged=true]"),
        format!("summon minecraft:zombie {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", x as f64 + 5.5, y as f64 - 1.0, z as f64 + 0.5),
        format!("tp Diver {} {} {}", x as f64 + 0.5, y as f64 - 5.0, z as f64 + 5.5),
    ];
    if !frame {
        // One block of the frame short of hunting (41), still active (16 are enough).
        cmds.push(format!("setblock {x} {} {z} minecraft:stone", y + 2));
        cmds.push(format!("setblock {} {y} {z} minecraft:stone", x + 2));
    }
    run(&mut sim, &cmds);
    (sim, client)
}

fn effect(sim: &Sim, name: &str) -> Option<(i32, i32)> {
    sim.effects(1).unwrap().iter().find(|e| e.0 == name).map(|e| (e.1, e.2))
}

#[test]
fn a_finished_conduit_gives_conduit_power_to_a_player_in_its_water() {
    let (mut sim, mut client) = world(true);
    assert_eq!(effect(&sim, "minecraft:conduit_power"), None, "nothing happens before the conduit looks at its frame");
    // the check runs when the game time is a multiple of 40
    ticks(&mut sim, &mut client, 45);
    let (amp, dur) = effect(&sim, "minecraft:conduit_power").expect("a conduit with a full frame powers the players in its water");
    assert_eq!(amp, 0);
    assert!((200..=260).contains(&dur), "260 ticks given at the last check, {dur} left");
    // out of the water: no power given any more
    run(&mut sim, &[format!("tp Diver {} {} {}", C[0] as f64 + 0.5, C[1] as f64 + 20.0, C[2] as f64 + 5.5)]);
    ticks(&mut sim, &mut client, 300);
    assert_eq!(effect(&sim, "minecraft:conduit_power"), None, "the effect ran out and was not renewed in the air");
}

#[test]
fn a_conduit_with_all_42_frame_blocks_hurts_a_hostile_mob_in_its_water_every_40_ticks() {
    let (mut sim, mut client) = world(true);
    let zombie = sim.entity_ids_of("minecraft:zombie")[0];
    let health = |sim: &Sim| sim.mob_state(zombie).unwrap().health;
    assert_eq!(health(&sim), 20.0);
    // 40 ticks later (at the latest) it has been hit once, 40 more: twice (4 magic damage each)
    ticks(&mut sim, &mut client, 41);
    assert_eq!(health(&sim), 16.0);
    ticks(&mut sim, &mut client, 40);
    assert_eq!(health(&sim), 12.0);
    let target = sim.block_entity_nbt(C[0], C[1], C[2]).and_then(|t| t.get("Target").cloned());
    assert!(matches!(target, Some(kiln_proto::nbt::Tag::IntArray(ref v)) if v.len() == 4), "the conduit tells its clients the target: {target:?}");
}

#[test]
fn a_conduit_short_of_42_blocks_powers_but_does_not_hunt() {
    let (mut sim, mut client) = world(false);
    let zombie = sim.entity_ids_of("minecraft:zombie")[0];
    ticks(&mut sim, &mut client, 90);
    assert!(effect(&sim, "minecraft:conduit_power").is_some(), "16 blocks are enough to power");
    assert_eq!(sim.mob_state(zombie).unwrap().health, 20.0, "it takes 42 to hunt");
}
