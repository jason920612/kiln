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
use crate::combat::{self, Attr};
use crate::health::{Cause, DamageCtx, Source};
use kiln_item::component::AttributeOperation as Op;
use kiln_item::registry::MOB_EFFECT;
use kiln_proto::packets::entity;
use std::sync::OnceLock;

/// What an effect does on its ticks (the `MobEffect` subclass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// No tick behaviour (attribute modifiers or client-side effects only).
    Plain,
    Regeneration,
    Poison,
    Wither,
    Hunger,
    Saturation,
    Absorption,
    /// `HealOrHarmMobEffect`: instant health, or instant damage when `harm`.
    HealOrHarm { harm: bool },
}

impl Kind {
    /// `MobEffect.isInstantaneous`.
    pub(crate) fn instantaneous(self) -> bool {
        matches!(self, Kind::Saturation | Kind::HealOrHarm { .. })
    }

    /// `shouldApplyEffectTickThisTick(tick, amplifier)`: `tick` is the remaining duration, or
    /// the entity's age for infinite effects. Java masks shift counts to five bits.
    fn applies_this_tick(self, tick: i32, amplifier: i32) -> bool {
        let every = |base: i32| {
            let interval = base.wrapping_shr(amplifier as u32);
            interval <= 0 || tick % interval == 0
        };
        match self {
            Kind::Plain => false,
            Kind::Regeneration => every(50),
            Kind::Poison => every(25),
            Kind::Wither => every(40),
            Kind::Hunger | Kind::Absorption => true,
            Kind::Saturation | Kind::HealOrHarm { .. } => tick >= 1,
        }
    }
}

/// `MobEffectCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Beneficial,
    Harmful,
    Neutral,
}

/// An effect's attribute modifier template (`MobEffect.AttributeTemplate`): the modifier is
/// `amount * (amplifier + 1)`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Modifier {
    pub attr: Attr,
    pub id: &'static str,
    pub amount: f64,
    pub op: Op,
}

/// A `minecraft:mob_effect` entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EffectType {
    pub name: &'static str,
    pub kind: Kind,
    #[allow(dead_code)]
    pub category: Category,
    /// RGB colour of the effect's particles.
    pub color: i32,
    pub modifier: Option<Modifier>,
}

macro_rules! effects {
    ($($name:literal $kind:expr, $cat:ident $color:literal $( => $attr:ident $id:literal $amount:literal $op:ident)?;)*) => {
        &[$(EffectType {
            name: concat!("minecraft:", $name),
            kind: $kind,
            category: Category::$cat,
            color: $color,
            modifier: effects!(@m $($attr $id $amount $op)?),
        },)*]
    };
    (@m) => { None };
    (@m $attr:ident $id:literal $amount:literal $op:ident) => {
        Some(Modifier { attr: combat::$attr, id: concat!("minecraft:", $id), amount: $amount, op: Op::$op })
    };
}

