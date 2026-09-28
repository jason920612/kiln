//! Mob effects on players (vanilla `MobEffectInstance`, `LivingEntity.addEffect`,
//! `tickEffects`, `removeEffect`, `removeAllEffects`, and `ServerPlayer`'s effect packets).
//!
//! An effect instance has a duration (-1 is infinite), an amplifier and display flags, and may
//! hide a weaker or shorter instance of the same effect underneath that takes over when it runs
//! out (`MobEffectInstance.update`, `downgradeToHiddenEffect`). Every player tick each effect
//! applies its tick behaviour when its interval says so (regeneration, poison, wither, hunger,
//! absorption, and the instantaneous ones on their only tick), then counts down.
//!
//! Attribute effects (speed, slowness, haste, mining fatigue, strength, weakness, health boost,
//! absorption, luck, jump boost, invisibility) add their modifier while active; see
//! [`Player::effect_modifiers`]. Damage-related ones are read where damage is dealt:
//! resistance and fire resistance in [`crate::health`], water breathing, conduit power and the
//! breath of the nautilus in the air supply ([`crate::hazards`]), haste and mining fatigue in
//! digging. Night vision, blindness, darkness, nausea, levitation, slow falling and the like
//! act on the client only.
//!
//! Vanilla keeps effects in a hash map keyed by identity-hashed holders, so the order in which
//! different effects tick is arbitrary; Kiln ticks them in registry order.

use crate::Player;
use crate::combat;
use crate::health::{Cause, DamageCtx, Source};
use kiln_item::component::AttributeOperation as Op;
use kiln_proto::packets::entity;

pub(crate) use kiln_entity::effect::{Effect, INFINITE, Kind, effect_id, effect_type};

/// `ClientboundUpdateMobEffectPacket(entity, effect, blend)`.
pub(crate) fn effect_packet(e: &Effect, entity_id: i32, blend: bool) -> bytes::Bytes {
    use entity::effect_flags as f;
    let mut flags = 0;
    if e.ambient {
        flags |= f::AMBIENT;
    }
    if e.visible {
        flags |= f::VISIBLE;
    }
    if e.show_icon {
        flags |= f::SHOW_ICON;
    }
    if blend {
        flags |= f::BLEND;
    }
    let m = entity::MobEffect { effect: e.id, amplifier: e.amplifier, duration: e.duration, flags };
    entity::update_mob_effect(entity_id, &m)
}

impl Player {
    pub(crate) fn has_effect(&self, name: &str) -> bool {
        effect_id(name).is_some_and(|id| self.effects.contains_key(&id))
    }

    /// The amplifier of the active `name` effect.
    pub(crate) fn effect_amplifier(&self, name: &str) -> Option<i32> {
        effect_id(name).and_then(|id| self.effects.get(&id)).map(|e| e.amplifier)
    }

    /// `getMaxHealth`.
    pub(crate) fn max_health(&self) -> f32 {
        self.attribute(combat::MAX_HEALTH) as f32
    }

    /// `getMaxAbsorption`.
    pub(crate) fn max_absorption(&self) -> f32 {
        self.attribute(combat::MAX_ABSORPTION) as f32
    }

    /// `setAbsorptionAmount`: clamped to the maximum absorption.
    pub(crate) fn set_absorption(&mut self, amount: f32) {
        self.absorption = amount.clamp(0.0, self.max_absorption());
    }

    /// The attribute modifiers of the active effects on `attr` (`MobEffect.addAttributeModifiers`
    /// with the current amplifier), in registry order: (id, amount, operation).
    pub(crate) fn effect_modifiers(&self, attr: &str) -> Vec<(&'static str, f64, Op)> {
        self.effects
            .values()
            .filter_map(|e| {
                let m = effect_type(e.id)?.modifier?;
                (m.attr == attr).then(|| (m.id, m.amount_at(e.amplifier), m.op))
            })
            .collect()
    }

    /// `LivingEntity.addEffect(effect, source)`: players accept every effect.
    pub(crate) fn add_effect(&mut self, e: Effect) -> bool {
        let changed = match self.effects.get_mut(&e.id) {
            None => {
                self.effects.insert(e.id, e.clone());
                self.on_effect_added(&e);
                true
            }
            Some(old) => {
                if old.update(&e) {
                    let updated = old.clone();
                    self.on_effect_updated(&updated, true);
                    true
                } else {
                    false
                }
            }
        };
        // `onEffectStarted` with the new instance, whether or not it changed anything.
        if e.kind() == Kind::Absorption {
            let amount = self.absorption.max((4 * (1 + e.amplifier)) as f32);
            self.set_absorption(amount);
        }
        changed
    }

