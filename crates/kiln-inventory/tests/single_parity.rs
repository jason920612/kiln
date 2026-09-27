//! Cooking and brewing lookups against vanilla's (`tools/InventoryVectors.java single`): every
//! item in each cooking type, every potion with every brewing reagent.

mod common;

use common::{same, show, stack};
use kiln_inventory::recipe::CookingKind;
use serde_json::Value as Json;

#[test]
fn cooking_and_brewing_match_vanilla() {
    let path = common::work().join("wp3-inventory/single.jsonl");
    let (Ok(text), Some(rules)) = (std::fs::read_to_string(&path), common::rules()) else {
        eprintln!("skipping: vectors or datapack not found");
        return;
    };
    let recipes = &rules.recipes;
    let (mut records, mut matched, mut failures) = (0, 0, Vec::new());
    for line in text.lines().filter(|l| !l.is_empty()) {
        let v: Json = serde_json::from_str(line).unwrap();
        records += 1;
        let (what, found) = if let Some(kind) = v.get("cook") {
            let kind = match kind.as_str().unwrap() {
                "smelting" => CookingKind::Smelting,
                "blasting" => CookingKind::Blasting,
                "smoking" => CookingKind::Smoking,
                _ => CookingKind::Campfire,
            };
            let input = stack(v["input"].as_str().unwrap());
            (format!("{kind:?} {}", show(&input)), recipes.find_cooking(kind, &input, None))
        } else {
            let [input, reagent] = [0, 1].map(|i| stack(v["brew"][i].as_str().unwrap()));
            (format!("brewing {} + {}", show(&input), show(&reagent)), recipes.find_brewing(&input, &reagent, None))
        };
        let expected = v["recipe"].as_str();
        let got = found.map(|i| recipes.id(i));
        if expected != got {
            failures.push(format!("{what}: vanilla {expected:?}, kiln {got:?}"));
            continue;
        }
        let Some(i) = found else { continue };
        matched += 1;
        let result = recipes.assemble_single(i);
        if !same(&result, &stack(v["result"].as_str().unwrap())) {
            failures.push(format!("{what}: result {}", show(&result)));
        }
        if let (Some(time), kiln_inventory::recipe::Recipe::Cooking(c)) = (v.get("time"), &recipes.get(i).recipe) {
            let xp = v["xp"].as_f64().unwrap() as f32;
            if time.as_i64() != Some(c.cooking_time as i64) || xp != c.experience {
                failures.push(format!("{what}: time {} xp {} (vanilla {time} {xp})", c.cooking_time, c.experience));
            }
        }
    }
    for f in failures.iter().take(20) {
        eprintln!("{f}");
    }
    eprintln!("{records} lookups, {matched} matched a recipe, {} failures", failures.len());
    assert!(failures.is_empty());
}
