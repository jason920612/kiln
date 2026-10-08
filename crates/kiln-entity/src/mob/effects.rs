//! Mob effects on mobs: `LivingEntity`'s effect map (`activeEffects`) with `addEffect`,
//! `canBeAffected`, `removeEffect`, `removeAllEffects`, `tickEffects`, the attribute modifiers
//! of the active effects, the instantaneous effects (instant health and damage, inverted for
//! the undead) and what happens to a bearer that is hurt or dies (infested, oozing, weaving).
//!
//! The instance logic (merging, hidden effects, counting down) is shared with players in
//! [`crate::effect`]. The effects other code reads live where they act: fire resistance and
//! resistance in `hurt`, water breathing in the air supply, jump boost in the jump, slow
//! falling and levitation in travel, dolphin's grace in water, invisibility in targeting
//! (`Living::invisible`) and in the entity data, glowing in the entity data.

use super::attributes::{Attr, Op};
use super::{DamageSource, MobData, MobKind};
use crate::effect::{self, Effect, Kind, ids};
use crate::entity::Entity;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use kiln_item::component::AttributeOperation;
use kiln_javamath::random::RandomSource;

pub fn has(m: &MobData, id: i32) -> bool {
    m.effects.contains_key(&id)
}

/// The active `id` effect's amplifier.
pub fn amplifier(m: &MobData, id: i32) -> Option<i32> {
    m.effects.get(&id).map(|e| e.amplifier)
}

/// `hasEffect` by `minecraft:mob_effect` name.
pub fn has_named(m: &MobData, name: &str) -> bool {
    effect::effect_id(name).is_some_and(|id| has(m, id))
}

/// `isInvertedHealAndHarm`: `#minecraft:inverted_healing_and_harm` (the undead).
pub fn inverted_heal_and_harm(kind: MobKind) -> bool {
    super::entity_type_tag(kind.type_name(), "minecraft:inverted_healing_and_harm")
}

/// `canBeAffected`: the type tags (infested, oozing, poison and regeneration immunities), then
/// the type's own override (spiders shrug off poison, ...).
pub fn can_be_affected(m: &MobData, e: &Effect) -> bool {
    let name = m.kind.type_name();
    let base = if super::entity_type_tag(name, "minecraft:immune_to_infested") {
        e.id != ids::infested()
    } else if super::entity_type_tag(name, "minecraft:immune_to_oozing") {
        e.id != ids::oozing()
    } else if super::entity_type_tag(name, "minecraft:ignores_poison_and_regen") {
        e.id != ids::regeneration() && e.id != ids::poison()
    } else {
        true
    };
    match m.kind.ext() {
        Some(k) => k.can_be_affected(m, e, base),
        None => base && !(matches!(m.kind, MobKind::Spider | MobKind::CaveSpider) && e.id == ids::poison()),
    }
}

