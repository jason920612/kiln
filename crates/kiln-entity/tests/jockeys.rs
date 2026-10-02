//! Natural jockeys made by `finalizeSpawn`: spider jockeys, chicken jockeys and the strider's
//! riders (the draws and the riders are compared with vanilla by `finalize_parity`, from
//! `tools/mob_vectors.py --filter finalize`).

use kiln_entity::mob::{self, GroupData, MobKind, Seat, SpawnContext};
use kiln_javamath::random::LegacyRandom;

fn ctx() -> SpawnContext {
    SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome: None, moon_brightness: 1.0 }
}

fn finalize(kind: MobKind, seed: i64, natural: bool) -> (kiln_entity::Entity, GroupData) {
    let mut e = mob::new(kind, 1, 0, seed);
    let mut group = GroupData::default();
    mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, natural);
    (e, group)
}

#[test]
fn one_spider_in_a_hundred_carries_a_skeleton() {
    let mut jockeys = 0;
    for seed in 0..20000 {
        let (_, g) = finalize(MobKind::Spider, seed, true);
        if let Some(c) = g.companions.first() {
            jockeys += 1;
            assert_eq!(c.entity.type_name, "minecraft:skeleton");
            assert_eq!(c.seat, Seat::OnMob);
            assert_eq!(g.companions.len(), 1);
            // Armed by its own finalizeSpawn.
            assert!(!mob::data(&c.entity).unwrap().equipment[mob::MAINHAND].is_empty());
        }
    }
    assert!((150..250).contains(&jockeys), "{jockeys} of 20000");
}

#[test]
fn baby_zombies_ride_chickens() {
    let (mut babies, mut new_chickens, mut asking) = (0, 0, 0);
    for seed in 0..60000 {
        let (e, g) = finalize(MobKind::Zombie, seed, false);
        let baby = mob::data(&e).unwrap().baby();
        if baby {
            babies += 1;
        }
        if let Some(c) = g.companions.first() {
            assert!(baby, "only babies ride");
            assert_eq!(c.entity.type_name, "minecraft:chicken");
            assert_eq!(c.seat, Seat::UnderMob);
            assert!(mob::data(&c.entity).unwrap().chicken_jockey);
            new_chickens += 1;
        }
        if g.nearby_chicken {
            assert!(baby);
            asking += 1;
        }
    }
    // 5% are babies; of those 5% look for a chicken, 0.95 * 5% bring one.
    assert!((2500..3500).contains(&babies), "{babies}");
    assert!((100..220).contains(&asking), "{asking}");
    assert!((100..220).contains(&new_chickens), "{new_chickens}");
}

#[test]
fn striders_carry_zombified_piglins_and_baby_striders() {
    let (mut piglins, mut babies) = (0, 0);
    for seed in 0..30000 {
        let (e, g) = finalize(MobKind::Strider, seed, true);
        let Some(c) = g.companions.first() else { continue };
        assert_eq!(c.seat, Seat::OnMob);
        match c.entity.type_name {
            "minecraft:zombified_piglin" => {
                piglins += 1;
                let m = mob::data(&c.entity).unwrap();
                assert_eq!(kiln_entity::mob::item_name(&m.equipment[mob::MAINHAND]), "minecraft:warped_fungus_on_a_stick");
                let tag = kiln_entity::persist::save(&e, &|_| None);
                let saddle = tag.get("equipment").and_then(|q| q.get("saddle")).and_then(|s| s.get("id")).and_then(|i| i.as_str());
                assert_eq!(saddle, Some("minecraft:saddle"), "the strider is saddled");
            }
            "minecraft:strider" => {
                babies += 1;
                assert!(mob::data(&c.entity).unwrap().baby());
            }
            other => panic!("{other}"),
        }
    }
    // 1 in 30 piglins, then 1 in 10 of the rest babies.
    assert!((850..1150).contains(&piglins), "{piglins}");
    assert!((2500..3000).contains(&babies), "{babies}");
    // Without monsters spawning there are no piglins.
    let mut e = mob::new(MobKind::Strider, 1, 0, 3);
    let mut g = GroupData { monsters_disabled: true, ..Default::default() };
    for seed in 0..3000 {
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut g, true);
    }
    assert!(g.companions.iter().all(|c| c.entity.type_name == "minecraft:strider"));
}

