//! wp81 (lock step against vanilla, `tools/wp81/maps81.py`): the structure search of `exploration_map` loot functions (explorer and treasure maps, which the cartographer's
//! trades use too) must survive a data pack reload: after `/reload` or `/datapack enable` Kiln's new loot data had no explorer and every such function gave the plain item
//! (and the `filtered` modifier of the trades threw it away). Needs the vanilla datapack; skips without it.

use kiln_link::ToSim;
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

#[test]
fn explorer_maps_work_after_a_data_pack_reload() {
    let Some(pack) = datapack() else { return };
    let dir = std::env::temp_dir().join(format!("kiln-map-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let tables = dir.join("datapacks/p81/data/p81/loot_table");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(dir.join("datapacks/p81/pack.mcmeta"), r#"{"pack":{"description":"p81","min_format":121,"max_format":121}}"#).unwrap();
    // The cartographer's ocean monument map: the trade's own modifiers on the item it gives.
    std::fs::write(
        tables.join("monument.json"),
        r##"{"type":"minecraft:chest","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:ocean_monument_map","modifier":[
            {"type":"minecraft:exploration_map","decoration":"minecraft:monument","destination":"#minecraft:on_ocean_monument_maps","search_radius":100},
            {"type":"minecraft:filtered","item_filter":{"predicates":{"minecraft:map_id":{}}},"on_fail":{"type":"minecraft:discard"}}]}]}]}"##,
    )
    .unwrap();
    let mut config = SimConfig::new(4, 3, Some(dir.clone()));
    config.noise = Some(NoiseConfig { seed: 78, datapack: pack, threads: 4 });
    let mut sim = Sim::new(config);
    assert!(sim.step([]));
    sim.console_now("reload");
    for _ in 0..4 {
        assert!(sim.step([]));
    }
    sim.console_now("datapack enable \"file/p81\"");
    for _ in 0..4 {
        assert!(sim.step([]));
    }
    sim.console_now("loot spawn 0 100 0 loot p81:monument");
    for _ in 0..3 {
        assert!(sim.step([ToSim::Console("say ".into())]));
    }
    let maps: Vec<_> = sim.item_stacks().into_iter().filter(|s| s.item_name() == "minecraft:ocean_monument_map").collect();
    assert_eq!(maps.len(), 1, "a map came out: {maps:?}");
    assert!(maps[0].get(kiln_item::keys::MAP_ID).is_some(), "with a map of its own, made by the structure search");
    drop(sim);
    let _ = std::fs::remove_dir_all(&dir);
}
