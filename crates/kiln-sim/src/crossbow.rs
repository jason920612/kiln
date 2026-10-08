//! Crossbows (`CrossbowItem`): held to load (the charge time with quick charge, the loading
//! sounds at a fifth and a half, the arrows drawn into `charged_projectiles` at full charge,
//! multishot's copies), then shot with the next use: 3.15 speed, every arrow critical, spread
//! around the up vector, `shot_crossbow` fires.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::consume::Using;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use kiln_entity::math::Vec3;
use kiln_inventory::Container;
use kiln_item::ItemStack;
use kiln_item::component::ChargedProjectiles;
use kiln_item::stack::ItemStackTemplate;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::world_fx::SoundSource;

/// `CrossbowItem.isCharged`.
pub(crate) fn is_charged(stack: &ItemStack) -> bool {
    stack.get(kiln_item::keys::CHARGED_PROJECTILES).is_some_and(|c| !c.0.is_empty())
}

/// `CrossbowItem.use`: a loaded crossbow shoots; an empty one starts loading when there is
/// something to load.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let stack = p.in_hand(off_hand).clone();
    if is_charged(&stack) {
        perform_shooting(p, level, off_hand, &stack, spawns);
        return;
    }
    if p.infinite_materials() || crate::ranged::projectile_slot(p, &stack).is_some() {
        p.start_using(off_hand, &stack, 72000);
    }
}

/// The highest level of the crossbow's enchantments with an unconditional value effect named
/// `pick` (`EnchantmentHelper.pickHighestLevel`), and that effect's result on `value`.
fn charge_time(p: &Player, stack: &ItemStack) -> f32 {
    let Some(loot) = p.loot.clone() else { return 1.25 };
    let mut best: Option<(i32, &kiln_loot::effects::ValueEffect)> = None;
    loot.for_each_enchantment(stack, |e, lvl| {
        if let Some(effect) = &e.effects.crossbow_charge_time
            && best.is_none_or(|(l, _)| lvl > l)
        {
            best = Some((lvl, effect));
        }
    });
    match best {
        Some((lvl, effect)) => effect.process(lvl, &mut LegacyRandom::new(0), 1.25).max(0.0),
        None => 1.25,
    }
}

/// `CrossbowItem.getChargeDuration`.
pub(crate) fn charge_duration(p: &Player, stack: &ItemStack) -> i32 {
    (charge_time(p, stack) * 20.0).floor() as i32
}

/// The highest quick charge level (its loading start sound).
fn quick_charge_level(p: &Player, stack: &ItemStack) -> i32 {
    let Some(loot) = p.loot.clone() else { return 0 };
    let mut best = 0;
    loot.for_each_enchantment(stack, |e, lvl| {
        if e.effects.other.iter().any(|id| id.as_str() == "minecraft:crossbow_charging_sounds") {
            best = best.max(lvl);
        }
    });
    best
}

/// `CrossbowItem.onUseTick`: the loading sounds, and the projectiles drawn once fully charged.
pub(crate) fn on_use_tick(p: &mut Player, u: &mut Using) {
    let stack = p.in_hand(u.off_hand).clone();
    let f = (u.duration - u.remaining) as f32 / charge_duration(p, &stack) as f32;
    if f < 0.2 {
        u.sounds = 0;
    }
    if f >= 0.2 && u.sounds & 1 == 0 {
        u.sounds |= 1;
        let start = match quick_charge_level(p, &stack) {
            0 => "minecraft:item.crossbow.loading_start".to_owned(),
            n => format!("minecraft:item.crossbow.quick_charge_{}", n.min(3)),
        };
        p.sound_for_all(&start, SoundSource::Players, 0.5, 1.0);
    }
    if f >= 0.5 && u.sounds & 2 == 0 {
        u.sounds |= 2;
        p.sound_for_all("minecraft:item.crossbow.loading_middle", SoundSource::Players, 0.5, 1.0);
    }
    if f >= 1.0 && !is_charged(&stack) && try_load(p, u.off_hand, &stack) {
        let pitch = 1.0 / (p.entity_rng.next_float() * 0.5 + 1.0) + 0.2;
        p.sound_for_all("minecraft:item.crossbow.loading_end", SoundSource::Players, 1.0, pitch);
    }
}

