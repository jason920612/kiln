//! Fishing in the running simulation: a rod casts a bobber into a pool, a fish bites, reeling
//! in brings up the catch (from the vanilla fishing loot tables) with experience, wears the rod
//! and fires `fishing_rod_hooked`; reeling in with nothing biting just brings the bobber back.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
        return std::path::Path::new(&dir).join("data/minecraft/loot_table").is_dir();
    }
    false
}

#[test]
fn a_rod_catches_fish_from_a_pool() {
    if !have_datapack() {
        eprintln!("skipped: set KILN_DATAPACK");
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Angler", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Angler".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
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
    let g = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
    // A pool south of the player (it faces +z), open to the sky.
    let fill = |sim: &mut Sim, a: [i32; 3], b: [i32; 3], block: &str| {
        assert!(sim.step([ToSim::Console(format!("fill {} {} {} {} {} {} {block}", a[0], a[1], a[2], b[0], b[1], b[2]))]));
    };
    fill(&mut sim, [g[0] - 4, g[1] - 3, g[2] + 2], [g[0] + 4, g[1], g[2] + 12], "minecraft:water");
    let rod = kiln_data::builtin_id("minecraft:item", "minecraft:fishing_rod").unwrap();
    assert!(sim.step([ToSim::Console("gamemode creative Angler".into())]));
    let stack = ItemStack { item: rod, count: 1, added: Vec::new(), removed: Vec::new() };
    assert!(sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    assert!(sim.step([ToSim::Console("gamemode survival Angler".into())]));
    tick(&mut sim, &mut client, 2);
    let use_rod = |sim: &mut Sim, seq: i32| {
        assert!(sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: Hand::Main, sequence: seq, yaw: 0.0, pitch: 0.0 })]));
    };
    // Reeling in right away brings the bobber back and costs nothing.
    use_rod(&mut sim, 1);
    tick(&mut sim, &mut client, 3);
    assert_eq!(sim.fishing_bobbers().len(), 1, "a bobber is out");
    use_rod(&mut sim, 2);
    assert!(sim.fishing_bobbers().is_empty(), "reeled in");
    assert_eq!(sim.item_damage(1, 36), Some(0));
    // Cast again and wait for a bite.
    use_rod(&mut sim, 3);
    let mut bobbing = false;
    let mut bit = false;
    for _ in 0..1500 {
        tick(&mut sim, &mut client, 1);
        let b = sim.fishing_bobbers();
        assert_eq!(b.len(), 1, "the bobber stays out");
        bobbing |= b[0].2;
        if b[0].1 {
            bit = true;
            break;
        }
    }
    assert!(bobbing, "the bobber landed in the water");
    assert!(bit, "a fish bit");
    let xp_before = sim.experience(1).unwrap().2;
    use_rod(&mut sim, 4);
    assert!(sim.fishing_bobbers().is_empty());
    assert_eq!(sim.item_damage(1, 36), Some(1), "a catch wears the rod by one");
    // The catch flies to the player: it and the experience arrive.
    tick(&mut sim, &mut client, 40);
    let inv = sim.inventory(1).unwrap();
    let caught = inv.iter().enumerate().any(|(i, s)| i != 36 && s.is_some());
    assert!(caught, "something was caught: {inv:?}");
    assert!(sim.experience(1).unwrap().2 > xp_before, "fishing experience");
    // `fishy_business` for each fish that came up (junk and treasure do not count).
    for fish in ["cod", "salmon", "tropical_fish", "pufferfish"] {
        let id = kiln_data::builtin_id("minecraft:item", &format!("minecraft:{fish}")).unwrap();
        let got = inv.iter().flatten().any(|&(item, _)| item == id);
        if let Some(done) = sim.criterion_done(1, "minecraft:husbandry/fishy_business", fish) {
            assert_eq!(done, got, "fishy_business {fish}");
        }
    }
}
