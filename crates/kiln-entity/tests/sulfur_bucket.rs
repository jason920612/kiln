//! A sulfur cube and a bucket (wp68): an empty bucket takes an adult cube with what it holds, its name, health and age
//! (`Bucketable.bucketMobPickup`, `SulfurCube.saveToBucketTag`); emptying the bucket makes the cube again
//! (`MobBucketItem.spawn`: `finalizeSpawn`, the bucket's components, `loadFromBucketTag`, `FromBucket`).
//! The real client predicts the pickup by itself, so a server that does not take the cube leaves a ghost bucket in the
//! player's hand until the next inventory sync: found by playing it (the cube was never removed on the server).

use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::interact::{HeldChange, Interactor};
use kiln_entity::mob::{self, GroupData, MobKind, SpawnContext};
use kiln_javamath::random::LegacyRandom;
use kiln_proto::nbt::Tag;

fn ctx() -> SpawnContext {
    let biome = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains");
    SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome, moon_brightness: 1.0 }
}

fn who() -> Interactor {
    Interactor { id: 99, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO }
}

fn cube(age: i32) -> kiln_entity::Entity {
    let mut e = mob::new(MobKind::SulfurCube, 5, 0, 77);
    mob::finalize_spawn(&mut e, &mut LegacyRandom::new(3), &ctx(), &mut GroupData::default(), true);
    mob::persist::apply_nbt(&mut e, &Tag::Compound(vec![("Health".into(), Tag::Float(7.0)), ("Age".into(), Tag::Int(age))]));
    e
}

#[test]
fn an_empty_bucket_takes_a_cube_with_its_body_and_the_bucket_lets_it_out_again() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = cube(0);
    let stone = kiln_item::ItemStack::of("minecraft:stone", 1).unwrap();
    assert!(mob::interact::interact(&mut e, &mut level, &who(), &stone).success, "the cube swallows the stone");
    e.extra.push(("CustomName".into(), Tag::String("Cubey".into())));

    let bucket = kiln_item::ItemStack::of("minecraft:bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who(), &bucket);
    assert!(out.success);
    let HeldChange::Fill(filled) = out.held else { panic!("a filled bucket, got {:?}", out.held) };
    assert_eq!(filled.item_name(), "minecraft:sulfur_cube_bucket");
    assert!(e.is_removed(), "the cube is in the bucket now");
    let content = filled.get(kiln_item::keys::SULFUR_CUBE_CONTENT).expect("the body is in the bucket");
    assert_eq!(content.0.create().item_name(), "minecraft:stone");
    assert_eq!(filled.get(kiln_item::keys::CUSTOM_NAME).map(|n| n.nbt().clone()), Some(Tag::String("Cubey".into())));
    let Some(Tag::Compound(data)) = filled.get(kiln_item::keys::BUCKET_ENTITY_DATA).map(|d| d.0.clone()) else { panic!("bucket_entity_data") };
    let get = |k: &str| data.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("Health"), Some(Tag::Float(7.0)));
    assert_eq!(get("age"), Some(Tag::Int(0)));
    assert_eq!(get("age_locked"), Some(Tag::Byte(0)));

    // Emptied: a cube again, holding the stone, from a bucket, with its health and name.
    let mut back = mob::new(MobKind::SulfurCube, 6, 0, 78);
    mob::finalize_spawn(&mut back, &mut LegacyRandom::new(4), &ctx(), &mut GroupData::default(), false);
    mob::kinds::sulfur_cube::apply_bucket(&mut back, &filled);
    let saved = kiln_entity::persist::save(&back, &|_| None);
    assert_eq!(saved.get("from_bucket").and_then(Tag::as_i64), Some(1));
    assert_eq!(saved.get("Health").and_then(Tag::as_f64), Some(7.0));
    assert_eq!(saved.get("Size").and_then(Tag::as_i64), Some(2));
    assert_eq!(saved.get("CustomName"), Some(&Tag::String("Cubey".into())));
    let Some(Tag::Compound(eq)) = saved.get("equipment") else { panic!("the body: {saved:?}") };
    assert!(eq.iter().any(|(k, v)| k == "body" && v.get("id").and_then(Tag::as_str) == Some("minecraft:stone")));
}

#[test]
fn a_baby_cube_stays_out_of_the_bucket() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = cube(-24000);
    let bucket = kiln_item::ItemStack::of("minecraft:bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who(), &bucket);
    assert!(!out.success);
    assert!(!e.is_removed());
}
