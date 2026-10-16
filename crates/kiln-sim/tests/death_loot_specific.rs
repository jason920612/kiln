//! A killed sheep drops its wool and a killed small slime its slime balls (wp68). The death loot tables ask the dying mob's class
//! (`type_specific/sheep` `sheared: false` for the wool, `type_specific/cube_mob` `size` for the slime balls and the magma cream),
//! and Kiln's death context answered "no" to every class predicate: no wool from a killed sheep, no slime balls, no magma cream.
//! Found by killing the same 12 sheep and slimes on Kiln and on a vanilla server.
//!
//! Loot tables come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

fn items_dropped(sim: &Sim, id: &str) -> usize {
    sim.entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item"))
        .filter(|t| matches!(t.get("Item"), Some(i) if i.get("id").and_then(Tag::as_str) == Some(id)))
        .count()
}

#[test]
fn killed_sheep_drop_wool_and_killed_small_slimes_drop_slime_balls() {
    if !have_datapack() {
        eprintln!("no vanilla datapack: skipped");
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "User", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    for _ in 0..16 {
        assert!(sim.step([ToSim::Console(format!("summon minecraft:sheep {} {} {} {{NoAI:1b,Color:0b}}", p[0] + 4.0, p[1], p[2]))]));
        assert!(sim.step([ToSim::Console(format!("summon minecraft:slime {} {} {} {{NoAI:1b,Size:0}}", p[0] - 4.0, p[1], p[2]))]));
    }
    ticks(&mut sim, &mut client, 3);
    assert!(sim.step([ToSim::Console("kill @e[type=sheep]".into()), ToSim::Console("kill @e[type=slime]".into())]));
    ticks(&mut sim, &mut client, 3);
    assert!(items_dropped(&sim, "minecraft:white_wool") > 0, "wool from killed white sheep");
    assert!(items_dropped(&sim, "minecraft:mutton") > 0, "mutton from killed sheep");
    assert!(items_dropped(&sim, "minecraft:slime_ball") > 0, "slime balls from killed small slimes");
}
