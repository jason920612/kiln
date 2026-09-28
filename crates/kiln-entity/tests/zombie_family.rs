//! The zombie family outside what the parity harness can record (it runs on normal
//! difficulty): reinforcements on hard difficulty, and saved conversion state.

use kiln_entity::level::{DamageKind, EntityLevel, PlayerView};
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::attributes::Attr;
use kiln_entity::mob::{self, DamageSource, MobKind};

fn floor_level(difficulty: u8) -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 42);
    level.difficulty = difficulty;
    level.sky_darken = 11;
    for x in -48..=48 {
        for z in -48..=48 {
            level.blocks.insert(BlockPos::new(x, 99, z), kiln_data::blocks::default_state::STONE);
        }
    }
    level.players.push(PlayerView::new(1, Vec3::new(2.5, 100.0, 0.5)));
    level.immediate_adds = true;
    level
}

fn hit_zombie(level: &mut MemoryLevel, id: i32) {
    let source = DamageSource { kind: DamageKind::PlayerAttack, attacker: Some(1), direct: Some(1), pos: Some(Vec3::new(2.5, 100.0, 0.5)), attacker_is_player: true };
    let e = level.entity_mut(id).unwrap();
    let mut z = std::mem::replace(e, mob::new(MobKind::Pig, -1, 0, 0));
    mob::hurt_entity(&mut z, level, source, 1.0);
    *level.entity_mut(id).unwrap() = z;
}

fn spawn_zombie(level: &mut MemoryLevel, kind: MobKind, chance: f64) -> i32 {
    let mut e = mob::new(kind, 10, 0, 7);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    let m = mob::data_mut(&mut e).unwrap();
    m.attrs.get_mut(Attr::SpawnReinforcements).unwrap().base = chance;
    level.insert(e);
    level.set_next_entity_id(11);
    10
}

#[test]
fn zombies_call_reinforcements_on_hard_difficulty() {
    for kind in [MobKind::Zombie, MobKind::Husk] {
        let mut level = floor_level(3);
        let id = spawn_zombie(&mut level, kind, 1.0);
        let mut called = 0;
        for _ in 0..20 {
            let before = level.len();
            hit_zombie(&mut level, id);
            level.flush_spawned();
            called += level.len() - before;
            if let Some(m) = level.entity_mut(id).and_then(mob::data_mut) {
                m.damage_cooldown = 0;
                m.health = 20.0;
            }
        }
        assert!(called > 0, "{kind:?} called no reinforcements in 20 hits");
        let caller = mob::data(level.entity(id).unwrap()).unwrap();
        let charge = caller.attrs.get(Attr::SpawnReinforcements).unwrap().modifiers.iter().find(|m| m.id == "minecraft:reinforcement_caller_charge").unwrap().amount;
        assert!((charge + 0.05 * called as f64).abs() < 1e-9, "each call costs the caller 0.05 ({charge})");
        // The reinforcements are of the caller's type, go for the attacker and pay the callee charge.
        for i in 0..level.len() {
            let e = level.entity_at(i).unwrap();
            if e.id == id || e.id == 1 {
                continue;
            }
            let m = mob::data(e).unwrap();
            assert_eq!(m.kind, kind);
            assert_eq!(m.target, Some(1));
            assert!(m.attrs.get(Attr::SpawnReinforcements).unwrap().modifiers.iter().any(|m| m.id == "minecraft:reinforcement_callee_charge"));
            let d = e.position().distance_to_sqr(Vec3::new(0.5, 100.0, 0.5)).sqrt();
            assert!(d >= 7.0, "at least 7 blocks away ({d})");
        }
    }
}

#[test]
fn no_reinforcements_below_hard_difficulty() {
    let mut level = floor_level(2);
    let id = spawn_zombie(&mut level, MobKind::Zombie, 1.0);
    let before = level.len();
    for _ in 0..10 {
        hit_zombie(&mut level, id);
        if let Some(m) = level.entity_mut(id).and_then(mob::data_mut) {
            m.damage_cooldown = 0;
        }
    }
    level.flush_spawned();
    assert_eq!(level.len(), before);
}

#[test]
fn conversion_state_saves_and_loads() {
    use kiln_proto::nbt::Tag;
    let mut e = mob::new(MobKind::Zombie, 5, 0, 1);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    mob::persist::apply_nbt(&mut e, &Tag::Compound(vec![("DrownedConversionTime".into(), Tag::Int(123)), ("IsBaby".into(), Tag::Byte(1))]));
    let saved = kiln_entity::persist::save(&e, &|_| None);
    let get = |k: &str| saved.get(k).cloned();
    assert_eq!(get("DrownedConversionTime"), Some(Tag::Int(123)));
    assert_eq!(get("IsBaby"), Some(Tag::Byte(1)));
    assert!(mob::data(&e).unwrap().baby());
    // Zombified piglins keep their anger.
    let mut p = mob::new(MobKind::ZombifiedPiglin, 6, 0, 1);
    mob::persist::apply_nbt(&mut p, &Tag::Compound(vec![("anger_end_time".into(), Tag::Long(500))]));
    let saved = kiln_entity::persist::save(&p, &|_| None);
    assert_eq!(saved.get("anger_end_time").cloned(), Some(Tag::Long(500)));
}