/// `CrossbowItem.tryLoadProjectiles`: the drawn projectiles go into `charged_projectiles`.
fn try_load(p: &mut Player, off_hand: bool, crossbow: &ItemStack) -> bool {
    let slot = crate::ranged::projectile_slot(p, crossbow);
    if slot.is_none() && !p.infinite_materials() {
        return false;
    }
    let drawn = crate::ranged::draw(p, crossbow, slot);
    if drawn.is_empty() {
        return false;
    }
    let templates: Vec<ItemStackTemplate> = drawn.iter().map(ItemStackTemplate::from_stack).collect();
    let i = p.hand_index(off_hand);
    let held = p.inv.item_mut(i);
    if held.item() == crossbow.item() {
        held.insert(kiln_item::keys::CHARGED_PROJECTILES, ChargedProjectiles(templates));
    }
    p.inv.times_changed += 1;
    true
}

/// `CrossbowItem.releaseUsing` after `ticks`: nothing but the use ending (loading happened in
/// [`on_use_tick`]).
pub(crate) fn release(_p: &mut Player, _level: &mut RegionLevel, _off_hand: bool, _stack: &ItemStack, _ticks: i32) {}

/// `CrossbowItem.performShooting`: the charged projectiles fly, the crossbow is empty again.
fn perform_shooting(p: &mut Player, level: &mut RegionLevel, off_hand: bool, crossbow: &ItemStack, spawns: &mut Vec<Spawn>) {
    let Some(charged) = crossbow.get(kiln_item::keys::CHARGED_PROJECTILES).cloned() else { return };
    let i = p.hand_index(off_hand);
    p.inv.item_mut(i).remove(kiln_item::component::ids::CHARGED_PROJECTILES);
    p.inv.times_changed += 1;
    let projectiles: Vec<ItemStack> = charged.0.iter().map(ItemStackTemplate::create).collect();
    let speed = 3.15;
    let aim = |e: &mut kiln_entity::Entity, p: &Player, _: usize, angle: f32| {
        let v = shot_vector(p.rot, angle);
        kiln_entity::mob::species::shoot(e, v.x, v.y, v.z, speed, 1.0);
        if let kiln_entity::EntityKind::Arrow(_) = e.kind {
            // `setSoundEvent(CROSSBOW_HIT)` is the client's sound; nothing to keep here.
        }
    };
    let weapon = p.in_hand(off_hand).clone();
    let n = crate::ranged::shoot(p, level, off_hand, &weapon, projectiles, true, spawns, &aim);
    for index in 0..n {
        // `getShotPitch`: the first at 1, the others alternating high and low.
        let pitch = if index == 0 {
            1.0
        } else {
            let high = index & 1 == 1;
            1.0 / (p.entity_rng.next_float() * 0.5 + 1.8) + if high { 0.63 } else { 0.43 }
        };
        p.sound_for_all("minecraft:item.crossbow.shoot", SoundSource::Players, 1.0, pitch);
    }
    p.fire_conds("minecraft:shot_crossbow", None, |c, _, loot| {
        c.item("item").is_none_or(|ip| kiln_loot::predicate::item_matches(&loot.tags, ip, crossbow))
    });
    p.award_stat(Stat::item(player_stats::USED, crossbow.item()), 1);
}

/// `CrossbowItem.shootProjectile` without a target: the view vector turned `angle` degrees
/// about the up vector.
pub(crate) fn shot_vector(rot: [f32; 2], angle: f32) -> Vec3 {
    let view = crate::use_item::view_vector(rot);
    let up = crate::use_item::view_vector([rot[0], rot[1] - 90.0]);
    let (v, k) = ([view.x as f32, view.y as f32, view.z as f32], [up.x as f32, up.y as f32, up.z as f32]);
    // Rodrigues' rotation of `v` about the unit axis `k`.
    let a = angle * 0.017453292;
    let (s, c) = (kiln_javamath::trig::sin(a as f64) as f32, kiln_javamath::trig::cos(a as f64) as f32);
    let cross = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]];
    let dot = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
    let r: [f32; 3] = std::array::from_fn(|i| v[i] * c + cross[i] * s + k[i] * dot * (1.0 - c));
    Vec3::new(r[0] as f64, r[1] as f64, r[2] as f64)
}