/// `LivingEntity.addEffect(effect, source)`: returns whether it changed anything.
pub fn add(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, new: Effect, source: Option<i32>) -> bool {
    let _ = source;
    let sound = new.effect_type().and_then(|t| t.sound_on_added).filter(|_| !m.effects.contains_key(&new.id));
    let changed = add_quiet(m, new);
    // `MobEffectInstance.onEffectAdded` -> `MobEffect.onEffectAdded`: the effect's sound.
    if changed && let Some(sound) = sound {
        level.emit(Event::Sound { pos: e.position(), sound, source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
    }
    changed
}

/// `addEffect` without the level (no effect sound): loading and constructors.
pub fn add_quiet(m: &mut MobData, new: Effect) -> bool {
    if !can_be_affected(m, &new) {
        return false;
    }
    let changed = match m.effects.get_mut(&new.id) {
        None => {
            m.effects.insert(new.id, new.clone());
            on_added(m, &new);
            true
        }
        Some(old) => {
            if old.update(&new) {
                let updated = old.clone();
                on_updated(m, &updated, true);
                true
            } else {
                false
            }
        }
    };
    // `onEffectStarted` with the new instance, whether or not it changed anything.
    if new.kind() == Kind::Absorption {
        let amount = m.absorption.max((4 * (1 + new.amplifier)) as f32);
        set_absorption(m, amount);
    }
    changed
}

/// `forceAddEffect`: replaces the active instance outright.
pub fn force_add(m: &mut MobData, new: Effect) {
    if !can_be_affected(m, &new) {
        return;
    }
    let old = m.effects.insert(new.id, new.clone());
    if old.is_none() {
        on_added(m, &new);
    } else {
        on_updated(m, &new, true);
    }
}

/// `removeEffect`.
pub fn remove(m: &mut MobData, id: i32) -> bool {
    match m.effects.remove(&id) {
        Some(e) => {
            on_removed(m, &[e]);
            true
        }
        None => false,
    }
}

/// `removeAllEffects` (milk, `/effect clear`).
pub fn remove_all(m: &mut MobData) -> bool {
    if m.effects.is_empty() {
        return false;
    }
    let all: Vec<Effect> = std::mem::take(&mut m.effects).into_values().collect();
    on_removed(m, &all);
    true
}

fn attr_of(name: &str) -> Option<Attr> {
    Attr::by_name(name)
}

fn op(o: AttributeOperation) -> Op {
    match o {
        AttributeOperation::AddValue => Op::AddValue,
        AttributeOperation::AddMultipliedBase => Op::AddMultipliedBase,
        AttributeOperation::AddMultipliedTotal => Op::AddMultipliedTotal,
    }
}

/// `MobEffect.addAttributeModifiers` (for the attributes the mob has).
fn add_modifiers(m: &mut MobData, e: &Effect) {
    if let Some(md) = e.effect_type().and_then(|t| t.modifier)
        && let Some(a) = attr_of(md.attr)
    {
        m.attrs.remove_modifier(a, md.id);
        m.attrs.set_modifier(a, md.id, md.amount_at(e.amplifier), op(md.op));
    }
}

fn remove_modifiers(m: &mut MobData, id: i32) {
    if let Some(md) = effect::effect_type(id).and_then(|t| t.modifier)
        && let Some(a) = attr_of(md.attr)
    {
        m.attrs.remove_modifier(a, md.id);
    }
}

/// `onEffectAdded`.
fn on_added(m: &mut MobData, e: &Effect) {
    add_modifiers(m, e);
}

/// `onEffectUpdated`: `refresh` re-applies the attribute modifiers.
fn on_updated(m: &mut MobData, e: &Effect, refresh: bool) {
    if refresh {
        remove_modifiers(m, e.id);
        add_modifiers(m, e);
        refresh_dirty_attributes(m);
    }
}

/// `onEffectsRemoved`.
fn on_removed(m: &mut MobData, removed: &[Effect]) {
    for e in removed {
        remove_modifiers(m, e.id);
    }
    refresh_dirty_attributes(m);
}

/// `refreshDirtyAttributes`: health and absorption above their new maximum drop to it.
fn refresh_dirty_attributes(m: &mut MobData) {
    let max = m.max_health();
    if m.health > max {
        m.set_health(max);
    }
    let max = m.attrs.value(Attr::MaxAbsorption) as f32;
    if m.absorption > max {
        set_absorption(m, max);
    }
}

/// `setAbsorptionAmount`: clamped to the maximum absorption.
pub fn set_absorption(m: &mut MobData, amount: f32) {
    let max = m.attrs.value(Attr::MaxAbsorption) as f32;
    m.absorption = amount.clamp(0.0, max);
}

/// `LivingEntity.heal`.
pub fn heal(m: &mut MobData, amount: f32) {
    if m.health > 0.0 {
        let h = m.health + amount;
        m.set_health(h);
    }
}

/// `LivingEntity.tickEffects` (server side), at the end of `baseTick`.
pub fn tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if m.effects.is_empty() {
        return;
    }
    let ids: Vec<i32> = m.effects.keys().copied().collect();
    for id in ids {
        let Some(fx) = m.effects.get(&id) else { continue };
        let keep = if !fx.has_remaining_duration() {
            false
        } else {
            let (kind, amp) = (fx.kind(), fx.amplifier);
            let tick = if fx.is_infinite() { e.tick_count } else { fx.duration };
            if kind.applies_this_tick(tick, amp) && !apply_tick(e, m, level, kind, amp) {
                false
            } else {
                // The tick may have removed it (a death clears nothing, but a type's hook might).
                let Some(fx) = m.effects.get_mut(&id) else { continue };
                fx.tick_down();
                if fx.downgrade() {
                    let fx = fx.clone();
                    on_updated(m, &fx, true);
                }
                m.effects.get(&id).is_some_and(Effect::has_remaining_duration)
            }
        };
        if !keep && let Some(fx) = m.effects.remove(&id) {
            on_removed(m, &[fx]);
        }
    }
}

/// `MobEffect.applyEffectTick` on a mob: false removes the effect.
fn apply_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, kind: Kind, amp: i32) -> bool {
    match kind {
        Kind::Regeneration => {
            if m.health < m.max_health() {
                heal(m, 1.0);
            }
        }
        Kind::Poison => {
            if m.health > 1.0 {
                super::hurt(e, m, level, DamageSource::of(DamageKind::Magic), 1.0);
            }
        }
        Kind::Wither => {
            super::hurt(e, m, level, DamageSource::of(DamageKind::Wither), 1.0);
        }
        Kind::Absorption => return m.absorption > 0.0,
        Kind::HealOrHarm { harm } => {
            if harm == inverted_heal_and_harm(m.kind) {
                heal(m, 4i32.wrapping_shl(amp as u32).max(0) as f32);
            } else {
                super::hurt(e, m, level, DamageSource::of(DamageKind::Magic), 6i32.wrapping_shl(amp as u32) as f32);
            }
        }
        // Hunger and saturation feed players only; the omens act on players.
        Kind::Plain | Kind::Hunger | Kind::Saturation | Kind::BadOmen | Kind::RaidOmen => {}
        Kind::Infested | Kind::Oozing | Kind::Weaving | Kind::WindCharged => {}
    }
    true
}

