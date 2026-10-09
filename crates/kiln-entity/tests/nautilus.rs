//! Nautilus and zombie nautilus beyond what the parity harness records: taming, the saddle and
//! saving. (Their swimming, tempting, hunting and drying out are compared with vanilla by
//! `mob_parity`, `work/wp33/vectors.jsonl`.)

use kiln_entity::level::PlayerView;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::interact::{HeldChange, Interactor};
use kiln_entity::mob::kinds::nautilus;
use kiln_entity::mob::{self, MobKind};
use kiln_entity::Entity;
use kiln_item::ItemStack;

fn level() -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 42);
    for x in -12..=12 {
        for z in -12..=12 {
            level.blocks.insert(BlockPos::new(x, 99, z), kiln_data::blocks::default_state::STONE);
        }
    }
    level.players.push(PlayerView::new(1, Vec3::new(2.5, 100.0, 0.5)));
    level.immediate_adds = true;
    level
}

fn nautilus_at(kind: MobKind, seed: i64) -> Entity {
    let mut e = mob::new(kind, 10, 0, seed);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    e
}

fn who() -> Interactor {
    Interactor { id: 1, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO }
}

fn is_tame(e: &Entity) -> bool {
    nautilus::is_tame(mob::data(e).unwrap())
}

#[test]
fn a_pufferfish_tames_a_nautilus_one_time_in_three() {
    let mut lv = level();
    let puffer = ItemStack::of("minecraft:pufferfish", 1).unwrap();
    let mut tamed = 0;
    for seed in 0..300 {
        let mut e = nautilus_at(MobKind::Nautilus, seed);
        let out = mob::interact::interact(&mut e, &mut lv, &who(), &puffer);
        assert!(out.success);
        assert_eq!(out.held, HeldChange::Consume(1));
        if is_tame(&e) {
            tamed += 1;
        }
    }
    assert!((70..130).contains(&tamed), "{tamed} of 300");
}

#[test]
fn a_tame_nautilus_wears_a_saddle_and_keeps_its_owner_when_saved() {
    let mut lv = level();
    let puffer = ItemStack::of("minecraft:pufferfish", 1).unwrap();
    let mut e = (0..100)
        .map(|seed| {
            let mut e = nautilus_at(MobKind::Nautilus, seed);
            mob::interact::interact(&mut e, &mut lv, &who(), &puffer);
            e
        })
        .find(is_tame)
        .expect("some nautilus is tamed");
    // Nobody rides it without a saddle, and an empty hand on a wild one does nothing.
    let saddle = ItemStack::of("minecraft:saddle", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut lv, &who(), &saddle);
    assert!(out.success);
    assert_eq!(out.held, HeldChange::Consume(1));
    let tag = kiln_entity::persist::save(&e, &|_| None);
    let saddle_id = tag.get("equipment").and_then(|q| q.get("saddle")).and_then(|s| s.get("id")).and_then(|i| i.as_str());
    assert_eq!(saddle_id, Some("minecraft:saddle"));
    let back = kiln_entity::persist::load(&tag, 11, 0).unwrap();
    assert!(is_tame(&back));
    let owner_before = nautilus::tame_of(mob::data(&e).unwrap()).unwrap().owner;
    let owner_after = nautilus::tame_of(mob::data(&back).unwrap()).unwrap().owner;
    assert!(owner_before.is_some());
    assert_eq!(owner_before, owner_after);
    let again = kiln_entity::persist::save(&back, &|_| None);
    assert_eq!(again.get("equipment").and_then(|q| q.get("saddle")).is_some(), true);
    // An empty hand now sits the player on it.
    let mut back = back;
    let out = mob::interact::interact(&mut back, &mut lv, &who(), &ItemStack::empty());
    assert!(out.success);
    assert!(out.ride);
}

#[test]
fn a_wild_nautilus_cannot_be_ridden_or_saddled() {
    let mut lv = level();
    let mut e = nautilus_at(MobKind::Nautilus, 3);
    let saddle = ItemStack::of("minecraft:saddle", 1).unwrap();
    assert!(!mob::interact::interact(&mut e, &mut lv, &who(), &saddle).success);
    let out = mob::interact::interact(&mut e, &mut lv, &who(), &ItemStack::empty());
    assert!(!out.success && !out.ride);
}

#[test]
fn a_zombie_nautilus_keeps_its_variant_and_is_never_a_calf() {
    for seed in 0..50 {
        let mut e = nautilus_at(MobKind::ZombieNautilus, seed);
        let mut group = mob::GroupData::default();
        let ctx = mob::SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome: None, moon_brightness: 1.0 };
        mob::finalize_spawn(&mut e, &mut kiln_javamath::random::LegacyRandom::new(seed), &ctx, &mut group, true);
        assert!(!mob::data(&e).unwrap().baby());
        let tag = kiln_entity::persist::save(&e, &|_| None);
        assert!(tag.get("variant").is_some(), "{tag:?}");
        assert!(tag.get("Age").is_none() || tag.get("Age").and_then(|a| a.as_i64()) == Some(0));
        let back = kiln_entity::persist::load(&tag, 12, 0).unwrap();
        assert_eq!(mob::data(&back).unwrap().variant, mob::data(&e).unwrap().variant);
    }
}