/// `MobEffects` of 26.3, in registry order (checked against the vanilla vectors' registry dump
/// by `effect_parity`).
const EFFECTS: &[EffectType] = effects! {
    "speed" Kind::Plain, Beneficial 3402751 => MOVEMENT_SPEED "effect.speed" 0.20000000298023224 AddMultipliedTotal;
    "slowness" Kind::Plain, Harmful 9154528 => MOVEMENT_SPEED "effect.slowness" -0.15000000596046448 AddMultipliedTotal;
    "haste" Kind::Plain, Beneficial 14270531 => ATTACK_SPEED "effect.haste" 0.10000000149011612 AddMultipliedTotal;
    "mining_fatigue" Kind::Plain, Harmful 4866583 => ATTACK_SPEED "effect.mining_fatigue" -0.10000000149011612 AddMultipliedTotal;
    "strength" Kind::Plain, Beneficial 16762624 => ATTACK_DAMAGE "effect.strength" 3.0 AddValue;
    "instant_health" Kind::HealOrHarm { harm: false }, Beneficial 16262179;
    "instant_damage" Kind::HealOrHarm { harm: true }, Harmful 11101546;
    "jump_boost" Kind::Plain, Beneficial 16646020 => SAFE_FALL_DISTANCE "effect.jump_boost" 1.0 AddValue;
    "nausea" Kind::Plain, Harmful 5578058;
    "regeneration" Kind::Regeneration, Beneficial 13458603;
    "resistance" Kind::Plain, Beneficial 9520880;
    "fire_resistance" Kind::Plain, Beneficial 16750848;
    "water_breathing" Kind::Plain, Beneficial 10017472;
    "invisibility" Kind::Plain, Beneficial 16185078 => WAYPOINT_TRANSMIT_RANGE "effect.waypoint_transmit_range_hide" -1.0 AddMultipliedTotal;
    "blindness" Kind::Plain, Harmful 2039587;
    "night_vision" Kind::Plain, Beneficial 12779366;
    "hunger" Kind::Hunger, Harmful 5797459;
    "weakness" Kind::Plain, Harmful 4738376 => ATTACK_DAMAGE "effect.weakness" -4.0 AddValue;
    "poison" Kind::Poison, Harmful 8889187;
    "wither" Kind::Wither, Harmful 7561558;
    "health_boost" Kind::Plain, Beneficial 16284963 => MAX_HEALTH "effect.health_boost" 4.0 AddValue;
    "absorption" Kind::Absorption, Beneficial 2445989 => MAX_ABSORPTION "effect.absorption" 4.0 AddValue;
    "saturation" Kind::Saturation, Beneficial 16262179;
    "glowing" Kind::Plain, Neutral 9740385;
    "levitation" Kind::Plain, Harmful 13565951;
    "luck" Kind::Plain, Beneficial 5882118 => LUCK "effect.luck" 1.0 AddValue;
    "unluck" Kind::Plain, Harmful 12624973 => LUCK "effect.unluck" -1.0 AddValue;
    "slow_falling" Kind::Plain, Beneficial 15978425;
    "conduit_power" Kind::Plain, Beneficial 1950417;
    "dolphins_grace" Kind::Plain, Beneficial 8954814;
    "bad_omen" Kind::Plain, Neutral 745784;
    "hero_of_the_village" Kind::Plain, Beneficial 4521796;
    "darkness" Kind::Plain, Harmful 2696993;
    "trial_omen" Kind::Plain, Neutral 1484454;
    "raid_omen" Kind::Plain, Neutral 14565464;
    "wind_charged" Kind::Plain, Harmful 12438015;
    "weaving" Kind::Plain, Harmful 7891290;
    "oozing" Kind::Plain, Harmful 10092451;
    "infested" Kind::Plain, Harmful 9214860;
    "breath_of_the_nautilus" Kind::Plain, Beneficial 65518;
};

/// The effect with network id `id`.
pub(crate) fn effect_type(id: i32) -> Option<&'static EffectType> {
    static BY_ID: OnceLock<Vec<Option<&'static EffectType>>> = OnceLock::new();
    let table = BY_ID.get_or_init(|| {
        let n = MOB_EFFECT.entries().len();
        (0..n as i32).map(|i| MOB_EFFECT.name(i).and_then(|name| EFFECTS.iter().find(|e| e.name == name))).collect()
    });
    usize::try_from(id).ok().and_then(|i| table.get(i).copied().flatten())
}

/// Network id of a `minecraft:mob_effect` entry.
pub(crate) fn effect_id(name: &str) -> Option<i32> {
    MOB_EFFECT.id(name)
}

/// `MobEffectInstance.INFINITE_DURATION`.
pub(crate) const INFINITE: i32 = -1;

/// `MobEffectInstance`: one active effect.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Effect {
    /// Network id in `minecraft:mob_effect`.
    pub id: i32,
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
    /// What remains when this instance runs out.
    pub hidden: Option<Box<Effect>>,
}

