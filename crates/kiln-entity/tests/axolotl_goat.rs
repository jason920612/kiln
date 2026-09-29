//! Axolotls and goats beyond what the parity harness records: saved state, buckets, horns,
//! spawn initialization and breeding.

use kiln_entity::entity::EntityKind;
use kiln_entity::level::{EntityLevel, PlayerView};
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::brain::Mem;
use kiln_entity::mob::ext;
use kiln_entity::mob::interact::{HeldChange, Interactor};
use kiln_entity::mob::kinds::{axolotl, goat};
use kiln_entity::mob::{self, GroupData, MobKind, SpawnContext};
use kiln_entity::Entity;
use kiln_item::ItemStack;
use kiln_javamath::random::LegacyRandom;

fn floor_level() -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 42);
    for x in -24..=24 {
        for z in -24..=24 {
            level.blocks.insert(BlockPos::new(x, 99, z), kiln_data::blocks::default_state::STONE);
        }
    }
    level.players.push(PlayerView::new(1, Vec3::new(2.5, 100.0, 0.5)));
    level.immediate_adds = true;
    level
}

fn ctx() -> SpawnContext {
    SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome: None, moon_brightness: 1.0 }
}

fn goat_at(id: i32, seed: i64) -> Entity {
    let mut e = mob::new(MobKind::Goat, id, 0x1234_5678_9abc_def0_1122_3344_5566_7788, seed);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    e
}

/// Runs `f` with the mob data taken out of the entity (as the mob tick does).
fn with_mob<R>(e: &mut Entity, f: impl FnOnce(&mut Entity, &mut mob::MobData) -> R) -> R {
    let mut k = std::mem::replace(&mut e.kind, EntityKind::MobTicking { gravity: 0.08 });
    let r = match &mut k {
        EntityKind::Mob(m) => f(e, m),
        _ => panic!("not a mob"),
    };
    e.kind = k;
    r
}

#[test]
fn axolotl_saves_its_variant_and_bucket_flag() {
    let mut e = mob::new(MobKind::Axolotl, 5, 0, 3);
    with_mob(&mut e, |_, m| {
        let s = ext::state_mut::<axolotl::State>(m).unwrap();
        s.variant = 4;
        s.from_bucket = true;
    });
    let tag = kiln_entity::persist::save(&e, &|_| None);
    assert_eq!(tag.get("Variant").and_then(|t| t.as_i64()), Some(4));
    assert_eq!(tag.get("FromBucket").and_then(|t| t.as_i64()), Some(1));
    let back = kiln_entity::persist::load(&tag, 6, 9).unwrap();
    let m = mob::data(&back).unwrap();
    assert_eq!(axolotl::variant(m), 4);
    assert!(ext::state::<axolotl::State>(m).unwrap().from_bucket);
    // A variant out of range reads as the default (lucy).
    let mut bad = tag.clone();
    if let kiln_proto::nbt::Tag::Compound(fields) = &mut bad {
        for (k, v) in fields.iter_mut() {
            if k == "Variant" {
                *v = kiln_proto::nbt::Tag::Int(99);
            }
        }
    }
    assert_eq!(axolotl::variant(mob::data(&kiln_entity::persist::load(&bad, 7, 9).unwrap()).unwrap()), 0);
}

