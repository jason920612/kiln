//! Enchantment effects that run by themselves for a mob's equipment (vanilla runs them for every `LivingEntity`, not only for players):
//! the `tick` effects every tick, the `location_changed` effects whenever the mob's block position changes
//! (`LivingEntity.baseTick` -> `onChangedBlock`), when it lands (`checkFallDamage`) and when a piece of equipment is put on, taken off or
//! changed (`collectEquipmentChanges`).
//!
//! The same rules as the players' (`kiln-sim`'s `enchant_fx`): a `location_changed` effect is *active* while its requirements hold, and the
//! requirements can ask (`enchantment_active_check`) whether it was active before; an `attribute` effect adds a transient modifier when it
//! becomes active and takes it away when it stops; every other effect runs each time the requirements hold. The effects are resolved by
//! [`kiln_loot::fx`] and carried out here (an ignition, a mob effect, damage, an impulse, item wear) or by the level
//! ([`crate::enchanting::level_action`]: sounds, particles, summoned entities, blocks, explosions).
//!
//! Vanilla: `EnchantmentHelper.tickEffects` / `runLocationChangedEffects` / `stopLocationBasedEffects`, `Enchantment.tick` /
//! `runLocationChangedEffects` / `stopLocationBasedEffects`, `EnchantmentLocationBasedEffect`.

use super::attributes::{Attr, Op};
use super::{CHEST, FEET, HEAD, LEGS, MAINHAND, MobData, OFFHAND};
use crate::enchanting::{self, LevelProbe};
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::BlockPos;
use kiln_item::component::{AttributeOperation, EquipmentSlot};
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::LegacyRandom;
use kiln_loot::effects::EntityEffect;
use kiln_loot::fx::{Action, Subject};
use kiln_loot::probe::WorldFacts;
use kiln_loot::view::{EntityView, FxContext};

/// The mob's six slots in `EquipmentSlot.VALUES` order (the body and saddle of the mounts that have them carry no enchantments).
const SLOTS: [(EquipmentSlot, usize); 6] = [
    (EquipmentSlot::MainHand, MAINHAND),
    (EquipmentSlot::OffHand, OFFHAND),
    (EquipmentSlot::Feet, FEET),
    (EquipmentSlot::Legs, LEGS),
    (EquipmentSlot::Chest, CHEST),
    (EquipmentSlot::Head, HEAD),
];

/// A `location_changed` effect that is active: the slot (index in [`MobData::equipment`]), the enchantment, and its index among the
/// enchantment's `location_changed` effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Active {
    slot: usize,
    enchantment: i32,
    index: usize,
}

/// What a mob remembers of the enchantment effects running for it.
#[derive(Clone, Debug, Default)]
pub struct Fx {
    /// `LivingEntity.lastPos`.
    last_pos: Option<BlockPos>,
    active: Vec<Active>,
    /// The transient modifiers the active `attribute` effects added.
    mods: Vec<(Active, Attr, String)>,
    /// The equipment at the last look (`lastEquipmentItems`), kept only while something in it is enchanted.
    seen: Option<Box<[ItemStack; 6]>>,
}

/// An effect found to be due, to be resolved and carried out.
struct Due {
    key: Active,
    level: i32,
    effect: EntityEffect,
    /// A `location_changed` effect that has just become active.
    fresh: bool,
}

fn enchantments(stack: &ItemStack) -> Vec<(i32, i32)> {
    if stack.is_empty() {
        return Vec::new();
    }
    stack.get(keys::ENCHANTMENTS).map_or_else(Vec::new, |e| e.0.clone())
}

/// Whether anything the mob wears or holds is enchanted.
fn wearing_enchanted(m: &MobData) -> bool {
    m.equipment.iter().any(|s| !s.is_empty() && s.get(keys::ENCHANTMENTS).is_some_and(|l| !l.is_empty()))
}