impl Effect {
    /// The full constructor; the amplifier is clamped to 0..=255.
    pub(crate) fn new(id: i32, duration: i32, amplifier: i32, ambient: bool, visible: bool, show_icon: bool) -> Self {
        Effect { id, duration, amplifier: amplifier.clamp(0, 255), ambient, visible, show_icon, hidden: None }
    }

    /// `new MobEffectInstance(effect, duration, amplifier)`: visible, with an icon.
    pub(crate) fn simple(id: i32, duration: i32, amplifier: i32) -> Self {
        Effect::new(id, duration, amplifier, false, true, true)
    }

    pub(crate) fn kind(&self) -> Kind {
        effect_type(self.id).map_or(Kind::Plain, |t| t.kind)
    }

    /// The copy constructor: the details without the hidden effect.
    fn copy_details(&self) -> Effect {
        Effect { hidden: None, ..self.clone() }
    }

    fn set_details_from(&mut self, o: &Effect) {
        self.duration = o.duration;
        self.amplifier = o.amplifier;
        self.ambient = o.ambient;
        self.visible = o.visible;
        self.show_icon = o.show_icon;
    }

    pub(crate) fn is_infinite(&self) -> bool {
        self.duration == INFINITE
    }

    fn is_shorter_duration_than(&self, o: &Effect) -> bool {
        !self.is_infinite() && (self.duration < o.duration || o.is_infinite())
    }

    /// `endsWithin(ticks)`.
    #[allow(dead_code)]
    pub(crate) fn ends_within(&self, ticks: i32) -> bool {
        !self.is_infinite() && self.duration <= ticks
    }

    fn has_remaining_duration(&self) -> bool {
        self.is_infinite() || self.duration > 0
    }

    /// `mapDuration`: infinite and zero durations stay as they are.
    fn map_duration(&self, f: impl Fn(i32) -> i32) -> i32 {
        if self.is_infinite() || self.duration == 0 { self.duration } else { f(self.duration) }
    }

    /// `withScaledDuration(scale)` (potion duration scale): at least one tick.
    pub(crate) fn scaled(&self, scale: f32) -> Effect {
        let mut e = self.copy_details();
        e.duration = self.map_duration(|d| combat::floor_f32(d as f32 * scale).max(1));
        e
    }

    fn tick_down(&mut self) {
        if let Some(h) = &mut self.hidden {
            h.tick_down();
        }
        self.duration = self.map_duration(|d| d - 1);
    }

    /// `downgradeToHiddenEffect`: at zero, the hidden instance takes over.
    fn downgrade(&mut self) -> bool {
        if self.duration == 0
            && let Some(h) = self.hidden.take()
        {
            self.set_details_from(&h);
            self.hidden = h.hidden;
            return true;
        }
        false
    }

    /// `update(takeOver)`: merges a new instance of the same effect; returns whether this one
    /// changed (a weaker one only goes underneath).
    pub(crate) fn update(&mut self, o: &Effect) -> bool {
        let mut changed = false;
        if o.amplifier > self.amplifier {
            if o.is_shorter_duration_than(self) {
                let old = self.hidden.take();
                let mut hidden = self.copy_details();
                hidden.hidden = old;
                self.hidden = Some(Box::new(hidden));
            }
            self.amplifier = o.amplifier;
            self.duration = o.duration;
            changed = true;
        } else if self.is_shorter_duration_than(o) {
            if o.amplifier == self.amplifier {
                self.duration = o.duration;
                changed = true;
            } else {
                match &mut self.hidden {
                    None => self.hidden = Some(Box::new(o.copy_details())),
                    Some(h) => {
                        h.update(o);
                    }
                }
            }
        }
        if (!o.ambient && self.ambient) || changed {
            self.ambient = o.ambient;
            changed = true;
        }
        if o.visible != self.visible {
            self.visible = o.visible;
            changed = true;
        }
        if o.show_icon != self.show_icon {
            self.show_icon = o.show_icon;
            changed = true;
        }
        changed
    }

