//! What a mob's charging spear meets besides living things (`PiercingWeapon.canHitEntity`):
//! boats and minecarts are hit (a vehicle breaks past 40 damage, ten times a hit), a fireball is
//! hit and nothing happens to it (`Projectile.hurtServer`), an ender dragon is hit through its
//! parts, and the zombie that wields the spear is never its own target. The parity harness
//! cannot record these (mobs and vehicles push each other apart before the first stab), so they
//! are checked here against the code of vanilla.

use kiln_entity::entity::EntityKind;
use kiln_entity::level::{EntityLevel, PlayerView};
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::{self, MobKind};
use kiln_proto::nbt::Tag;

fn level() -> MemoryLevel {
    let mut level = MemoryLevel::new(-64, 42);
    level.difficulty = 2;
    for x in -30..=30 {
        for z in -30..=30 {
            level.blocks.insert(BlockPos::new(x, 99, z), kiln_data::blocks::default_state::STONE);
        }
    }
    level.players.push(PlayerView::new(1, Vec3::new(-20.5, 100.0, 0.5)));
    level.immediate_adds = true;
    level
}

/// A zombie at (0.5, 100, 0.5) looking along +z, a spear in hand, charging fast enough for every
/// condition of the weapon.
fn spear_zombie(level: &mut MemoryLevel) -> i32 {
    let mut e = mob::new(MobKind::Zombie, 10, 0, 7);
    e.set_pos(Vec3::new(0.5, 100.0, 0.5));
    e.last_known_speed = Vec3::new(0.0, 0.0, 0.35);
    let m = mob::data_mut(&mut e).unwrap();
    m.equipment[mob::MAINHAND] = kiln_item::ItemStack::of("minecraft:iron_spear", 1).unwrap();
    m.using_item = Some(30);
    level.insert(e);
    level.set_next_entity_id(11);
    10
}

fn other(level: &mut MemoryLevel, id: i32, kind: &str, pos: Vec3) {
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String(kind.into())),
        ("Pos".into(), Tag::List(vec![Tag::Double(pos.x), Tag::Double(pos.y), Tag::Double(pos.z)])),
        ("Motion".into(), Tag::List(vec![Tag::Double(0.0), Tag::Double(0.0), Tag::Double(0.0)])),
    ]);
    let e = kiln_entity::persist::load(&tag, id, 0).expect("entity loads");
    level.insert(e);
}

/// One tick of the zombie's use of the spear (`ItemStack.onUseTick`) at game time `now`.
fn stab(level: &mut MemoryLevel, id: i32, now: i64) {
    level.game_time = now;
    let i = level.entities().position(|e| e.id == id).unwrap();
    level.tick_one(i, |e, level| mob::kinetic_tick_alone(e, level));
}

fn gone(level: &MemoryLevel, id: i32) -> bool {
    level.entity(id).is_none_or(|e| e.is_removed())
}

#[test]
fn a_minecart_in_the_line_breaks_at_once() {
    // 3 base damage plus floor(6.98 * 0.95) = 9, times ten for a vehicle: past 40.
    let mut level = level();
    let z = spear_zombie(&mut level);
    other(&mut level, 20, "minecraft:minecart", Vec3::new(0.5, 101.4, 2.4));
    stab(&mut level, z, 1000);
    assert!(gone(&level, 20), "a stab at full speed breaks a minecart");
}

#[test]
fn a_boat_breaks_too() {
    let mut level = level();
    let z = spear_zombie(&mut level);
    other(&mut level, 20, "minecraft:oak_boat", Vec3::new(0.5, 101.5, 2.4));
    stab(&mut level, z, 1000);
    assert!(gone(&level, 20), "the boat took a stab");
}

#[test]
fn a_fireball_is_hit_and_left_alone() {
    let mut level = level();
    let z = spear_zombie(&mut level);
    other(&mut level, 20, "minecraft:fireball", Vec3::new(0.5, 101.6, 2.4));
    let before = level.entity(20).unwrap().delta;
    stab(&mut level, z, 1000);
    stab(&mut level, z, 1100);
    let stabs = &mob::data(level.entity(z).unwrap()).unwrap().recent_stabs;
    assert!(stabs.iter().any(|&(id, _)| id == 20), "the fireball was in the stab's way");
    let f = level.entity(20).unwrap();
    assert!(!f.is_removed());
    assert_eq!(f.delta, before, "a mob's stab does not turn a fireball around");
}

#[test]
fn a_villager_in_the_line_is_hurt() {
    let mut level = level();
    let z = spear_zombie(&mut level);
    let villager = mob::new(MobKind::Villager, 21, 0, 9);
    level.insert(villager);
    level.entity_mut(21).unwrap().set_pos(Vec3::new(0.5, 100.0, 2.6));
    stab(&mut level, z, 1000);
    let h = mob::data(level.entity(21).unwrap()).unwrap().health;
    assert!(h < 20.0, "the villager behind them was hit: {h}");
}

/// An ender dragon is not pickable itself; its parts are, and the stab lands on the dragon.
#[test]
fn an_ender_dragon_is_hit_through_its_parts() {
    let mut level = level();
    let z = spear_zombie(&mut level);
    let mut d = mob::new(MobKind::EnderDragon, 30, 0, 11);
    d.set_pos(Vec3::new(0.5, 96.0, 10.0));
    level.insert(d);
    // The body part in the zombie's way, the dragon's own box far from it.
    {
        let e = level.entity_mut(30).unwrap();
        let m = mob::data_mut(e).unwrap();
        let s = kiln_entity::mob::ext::state_mut::<kiln_entity::mob::kinds::ender_dragon::DragonState>(m).expect("dragon state");
        s.parts = [Vec3::new(0.5, 100.0, 10.0); 8];
        s.parts[2] = Vec3::new(0.5, 101.4, 2.5);
    }
    stab(&mut level, z, 1000);
    // (Only a player's blow, or an `always_hurts_ender_dragons` source, hurts the dragon: a
    // zombie's stab is made all the same.)
    let stabs = &mob::data(level.entity(z).unwrap()).unwrap().recent_stabs;
    assert!(stabs.iter().any(|&(id, _)| id == 30), "the dragon was stabbed through its part");
    let dragon = level.entity(30).unwrap();
    assert!(matches!(dragon.kind, EntityKind::Mob(_)));
}
