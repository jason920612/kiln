//! wp83: two packs on top of vanilla: the later pack's file replaces the earlier one's (same id), a tag with `replace` drops what
//! the earlier pack added, namespaces and nested paths sort the way `Identifier.compareTo` does (path first, then namespace).
//! `tools/wp83/check_registries.sh kiln_wp83 kiln_wp83b` checks the same two packs against vanilla's own loading.

mod support;

use kiln_sim::{Sim, SimConfig};
use support::pack_world;

#[test]
fn a_later_pack_overrides_an_earlier_one_and_namespaces_sort_by_path_first() {
    let _x = kiln_data::live::exclusive();
    let dir = pack_world("two-packs", &["kiln_wp83", "kiln_wp83b"]);
    let sim = Sim::new(SimConfig::new(4, 4, Some(dir.clone())));
    let names: Vec<&str> = kiln_data::live::synced_entries("minecraft:painting_variant").unwrap().to_vec();
    let at = |n: &str| names.iter().position(|x| *x == n).unwrap_or_else(|| panic!("{n}"));
    // `aaa83:alban` and `minecraft:alban` have the same path: the namespace decides.
    assert_eq!(at("aaa83:alban") + 1, at("minecraft:alban"));
    // `kiln83:sub/dir/zz` is in there too, sorted as its whole path.
    assert!(names.contains(&"kiln83:sub/dir/zz"));
    let path = |i: usize| names[i].split_once(':').unwrap().1;
    let sorted = (1..names.len()).all(|i| (path(i - 1), names[i - 1]) <= (path(i), names[i]));
    assert!(sorted, "paths in order, namespaces breaking ties");
    // The second pack's `kiln83:speck` replaced the first's: 2x2.
    let sizes = kiln_entity::ext_entity::painting_variants::variants();
    let speck = &sizes[at("kiln83:speck")];
    assert_eq!((speck.width, speck.height), (2, 2));
    // Its `replace` tag dropped the first pack's stone and the vanilla ladders; `#kiln83:softer` is dirt and sand (pack one) plus gravel (pack two).
    let climbable = kiln_data::live::tag("minecraft:block", "minecraft:climbable").unwrap();
    let id = |n: &str| kiln_data::builtin_id("minecraft:block", n).unwrap();
    assert_eq!(climbable, [id("minecraft:dirt"), id("minecraft:sand"), id("minecraft:gravel")]);
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_biome_of_a_pack_is_numbered_and_found_by_the_commands() {
    let _x = kiln_data::live::exclusive();
    let dir = pack_world("two-packs-biome", &["kiln_wp83", "kiln_wp83b"]);
    let mut sim = Sim::new(SimConfig::new(4, 4, Some(dir.clone())));
    let names: Vec<&str> = kiln_data::live::synced_entries("minecraft:worldgen/biome").unwrap().to_vec();
    let glade = names.iter().position(|n| *n == "kiln83:glade").expect("the pack's biome is a registry entry");
    // Sorted by path among vanilla's (`glade` sits between `frozen_river` and `grove`), the biomes after it moved up by one.
    assert_eq!((names[glade - 1], names[glade + 1]), ("minecraft:frozen_river", "minecraft:grove"));
    let vanilla = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome").unwrap().1;
    assert_eq!(kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains"), vanilla.iter().position(|n| *n == "minecraft:plains").map(|i| i as i32 + 1));
    assert!(sim.step([]));
    sim.capture_console();
    sim.console_now("forceload add 0 0");
    for _ in 0..40 {
        assert!(sim.step([]));
    }
    sim.take_console();
    sim.console_now("fillbiome 0 -60 0 15 -50 15 kiln83:glade");
    let said = sim.take_console().join("\n");
    assert!(said.contains("commands.fillbiome.success"), "{said}");
    sim.console_now("execute if biome 8 -55 8 kiln83:glade");
    assert!(sim.take_console().join("\n").contains("conditional.pass"));
    sim.console_now("execute if biome 8 -55 8 minecraft:plains");
    assert!(!sim.take_console().join("\n").contains("conditional.pass"));
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}