    /// `ClientboundUpdateMobEffectPacket(entity, effect, blend)`.
    pub(crate) fn packet(&self, entity_id: i32, blend: bool) -> bytes::Bytes {
        use entity::effect_flags as f;
        let mut flags = 0;
        if self.ambient {
            flags |= f::AMBIENT;
        }
        if self.visible {
            flags |= f::VISIBLE;
        }
        if self.show_icon {
            flags |= f::SHOW_ICON;
        }
        if blend {
            flags |= f::BLEND;
        }
        let e = entity::MobEffect { effect: self.id, amplifier: self.amplifier, duration: self.duration, flags };
        entity::update_mob_effect(entity_id, &e)
    }

    /// From the item/save form (`MobEffectInstance.CODEC` / stream codec).
    pub(crate) fn from_item(e: &kiln_item::component::MobEffectInstance) -> Effect {
        Effect::from_details(e.effect, &e.details)
    }

    fn from_details(id: i32, d: &kiln_item::component::EffectDetails) -> Effect {
        let mut e = Effect::new(id, d.duration, d.amplifier, d.ambient, d.show_particles, d.show_icon);
        e.hidden = d.hidden_effect.as_deref().map(|h| Box::new(Effect::from_details(id, h)));
        e
    }

    pub(crate) fn to_item(&self) -> kiln_item::component::MobEffectInstance {
        kiln_item::component::MobEffectInstance { effect: self.id, details: self.details() }
    }

    fn details(&self) -> kiln_item::component::EffectDetails {
        kiln_item::component::EffectDetails {
            amplifier: self.amplifier,
            duration: self.duration,
            ambient: self.ambient,
            show_particles: self.visible,
            show_icon: self.show_icon,
            hidden_effect: self.hidden.as_ref().map(|h| Box::new(h.details())),
        }
    }
}

/// `MobEffect.AMBIENT_ALPHA`: `Mth.floor(38.25f)`.
const AMBIENT_ALPHA: i32 = 38;

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
                (m.attr.name() == attr).then(|| (m.id, m.amount * (e.amplifier + 1) as f64, m.op))
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
        self.send(e.packet(self.entity_id, true));
        if effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
            self.attributes_dirty = true;
        }
    }

    /// `ServerPlayer.onEffectUpdated`: `refresh` re-applies the attribute modifiers.
    fn on_effect_updated(&mut self, e: &Effect, refresh: bool) {
        self.effects_dirty = true;
        if refresh && effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
            self.attributes_dirty = true;
            self.refresh_dirty_attributes();
        }
        self.send(e.packet(self.entity_id, false));
    }

    /// `ServerPlayer.onEffectsRemoved`: a removal packet each, then the attributes.
    fn on_effects_removed(&mut self, removed: &[Effect]) {
        self.effects_dirty = true;
        for e in removed {
            self.send(entity::remove_mob_effect(self.entity_id, e.id));
            if effect_type(e.id).is_some_and(|t| t.modifier.is_some()) {
                self.attributes_dirty = true;
            }
        }
        self.refresh_dirty_attributes();
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

    /// `updateSynchronizedMobEffectParticles`: one coloured `entity_effect` particle per visible
    /// effect (faint when ambient).
    pub(crate) fn effect_particles(&self) -> Vec<entity::metadata::Particle> {
        let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:entity_effect") else { return Vec::new() };
        self.effects
            .values()
            .filter(|e| e.visible)
            .map(|e| {
                let alpha = if e.ambient { AMBIENT_ALPHA } else { 255 };
                let color = effect_type(e.id).map_or(0, |t| t.color);
                let argb = (alpha << 24) | (color & 0xFF_FFFF);
                entity::metadata::Particle { kind, options: argb.to_be_bytes().to_vec() }
            })
            .collect()
    }

    /// `areAllEffectsAmbient`: no visible effect that is not ambient.
    pub(crate) fn effects_ambient(&self) -> bool {
        self.effects.values().all(|e| !e.visible || e.ambient)
    }

    /// Update Mob Effect for every active effect (joining, respawning).
    pub(crate) fn send_all_effects(&mut self) {
        let packets: Vec<_> = self.effects.values().map(|e| e.packet(self.entity_id, false)).collect();
        for p in packets {
            self.send(p);
        }
    }

    /// The saved `active_effects` list (`MobEffectInstance.CODEC`).
    pub(crate) fn effects_nbt(&self) -> Option<kiln_proto::nbt::Tag> {
        if self.effects.is_empty() {
            return None;
        }
        let list = self.effects.values().map(|e| e.to_item().to_value().to_nbt()).collect();
        Some(kiln_proto::nbt::Tag::List(list))
    }

}