/// The mob as the effects' requirements see it.
fn view(e: &Entity, level: &dyn EntityLevel) -> EntityView {
    let p = e.position();
    let at = |b: BlockPos| [b.x, b.y, b.z];
    EntityView {
        type_id: kiln_item::registry::ENTITY_TYPE.id(e.type_name).unwrap_or(-1),
        pos: [p.x, p.y, p.z],
        on_ground: e.on_ground,
        on_fire: e.is_on_fire(),
        has_vehicle: e.vehicle.is_some(),
        in_water: e.is_in_water(),
        fall_distance: e.fall_distance,
        // (`LivingEntity.getKnownMovement`: the delta, for a mob nobody rides.)
        movement: [e.delta.x, e.delta.y, e.delta.z],
        movement_block: Some(at(e.block_pos_below_that_affects_movement(level))),
        on_pos: Some(at(e.on_pos(level, 1.0e-5))),
        tick_count: e.tick_count as i32,
        ..EntityView::default()
    }
}

fn subject(e: &Entity) -> Subject {
    let p = e.position();
    Subject { pos: [p.x, p.y, p.z], rot: [e.y_rot, e.x_rot], width: e.width, height: e.height, movement: [e.delta.x, e.delta.y, e.delta.z], silent: e.silent, living: true }
}

fn attr_of(network_id: i32) -> Option<Attr> {
    let name = kiln_data::builtin_entries("minecraft:attribute").and_then(|e| e.get(usize::try_from(network_id).ok()?).copied())?;
    Attr::by_name(name)
}

fn op_of(op: AttributeOperation) -> Op {
    match op {
        AttributeOperation::AddValue => Op::AddValue,
        AttributeOperation::AddMultipliedBase => Op::AddMultipliedBase,
        AttributeOperation::AddMultipliedTotal => Op::AddMultipliedTotal,
    }
}

/// `EnchantmentHelper.tickEffects`, the start of `LivingEntity.baseTick`: the `tick` effects of the equipment.
pub fn tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !wearing_enchanted(m) {
        return;
    }
    let view = view(e, &*level);
    let at = [e.x(), e.y(), e.z()];
    let mut rng = std::mem::replace(&mut m.brain_random, LegacyRandom::new(0));
    let mut due: Vec<Due> = Vec::new();
    let ran = enchanting::with_loot(|loot| {
        let probe = LevelProbe(&*level);
        let facts = WorldFacts { probe: &probe, dim: "", raining: level.is_raining(), thundering: level.is_thundering() };
        for (slot, index) in SLOTS {
            for (id, lvl) in enchantments(&m.equipment[index]) {
                let Some(en) = loot.enchantment(id) else { continue };
                if en.effects.tick.is_empty() || !en.matching_slot(slot) {
                    continue;
                }
                for (i, ce) in en.effects.tick.iter().enumerate() {
                    let ctx = FxContext::entity(lvl, &view, at, Some(facts));
                    if ce.matches(loot, &ctx, &mut rng) {
                        due.push(Due { key: Active { slot: index, enchantment: id, index: i }, level: lvl, effect: ce.effect.clone(), fresh: false });
                    }
                }
            }
        }
    });
    m.brain_random = rng;
    if ran.is_some() {
        carry_out(e, m, level, due);
    }
}

/// `LivingEntity.baseTick`: the block position changed since the last tick.
pub fn block_changed(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let here = BlockPos::containing(e.x(), e.y(), e.z());
    let fx = m.fx.get_or_insert_with(Default::default);
    if fx.last_pos == Some(here) {
        return;
    }
    fx.last_pos = Some(here);
    changed_location(e, m, level);
}

/// `LivingEntity.checkFallDamage`: a landing runs them too.
pub fn landed(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    changed_location(e, m, level);
}

/// `EnchantmentHelper.runLocationChangedEffects(level, entity)`: every slot's equipment.
fn changed_location(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !wearing_enchanted(m) && m.fx.as_ref().is_none_or(|f| f.active.is_empty()) {
        return;
    }
    for (slot, index) in SLOTS {
        location_slot(e, m, level, index, slot);
    }
}