#[test]
fn a_water_bucket_takes_an_axolotl_and_lets_it_out_again() {
    let mut level = floor_level();
    let mut e = mob::new(MobKind::Axolotl, 10, 0, 3);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    with_mob(&mut e, |_, m| {
        ext::state_mut::<axolotl::State>(m).unwrap().variant = 3;
        m.health = 9.0;
    });
    let who = Interactor { id: 1, creative: false, sneaking: false };
    let bucket = ItemStack::of("minecraft:water_bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &bucket);
    assert!(out.success);
    let HeldChange::Fill(filled) = out.held else { panic!("a filled bucket, got {:?}", out.held) };
    assert_eq!(mob::item_name(&filled), "minecraft:axolotl_bucket");
    assert!(e.is_removed(), "the axolotl is in the bucket");
    // The variant is the bucket's component, the state its entity data.
    assert!(filled.get(kiln_item::keys::AXOLOTL_VARIANT).is_some());
    assert!(filled.get(kiln_item::keys::BUCKET_ENTITY_DATA).is_some());

    let mut out_again = mob::new(MobKind::Axolotl, 11, 0, 4);
    axolotl::apply_bucket(&mut out_again, &filled);
    let m = mob::data(&out_again).unwrap();
    assert_eq!(axolotl::variant(m), 3);
    assert!(ext::state::<axolotl::State>(m).unwrap().from_bucket);
    assert_eq!(m.health, 9.0);
    // A bucketed axolotl is not removed for being far from players.
    assert_eq!(m.kind.ext().unwrap().remove_when_far_away(m), Some(false));
}

#[test]
fn a_tropical_fish_bucket_feeds_an_axolotl_and_comes_back_as_water() {
    let mut level = floor_level();
    let mut e = mob::new(MobKind::Axolotl, 10, 0, 3);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    let who = Interactor { id: 1, creative: false, sneaking: false };
    let bucket = ItemStack::of("minecraft:tropical_fish_bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &bucket);
    assert!(out.success);
    assert_eq!(out.held, HeldChange::Fill(ItemStack::of("minecraft:water_bucket", 1).unwrap()));
    assert!(mob::data(&e).unwrap().in_love > 0);
}

#[test]
fn axolotls_play_dead_and_cannot_be_attacked_then() {
    let level = floor_level();
    let mut a = mob::new(MobKind::Axolotl, 10, 0, 3);
    a.set_pos(Vec3::new(0.5, 100.0, 0.5));
    let zombie = mob::new(MobKind::Zombie, 11, 0, 3);
    let target = |a: &Entity| mob::goals::living(&level, a.id);
    let mut lv = level;
    lv.insert(a);
    let z = zombie;
    let zm = mob::data(&z).unwrap();
    let alive = mob::goals::living(&lv, 10).unwrap();
    assert!(mob::goals::can_attack(zm, &lv, &alive));
    with_mob(lv.entity_mut(10).unwrap(), |_, m| ext::state_mut::<axolotl::State>(m).unwrap().playing_dead = true);
    let dead = mob::goals::living(&lv, 10).unwrap();
    assert!(!mob::goals::can_attack(zm, &lv, &dead), "a playing dead axolotl is not seen as an enemy");
    let _ = target;
}

#[test]
fn axolotl_groups_get_common_variants_and_babies_after_two() {
    let mut babies = 0;
    for seed in 0..200 {
        let mut group = GroupData::default();
        for i in 0..4 {
            let mut e = mob::new(MobKind::Axolotl, 20 + i, 0, seed * 10 + i as i64);
            let mut r = LegacyRandom::new(seed + 1000);
            if i > 0 {
                r = LegacyRandom::new(seed * 7 + i as i64);
            }
            mob::finalize_spawn(&mut e, &mut r, &ctx(), &mut group, true);
            let m = mob::data(&e).unwrap();
            assert!(axolotl::variant(m) < 4, "wild axolotls are never blue");
            if m.baby() {
                assert!(i >= 2, "only the third member of a group on is a baby");
                babies += 1;
            } else {
                assert!(i < 2 || m.age == 0);
            }
        }
    }
    assert_eq!(babies, 400, "every third and fourth member is a baby");
}

#[test]
fn bred_axolotls_take_a_parents_variant() {
    let mut level = floor_level();
    for (i, v) in [(30, 1), (31, 2)] {
        let mut e = mob::new(MobKind::Axolotl, i, 0, 5 + i as i64);
        e.set_pos(Vec3::new(0.5, 100.0, 0.5));
        with_mob(&mut e, |_, m| {
            ext::state_mut::<axolotl::State>(m).unwrap().variant = v;
            m.in_love = 100;
        });
        level.insert(e);
    }
    level.set_next_entity_id(40);
    let mut seen = [0; 5];
    for _ in 0..40 {
        let mut e0 = std::mem::replace(level.entity_mut(30).unwrap(), mob::new(MobKind::Pig, -1, 0, 0));
        with_mob(&mut e0, |e, m| mob::breed::spawn_child(e, m, &mut level, 31));
        *level.entity_mut(30).unwrap() = e0;
        level.flush_spawned();
    }
    for i in 0..level.len() {
        let e = level.entity_at(i).unwrap();
        if e.id >= 40
            && let Some(m) = mob::data(e)
            && m.kind == MobKind::Axolotl
        {
            assert!(m.baby());
            seen[axolotl::variant(m) as usize] += 1;
        }
    }
    assert_eq!(seen[0] + seen[3] + seen[4], 0, "babies take a parent's color (a blue one 1 time in 1200)");
    assert!(seen[1] > 5 && seen[2] > 5, "both parents' colors turn up: {seen:?}");
}

#[test]
fn goats_start_with_cooldowns_and_scream_two_percent_of_the_time() {
    let mut screamers = 0;
    let mut one_horn = 0;
    for seed in 0..5000 {
        let mut e = goat_at(1, seed);
        let mut r = LegacyRandom::new(seed * 31 + 7);
        mob::finalize_spawn(&mut e, &mut r, &ctx(), &mut GroupData::default(), true);
        let m = mob::data(&e).unwrap();
        let mem = &m.brain.as_ref().unwrap().st.mem;
        let jump = mem.int(Mem::LongJumpCooldownTicks).expect("a long jump cooldown");
        let ram = mem.int(Mem::RamCooldownTicks).expect("a ram cooldown");
        assert!((600..=1200).contains(&jump), "{jump}");
        assert!((600..=6000).contains(&ram), "{ram}");
        let s = ext::state::<goat::State>(m).unwrap();
        screamers += s.screaming as i32;
        one_horn += (!s.left_horn || !s.right_horn) as i32;
    }
    assert!((60..=140).contains(&screamers), "{screamers} of 5000 scream");
    assert!((350..=650).contains(&one_horn), "{one_horn} of 5000 have one horn (10%)");
}

#[test]
fn goats_drop_their_horns_one_at_a_time() {
    let mut level = floor_level();
    let mut e = goat_at(2, 5);
    for expected_left in [false, true] {
        let dropped = with_mob(&mut e, |e, m| goat::drop_horn(e, m, &mut level));
        assert!(dropped);
        let s = with_mob(&mut e, |_, m| ext::state::<goat::State>(m).unwrap().clone());
        assert_eq!(s.left_horn && s.right_horn, false);
        let _ = expected_left;
    }
    // Both are gone now.
    let s = with_mob(&mut e, |_, m| ext::state::<goat::State>(m).unwrap().clone());
    assert!(!s.left_horn && !s.right_horn);
    assert!(!with_mob(&mut e, |e, m| goat::drop_horn(e, m, &mut level)), "nothing left to drop");
    level.flush_spawned();
    let horns: Vec<_> = (0..level.len())
        .filter_map(|i| level.entity_at(i))
        .filter_map(|e| match &e.kind {
            EntityKind::Item(d) => Some(d.stack.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(horns.len(), 2);
    for h in horns {
        assert_eq!(mob::item_name(&h), "minecraft:goat_horn");
        assert!(h.get(kiln_item::keys::INSTRUMENT).is_some(), "the horn has an instrument");
    }
    // A baby has no horns to lose.
    let mut baby = goat_at(3, 6);
    with_mob(&mut baby, |e, m| mob::set_age(e, m, -24000));
    assert!(!with_mob(&mut baby, |e, m| goat::drop_horn(e, m, &mut level)));
}

#[test]
fn goat_state_is_saved_and_babies_hit_softer() {
    let mut e = goat_at(4, 8);
    with_mob(&mut e, |_, m| {
        let s = ext::state_mut::<goat::State>(m).unwrap();
        s.screaming = true;
        s.left_horn = false;
    });
    let tag = kiln_entity::persist::save(&e, &|_| None);
    assert_eq!(tag.get("IsScreamingGoat").and_then(|t| t.as_i64()), Some(1));
    assert_eq!(tag.get("HasLeftHorn").and_then(|t| t.as_i64()), Some(0));
    assert_eq!(tag.get("HasRightHorn").and_then(|t| t.as_i64()), Some(1));
    let back = kiln_entity::persist::load(&tag, 9, 1).unwrap();
    let m = mob::data(&back).unwrap();
    let s = ext::state::<goat::State>(m).unwrap();
    assert!(s.screaming && !s.left_horn && s.right_horn);
    assert!(m.brain.is_some());
    // The attack damage follows the age: 2 for adults, 1 for babies.
    let damage = |m: &mob::MobData| m.attrs.value(mob::attributes::Attr::AttackDamage);
    assert_eq!(damage(m), 2.0);
    with_mob(&mut e, |e, m| mob::set_age(e, m, -24000));
    assert_eq!(with_mob(&mut e, |_, m| damage(m)), 1.0);
    with_mob(&mut e, |e, m| mob::set_age(e, m, 0));
    assert_eq!(with_mob(&mut e, |_, m| damage(m)), 2.0);
}

#[test]
fn goats_are_milked_with_a_bucket_unless_babies() {
    let mut level = floor_level();
    let mut e = goat_at(6, 8);
    let who = Interactor { id: 1, creative: false, sneaking: false };
    let bucket = ItemStack::of("minecraft:bucket", 1).unwrap();
    let out = mob::interact::interact(&mut e, &mut level, &who, &bucket);
    assert!(out.success);
    assert_eq!(out.held, HeldChange::Fill(ItemStack::of("minecraft:milk_bucket", 1).unwrap()));
    assert_eq!(out.player_sound, Some("minecraft:entity.goat.milk"));
    with_mob(&mut e, |_, m| ext::state_mut::<goat::State>(m).unwrap().screaming = true);
    let out = mob::interact::interact(&mut e, &mut level, &who, &bucket);
    assert_eq!(out.player_sound, Some("minecraft:entity.goat.screaming.milk"));
    with_mob(&mut e, |e, m| mob::set_age(e, m, -24000));
    assert!(!mob::interact::interact(&mut e, &mut level, &who, &bucket).success, "no milk from a baby");
}

#[test]
fn goats_shrug_off_ten_points_of_fall_damage() {
    let goat = MobKind::Goat.ext().unwrap();
    assert_eq!(goat.fall_damage_reduction(), 10);
    let axolotl = MobKind::Axolotl.ext().unwrap();
    assert_eq!(axolotl.fall_damage_reduction(), 0);
}
