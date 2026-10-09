//! Frogs and tadpoles: variants by biome, saving, breeding food, tadpole growth, feeding and
//! buckets. (Their brains are compared with vanilla by `mob_parity`, scenarios `*_frog_*` and
//! `*_tadpole_*`.)

use kiln_entity::entity::EntityKind;
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::interact::{HeldChange, Interactor};
use kiln_entity::mob::{self, GroupData, MobKind, SpawnContext};
use kiln_javamath::random::LegacyRandom;
use kiln_proto::nbt::Tag;

fn ctx(biome: &str) -> SpawnContext {
    let biome = kiln_data::synced_id("minecraft:worldgen/biome", biome);
    SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome, moon_brightness: 1.0 }
}

fn variant_name(e: &kiln_entity::Entity) -> &'static str {
    mob::kinds::wolf::synced_name("minecraft:frog_variant", mob::data(e).unwrap().variant).unwrap()
}

#[test]
fn frogs_take_the_variant_of_their_biome_and_wait_for_their_first_jump() {
    for (biome, want) in [("minecraft:desert", "minecraft:warm"), ("minecraft:snowy_plains", "minecraft:cold"), ("minecraft:swamp", "minecraft:temperate")] {
        let mut e = mob::new(MobKind::Frog, 1, 0, 5);
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(9), &ctx(biome), &mut GroupData::default(), true);
        assert_eq!(variant_name(&e), want, "{biome}");
        // `FrogAi.initMemories`: 100 to 140 ticks until the first long jump.
        let m = mob::data(&e).unwrap();
        let cooldown = m.brain.as_ref().unwrap().st.mem.int(mob::brain::Mem::LongJumpCooldownTicks).unwrap();
        assert!((100..=140).contains(&cooldown), "{cooldown}");
    }
}

#[test]
fn a_frog_saves_and_loads_its_variant_and_pregnancy() {
    let mut e = mob::new(MobKind::Frog, 7, 0, 3);
    mob::finalize_spawn(&mut e, &mut LegacyRandom::new(1), &ctx("minecraft:snowy_plains"), &mut GroupData::default(), true);
    mob::data_mut(&mut e).unwrap().brain.as_mut().unwrap().st.mem.set(mob::brain::Mem::IsPregnant, mob::brain::Val::Unit);
    let tag = kiln_entity::persist::save(&e, &|_| None);
    assert_eq!(tag.get("id").and_then(Tag::as_str), Some("minecraft:frog"));
    assert_eq!(tag.get("variant").and_then(Tag::as_str), Some("minecraft:cold"));
    let back = kiln_entity::persist::load(&tag, 7, 3).unwrap();
    assert_eq!(variant_name(&back), "minecraft:cold");
    assert!(mob::data(&back).unwrap().brain.as_ref().unwrap().st.mem.has(mob::brain::Mem::IsPregnant));
}

#[test]
fn slime_balls_put_frogs_in_love_and_are_their_tempting_food() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = mob::new(MobKind::Frog, 2, 0, 1);
    let who = Interactor { id: 99, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO };
    let ball = kiln_item::ItemStack::of("minecraft:slime_ball", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &ball);
    assert!(out.success && out.held == HeldChange::Consume(1));
    assert!(mob::data(&e).unwrap().in_love > 0);
    // Not a baby, whatever its age.
    mob::persist::apply_nbt(&mut e, &Tag::Compound(vec![("Age".into(), Tag::Int(-24000))]));
    assert_eq!(mob::data(&e).unwrap().age, -24000);
    assert!(!mob::data(&e).unwrap().baby());
}

fn tadpole_with_age(age: i32) -> kiln_entity::Entity {
    let mut e = mob::new(MobKind::Tadpole, 5, 0, 11);
    e.set_pos(kiln_entity::math::Vec3::new(0.5, 64.0, 0.5));
    mob::persist::apply_nbt(&mut e, &Tag::Compound(vec![("Age".into(), Tag::Int(age))]));
    e
}

#[test]
fn a_tadpole_grows_into_a_frog_after_24000_ticks() {
    let mut level = MemoryLevel::new(-64, 1);
    level.immediate_adds = true;
    level.insert(tadpole_with_age(23999));
    level.tick_one(0, |e, level| {
        e.common_tick();
        e.tick(level);
    });
    level.flush_spawned();
    let tadpole = level.entity_at(0).unwrap();
    assert!(tadpole.is_removed(), "the tadpole is gone");
    let frog = level.entity_at(1).expect("a frog took its place");
    assert_eq!(mob::data(frog).unwrap().kind, MobKind::Frog);
    assert!(mob::data(frog).unwrap().persistence_required);
    assert_eq!(frog.position().x, 0.5);
}

#[test]
fn slime_balls_speed_up_a_tadpole_and_a_dandelion_locks_it() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = tadpole_with_age(0);
    let who = Interactor { id: 99, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO };
    let ball = kiln_item::ItemStack::of("minecraft:slime_ball", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &ball);
    assert!(out.success);
    // `getSpeedUpSecondsWhenFeeding(24000)`: 120 seconds.
    let saved = kiln_entity::persist::save(&e, &|_| None);
    assert_eq!(saved.get("Age").and_then(Tag::as_i64), Some(2400));
    let dandelion = kiln_item::ItemStack::of("minecraft:golden_dandelion", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &dandelion);
    assert!(out.success);
    let saved = kiln_entity::persist::save(&e, &|_| None);
    assert_eq!(saved.get("AgeLocked").and_then(Tag::as_i64), Some(1));
    assert_eq!(saved.get("Age").and_then(Tag::as_i64), Some(0));
    // Locked: no growing, no feeding.
    let out = mob::interact::interact(&mut e, &mut level, &who, &ball);
    assert!(!out.success);
}

#[test]
fn a_water_bucket_takes_a_tadpole_with_its_age() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = tadpole_with_age(1234);
    let who = Interactor { id: 99, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO };
    let bucket = kiln_item::ItemStack::of("minecraft:water_bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &bucket);
    assert!(out.success);
    let HeldChange::Fill(filled) = out.held else { panic!("a filled bucket, got {:?}", out.held) };
    assert_eq!(filled.item_name(), "minecraft:tadpole_bucket");
    assert!(e.is_removed());
    // Emptied again: a tadpole with the age it had.
    let mut back = mob::new(MobKind::Tadpole, 8, 0, 4);
    mob::kinds::tadpole::apply_bucket(&mut back, &filled);
    let saved = kiln_entity::persist::save(&back, &|_| None);
    assert_eq!(saved.get("Age").and_then(Tag::as_i64), Some(1234));
    assert!(matches!(back.kind, EntityKind::Mob(_)));
}
