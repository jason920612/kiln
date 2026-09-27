//! `ClientboundUpdateRecipesPacket` against vanilla's (`tools/InventoryVectors.java sync`).
//! Property sets are sets in vanilla (hash order), so they are compared as sets.

mod common;

use kiln_proto::Reader;
use std::collections::BTreeMap;

type Sets = BTreeMap<String, Vec<i32>>;

/// Property sets (sorted) and the raw stonecutter section.
fn parse(body: &[u8]) -> (Sets, Vec<u8>) {
    let mut r = Reader::new(body);
    let mut sets = BTreeMap::new();
    for _ in 0..r.varint().unwrap() {
        let key = r.string(32767).unwrap().to_owned();
        let n = r.varint().unwrap();
        let mut ids: Vec<i32> = (0..n).map(|_| r.varint().unwrap()).collect();
        ids.sort_unstable();
        sets.insert(key, ids);
    }
    (sets, r.rest().to_vec())
}

#[test]
fn update_recipes_matches_vanilla() {
    let path = common::work().join("wp3-inventory/sync.json");
    let (Ok(text), Some(rules)) = (std::fs::read_to_string(&path), common::rules()) else {
        eprintln!("skipping: vectors or datapack not found");
        return;
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let vanilla = common::unhex(v["update_recipes"].as_str().unwrap());
    let ours = kiln_inventory::recipe::sync::update_recipes(&rules.recipes);
    let mut r = Reader::new(&ours);
    assert_eq!(r.varint().unwrap(), kiln_data::packets::play::clientbound::UPDATE_RECIPES);
    let (their_sets, their_stonecutter) = parse(&vanilla);
    let (our_sets, our_stonecutter) = parse(r.rest());
    assert_eq!(our_sets, their_sets);
    assert_eq!(our_stonecutter, their_stonecutter, "stonecutter entries differ");
}