/// `EnchantmentHelper.runLocationChangedEffects(level, stack, entity, slot)`.
fn location_slot(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, index: usize, slot: EquipmentSlot) {
    let enchants = enchantments(&m.equipment[index]);
    if enchants.is_empty() {
        return;
    }
    let view = view(e, &*level);
    let at = [e.x(), e.y(), e.z()];
    let mut fx = m.fx.take().unwrap_or_default();
    let mut rng = std::mem::replace(&mut m.brain_random, LegacyRandom::new(0));
    let mut due: Vec<Due> = Vec::new();
    let mut stopped: Vec<Active> = Vec::new();
    let ran = enchanting::with_loot(|loot| {
        let probe = LevelProbe(&*level);
        let facts = WorldFacts { probe: &probe, dim: "", raining: level.is_raining(), thundering: level.is_thundering() };
        for (id, lvl) in enchants {
            let Some(en) = loot.enchantment(id) else { continue };
            if !en.matching_slot(slot) {
                // (`Enchantment.runLocationChangedEffects`: it no longer applies in this slot, so everything of it that was active stops.)
                let gone: Vec<Active> = fx.active.iter().filter(|k| k.slot == index && k.enchantment == id).copied().collect();
                fx.active.retain(|k| !(k.slot == index && k.enchantment == id));
                stopped.extend(gone);
                continue;
            }
            for (i, ce) in en.effects.location_changed.iter().enumerate() {
                let key = Active { slot: index, enchantment: id, index: i };
                let was_active = fx.active.contains(&key);
                let ctx = FxContext { active: Some(was_active), ..FxContext::entity(lvl, &view, at, Some(facts)) };
                if ce.matches(loot, &ctx, &mut rng) {
                    if !was_active {
                        fx.active.push(key);
                    }
                    due.push(Due { key, level: lvl, effect: ce.effect.clone(), fresh: !was_active });
                } else if was_active {
                    fx.active.retain(|k| *k != key);
                    stopped.push(key);
                }
            }
        }
    });
    m.brain_random = rng;
    m.fx = Some(fx);
    if ran.is_none() {
        return;
    }
    for key in stopped {
        deactivate(m, key);
    }
    carry_out(e, m, level, due);
}

/// `onDeactivated`: the modifiers of the effect go.
fn deactivate(m: &mut MobData, key: Active) {
    let Some(fx) = m.fx.as_mut() else { return };
    let mods: Vec<_> = fx.mods.iter().filter(|(k, _, _)| *k == key).cloned().collect();
    fx.mods.retain(|(k, _, _)| *k != key);
    for (_, attr, id) in mods {
        m.attrs.remove_modifier(attr, &id);
    }
}

/// `EnchantmentHelper.stopLocationBasedEffects(stack, entity, slot)`: everything active in the slot stops.
fn stop_slot(m: &mut MobData, index: usize) {
    let Some(fx) = m.fx.as_mut() else { return };
    let stopped: Vec<Active> = fx.active.iter().filter(|k| k.slot == index).copied().collect();
    fx.active.retain(|k| k.slot != index);
    for key in stopped {
        deactivate(m, key);
    }
}

/// `LivingEntity.collectEquipmentChanges`: the pieces of equipment that changed since the last tick stop their effects, then the new ones start.
pub fn equipment_changed(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let enchanted = wearing_enchanted(m);
    if !enchanted && m.fx.as_ref().is_none_or(|f| f.seen.is_none() && f.active.is_empty()) {
        return;
    }
    let fx = m.fx.get_or_insert_with(Default::default);
    let before = fx.seen.take();
    let changed: Vec<(EquipmentSlot, usize)> = SLOTS
        .into_iter()
        .filter(|&(_, i)| match &before {
            Some(b) => b[i] != m.equipment[i],
            None => !m.equipment[i].is_empty(),
        })
        .collect();
    if enchanted {
        m.fx.as_mut().expect("fx").seen = Some(Box::new(std::array::from_fn(|i| m.equipment[i].clone())));
    }
    for &(_, i) in &changed {
        stop_slot(m, i);
    }
    for (slot, i) in changed {
        let stack = &m.equipment[i];
        // (Empty and broken items do not start anything.)
        if stack.is_empty() || (stack.is_damageable_item() && stack.damage() >= stack.max_damage()) {
            continue;
        }
        location_slot(e, m, level, i, slot);
    }
}