/// `MobEffect.applyInstantaneousEffect(level, source, owner, mob, amplifier, scale)`: `source`
/// is the potion (or cloud) that carried it, `owner` who threw it.
pub fn apply_instantaneous(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, fx: &Effect, source: Option<(i32, Vec3)>, owner: Option<i32>, owner_is_player: bool, scale: f64) {
    match fx.kind() {
        Kind::HealOrHarm { harm } => {
            if harm == inverted_heal_and_harm(m.kind) {
                let amount = (scale * 4i32.wrapping_shl(fx.amplifier as u32) as f64 + 0.5) as i32;
                heal(m, amount as f32);
            } else {
                let amount = (scale * 6i32.wrapping_shl(fx.amplifier as u32) as f64 + 0.5) as i32;
                let src = match source {
                    None => DamageSource::of(DamageKind::Magic),
                    // `indirectMagic(source, owner)`: caused by the thrower, dealt by the potion.
                    Some((id, pos)) => DamageSource {
                        kind: DamageKind::IndirectMagic,
                        attacker: owner.or(Some(id)),
                        direct: Some(id),
                        pos: Some(pos),
                        attacker_is_player: owner_is_player,
                    },
                };
                super::hurt(e, m, level, src, amount as f32);
            }
        }
        kind => {
            apply_tick(e, m, level, kind, fx.amplifier);
        }
    }
}

/// `PotionContents.applyToLivingEntity` / `Consumable` effects: instantaneous effects at once,
/// others added.
pub fn apply_potion_effect(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, fx: Effect, source: Option<i32>) {
    if fx.kind().instantaneous() {
        apply_instantaneous(e, m, level, &fx, None, source, false, 1.0);
    } else {
        add(e, m, level, fx, source);
    }
}