/// Loads `active_effects` (unknown or invalid entries are dropped, as vanilla's lenient list
/// codec does).
pub(crate) fn load_effects(tag: &kiln_proto::nbt::Tag) -> std::collections::BTreeMap<i32, Effect> {
    let mut out = std::collections::BTreeMap::new();
    let kiln_proto::nbt::Tag::List(items) = tag else { return out };
    for item in items {
        let value = kiln_item::Value::from_nbt(item);
        if let Ok(e) = kiln_item::component::MobEffectInstance::from_value(&value) {
            let e = Effect::from_item(&e);
            out.insert(e.id, e);
        }
    }
    out
}

/// `minecraft:potion` entries' effects (`Potions`): (effect, duration, amplifier).
const POTIONS: &[(&str, &[(&str, i32, i32)])] = &[
    ("water", &[]),
    ("mundane", &[]),
    ("thick", &[]),
    ("awkward", &[]),
    ("night_vision", &[("night_vision", 3600, 0)]),
    ("long_night_vision", &[("night_vision", 9600, 0)]),
    ("invisibility", &[("invisibility", 3600, 0)]),
    ("long_invisibility", &[("invisibility", 9600, 0)]),
    ("leaping", &[("jump_boost", 3600, 0)]),
    ("long_leaping", &[("jump_boost", 9600, 0)]),
    ("strong_leaping", &[("jump_boost", 1800, 1)]),
    ("fire_resistance", &[("fire_resistance", 3600, 0)]),
    ("long_fire_resistance", &[("fire_resistance", 9600, 0)]),
    ("swiftness", &[("speed", 3600, 0)]),
    ("long_swiftness", &[("speed", 9600, 0)]),
    ("strong_swiftness", &[("speed", 1800, 1)]),
    ("slowness", &[("slowness", 1800, 0)]),
    ("long_slowness", &[("slowness", 4800, 0)]),
    ("strong_slowness", &[("slowness", 400, 3)]),
    ("turtle_master", &[("slowness", 400, 3), ("resistance", 400, 2)]),
    ("long_turtle_master", &[("slowness", 800, 3), ("resistance", 800, 2)]),
    ("strong_turtle_master", &[("slowness", 400, 5), ("resistance", 400, 3)]),
    ("water_breathing", &[("water_breathing", 3600, 0)]),
    ("long_water_breathing", &[("water_breathing", 9600, 0)]),
    ("healing", &[("instant_health", 1, 0)]),
    ("strong_healing", &[("instant_health", 1, 1)]),
    ("harming", &[("instant_damage", 1, 0)]),
    ("strong_harming", &[("instant_damage", 1, 1)]),
    ("poison", &[("poison", 900, 0)]),
    ("long_poison", &[("poison", 1800, 0)]),
    ("strong_poison", &[("poison", 432, 1)]),
    ("regeneration", &[("regeneration", 900, 0)]),
    ("long_regeneration", &[("regeneration", 1800, 0)]),
    ("strong_regeneration", &[("regeneration", 450, 1)]),
    ("strength", &[("strength", 3600, 0)]),
    ("long_strength", &[("strength", 9600, 0)]),
    ("strong_strength", &[("strength", 1800, 1)]),
    ("weakness", &[("weakness", 1800, 0)]),
    ("long_weakness", &[("weakness", 4800, 0)]),
    ("luck", &[("luck", 6000, 0)]),
    ("slow_falling", &[("slow_falling", 1800, 0)]),
    ("long_slow_falling", &[("slow_falling", 4800, 0)]),
    ("wind_charged", &[("wind_charged", 3600, 0)]),
    ("weaving", &[("weaving", 3600, 0)]),
    ("oozing", &[("oozing", 3600, 0)]),
    ("infested", &[("infested", 3600, 0)]),
];

