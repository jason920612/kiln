//! What a mob's weapon adds to a blow (`Mob.doHurtTarget`, `LivingEntity.stabAttack`): the damage
//! type of the item (`ItemStack.getDamageSource`), the weapon's enchantments (`modifyDamage`,
//! `getKnockback`'s `modifyKnockback`, `doPostAttackEffects`), which the installed
//! [`Enchanter`](crate::enchanting::Enchanter) evaluates against the datapack's definitions.

use super::attributes::Attr;
use super::goals::Living;
use super::{MAINHAND, MobData};
use crate::enchanting::{self, EntityView, Hit, MobPostAttack};
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel};
use crate::math::Vec3;

/// `ItemStack.getDamageSource(attacker, mobAttack)` of the main hand: the item's `damage_type`
/// (a spear's `spear`), else a mob attack.
pub fn damage_kind(m: &MobData) -> DamageKind {
    match m.equipment[MAINHAND].get(kiln_item::keys::DAMAGE_TYPE).and_then(|d| kiln_item::registry::DAMAGE_TYPE.name(d.0)) {
        Some(name) => DamageKind::of_type(name),
        None => DamageKind::MobAttack,
    }
}

fn type_id(name: &str) -> i32 {
    kiln_item::registry::ENTITY_TYPE.id(name).unwrap_or(-1)
}

/// The wielder as enchantment requirements see it.
pub fn attacker_view(e: &Entity) -> EntityView {
    let p = e.position();
    EntityView {
        type_id: type_id(e.type_name),
        pos: [p.x, p.y, p.z],
        on_ground: e.on_ground,
        on_fire: e.is_on_fire(),
        has_vehicle: e.vehicle.is_some(),
        in_water: e.is_in_water(),
        ..EntityView::default()
    }
}

/// The target as enchantment requirements see it.
pub fn victim_view(level: &dyn EntityLevel, t: &Living) -> EntityView {
    if !t.player
        && let Some(o) = level.entity(t.id)
    {
        return attacker_view(o);
    }
    let player = level.player(t.id);
    EntityView {
        type_id: type_id(t.type_name),
        pos: [t.pos.x, t.pos.y, t.pos.z],
        sneaking: t.sneaking,
        sprinting: player.as_ref().is_some_and(|p| p.sprinting),
        has_vehicle: player.as_ref().is_some_and(|p| p.vehicle.is_some()),
        in_water: player.as_ref().and_then(|p| p.in_water).unwrap_or(false),
        player: t.player.then(|| kiln_loot::view::PlayerFacts { game_mode: if t.creative { 1 } else if t.spectator { 3 } else { 0 }, ..Default::default() }),
        ..EntityView::default()
    }
}

fn damage_type_id(kind: DamageKind) -> i32 {
    kiln_data::synced_id("minecraft:damage_type", kind.type_name()).unwrap_or(0)
}

/// `EnchantmentHelper.modifyDamage(level, getWeaponItem(), target, source, damage)`: the weapon's
/// enchantments change the damage of a blow (sharpness, smite, density, impaling...).
pub fn modify_damage(level: &mut dyn EntityLevel, e: &Entity, m: &MobData, t: &Living, kind: DamageKind, damage: f32) -> f32 {
    let weapon = &m.equipment[MAINHAND];
    if weapon.is_empty() {
        return damage;
    }
    let (attacker, victim) = (attacker_view(e), victim_view(level, t));
    let hit = Hit { weapon, attacker: &attacker, victim: &victim, damage_type: damage_type_id(kind) };
    enchanting::modify_damage(&hit, damage, level.random())
}

/// `LivingEntity.getKnockback(target, source)`: the attack knockback attribute through the
/// weapon's `knockback` enchantments, halved.
pub fn attack_knockback(level: &mut dyn EntityLevel, e: &Entity, m: &MobData, t: &Living, kind: DamageKind) -> f32 {
    let base = m.attrs.value(Attr::AttackKnockback) as f32;
    let weapon = &m.equipment[MAINHAND];
    if weapon.is_empty() {
        return base / 2.0;
    }
    let (attacker, victim) = (attacker_view(e), victim_view(level, t));
    let hit = Hit { weapon, attacker: &attacker, victim: &victim, damage_type: damage_type_id(kind) };
    enchanting::modify_knockback(&hit, base, level.random()) / 2.0
}

/// `LivingEntity.causeExtraKnockback`: a living target is thrown from the wielder (`old` is its
/// motion before the blow), who slows down.
pub fn cause_extra_knockback(e: &mut Entity, level: &mut dyn EntityLevel, t: &Living, strength: f32, old: Vec3) {
    if strength <= 0.0 {
        return;
    }
    let rad = (e.y_rot * 0.017453292) as f64;
    level.knockback_target(t.id, strength as f64, super::mth::sin(rad) as f64, -(super::mth::cos(rad)) as f64, old);
    e.delta = Vec3::new(e.delta.x * 0.6, e.delta.y, e.delta.z * 0.6);
}

/// `EnchantmentHelper.doPostAttackEffects(level, target, source)` for a blow that landed: what
/// the weapon's enchantments do to the target (fire aspect sets it alight, bane of arthropods
/// slows it).
pub fn post_attack(level: &mut dyn EntityLevel, e: &Entity, m: &MobData, t: &Living, kind: DamageKind) {
    let weapon = &m.equipment[MAINHAND];
    if weapon.is_empty() {
        return;
    }
    let (attacker, victim) = (attacker_view(e), victim_view(level, t));
    let hit = Hit { weapon, attacker: &attacker, victim: &victim, damage_type: damage_type_id(kind) };
    for effect in enchanting::post_attack(&hit, level.random()) {
        match effect {
            MobPostAttack::Ignite { seconds } => level.ignite(t.id, seconds),
            MobPostAttack::MobEffect { effect, duration, amplifier } => {
                if let Some(name) = kiln_data::builtin_entries("minecraft:mob_effect").and_then(|l| l.iter().find(|n| **n == effect).copied()) {
                    level.add_effect(t.id, name, duration, amplifier, Some(e.id));
                }
            }
        }
    }
}
