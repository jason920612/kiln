//! Blocking with an item in use that has `minecraft:blocks_attacks` (shields):
//! `LivingEntity.applyItemBlocking` and `BlocksAttacks` (`resolveBlockedDamage`,
//! `hurtBlockingItem`, `onBlocked`, `disable`). Blocking starts `block_delay_seconds` into the
//! use; damage from within the horizontal blocking angle of the player's facing is reduced by
//! the item's damage reductions (all of it for a shield), the item wears, the block sound
//! plays, and a weapon with `disable_blocking_for_seconds` (axes) puts the shield on
//! cooldown.

use crate::Player;
use crate::health::Source;
use kiln_item::component::{BlocksAttacks, EquipmentSlot};
use kiln_item::{HolderSet, ItemStack};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::world_fx::SoundSource;

fn in_set(source: &Source, set: &HolderSet) -> bool {
    match set {
        HolderSet::Direct(ids) => ids.contains(&source.type_id()),
        HolderSet::Tag(tag) => source.is(tag.as_str().trim_start_matches('#')),
    }
}

fn sound_name(s: &kiln_item::component::SoundEvent) -> Option<String> {
    match s {
        kiln_item::Holder::Reference(id) => kiln_item::registry::SOUND_EVENT.name(*id).map(str::to_owned),
        kiln_item::Holder::Direct(d) => Some(d.sound_id.to_string()),
    }
}

impl Player {
    /// `getItemBlockingWith`: the item in use when it blocks and has blocked long enough.
    pub(crate) fn item_blocking_with(&self) -> Option<(ItemStack, BlocksAttacks, bool)> {
        let u = self.using?;
        let stack = self.in_hand(u.off_hand);
        if stack.is_empty() || stack.item() != u.item {
            return None;
        }
        let ba = stack.get(kiln_item::keys::BLOCKS_ATTACKS)?;
        let used = u.duration - u.remaining;
        (used as f32 >= ba.block_delay_seconds * 20.0).then(|| (stack.clone(), ba.clone(), u.off_hand))
    }

    /// `LivingEntity.applyItemBlocking`: the part of `amount` the item blocks.
    pub(crate) fn apply_item_blocking(&mut self, amount: f32, source: &Source) -> f32 {
        if amount <= 0.0 {
            return 0.0;
        }
        let Some((stack, ba, off_hand)) = self.item_blocking_with() else { return 0.0 };
        if ba.bypassed_by.as_ref().is_some_and(|set| in_set(source, set)) {
            return 0.0;
        }
        // `getSourcePosition`: a projectile's own position (a point behind it along its flight),
        // else where the attacker is.
        let at = source.position.or(source.attacker.as_ref().map(|a| a.pos));
        let angle = match at {
            Some(at) => {
                let view = crate::use_item::view_vector([self.rot[0], 0.0]);
                let (dx, dz) = (at[0] - self.pos[0], at[2] - self.pos[2]);
                let len = (dx * dx + dz * dz).sqrt();
                let (nx, nz) = if len < 1.0e-4 { (0.0, 0.0) } else { (dx / len, dz / len) };
                (nx * view.x + nz * view.z).acos()
            }
            None => 3.1415927410125732,
        };
        // `resolveBlockedDamage`.
        let mut blocked = 0.0f32;
        for r in &ba.damage_reductions {
            if angle > (0.017453292f32 * r.horizontal_blocking_angle) as f64 {
                continue;
            }
            if r.types.as_ref().is_some_and(|set| !in_set(source, set)) {
                continue;
            }
            blocked += (r.base + r.factor * amount).clamp(0.0, amount);
        }
        let blocked = blocked.clamp(0.0, amount);
        // `hurtBlockingItem`.
        let f = &ba.item_damage;
        let wear = if blocked < f.threshold { 0 } else { (f.base + f.factor * blocked).floor() as i32 };
        if wear > 0 {
            self.hurt_and_break(if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand }, wear, None);
        }
        // `blockUsingItem` (melee only): a weapon that disables blocking.
        if blocked > 0.0
            && !source.is("minecraft:is_projectile")
            && source.direct.is_none()
            && let Some(w) = &source.weapon
            && let Some(weapon) = w.get(kiln_item::keys::WEAPON)
            && weapon.disable_blocking_for_seconds > 0.0
        {
            let ticks = (weapon.disable_blocking_for_seconds * 20.0 * ba.disable_cooldown_scale) as i32;
            if ticks > 0 {
                self.add_cooldown(&stack, ticks);
                self.stop_using();
                if let Some(s) = ba.disabled_sound.as_ref().and_then(sound_name) {
                    let pitch = 0.8 + self.level_rng.next_float() * 0.4;
                    self.sound_for_all(&s, SoundSource::Players, 0.8, pitch);
                }
            }
        }
        blocked
    }

    /// `BlocksAttacks.onBlocked`: the block sound.
    pub(crate) fn on_blocked(&mut self, stack: &ItemStack) {
        let Some(ba) = stack.get(kiln_item::keys::BLOCKS_ATTACKS) else { return };
        if let Some(s) = ba.block_sound.as_ref().and_then(sound_name) {
            let pitch = 0.8 + self.level_rng.next_float() * 0.4;
            self.sound_for_all(&s, SoundSource::Players, 1.0, pitch);
        }
    }
}