/// Resolves and carries out the effects found due.
fn carry_out(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, due: Vec<Due>) {
    for d in due {
        carry_out_one(e, m, level, &d.effect, &d);
    }
}

fn carry_out_one(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, effect: &EntityEffect, d: &Due) {
    match effect {
        // `EnchantmentAttributeEffect.onChangedBlock`: a modifier while active (`location_changed` only).
        EntityEffect::Attribute(a) => {
            if d.fresh
                && let Some(attr) = attr_of(a.attribute)
                && m.attrs.get(attr).is_some()
            {
                let slot_name = SLOTS.iter().find(|(_, i)| *i == d.key.slot).map_or("mainhand", |(s, _)| s.name());
                let (id, amount) = a.modifier(d.level, slot_name);
                m.attrs.set_transient_modifier(attr, &id, amount, op_of(a.operation));
                if let Some(fx) = m.fx.as_mut() {
                    fx.mods.retain(|(_, at, i)| !(*at == attr && *i == id));
                    fx.mods.push((d.key, attr, id));
                }
            }
        }
        EntityEffect::AllOf(list) => {
            for inner in list {
                carry_out_one(e, m, level, inner, d);
            }
        }
        other => {
            let at = [e.x(), e.y(), e.z()];
            let mut actions = Vec::new();
            let mut level_rng = std::mem::replace(&mut m.brain_random, LegacyRandom::new(0));
            kiln_loot::fx::resolve(other, d.level, at, &subject(e), &mut e.random, &mut level_rng, &mut actions);
            m.brain_random = level_rng;
            for action in actions {
                apply(e, m, level, d.key.slot, action);
            }
        }
    }
}

/// What one resolved action does to the mob that owns the enchanted equipment.
fn apply(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, slot: usize, action: Action) {
    match action {
        Action::Ignite { seconds } => e.ignite_for_seconds(seconds),
        Action::MobEffect { effect, duration, amplifier } => {
            if let Some(effect) = crate::effect::Effect::named(effect.as_str(), duration, amplifier) {
                super::effects::add(e, m, level, effect, None);
            }
        }
        Action::Damage { damage_type, amount } => {
            let mut source = super::DamageSource::of(DamageKind::Named(kiln_data::live::leak_str(damage_type.as_str())));
            source.attacker = Some(e.id);
            super::hurt(e, m, level, source, amount);
        }
        Action::ItemDamage { amount } => wear(e, m, level, slot, amount),
        Action::Impulse { delta } => {
            e.delta = e.delta.add(delta[0], delta[1], delta[2]);
            e.needs_sync = true;
        }
        // (A player's food exhaustion.)
        Action::Exhaustion { .. } => {}
        other => enchanting::level_action(level, Some(e.id), m.kind.sound_source(), other),
    }
}

/// `ItemStack.hurtAndBreak(amount, level, entity, slot)` on a mob's equipped item: unbreaking may save it; one that breaks is gone with the
/// equipment-break entity event.
fn wear(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, slot: usize, amount: i32) {
    let stack = &mut m.equipment[slot];
    if !stack.is_damageable_item() || stack.is_empty() {
        return;
    }
    let mut rng = std::mem::replace(&mut m.brain_random, LegacyRandom::new(0));
    let amount = enchanting::with_loot(|loot| if amount > 0 { loot.process_durability_change(stack, &mut rng, amount) } else { amount }).unwrap_or(amount);
    m.brain_random = rng;
    let stack = &mut m.equipment[slot];
    if amount <= 0 {
        return;
    }
    let damage = stack.damage() + amount;
    stack.insert(keys::DAMAGE, damage.clamp(0, stack.max_damage()));
    if damage >= stack.max_damage() {
        stack.shrink(1);
        // `entityEventForEquipmentBreak`.
        let event = match slot {
            MAINHAND => 47,
            OFFHAND => 48,
            HEAD => 49,
            CHEST => 50,
            LEGS => 51,
            _ => 52,
        };
        level.emit(Event::EntityEvent { entity: e.id, event });
    }
}
