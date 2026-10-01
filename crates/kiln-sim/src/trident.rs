//! Tridents in players' hands (`TridentItem`): held to charge, thrown on release after ten
//! ticks (2.5 speed, one durability; loyalty brings it back, channeling calls lightning in a
//! thunderstorm), or with riptide a spin that launches the player (the client moves itself;
//! the server plays the sound). A trident about to break is not used.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use crate::player_stats::{self, Stat};
use kiln_entity::math::Vec3;
use kiln_inventory::Container;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;
use kiln_loot::effects::ValueComponent;
use kiln_proto::packets::world_fx::SoundSource;

/// `ItemStack.nextDamageWillBreak`.
fn next_damage_will_break(s: &ItemStack) -> bool {
    s.is_damageable_item() && s.damage() >= s.max_damage() - 1
}

/// `EnchantmentHelper.getTridentSpinAttackStrength` (riptide).
fn spin_strength(p: &Player, stack: &ItemStack) -> f32 {
    let Some(loot) = p.loot.clone() else { return 0.0 };
    let mut best: Option<(i32, &kiln_loot::effects::ValueEffect)> = None;
    loot.for_each_enchantment(stack, |e, lvl| {
        if let Some(effect) = &e.effects.trident_spin_attack_strength
            && best.is_none_or(|(l, _)| lvl > l)
        {
            best = Some((lvl, effect));
        }
    });
    best.map_or(0.0, |(lvl, effect)| effect.process(lvl, &mut kiln_javamath::random::LegacyRandom::new(0), 0.0))
}

/// `Entity.isInWaterOrRain` (Kiln's view: water at the feet or rain on the player).
fn in_water_or_rain(p: &Player, level: &RegionLevel) -> bool {
    use kiln_blocks::Level;
    let feet = kiln_blocks::BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
    let head = feet.above();
    let water = |pos| kiln_data::block_logic::fluid(level.block(pos)).kind == kiln_data::block_logic::FluidKind::Water;
    water(feet) || crate::weather::is_raining_at(level.cells, level.env, feet) || crate::weather::is_raining_at(level.cells, level.env, head)
}

/// `TridentItem.use`.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool) {
    let stack = p.in_hand(off_hand).clone();
    if next_damage_will_break(&stack) {
        return;
    }
    if spin_strength(p, &stack) > 0.0 && !in_water_or_rain(p, level) {
        return;
    }
    p.start_using(off_hand, &stack, 72000);
}

/// `TridentItem.releaseUsing` after `ticks` of charging.
pub(crate) fn release(p: &mut Player, level: &mut RegionLevel, off_hand: bool, stack: &ItemStack, ticks: i32, spawns: &mut Vec<Spawn>) {
    if ticks < 10 {
        return;
    }
    let spin = spin_strength(p, stack);
    if spin > 0.0 && !in_water_or_rain(p, level) || next_damage_will_break(stack) {
        return;
    }
    let sound = if spin > 0.0 {
        match (riptide_level(p, stack)).min(3) {
            1 => "minecraft:item.trident.riptide_1",
            2 => "minecraft:item.trident.riptide_2",
            _ => "minecraft:item.trident.riptide_3",
        }
    } else {
        "minecraft:item.trident.throw"
    };
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
    // `hurtWithoutBreaking(1)`.
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    p.hurt_and_break(slot, 1, None);
    if spin == 0.0 {
        let thrown = p.in_hand(off_hand).with_count(1);
        if !p.infinite_materials() {
            let i = p.hand_index(off_hand);
            p.inv.item_mut(i).shrink(1);
            p.inv.times_changed += 1;
        }
        let loyalty = crate::ranged::weapon_value(p, &thrown, &thrown, ValueComponent::TridentReturnAcceleration, 0.0).clamp(0.0, 127.0) as u8;
        let channeling = has_enchantment(&thrown, "minecraft:channeling");
        let bonus = crate::ranged::weapon_value(p, &thrown, &thrown, ValueComponent::Damage, 8.0) - 8.0;
        let pos = Vec3::new(p.pos[0], p.eye_position()[1] - 0.10000000149011612, p.pos[2]);
        let seed = crate::container::pos_random(level, kiln_blocks::BlockPos::new(p.entity_id, p.tick_count, 7), 0x7472_6964).next_long();
        let mut e = kiln_entity::ext_entity::trident::thrown_by_player(pos, p.entity_id, thrown, loyalty, channeling, bonus, p.infinite_materials(), seed);
        crate::ranged::shoot_from_rotation(&mut e, p, p.rot[1], p.rot[0], 0.0, 2.5, 1.0);
        let (at, vel) = (e.position(), e.delta);
        spawns.push(Spawn {
            kind: &kiln_data::entities::types::TRIDENT,
            pos: [at.x, at.y, at.z],
            vel: [vel.x, vel.y, vel.z],
            body: Body::Ready(Box::new(e)),
        });
        p.sound_for_all(sound, SoundSource::Players, 1.0, 1.0);
        return;
    }
    // Riptide: `Player.push` along the view (the strength spread over its three components),
    // sent to the player's client, which launches itself; the spin lasts 20 ticks and hits
    // what it touches for 8 (see [`crate::combat::spin_attack`]).
    use kiln_entity::mob::mth::{cos, sin};
    const RAD: f32 = 0.017453292;
    let (yaw, pitch) = (p.rot[0], p.rot[1]);
    let mut x = -sin((yaw * RAD) as f64) * cos((pitch * RAD) as f64);
    let mut y = -sin((pitch * RAD) as f64);
    let mut z = cos((yaw * RAD) as f64) * cos((pitch * RAD) as f64);
    let len = (x * x + y * y + z * z).sqrt();
    x = x * (spin / len);
    y = y * (spin / len);
    z = z * (spin / len);
    p.vel = [p.vel[0] + x as f64, p.vel[1] + y as f64, p.vel[2] + z as f64];
    p.sync_velocity = true;
    p.start_spin_attack(20, 8.0, stack.clone(), off_hand);
    p.sound_for_all(sound, SoundSource::Players, 1.0, 1.0);
}

fn has_enchantment(s: &ItemStack, name: &str) -> bool {
    let Some(id) = kiln_item::registry::ENCHANTMENT.id(name) else { return false };
    s.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|e| e.level(id) > 0)
}

fn riptide_level(_p: &Player, s: &ItemStack) -> i32 {
    let Some(id) = kiln_item::registry::ENCHANTMENT.id("minecraft:riptide") else { return 1 };
    s.get(kiln_item::keys::ENCHANTMENTS).map_or(1, |e| e.level(id).max(1))
}