#[test]
fn natural_husks_carry_a_camel_husk_and_a_parched() {
    let mut camels = 0;
    for seed in 0..20000 {
        let mut e = mob::new(MobKind::Husk, 1, 0, seed);
        let mut group = GroupData { camel_space: true, ..Default::default() };
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, true);
        // (A baby husk may bring a chicken as well.)
        let Some(at) = group.companions.iter().position(|c| c.entity.type_name == "minecraft:camel_husk") else { continue };
        camels += 1;
        assert_eq!(group.companions[at].seat, Seat::UnderMob);
        assert_eq!(group.companions[at + 1].entity.type_name, "minecraft:parched");
        assert_eq!(group.companions[at + 1].seat, Seat::OnCompanion(at));
        assert_eq!(kiln_entity::mob::item_name(&mob::data(&e).unwrap().equipment[mob::MAINHAND]), "minecraft:iron_spear");
    }
    // One in ten.
    assert!((1700..2300).contains(&camels), "{camels}");
    // Spawn egg husks and husks without room for the camel never bring one.
    for seed in 0..2000 {
        let mut e = mob::new(MobKind::Husk, 1, 0, seed);
        let mut group = GroupData { camel_space: true, ..Default::default() };
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, false);
        assert!(group.companions.iter().all(|c| c.entity.type_name != "minecraft:camel_husk"));
        let mut e = mob::new(MobKind::Husk, 1, 0, seed);
        let mut group = GroupData::default();
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, true);
        assert!(group.companions.iter().all(|c| c.entity.type_name != "minecraft:camel_husk"));
    }
}

#[test]
fn natural_zombie_horses_carry_a_zombie_with_an_iron_spear() {
    for seed in 0..200 {
        let mut e = mob::new(MobKind::ZombieHorse, 1, 0, seed);
        let mut group = GroupData::default();
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, true);
        let zombie = &group.companions[0];
        assert_eq!(zombie.entity.type_name, "minecraft:zombie");
        assert_eq!(zombie.seat, Seat::OnMob);
        assert_eq!(kiln_entity::mob::item_name(&mob::data(&zombie.entity).unwrap().equipment[mob::MAINHAND]), "minecraft:iron_spear");
        // Its jump strength is drawn (never a baby: no Age saved).
        let m = mob::data(&e).unwrap();
        let jump = m.attrs.base(kiln_entity::mob::attributes::Attr::JumpStrength);
        assert!((0.5..=0.7).contains(&jump), "{jump}");
        assert!(!m.baby());
        // Eggs and /summon bring no zombie.
        let mut e = mob::new(MobKind::ZombieHorse, 1, 0, seed);
        let mut group = GroupData::default();
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, false);
        assert!(group.companions.is_empty());
    }
}

#[test]
fn natural_drowned_with_a_trident_ride_zombie_nautilus_half_the_time() {
    let (mut tridents, mut riders) = (0, 0);
    for seed in 0..40000 {
        let mut e = mob::new(MobKind::Drowned, 1, 0, seed);
        let mut group = GroupData::default();
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, true);
        let m = mob::data(&e).unwrap();
        let trident = kiln_entity::mob::item_name(&m.equipment[mob::MAINHAND]) == "minecraft:trident";
        if trident && !m.baby() {
            tridents += 1;
        }
        let nautilus = group.companions.iter().find(|c| c.entity.type_name == "minecraft:zombie_nautilus");
        if let Some(c) = nautilus {
            assert!(trident && !m.baby());
            assert_eq!(c.seat, Seat::UnderMob);
            riders += 1;
        }
        // Spawn eggs bring none.
        let mut e = mob::new(MobKind::Drowned, 1, 0, seed);
        let mut group = GroupData::default();
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(), &mut group, false);
        assert!(group.companions.iter().all(|c| c.entity.type_name != "minecraft:zombie_nautilus"));
    }
    assert!(tridents > 500, "{tridents}");
    let ratio = riders as f64 / tridents as f64;
    assert!((0.4..0.6).contains(&ratio), "{riders} of {tridents}");
}
