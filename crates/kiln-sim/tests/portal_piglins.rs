//! `NetherPortalBlock.randomTick` (wp77): in the overworld, a nether portal block now and then lets a zombified piglin out, one random tick in
//! 2000 times the difficulty, when a player is close; the mob has its portal cooldown set. Kiln's portal blocks did nothing: the farm of
//! zombified piglins from portal blocks gave none.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn piglins(difficulty: &str, spawn_monsters: bool, ticks: usize) -> Vec<Tag> {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([
        msg,
        ToSim::Console("gamemode creative Keeper".into()),
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console(format!("gamerule minecraft:spawn_monsters {spawn_monsters}")),
        ToSim::Console("gamerule minecraft:random_tick_speed 4096".into()),
        ToSim::Console(format!("difficulty {difficulty}")),
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
    let p = client.pos;
    let [x, y, z] = [p[0].floor() as i32 + 4, p[1].floor() as i32, p[2].floor() as i32];
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    // A frame of obsidian (4 wide, 5 tall) with its interior filled with portal blocks, on stone.
    run(&mut sim, format!("fill {} {} {} {} {} {} minecraft:stone", x - 2, y - 1, z - 2, x + 5, y - 1, z + 2));
    run(&mut sim, format!("fill {x} {y} {z} {} {} {z} minecraft:obsidian", x + 3, y + 4));
    run(&mut sim, format!("fill {} {} {z} {} {} {z} minecraft:air", x + 1, y + 1, x + 2, y + 3));
    run(&mut sim, format!("fill {} {} {z} {} {} {z} minecraft:nether_portal[axis=x]", x + 1, y + 1, x + 2, y + 3));
    tick(&mut sim, &mut client, ticks);
    sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:zombified_piglin")).collect()
}

#[test]
fn a_portal_block_lets_zombified_piglins_out_in_the_overworld() {
    let hard = piglins("hard", true, 600);
    assert!(!hard.is_empty(), "no zombified piglin from the portal in 600 ticks of 4096 random ticks");
    let cooldown = hard[0].get("PortalCooldown").and_then(Tag::as_i64).unwrap_or(0);
    assert!((1..=300).contains(&cooldown), "portal cooldown {cooldown}");
    // Peaceful and without monster spawning there are none.
    assert!(piglins("peaceful", true, 600).is_empty(), "none on peaceful");
    assert!(piglins("hard", false, 600).is_empty(), "none with spawn_monsters off");
}
