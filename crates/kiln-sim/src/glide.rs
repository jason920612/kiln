//! Elytra gliding: `Player.tryToStartFallFlying` (the "start fall flying" player command),
//! `LivingEntity.updateFallFlying` (a glider ends when the player lands or has none left, and
//! every 20 ticks of flight one glider loses a point of durability) and the fall-flying
//! pose, hit box and eye height the other players see.

use crate::Player;
use crate::combat::SLOTS;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::RandomSource;

/// `ServerboundPlayerCommandPacket.Action.START_FALL_FLYING`: ordinal 6 in 26.x (STOP_SLEEPING, START_SPRINTING, STOP_SPRINTING,
/// START_RIDING_JUMP, STOP_RIDING_JUMP, OPEN_INVENTORY, START_FALL_FLYING; the sneak actions are gone, `PlayerInput` carries them).
pub(crate) const START_FALL_FLYING: i32 = 6;

/// `LivingEntity.canGlideUsing`: a glider worn in its own slot that the next point of damage
/// would not break.
pub(crate) fn can_glide_using(stack: &ItemStack, slot: EquipmentSlot) -> bool {
    stack.has(kiln_item::component::ids::GLIDER)
        && stack.get(keys::EQUIPPABLE).is_some_and(|e| e.slot == slot)
        && !(stack.is_damageable_item() && stack.damage() >= stack.max_damage() - 1)
}

impl Player {
    pub(crate) fn worn(&self, slot: EquipmentSlot) -> &ItemStack {
        kiln_inventory::Container::item(&self.inv, kiln_inventory::inventory::equipment_index(slot, self.inv.selected))
    }

    /// `LivingEntity.canGlide`.
    fn can_glide(&self) -> bool {
        !(self.on_ground || self.has_effect("minecraft:levitation")) && SLOTS.iter().any(|&s| can_glide_using(self.worn(s), s))
    }

    /// The start-gliding player command (`tryToStartFallFlying`, else `stopFallFlying`): glides when able, otherwise
    /// the flag is cleared, and sent again even when it was clear already (vanilla sets it and clears it, which marks
    /// it dirty): the client starts gliding on its own when it sends the command, and has to be told it does not.
    pub(crate) fn try_start_fall_flying(&mut self, in_liquid: bool) {
        if !self.fall_flying && self.can_glide() && !in_liquid {
            self.set_fall_flying(true);
        } else {
            self.set_fall_flying(false);
            self.flags_resend = true;
        }
    }

    fn set_fall_flying(&mut self, on: bool) {
        if self.fall_flying != on {
            self.fall_flying = on;
        }
    }

    /// `startAutoSpinAttack`: the riptide spin lasts `ticks` (the client moves the player) and
    /// hits what it touches for `damage` with `item` (the trident, thrown from `off_hand`).
    pub(crate) fn start_spin_attack(&mut self, ticks: i32, damage: f32, item: ItemStack, off_hand: bool) {
        self.spin_ticks = ticks;
        self.spin_damage = damage;
        self.spin_item = item;
        self.spin_off_hand = off_hand;
    }

    /// The countdown of the spin attack (`aiStep`'s `autoSpinAttackTicks`); what the spin
    /// touched is looked at afterwards with the region's entities ([`Player::spin_finished`]).
    pub(crate) fn tick_spin(&mut self) {
        if self.spin_ticks > 0 {
            self.spin_ticks -= 1;
            self.spin_check = true;
            if self.spin_ticks == 0 {
            }
        }
    }

    /// The end of `checkAutoSpinAttack`: a finished spin (its ticks ran out, or it hit
    /// something) forgets its damage and item.
    pub(crate) fn spin_finished(&mut self) {
        if self.spin_ticks <= 0 {
            self.spin_damage = 0.0;
            self.spin_item = ItemStack::empty();
        }
        // `updatePlayerPose` afterwards.
        self.spin_pose = self.spin_ticks > 0;
    }

    /// A spin that runs into a wall (the client reports a horizontal collision) and nothing else
    /// ends there, without a bounce (`autoSpinAttackTicks = 0`).
    pub(crate) fn end_spin_on_collision(&mut self) {
        self.spin_ticks = 0;
    }

    /// The spin hit something (`autoSpinAttackTicks = 0`, `setDeltaMovement(delta * -0.2)`): the
    /// flag goes down and the player bounces back.
    pub(crate) fn stop_spin_on_hit(&mut self) {
        self.spin_ticks = 0;
        self.vel = self.vel.map(|v| v * -0.2);
    }

    /// `LivingEntity.updateFallFlying` plus the `fallFlyTicks` counter of `aiStep`.
    pub(crate) fn tick_glide(&mut self) {
        if self.fall_flying {
            // `checkFallDistanceAccumulation`: a slow glide keeps the fall short.
            if self.known_movement[1] > -0.5 && self.fall_distance > 1.0 {
                self.fall_distance = 1.0;
            }
        }
        if self.fall_flying && !self.can_glide() {
            self.set_fall_flying(false);
        } else if self.fall_flying {
            let i = self.fall_fly_ticks + 1;
            if i % 10 == 0 && (i / 10) % 2 == 0 {
                let slots: Vec<EquipmentSlot> = SLOTS.iter().copied().filter(|&s| can_glide_using(self.worn(s), s)).collect();
                if !slots.is_empty() {
                    let slot = slots[self.level_rng.next_int_bounded(slots.len() as i32) as usize];
                    self.hurt_and_break(slot, 1, None);
                }
            }
        }
        self.fall_fly_ticks = if self.fall_flying { self.fall_fly_ticks + 1 } else { 0 };
    }
}