/// `onMobHurt` of the active effects after a hit landed: infested bearers let out one or two
/// silverfish 10% of the time, thrown along their look.
pub fn on_hurt(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource, _damage: f32) {
    if !has(m, ids::infested()) {
        return;
    }
    if e.random.next_float() > 0.1 {
        return;
    }
    let count = super::mth::next_int_between(&mut e.random, 1, 2);
    for _ in 0..count {
        // `spawnSilverfish`: the look vector scaled by (0.3, 0.45, 0.3), turned by a random
        // angle within a quarter turn either way.
        let half_pi = std::f32::consts::FRAC_PI_2;
        let angle = e.random.next_float() * (half_pi - -half_pi) + -half_pi;
        let look = e.view_vector();
        let (x, y, z) = (look.x as f32 * 0.3, look.y as f32 * 0.3 * 1.5, look.z as f32 * 0.3);
        // JOML `Vector3f.rotateY`: the cosine from the sine (`Math.cosFromSin`), in floats.
        let sin = kiln_javamath::trig::sin(angle as f64) as f32;
        let cos = ((1.0f32 - sin * sin) as f64).sqrt() as f32;
        let v = Vec3::new((cos * x + sin * z) as f64, y as f64, (-sin * x + cos * z) as f64);
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut fish = super::new(MobKind::Silverfish, id, 0, seed);
        fish.set_pos(Vec3::new(e.x(), e.y() + e.height as f64 / 2.0, e.z()));
        fish.y_rot = level.random().next_float() * 360.0;
        fish.x_rot = 0.0;
        fish.set_old_pos_and_rot();
        fish.delta = v;
        level.add_entity(fish);
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.silverfish.hurt", source: "hostile", volume: 1.0, pitch: 1.0 });
    }
}

/// `triggerOnDeathMobEffects` when a killed mob is removed (`remove(KILLED)`): oozing slimes,
/// weaving cobwebs, then the effects are gone.
pub fn on_killed_removal(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let effects: Vec<Effect> = m.effects.values().cloned().collect();
    for fx in &effects {
        match fx.kind() {
            Kind::Oozing => oozing(e, level),
            Kind::Weaving => weaving(e, level),
            // `WindChargedMobEffect.onMobRemoved`: a wind burst of 3 to 5 at the middle.
            Kind::WindCharged => {
                let at = Vec3::new(e.x(), e.y() + (e.height / 2.0) as f64, e.z());
                let strength = 3.0 + e.random.next_float() * 2.0;
                crate::explosion::explode_with(level, Some(e.id), at, strength, false, crate::explosion::Interaction::TriggerBlock, None, false);
                level.emit(Event::Sound { pos: at, sound: "minecraft:entity.breeze.wind_burst", source: "hostile", volume: 1.0, pitch: 1.0 });
            }
            _ => {}
        }
    }
    m.effects.clear();
}

/// `OozingMobEffect.onMobRemoved`: two size-2 slimes, fewer when slimes already crowd the spot.
fn oozing(e: &mut Entity, level: &mut dyn EntityLevel) {
    let requested = 2;
    let cramming = level.max_entity_cramming();
    let n = if cramming < 1 {
        requested
    } else {
        let area = e.bounding_box().inflate_all(2.0);
        let near = level
            .entities_in(&area, EntityFilter::Any, e.id)
            .into_iter()
            .filter(|&id| level.entity(id).is_some_and(|o| o.type_name == "minecraft:slime"))
            .take(cramming as usize)
            .count() as i32;
        (cramming - near).clamp(0, requested)
    };
    for _ in 0..n {
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut slime = super::new(MobKind::Slime, id, 0, seed);
        let mut sm = super::take(&mut slime);
        super::kinds::slime::set_size(&mut slime, &mut sm, 2, true);
        super::put(&mut slime, sm);
        slime.set_pos(Vec3::new(e.x(), e.y() + 0.5, e.z()));
        slime.y_rot = level.random().next_float() * 360.0;
        slime.set_old_pos_and_rot();
        level.add_entity(slime);
    }
}

/// `WeavingMobEffect.onMobRemoved`: two or three cobwebs on sturdy ground within 15 blocks
/// (`BlockPos.randomInCube(random, 15, pos, 1)`), when mob griefing is on.
fn weaving(e: &mut Entity, level: &mut dyn EntityLevel) {
    if !level.mob_griefing() {
        return;
    }
    let count = super::mth::next_int_between(&mut e.random, 2, 3) as usize;
    let origin = e.block_position();
    let mut chosen: Vec<BlockPos> = Vec::new();
    for _ in 0..15 {
        let p = BlockPos::new(
            origin.x + super::mth::next_int_between(&mut e.random, -1, 1),
            origin.y + super::mth::next_int_between(&mut e.random, -1, 1),
            origin.z + super::mth::next_int_between(&mut e.random, -1, 1),
        );
        let below = p.below();
        if !chosen.contains(&p) && crate::physics::can_be_replaced(level.block(p)) && crate::physics::is_face_sturdy(level.block(below), crate::math::Direction::Up) {
            chosen.push(p);
            if chosen.len() >= count {
                break;
            }
        }
    }
    for p in chosen {
        level.set_block(p, kiln_data::blocks::default_state::COBWEB, 3);
        level.emit(Event::LevelEvent { event: 3018, pos: p, data: 0 });
    }
}

