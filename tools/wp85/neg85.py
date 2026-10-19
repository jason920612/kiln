"""wp85 negative checks (on the VM, from the checkout): each new behaviour is switched off in the source, the test that guards it must fail,
and the source is restored.

usage: python3 tools/wp85/neg85.py [name ...]    (all checks by default; KILN_DATAPACK / KILN_WORK as for the tests)
Exit 0 when every sabotaged build fails its test.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DATA = os.environ.get("KILN_DATAPACK", os.path.expanduser("~/kiln/work/generated"))

WG = "crates/kiln-worldgen/src/"
SIM = "crates/kiln-sim/src/"

# (name, file, old, new, cargo test arguments)
CHECKS = [
    ("stack-of-roots", WG + "datapack.rs",
     "let mut merged: std::collections::BTreeMap<(String, String), Json> = std::collections::BTreeMap::new();\n    for root in roots {",
     "let mut merged: std::collections::BTreeMap<(String, String), Json> = std::collections::BTreeMap::new();\n    for root in roots.iter().take(1) {",
     "-p kiln-worldgen --test packs a_replaced_biome"),
    ("identifier-order", WG + "datapack.rs",
     "Ok(merged.into_iter().map(|((path, ns), json)| (format!(\"{ns}:{path}\"), json)).collect())",
     "let mut v: Vec<(String, Json)> = merged.into_iter().map(|((path, ns), json)| (format!(\"{ns}:{path}\"), json)).collect();\n    v.sort_by(|a, b| a.0.cmp(&b.0));\n    Ok(v)",
     "-p kiln-worldgen --lib a_later_pack"),
    ("tag-replace", WG + "datapack.rs",
     "                if replace {\n                    list.clear();\n                }\n",
     "",
     "-p kiln-worldgen --lib tags_add_up"),
    ("templates-of-the-packs", WG + "structure/template.rs",
     "for pack in roots.iter().skip(1).rev() {",
     "for pack in roots.iter().skip(1).rev().take(0) {",
     "-p kiln-worldgen --test packs a_structure_from"),
    ("checkerboard-source", WG + "generator.rs",
     "BiomeSource::Checkerboard { biomes, shift: scale + 2 }",
     "BiomeSource::Checkerboard { biomes, shift: scale + 3 }",
     "-p kiln-worldgen --test packs a_dimension_of_a_pack"),
    ("flat-layers", WG + "stem.rs",
     "            if layer == AIR {\n                continue;\n            }",
     "            if true {\n                continue;\n            }",
     "-p kiln-sim --test dp_dimensions a_superflat"),
    ("dimension-types-from-registry", SIM + "dimensions.rs",
     "    kiln_data::levels::install_dimension_types(types);",
     "    let _ = types;",
     "-p kiln-sim --test dp_dimensions the_simulation_reads"),
    ("stems-of-the-packs", SIM + "dimensions.rs",
     "        Ok(list) => map.extend(list),",
     "        Ok(_) => {}",
     "-p kiln-sim --test dp_dimensions the_levels_are"),
    ("per-level-state", SIM + "world_state.rs",
     "            borders: per_level(),",
     "            borders: Vec::new(),",
     "-p kiln-sim --test dp_dimensions clients_are_told"),
    ("weather-by-type", SIM + "weather.rs",
     "    kind.has_skylight && !kind.has_ceiling && dim != crate::END_ID",
     "    dim == OVERWORLD_ID",
     "-p kiln-sim --test dp_dimensions weather_follows"),
    ("registry-schema", SIM + "registry_schema.rs",
     "        Some(fields) => check(v, T::Obj(fields)),",
     "        Some(_) => Ok(()),",
     "-p kiln-sim --lib the_entries_vanilla_refuses"),
    ("transformer-files", SIM + "block_transformer.rs",
     "let vanilla = crate::world_state::worldgen_roots().into_iter().next();",
     "let vanilla: Option<std::path::PathBuf> = None;",
     "-p kiln-sim --test transformer_parity"),
    ("armadillo-brush-tool", SIM + "entities.rs",
     "} else if table == \"minecraft:brush/armadillo\" {",
     "} else if false {",
     "-p kiln-sim --test loot_calls brushing_an_armadillo"),
    ("spawns-of-the-packs", SIM + "spawner.rs",
     "let biomes = crate::world_state::stacked_json(roots, \"worldgen/biome\");",
     "let biomes = crate::world_state::stacked_json(&roots[..1], \"worldgen/biome\");",
     "-p kiln-sim --test dp_worldgen a_replaced_biome_spawns"),
]


def run(name, path, old, new, args):
    full = os.path.join(ROOT, path)
    text = open(full, encoding="utf-8").read()
    if text.count(old) != 1:
        print(f"{name}: the sabotage does not apply ({text.count(old)} matches in {path})")
        return False
    open(full, "w", encoding="utf-8", newline="").write(text.replace(old, new))
    try:
        env = dict(os.environ, KILN_DATAPACK=DATA)
        r = subprocess.run(["cargo", "test", "--release", *args.split(), "--", "--test-threads=2"], cwd=ROOT, env=env, capture_output=True, text=True)
    finally:
        open(full, "w", encoding="utf-8", newline="").write(text)
    failed = r.returncode != 0 and "test result: FAILED" in r.stdout + r.stderr
    compiled = "error[E" not in r.stderr and "could not compile" not in r.stderr
    print(f"{name}: {'fails without the change, as it must' if failed and compiled else 'DOES NOT FAIL' if compiled else 'does not compile'}")
    return failed and compiled


def main():
    want = sys.argv[1:]
    ok = True
    for check in CHECKS:
        if not want or check[0] in want:
            ok &= run(*check)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
