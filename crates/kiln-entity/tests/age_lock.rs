//! The golden dandelion (wp68): held to a baby it toggles the age lock of any ageable mob (`AgeableMob.mobInteract`).
//! The real client predicts the interaction by itself (it locks its own copy of the entity and uses the dandelion up),
//! so a server that ignores the click looks fine on screen: found by reading the server's `AgeLocked` back.

use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::interact::{HeldChange, Interactor};
use kiln_entity::mob::{self, MobKind};
use kiln_entity::level::Event;
use kiln_proto::nbt::Tag;

fn who() -> Interactor {
    Interactor { id: 99, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO }
}

fn baby(kind: MobKind) -> kiln_entity::Entity {
    let mut e = mob::new(kind, 4, 0, 11);
    mob::persist::apply_nbt(&mut e, &Tag::Compound(vec![("Age".into(), Tag::Int(-12000))]));
    e
}

fn saved(e: &kiln_entity::Entity) -> Tag {
    kiln_entity::persist::save(e, &|_| None)
}

#[test]
fn a_golden_dandelion_locks_a_baby_cow_and_the_next_one_unlocks_it() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = baby(MobKind::Cow);
    let dandelion = kiln_item::ItemStack::of("minecraft:golden_dandelion", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who(), &dandelion);
    assert!(out.success && out.held == HeldChange::Consume(1));
    let tag = saved(&e);
    assert_eq!(tag.get("AgeLocked").and_then(Tag::as_i64), Some(1));
    assert_eq!(tag.get("Age").and_then(Tag::as_i64), Some(-24000), "back to the start of babyhood");
    assert_eq!(tag.get("PersistenceRequired").and_then(Tag::as_i64), Some(1));
    assert!(level.events.iter().any(|ev| matches!(ev, Event::Sound { sound, .. } if sound.ends_with("golden_dandelion.use"))));

    // The particles run for 40 ticks and a new dandelion waits for them.
    let again = mob::interact::interact(&mut e, &mut level, &who(), &dandelion);
    assert!(!again.success, "not while the particles play");
    for _ in 0..41 {
        mob::tick(&mut e, &mut level);
    }
    let pauses = level.particles.iter().filter(|(p, _)| *p == "minecraft:pause_mob_growth").count();
    assert_eq!(pauses, 20, "one particle every second tick of 40");
    // Locked: it stays a baby however long it waits.
    assert_eq!(saved(&e).get("Age").and_then(Tag::as_i64), Some(-24000));

    let out = mob::interact::interact(&mut e, &mut level, &who(), &dandelion);
    assert!(out.success);
    assert_eq!(saved(&e).get("AgeLocked").and_then(Tag::as_i64), Some(0));
    assert!(level.events.iter().any(|ev| matches!(ev, Event::Sound { sound, .. } if sound.ends_with("golden_dandelion.unuse"))));
}

#[test]
fn adults_and_villagers_are_not_locked() {
    let mut level = MemoryLevel::new(-64, 1);
    let dandelion = kiln_item::ItemStack::of("minecraft:golden_dandelion", 1).unwrap();
    let mut adult = mob::new(MobKind::Cow, 5, 0, 12);
    assert!(!mob::interact::interact(&mut adult, &mut level, &who(), &dandelion).success);
    let mut villager = baby(MobKind::Villager);
    // (A baby villager shakes its head at any click; it is not locked, and keeps the dandelion.)
    let out = mob::interact::interact(&mut villager, &mut level, &who(), &dandelion);
    assert_ne!(out.held, HeldChange::Consume(1), "`#cannot_be_age_locked`");
    assert_eq!(saved(&villager).get("AgeLocked").and_then(Tag::as_i64).unwrap_or(0), 0);
}

#[test]
fn feeding_a_baby_shows_happy_villager_particles_for_40_ticks() {
    let mut level = MemoryLevel::new(-64, 1);
    let mut e = baby(MobKind::Cow);
    let wheat = kiln_item::ItemStack::of("minecraft:wheat", 1).unwrap();
    assert!(mob::interact::interact(&mut e, &mut level, &who(), &wheat).success);
    for _ in 0..41 {
        mob::tick(&mut e, &mut level);
    }
    let hearts = level.particles.iter().filter(|(p, _)| *p == "minecraft:happy_villager").count();
    assert_eq!(hearts, 10, "`forcedAgeTimer` 40, one particle every fourth tick");
}
