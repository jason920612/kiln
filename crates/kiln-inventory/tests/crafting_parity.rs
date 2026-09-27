//! Crafting grids recorded from vanilla 26.3 (`tools/InventoryVectors.java crafting`): for every
//! crafting recipe, grids built from its ingredients (and altered or random grids), with the
//! recipe vanilla picked, its result and its remaining items.
//!
//! Vectors: `$KILN_WORK/wp3-inventory/crafting.jsonl` (skipped when absent).

mod common;

use common::{same, show, stack, stacks};
use kiln_inventory::World;
use kiln_inventory::recipe::CraftingInput;
use serde_json::Value as Json;

/// The maps the harness registers: ids 0-5 with scale min(id, 4).
struct Maps;

impl World for Maps {
    fn map_scale(&self, map_id: i32) -> Option<i8> {
        (0..6).contains(&map_id).then(|| map_id.min(4) as i8)
    }
}

#[test]
fn recipes_load_in_vanilla_order() {
    let path = common::work().join("wp3-inventory/crafting.jsonl");
    let (Ok(text), Some(rules)) = (std::fs::read_to_string(&path), common::rules()) else {
        eprintln!("skipping: vectors or datapack not found");
        return;
    };
    let first: Json = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let vanilla: Vec<&str> = first["order"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let ours: Vec<&str> = rules.recipes.recipes().iter().map(|r| r.id.as_str()).collect();
    assert!(rules.recipes.errors.is_empty(), "recipes that failed to load: {:?}", rules.recipes.errors);
    assert_eq!(ours, vanilla);
}

#[test]
fn crafting_grids_match_vanilla() {
    let path = common::work().join("wp3-inventory/crafting.jsonl");
    let (Ok(text), Some(rules)) = (std::fs::read_to_string(&path), common::rules()) else {
        eprintln!("skipping: vectors or datapack not found");
        return;
    };
    let (mut grids, mut matched, mut failures) = (0, 0, Vec::new());
    let mut covered = std::collections::HashSet::new();
    for line in text.lines().skip(1) {
        let rec: Json = serde_json::from_str(line).unwrap();
        grids += 1;
        let size = rec["size"].as_u64().unwrap() as usize;
        let grid = stacks(&rec["grid"]);
        let input = CraftingInput::new(size, size, &grid);
        let found = rules.recipes.find_crafting(&input, None, &Maps);
        let ours = found.map(|i| rules.recipes.id(i));
        let theirs = rec["recipe"].as_str();
        let desc = rec["desc"].as_str().unwrap();
        let grid_s: Vec<String> = grid.iter().map(show).collect();
        if ours != theirs {
            failures.push(format!("{desc}: vanilla picks {theirs:?}, kiln {ours:?}\n  grid {grid_s:?}"));
            continue;
        }
        let Some(i) = found else { continue };
        matched += 1;
        covered.insert(theirs.unwrap().to_owned());
        let result = rules.recipes.assemble(i, &input, &Maps);
        let expected = stack(rec["result"].as_str().unwrap());
        if !same(&result, &expected) {
            failures.push(format!("{desc}: result vanilla {} {:?}\n  kiln {} {:?}\n  grid {grid_s:?}", show(&expected), expected.patch(), show(&result), result.patch()));
        }
        let remaining = rules.recipes.get(i).recipe.remaining_items(&input);
        let exp_rem = stacks(&rec["remaining"]);
        if remaining.len() != exp_rem.len() || !remaining.iter().zip(&exp_rem).all(|(a, b)| same(a, b)) {
            failures.push(format!(
                "{desc}: remaining vanilla {:?} kiln {:?}",
                exp_rem.iter().map(show).collect::<Vec<_>>(),
                remaining.iter().map(show).collect::<Vec<_>>()
            ));
        }
    }
    for f in failures.iter().take(20) {
        eprintln!("{f}\n");
    }
    let crafting = rules.recipes.recipes().iter().filter(|r| r.recipe.is_crafting()).count();
    eprintln!("{grids} grids, {matched} matched a recipe, {} of {crafting} crafting recipes produced, {} failures", covered.len(), failures.len());
    assert!(failures.is_empty(), "{} of {grids} grids differ from vanilla", failures.len());
}