/// `getJumpBoostPower`.
pub fn jump_boost_power(m: &MobData) -> f32 {
    amplifier(m, ids::jump_boost()).map_or(0.0, |a| 0.1 * (a as f32 + 1.0))
}

/// `MobEffectUtil.hasWaterBreathing`.
pub fn has_water_breathing(m: &MobData) -> bool {
    has(m, ids::water_breathing()) || has(m, ids::conduit_power()) || has(m, ids::breath_of_the_nautilus())
}

/// `MobEffectUtil.shouldEffectsRefillAirsupply`.
pub fn effects_refill_air(m: &MobData) -> bool {
    !has(m, ids::breath_of_the_nautilus()) || has(m, ids::water_breathing()) || has(m, ids::conduit_power())
}

/// `getDamageAfterMagicAbsorb`'s resistance: 20% less damage per level.
pub fn resist(m: &MobData, source: &DamageSource, damage: f32) -> f32 {
    if source.kind.is_tag("minecraft:bypasses_effects") {
        return damage;
    }
    let mut damage = damage;
    if let Some(a) = amplifier(m, ids::resistance())
        && !source.kind.is_tag("minecraft:bypasses_resistance")
    {
        let absorb = 25 - (a + 1) * 5;
        let v = damage * absorb as f32;
        damage = (v / 25.0).max(0.0);
    }
    damage
}

/// `DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID` (`updateSynchronizedMobEffectParticles`;
/// both stay as they were when the last effect ends: the particles empty, the ambience flag
/// unchanged, which a new mob starts false).
pub fn particles(m: &MobData) -> (Vec<kiln_proto::packets::entity::metadata::Particle>, bool) {
    (effect::particles(&m.effects), !m.effects.is_empty() && effect::all_ambient(&m.effects))
}

/// `Entity.isInvisible`: the shared invisible flag, which the effects set when the entity's data
/// is synchronised (`updateDataBeforeSync`, at the end of the tick: see
/// [`crate::mob::update_data_before_sync`]), not when the effect is added.
pub fn invisible(m: &MobData) -> bool {
    m.invisible_flag
}

/// Whether the effects make the mob invisible (`updateInvisibilityStatus`).
pub fn invisibility_effect(m: &MobData) -> bool {
    has(m, ids::invisibility())
}

/// `isCurrentlyGlowing` from the glowing effect.
pub fn glowing(m: &MobData) -> bool {
    has(m, ids::glowing())
}

/// `LivingEntity.addEffect` on entity `id` of the level (not the one ticking): a mob takes it,
/// other entities do not. For [`EntityLevel::add_effect`] implementations.
pub fn add_to_entity(level: &mut dyn EntityLevel, id: i32, fx: Effect, source: Option<i32>) -> bool {
    let Some(o) = level.entity_mut(id) else { return false };
    if super::data(o).is_none() {
        return false;
    }
    let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", i32::MIN, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0));
    let mut m = super::take(&mut o2);
    let r = add(&mut o2, &mut m, level, fx, source);
    super::put(&mut o2, m);
    if let Some(slot) = level.entity_mut(id) {
        *slot = o2;
    }
    r
}

/// `applyInstantaneousEffect` on entity `id` of the level (a splash potion's instant health or
/// harm).
pub fn apply_instantaneous_to_entity(level: &mut dyn EntityLevel, id: i32, fx: &Effect, source: Option<(i32, Vec3)>, owner: Option<i32>, owner_is_player: bool, scale: f64) {
    let Some(o) = level.entity_mut(id) else { return };
    if super::data(o).is_none() {
        return;
    }
    let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", i32::MIN, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0));
    let mut m = super::take(&mut o2);
    apply_instantaneous(&mut o2, &mut m, level, fx, source, owner, owner_is_player, scale);
    super::put(&mut o2, m);
    if let Some(slot) = level.entity_mut(id) {
        *slot = o2;
    }
}
