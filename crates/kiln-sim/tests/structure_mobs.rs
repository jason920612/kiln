//! The mobs structure generation places (elder guardians, vindicators, witches...) exist in the running
//! simulation, not only in the generated chunk. Needs the vanilla datapack (`KILN_DATAPACK`, else
//! `$KILN_WORK/generated`, else `work/generated`); skips without it.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};

fn datapack() -> Option<PathBuf> {
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    dir.join("reports/biome_parameters").is_dir().then_some(dir)
}

/// A player standing at `at`, waiting until the chunks around are loaded; the types of the entities within `radius`
/// blocks (horizontally) with how many there are.
fn visit(seed: i64, pack: PathBuf, at: [i32; 3], radius: f64, settle_ticks: usize) -> std::collections::BTreeMap<&'static str, usize> {
    let mut config = SimConfig::new(100, 10, None);
    config.noise = Some(NoiseConfig { seed, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Visitor", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Visitor".into()), ToSim::Console(format!("tp Visitor {} {} {}", at[0], at[1], at[2]))]));
    let mut client = Client::new(1, stats);
    for i in 0..settle_ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        std::thread::sleep(std::time::Duration::from_millis(5));
        if std::env::var_os("DIAG").is_some() && i % 20 == 0 {
            let el: Vec<_> = sim.entities().into_iter().filter(|(t, _)| *t == "minecraft:elder_guardian" || *t == "minecraft:item").map(|(t, p)| format!("{} {:.0},{:.0},{:.0}", &t[10..], p[0], p[1], p[2])).collect();
            eprintln!("tick {i}: {el:?}");
        }
    }
    let mut kinds = std::collections::BTreeMap::new();
    for (t, p) in sim.entities() {
        if t != "minecraft:player" && ((p[0] - at[0] as f64).powi(2) + (p[2] - at[2] as f64).powi(2)).sqrt() < radius {
            *kinds.entry(t).or_insert(0) += 1;
        }
    }
    kinds
}

/// wp60: an ocean monument in the running world (not only in the generated chunks) has its three elder guardians; seed 60's
/// monument at (-352, 160) (`/locate structure minecraft:monument` names its middle).
#[test]
fn an_ocean_monument_in_the_running_world_has_its_elder_guardians() {
    let Some(pack) = datapack() else { return };
    let found = visit(60, pack, [-352, 80, 160], 80.0, 1200);
    assert_eq!(found.get("minecraft:elder_guardian"), Some(&3), "the monument's mobs: {found:?}");
}