    /// `removeEffect`.
    pub(crate) fn remove_effect(&mut self, id: i32) -> bool {
        match self.effects.remove(&id) {
            Some(e) => {
                self.on_effects_removed(&[e]);
                true
            }
            None => false,
        }
    }

    /// `removeAllEffects` (milk, `/effect clear`).
    pub(crate) fn remove_all_effects(&mut self) -> bool {
        if self.effects.is_empty() {
            return false;
        }
        let all: Vec<Effect> = std::mem::take(&mut self.effects).into_values().collect();
        self.on_effects_removed(&all);
        true
    }

    /// `ServerPlayer.onEffectAdded`: the effect with the blend flag to the player, attributes.
    fn on_effect_added(&mut self, e: &Effect) {
        self.effects_dirty = true;
        self.send(effect_packet(e, self.entity_id, true));
        if effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
            self.attributes_dirty = true;
        }
        if Some(e.id) == effect_id("minecraft:levitation") {
            self.levitation_start = Some((self.tick_count, self.pos));
        }
        self.effects_changed();
    }

    /// `ServerPlayer.onEffectUpdated`: `refresh` re-applies the attribute modifiers.
    fn on_effect_updated(&mut self, e: &Effect, refresh: bool) {
        self.effects_dirty = true;
        if refresh && effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
            self.attributes_dirty = true;
            self.refresh_dirty_attributes();
        }
        self.send(effect_packet(e, self.entity_id, false));
        self.effects_changed();
    }

    /// `ServerPlayer.onEffectsRemoved`: a removal packet each, then the attributes.
    fn on_effects_removed(&mut self, removed: &[Effect]) {
        self.effects_dirty = true;
        for e in removed {
            self.send(entity::remove_mob_effect(self.entity_id, e.id));
            if effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
                self.attributes_dirty = true;
            }
            if Some(e.id) == effect_id("minecraft:levitation") {
                self.levitation_start = None;
            }
        }
        self.refresh_dirty_attributes();
        self.effects_changed();
    }

    /// `EffectsChangedTrigger.trigger` (Kiln's effects have no source entity).
    fn effects_changed(&mut self) {
        let effects: Vec<(i32, i32, i32, bool, bool)> = self.effects.values().map(|e| (e.id, e.amplifier, e.duration, e.ambient, e.visible)).collect();
        self.fire_conds("minecraft:effects_changed", None, |c, _, _| {
            c.get("effects").is_none_or(|j| crate::advancements::criteria::effects_match(j, &effects)) && c.cap("source").is_none()
        });
    }

    /// `refreshDirtyAttributes`: health and absorption above their new maximum drop to it.
    fn refresh_dirty_attributes(&mut self) {
        let max = self.max_health();
        if self.health > max {
            self.health = max;
        }
        let max = self.max_absorption();
        if self.absorption > max {
            self.set_absorption(max);
        }
    }

    /// `LivingEntity.tickEffects` (server side).
    pub(crate) fn tick_effects(&mut self, ctx: &mut DamageCtx) {
        let ids: Vec<i32> = self.effects.keys().copied().collect();
        for id in ids {
            let Some(e) = self.effects.get(&id) else { continue };
            let keep = if !e.has_remaining_duration() {
                false
            } else {
                let (kind, amplifier) = (e.kind(), e.amplifier);
                let tick = if e.is_infinite() { self.tick_count } else { e.duration };
                if kind.applies_this_tick(tick, amplifier) && !self.apply_effect_tick(kind, amplifier, ctx) {
                    false
                } else {
                    let Some(e) = self.effects.get_mut(&id) else { continue };
                    e.tick_down();
                    if e.downgrade() {
                        let e = e.clone();
                        self.on_effect_updated(&e, true);
                    }
                    self.effects.get(&id).is_some_and(Effect::has_remaining_duration)
                }
            };
            if !keep {
                if let Some(e) = self.effects.remove(&id) {
                    self.on_effects_removed(&[e]);
                }
            } else if let Some(e) = self.effects.get(&id)
                && e.duration % 600 == 0
            {
                let e = e.clone();
                self.on_effect_updated(&e, false);
            }
        }
    }

    /// `MobEffect.applyEffectTick`: false removes the effect.
    fn apply_effect_tick(&mut self, kind: Kind, amplifier: i32, ctx: &mut DamageCtx) -> bool {
        match kind {
            Kind::Plain => true,
            Kind::Regeneration => {
                if self.health < self.max_health() {
                    self.heal(1.0);
                }
                true
            }
            Kind::Poison => {
                if self.health > 1.0 {
                    self.hurt(1.0, &Cause::Other("minecraft:magic").into(), ctx);
                }
                true
            }
            Kind::Wither => {
                self.hurt(1.0, &Cause::Other("minecraft:wither").into(), ctx);
                true
            }
            Kind::Hunger => {
                self.exhaust(0.005 * (amplifier + 1) as f32);
                true
            }
            Kind::Saturation => {
                self.eat(amplifier + 1, (amplifier + 1) as f32 * 1.0 * 2.0);
                true
            }
            Kind::Absorption => self.absorption > 0.0,
            Kind::HealOrHarm { harm: false } => {
                self.heal(4i32.wrapping_shl(amplifier as u32).max(0) as f32);
                true
            }
            Kind::HealOrHarm { harm: true } => {
                self.hurt(6i32.wrapping_shl(amplifier as u32) as f32, &Cause::Other("minecraft:magic").into(), ctx);
                true
            }
            // Omens: raids are not simulated for players yet. The mob hurt/death effects never
            // tick.
            Kind::BadOmen | Kind::RaidOmen | Kind::Infested | Kind::Oozing | Kind::Weaving | Kind::WindCharged => true,
        }
    }

    /// `MobEffect.applyInstantaneousEffect(level, source, indirect, target, amplifier, 1.0)`
    /// for an effect the player gave itself (a drunk potion): heal, or indirect magic damage
    /// caused by the player.
    fn apply_instantaneous(&mut self, e: &Effect, ctx: &mut DamageCtx) {
        match e.kind() {
            Kind::HealOrHarm { harm: false } => {
                let amount = (4i32.wrapping_shl(e.amplifier as u32) as f64 + 0.5) as i32;
                self.heal(amount as f32);
            }
            Kind::HealOrHarm { harm: true } => {
                let amount = (6i32.wrapping_shl(e.amplifier as u32) as f64 + 0.5) as i32;
                let source = Source {
                    cause: Cause::Other("minecraft:indirect_magic"),
                    attacker: Some(self.as_attacker()),
                    direct: None,
                    weapon: None,
                };
                self.hurt(amount as f32, &source, ctx);
            }
            kind => {
                // Saturation and other instantaneous effects tick once.
                self.apply_effect_tick(kind, e.amplifier, ctx);
            }
        }
    }

    /// `PotionContents.applyToLivingEntity`: instantaneous effects at once, others added.
    pub(crate) fn apply_potion_effect(&mut self, e: Effect, ctx: &mut DamageCtx) {
        if e.kind().instantaneous() {
            self.apply_instantaneous(&e, ctx);
        } else {
            self.add_effect(e);
        }
    }

    /// `FoodData.eat(nutrition, saturation)` with the saturation already worked out.
    pub(crate) fn eat(&mut self, nutrition: i32, saturation: f32) {
        self.food = (nutrition + self.food).clamp(0, 20);
        self.saturation = (saturation + self.saturation).clamp(0.0, self.food as f32);
    }

    /// `updateSynchronizedMobEffectParticles`: one particle per visible effect (an
    /// `entity_effect` in the effect's colour, faint when ambient).
    pub(crate) fn effect_particles(&self) -> Vec<entity::metadata::Particle> {
        kiln_entity::effect::particles(&self.effects)
    }

    /// `areAllEffectsAmbient`: no visible effect that is not ambient.
    pub(crate) fn effects_ambient(&self) -> bool {
        self.effects.values().all(|e| !e.visible || e.ambient)
    }

    /// Update Mob Effect for every active effect (joining, respawning).
    pub(crate) fn send_all_effects(&mut self) {
        let packets: Vec<_> = self.effects.values().map(|e| effect_packet(e, self.entity_id, false)).collect();
        for p in packets {
            self.send(p);
        }
    }

    /// The saved `active_effects` list (`MobEffectInstance.CODEC`).
    pub(crate) fn effects_nbt(&self) -> Option<kiln_proto::nbt::Tag> {
        kiln_entity::effect::save(&self.effects)
    }
}

/// Loads `active_effects` (unknown or invalid entries are dropped, as vanilla's lenient list
/// codec does).
pub(crate) fn load_effects(tag: &kiln_proto::nbt::Tag) -> std::collections::BTreeMap<i32, Effect> {
    kiln_entity::effect::load(tag)
}

#[cfg(test)]
pub(crate) type PotionEntry = kiln_entity::effect::PotionEntry;

/// `PotionContents.forEachEffect(consumer, durationScale)`: the potion's effects, then the
/// custom ones, each with its duration scaled.
pub(crate) fn potion_effects(contents: &kiln_item::component::PotionContents, scale: f32) -> Vec<Effect> {
    kiln_entity::effect::potion_effects(contents, scale)
}

#[cfg(test)]
pub(crate) fn potion_table() -> &'static [PotionEntry] {
    kiln_entity::effect::POTIONS
}

#[cfg(test)]
pub(crate) fn effect_table() -> &'static [kiln_entity::effect::EffectType] {
    kiln_entity::effect::EFFECTS
}