/// `PotionContents.forEachEffect(consumer, durationScale)`: the potion's effects, then the
/// custom ones, each with its duration scaled.
pub(crate) fn potion_effects(contents: &kiln_item::component::PotionContents, scale: f32) -> Vec<Effect> {
    let mut out = Vec::new();
    if let Some(name) = contents.potion.and_then(|id| kiln_item::registry::POTION.name(id)) {
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        if let Some((_, list)) = POTIONS.iter().find(|(n, _)| *n == short) {
            for (effect, duration, amplifier) in *list {
                if let Some(id) = effect_id(&format!("minecraft:{effect}")) {
                    out.push(Effect::simple(id, *duration, *amplifier).scaled(scale));
                }
            }
        }
    }
    out.extend(contents.custom_effects.iter().map(|e| Effect::from_item(e).scaled(scale)));
    out
}

#[cfg(test)]
pub(crate) fn potion_table() -> &'static [(&'static str, &'static [(&'static str, i32, i32)])] {
    POTIONS
}

#[cfg(test)]
pub(crate) fn effect_table() -> &'static [EffectType] {
    EFFECTS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speed(duration: i32, amplifier: i32) -> Effect {
        Effect::simple(effect_id("minecraft:speed").unwrap(), duration, amplifier)
    }

    #[test]
    fn table_follows_the_registry() {
        for (i, t) in EFFECTS.iter().enumerate() {
            assert_eq!(effect_id(t.name), Some(i as i32), "{}", t.name);
        }
        assert_eq!(effect_type(effect_id("minecraft:poison").unwrap()).unwrap().kind, Kind::Poison);
    }

    #[test]
    fn stronger_shorter_effect_hides_the_old_one() {
        let mut e = speed(100, 0);
        assert!(e.update(&speed(20, 2)));
        assert_eq!((e.amplifier, e.duration), (2, 20));
        let hidden = e.hidden.as_deref().unwrap();
        assert_eq!((hidden.amplifier, hidden.duration), (0, 100));
        for _ in 0..20 {
            e.tick_down();
        }
        assert!(e.downgrade());
        assert_eq!((e.amplifier, e.duration), (0, 80));
        assert!(e.hidden.is_none());
    }

    #[test]
    fn weaker_longer_effect_goes_underneath() {
        let mut e = speed(20, 2);
        assert!(!e.update(&speed(50, 0)));
        assert!(!e.update(&speed(40, 1)));
        // The stronger of the two hidden ones sits on top of the weaker, longer one.
        let h = e.hidden.as_deref().unwrap();
        assert_eq!((h.amplifier, h.duration), (1, 40));
        let hh = h.hidden.as_deref().unwrap();
        assert_eq!((hh.amplifier, hh.duration), (0, 50));
    }

    #[test]
    fn intervals() {
        assert!(Kind::Regeneration.applies_this_tick(50, 0));
        assert!(!Kind::Regeneration.applies_this_tick(49, 0));
        assert!(Kind::Regeneration.applies_this_tick(7, 6));
        assert!(Kind::Poison.applies_this_tick(12, 1));
        assert!(!Kind::HealOrHarm { harm: true }.applies_this_tick(0, 0));
        // Java masks shift counts: 50 >> 32 is 50.
        assert!(!Kind::Regeneration.applies_this_tick(7, 32));
    }
}
