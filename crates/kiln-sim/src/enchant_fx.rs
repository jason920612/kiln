//! Enchantment effects that run by themselves for a player's equipment: `tick` every tick, and the
//! location-based `location_changed` effects, which vanilla runs whenever the wearer's block
//! position changes (`LivingEntity.baseTick` -> `onChangedBlock`), on landing (`checkFallDamage`) and
//! when a piece of equipment is put on, taken off, changed or broken
//! (`collectEquipmentChanges`, `onEquippedItemBroken`).
//!
//! A `location_changed` effect is *active* while its requirements hold; the requirements can ask
//! (`enchantment_active_check`) whether it was active before. Each (slot, enchantment, effect) that
//! is active is remembered ([`ActiveEffect`]); `attribute` effects add a transient attribute modifier
//! when they become active and remove it when they stop (the item left the slot, or the
//! requirements no longer hold); every other effect runs each time the requirements hold.
//! The effects themselves are resolved by [`kiln_loot::fx`]; what they do to a player is
//! [`Player::apply_fx`] and, for the ones that need the level's blocks or entities, [`run_level_jobs`].
//!
//! Vanilla: `EnchantmentHelper.tickEffects` / `runLocationChangedEffects` /
//! `stopLocationBasedEffects`, `Enchantment.tick` / `runLocationChangedEffects` /
//! `stopLocationBasedEffects`, `EnchantmentLocationBasedEffect`.

use crate::Player;
use kiln_loot::probe::{WorldFacts, WorldProbe};
use crate::blocks::RegionLevel;
use crate::health::DamageCtx;
use crate::hazards::BlockAt;
use kiln_blocks::{Effect, Level};
use kiln_blocks::BlockPos;
use kiln_item::component::{AttributeOperation, EquipmentSlot};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_loot::effects::{EQUIPMENT_ORDER, EntityEffect};
use kiln_loot::fx::{Action, Subject};
use kiln_loot::json::Json;
use kiln_loot::view::{EntityView, FxContext};

/// What the effects see of the world around the entity.
pub(crate) struct FxWorld<'a> {
    pub block: BlockAt<'a>,
    pub probe: &'a dyn WorldProbe,
    /// `minecraft:overworld` and the like.
    pub dim: &'static str,
    pub raining: bool,
    pub thundering: bool,
}

/// A `location_changed` effect that is active: the slot and enchantment it belongs to and its
/// index among the enchantment's `location_changed` effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveEffect {
    pub slot: EquipmentSlot,
    pub enchantment: i32,
    pub index: usize,
}

/// A transient attribute modifier an active `attribute` effect added.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocationModifier {
    pub key: ActiveEffect,
    /// `minecraft:attribute` network id.
    pub attribute: i32,
    pub id: String,
    pub amount: f64,
    pub operation: AttributeOperation,
}

/// A world effect a player's enchantments asked for, run when the region does the blocks (see
/// [`run_level_jobs`]).
#[derive(Debug, Clone)]
pub(crate) struct LevelJob {
    pub action: Action,
}

impl FxWorld<'_> {
    fn facts(&self) -> WorldFacts<'_> {
        WorldFacts { probe: self.probe, dim: self.dim, raining: self.raining, thundering: self.thundering }
    }
}

impl Player {
    /// The equipment's enchantments `(id, level)` in `slot`, in the stored order (nothing for an empty stack).
    fn slot_enchantments(&self, slot: EquipmentSlot) -> Vec<(i32, i32)> {
        let stack = self.worn(slot);
        if stack.is_empty() {
            return Vec::new();
        }
        stack.get(kiln_item::keys::ENCHANTMENTS).map_or_else(Vec::new, |e| e.0.clone())
    }

    /// The player as the effects' requirements see it, with the blocks it stands on.
    fn fx_view(&self, w: &FxWorld) -> EntityView {
        let on = |p: kiln_entity::math::BlockPos| Some((w.block)(p));
        let at = |p: kiln_entity::math::BlockPos| [p.x, p.y, p.z];
        let mut v = self.view_in(w.block);
        v.on_pos = Some(at(self.on_pos(&on, 1.0e-5)));
        v.movement_block = Some(at(self.on_pos(&on, 0.500001)));
        v
    }

    pub(crate) fn fx_subject(&self) -> Subject {
        let (width, height, _) = self.dimensions();
        Subject { pos: self.pos, rot: self.rot, width, height, movement: self.move_delta, silent: false, living: true }
    }

    /// `EnchantmentHelper.tickEffects`, the start of `LivingEntity.baseTick`: the `tick` effects of
    /// the equipment, with the enchantments of each slot in the stored order.
    pub(crate) fn tick_enchant_effects(&mut self, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        let Some(loot) = self.loot.clone() else { return };
        let mut view: Option<EntityView> = None;
        for slot in EQUIPMENT_ORDER {
            for (id, level) in self.slot_enchantments(slot) {
                let Some(e) = loot.enchantment(id) else { continue };
                if e.effects.tick.is_empty() || !e.matching_slot(slot) {
                    continue;
                }
                let view = view.get_or_insert_with(|| self.fx_view(w)).clone();
                for ce in &e.effects.tick {
                    let fx = FxContext::entity(level, &view, self.pos, Some(w.facts()));
                    if ce.matches(&loot, &fx, &mut self.level_rng) {
                        self.run_entity_effect(&ce.effect, level, slot, w, ctx);
                    }
                }
            }
        }
    }

    /// `LivingEntity.baseTick`: the block position changed since the last tick.
    pub(crate) fn tick_location_changed(&mut self, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        let now = [self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32];
        if self.loc_last_pos != Some(now) {
            self.loc_last_pos = Some(now);
            self.run_location_changed(w, ctx);
        }
    }

    /// `LivingEntity.checkFallDamage`: a landing runs them too.
    pub(crate) fn landed_location_changed(&mut self, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        if std::mem::take(&mut self.loc_landed) {
            self.run_location_changed(w, ctx);
        }
    }

    /// `EnchantmentHelper.runLocationChangedEffects(level, entity)`: every slot's equipment.
    fn run_location_changed(&mut self, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        for slot in EQUIPMENT_ORDER {
            self.run_location_changed_slot(slot, w, ctx);
        }
    }

    /// `EnchantmentHelper.runLocationChangedEffects(level, stack, entity, slot)`.
    pub(crate) fn run_location_changed_slot(&mut self, slot: EquipmentSlot, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        let Some(loot) = self.loot.clone() else { return };
        let mut view: Option<EntityView> = None;
        for (id, level) in self.slot_enchantments(slot) {
            let Some(e) = loot.enchantment(id) else { continue };
            if e.effects.location_changed.is_empty() || !e.matching_slot(slot) {
                continue;
            }
            let view = view.get_or_insert_with(|| self.fx_view(w)).clone();
            for (index, ce) in e.effects.location_changed.iter().enumerate() {
                let key = ActiveEffect { slot, enchantment: id, index };
                let was_active = self.loc_active.contains(&key);
                let fx = FxContext { active: Some(was_active), ..FxContext::entity(level, &view, self.pos, Some(w.facts())) };
                if ce.matches(&loot, &fx, &mut self.level_rng) {
                    if !was_active {
                        self.loc_active.push(key);
                    }
                    self.run_location_effect(&ce.effect, key, level, !was_active, w, ctx);
                } else if was_active {
                    self.loc_active.retain(|k| *k != key);
                    self.deactivate(key);
                }
            }
        }
    }

    /// `EnchantmentLocationBasedEffect.onChangedBlock`: an `attribute` effect adds its modifier when it
    /// becomes active (`apply_transient`); `all_of` passes it on; the entity effects just run.
    fn run_location_effect(&mut self, effect: &EntityEffect, key: ActiveEffect, level: i32, apply_transient: bool, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        match effect {
            EntityEffect::Attribute(a) => {
                if apply_transient {
                    let (id, amount) = a.modifier(level, key.slot.name());
                    self.loc_modifiers.retain(|m| !(m.attribute == a.attribute && m.id == id));
                    self.loc_modifiers.push(LocationModifier { key, attribute: a.attribute, id, amount, operation: a.operation });
                    self.attributes_dirty = true;
                }
            }
            EntityEffect::AllOf(list) => {
                for inner in list {
                    self.run_location_effect(inner, key, level, apply_transient, w, ctx);
                }
            }
            other => self.run_entity_effect(other, level, key.slot, w, ctx),
        }
    }

    /// `onDeactivated`: the modifiers of the effect go.
    fn deactivate(&mut self, key: ActiveEffect) {
        let before = self.loc_modifiers.len();
        self.loc_modifiers.retain(|m| m.key != key);
        if self.loc_modifiers.len() != before {
            self.attributes_dirty = true;
        }
    }

    /// `EnchantmentHelper.stopLocationBasedEffects(stack, entity, slot)`: everything active in the slot stops.
    pub(crate) fn stop_location_effects(&mut self, slot: EquipmentSlot) {
        let stopped: Vec<ActiveEffect> = self.loc_active.iter().filter(|k| k.slot == slot).copied().collect();
        self.loc_active.retain(|k| k.slot != slot);
        for key in stopped {
            self.deactivate(key);
        }
    }

    /// `LivingEntity.collectEquipmentChanges`: the pieces of equipment that changed since the last tick
    /// stop their effects, then the new ones start (`runLocationChangedEffects` for the slot).
    pub(crate) fn equipment_changed_effects(&mut self, changed: &[EquipmentSlot], w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        for &slot in changed {
            self.stop_location_effects(slot);
        }
        for &slot in changed {
            let stack = self.worn(slot);
            // (Empty and broken items do not start anything.)
            if stack.is_empty() || (stack.is_damageable_item() && stack.damage() >= stack.max_damage()) {
                continue;
            }
            self.run_location_changed_slot(slot, w, ctx);
        }
    }

    /// `Enchantment.apply` of one entity effect to the player: resolved, then done.
    pub(crate) fn run_entity_effect(&mut self, effect: &EntityEffect, level: i32, slot: EquipmentSlot, w: &FxWorld, ctx: &mut DamageCtx<'_>) {
        let subject = self.fx_subject();
        let mut actions = Vec::new();
        let mut entity_rng = std::mem::replace(&mut self.entity_rng, LegacyRandom::new(0));
        kiln_loot::fx::resolve(effect, level, self.pos, &subject, &mut entity_rng, &mut self.level_rng, &mut actions);
        self.entity_rng = entity_rng;
        let _ = w;
        for action in actions {
            self.apply_fx(action, slot, ctx);
        }
    }

    /// What one resolved action does to the player (the world's part is queued for the region).
    pub(crate) fn apply_fx(&mut self, action: Action, slot: EquipmentSlot, ctx: &mut DamageCtx<'_>) {
        match action {
            Action::Ignite { seconds } => self.ignite_for_seconds(seconds),
            Action::MobEffect { effect, duration, amplifier } => {
                if let Some(id) = crate::effects::effect_id(effect.as_str()) {
                    self.add_effect(crate::effects::Effect::simple(id, duration, amplifier));
                }
            }
            Action::Damage { damage_type, amount } => {
                let source = crate::health::Source {
                    cause: crate::health::Cause::Other(crate::health::static_damage_type(damage_type.as_str())),
                    attacker: Some(self.as_attacker()),
                    direct: None,
                    weapon: None,
                    position: None,
                };
                self.hurt(amount, &source, ctx);
            }
            Action::ItemDamage { amount } => {
                let stack = self.inv.equipped(slot);
                if stack.get(kiln_item::keys::MAX_DAMAGE).is_some() && stack.get(kiln_item::keys::DAMAGE).is_some() {
                    let mut rng = std::mem::replace(&mut self.level_rng, LegacyRandom::new(0));
                    self.hurt_and_break(slot, amount, Some(&mut rng));
                    self.level_rng = rng;
                }
            }
            Action::Exhaustion { amount } => self.exhaust(amount),
            Action::Impulse { delta } => {
                self.vel = [self.vel[0] + delta[0], self.vel[1] + delta[1], self.vel[2] + delta[2]];
                self.send(kiln_proto::packets::entity::set_entity_motion(self.entity_id, self.vel));
                self.sync_velocity = false;
            }
            Action::Sound { sound, volume, pitch, .. } => {
                self.sound_for_all(sound.as_str(), kiln_proto::packets::world_fx::SoundSource::Players, volume, pitch);
            }
            Action::Particles { particle, has_options, pos, velocity, speed } => {
                if has_options {
                    return;
                }
                use kiln_proto::packets::world_fx;
                let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", particle.as_str()) else { return };
                let pkt = world_fx::level_particles(&world_fx::LevelParticles {
                    particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
                    override_limiter: false,
                    always_show: false,
                    pos,
                    offset: [velocity[0] as f32, velocity[1] as f32, velocity[2] as f32],
                    max_speed: [speed as f32; 3],
                    count: 0,
                    randomization: world_fx::ParticleRandomization::Alternative,
                });
                self.send(pkt.clone());
                self.pending_sounds.push(pkt);
            }
            action @ (Action::Summon { .. } | Action::Function { .. } | Action::Level { .. }) => self.fx_jobs.push(LevelJob { action }),
        }
    }
}

/// The world part of the effects the players queued: blocks replaced, entities summoned, functions
/// run. Runs with the region's blocks (after the players' ticks, in canonical player order).
pub(crate) fn run_level_jobs(level: &mut RegionLevel, p: &mut Player, difficulty: u8, spawns: &mut Vec<crate::entities::Spawn>) {
    for job in std::mem::take(&mut p.fx_jobs) {
        match job.action {
            Action::Level { effect, level: lvl, at } => run_level_effect(level, p, &effect, lvl, at),
            Action::Summon { entity, at, join_team: _ } => summon(level, p, &entity, at, difficulty, spawns),
            Action::Function { function, at } => p.function_requests.push((function.as_str().to_owned(), at)),
            _ => {}
        }
    }
}

/// `SummonEntityEffect.apply`: the entity at the block, `snapTo` the exact position; a lightning bolt credits a player.
fn summon(level: &mut RegionLevel, p: &Player, entity: &kiln_item::Identifier, at: [f64; 3], difficulty: u8, spawns: &mut Vec<crate::entities::Spawn>) {
    use crate::entities::{Body, Spawn};
    let block = BlockPos::new(at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    // `Level.isInSpawnableBounds`.
    if !(-30000000..30000000).contains(&block.x) || !(-30000000..30000000).contains(&block.z) || block.y < level.env.min_y - 20_000_000 || block.y > 20_000_000 {
        return;
    }
    let seed = crate::container::pos_random(level, BlockPos::new(p.entity_id, p.tick_count, 11), 0x7375_6d6d).next_long();
    if entity.as_str() == "minecraft:lightning_bolt" {
        let e = kiln_entity::ext_entity::lightning::channeled(0, 0, kiln_entity::math::Vec3::new(at[0], at[1], at[2]), p.entity_id, seed);
        spawns.push(Spawn { kind: &kiln_data::entities::types::LIGHTNING_BOLT, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(e)) });
        return;
    }
    let game_time = level.env.game_time;
    let _ = crate::mobs::summon(spawns, entity.as_str(), at, None, true, difficulty, game_time, 0, seed);
}

/// `ReplaceBlock`, `ReplaceDisk` and `SetBlockProperties` at the effect's position.
fn run_level_effect(level: &mut RegionLevel, p: &Player, effect: &EntityEffect, enchant_level: i32, at: [f64; 3]) {
    let here = BlockPos::new(at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    let offset = |o: &[i32; 3]| BlockPos::new(here.x + o[0], here.y + o[1], here.z + o[2]);
    match effect {
        EntityEffect::ReplaceBlock { offset: o, predicate, block_state, trigger_game_event } => {
            let pos = offset(o);
            if predicate.as_ref().is_none_or(|j| block_predicate(level, pos, j)) && place_state(level, pos, block_state) {
                game_event(level, trigger_game_event.as_ref(), pos);
            }
        }
        EntityEffect::ReplaceDisk { radius, height, offset: o, predicate, block_state, trigger_game_event } => {
            let origin = offset(o);
            let r = radius.calculate(enchant_level) as i32;
            let h = height.calculate(enchant_level) as i32;
            let r2 = (r * r) as f64;
            // `BlockPos.betweenClosed(origin + (-r, 0, -r), origin + (r, min(h - 1, 0), r))`: x fastest, then y, then z.
            for z in -r..=r {
                for y in 0..=(h - 1).min(0) {
                    for x in -r..=r {
                        let pos = BlockPos::new(origin.x + x, origin.y + y, origin.z + z);
                        let (dx, dz) = (pos.x as f64 + 0.5 - at[0], pos.z as f64 + 0.5 - at[2]);
                        if dx * dx + dz * dz >= r2 {
                            continue;
                        }
                        if predicate.as_ref().is_none_or(|j| block_predicate(level, pos, j)) && place_state(level, pos, block_state) {
                            game_event(level, trigger_game_event.as_ref(), pos);
                        }
                    }
                }
            }
        }
        EntityEffect::SetBlockProperties { properties, offset: o, trigger_game_event } => {
            let pos = offset(o);
            let old = level.block(pos);
            let mut state = old;
            for (k, v) in properties {
                if let Some(s) = kiln_data::blocks_types::block_of(state).with_property(state, k, v) {
                    state = s;
                }
            }
            if state != old && kiln_blocks::set_block_and_update(level, pos, state) {
                game_event(level, trigger_game_event.as_ref(), pos);
            }
        }
        // (A level explosion belongs to attacks: see `crate::melee`.)
        _ => {}
    }
    let _ = p;
}

fn game_event(level: &mut RegionLevel, event: Option<&kiln_item::Identifier>, pos: BlockPos) {
    if let Some(e) = event {
        level.effect(Effect::GameEvent { pos, event: kiln_data::live::leak_str(e.as_str()) });
    }
}

/// `setBlockAndUpdate(pos, provider.getState(..))` for a simple provider.
fn place_state(level: &mut RegionLevel, pos: BlockPos, source: &kiln_loot::effects::StateSource) -> bool {
    let kiln_loot::effects::StateSource::Simple { name, properties } = source else { return false };
    let text = if properties.is_empty() {
        name.as_str().to_owned()
    } else {
        format!("{}[{}]", name.as_str(), properties.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","))
    };
    let Some(state) = kiln_blocks::state::parse_state(&text) else { return false };
    kiln_blocks::set_block_and_update(level, pos, state)
}

/// `BlockPredicate.test(level, pos)` for the predicates effects use: the `matching_*`, `solid`, `replaceable`, `unobstructed`
/// and the combinators; the others do not hold.
pub(crate) fn block_predicate(level: &RegionLevel, pos: BlockPos, j: &Json) -> bool {
    let kind = j.get("type").and_then(Json::as_str).unwrap_or("");
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    let offset = j.get("offset").and_then(Json::as_array).map_or(pos, |o| {
        let n = |i: usize| o.get(i).and_then(Json::as_i32).unwrap_or(0);
        BlockPos::new(pos.x + n(0), pos.y + n(1), pos.z + n(2))
    });
    let state = || level.block(offset);
    match kind {
        "true" => true,
        "all_of" => j.get("predicates").and_then(Json::as_array).is_some_and(|l| l.iter().all(|p| block_predicate(level, pos, p))),
        "any_of" => j.get("predicates").and_then(Json::as_array).is_some_and(|l| l.iter().any(|p| block_predicate(level, pos, p))),
        "not" => j.get("predicate").is_some_and(|p| !block_predicate(level, pos, p)),
        "matching_blocks" => {
            let s = state();
            let name = kiln_data::blocks_types::block_of(s).name;
            let one = |v: &Json| match v.as_str() {
                Some(t) if t.starts_with('#') => kiln_entity::mob::kinds::wolf::block_in_tag(s, t.trim_start_matches('#')),
                Some(t) => t == name,
                None => false,
            };
            match j.get("blocks") {
                Some(Json::Arr(list)) => list.iter().any(one),
                Some(v) => one(v),
                None => false,
            }
        }
        "matching_block_tag" => j.get("tag").and_then(Json::as_str).is_some_and(|t| kiln_entity::mob::kinds::wolf::block_in_tag(state(), t.trim_start_matches('#'))),
        "matching_fluids" => crate::advancements::criteria::fluid_matches(j, state()),
        "solid" => kiln_worldgen::block_facts::is_solid(state()),
        "replaceable" => kiln_data::block_props::replaceable(state()),
        "unobstructed" => {
            let (min, max) = ([offset.x as f64, offset.y as f64, offset.z as f64], [offset.x as f64 + 1.0, offset.y as f64 + 1.0, offset.z as f64 + 1.0]);
            !level.bodies.iter().any(|b| b.blocks_building && b.intersects(min, max))
        }
        _ => false,
    }
}

/// `Projectile.applyOnProjectileSpawned`: the `projectile_spawned` effects of the ammunition's enchantments, then of the
/// weapon's (an arrow remembers the weapon; when it is another item than the ammunition), done to the arrow `e`.
pub(crate) fn projectile_spawned(
    p: &mut Player,
    level: &RegionLevel,
    e: &mut kiln_entity::Entity,
    ammo: &kiln_item::ItemStack,
    weapon: &kiln_item::ItemStack,
    spawns: &mut Vec<crate::entities::Spawn>,
) {
    use crate::entities::{Body, Spawn};
    let Some(loot) = p.loot.clone() else { return };
    let mut stacks = vec![ammo];
    if !weapon.is_empty() && weapon.item() != ammo.item() {
        stacks.push(weapon);
    }
    if stacks.iter().all(|s| s.get(kiln_item::keys::ENCHANTMENTS).is_none_or(|e| e.is_empty())) {
        return;
    }
    let pos = e.position();
    let at = [pos.x, pos.y, pos.z];
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    let facts = WorldFacts {
        probe: &probe,
        dim: crate::dim_key(level.env.dim),
        raining: level.env.weather.weather.raining,
        thundering: level.env.weather.weather.thundering,
    };
    let this = EntityView {
        type_id: kiln_item::registry::ENTITY_TYPE.id(e.type_name).unwrap_or(-1),
        pos: at,
        in_water: e.is_in_water(),
        movement: [e.delta.x, e.delta.y, e.delta.z],
        ..EntityView::default()
    };
    let subject = Subject { pos: at, rot: [e.y_rot, e.x_rot], width: e.width, height: e.height, movement: [e.delta.x, e.delta.y, e.delta.z], silent: e.silent, living: false };
    let mut actions = Vec::new();
    for stack in stacks {
        let mut found = Vec::new();
        loot.projectile_spawned_effects(stack, &mut p.level_rng, |l| FxContext::entity(l, &this, at, Some(facts)), |effect, l| found.push((effect, l)));
        for (effect, l) in found {
            kiln_loot::fx::resolve(effect, l, at, &subject, &mut e.random, &mut p.level_rng, &mut actions);
        }
    }
    for action in actions {
        match action {
            Action::Ignite { seconds } => e.ignite_for_seconds(seconds),
            Action::Impulse { delta } => e.delta = e.delta.add(delta[0], delta[1], delta[2]),
            Action::Sound { sound, volume, pitch, at } => p.sound_at(sound.as_str(), at, volume, pitch),
            Action::Summon { entity, at, .. } if entity.as_str() == "minecraft:lightning_bolt" => {
                let seed = e.random.next_long();
                let bolt = kiln_entity::ext_entity::lightning::channeled(0, 0, kiln_entity::math::Vec3::new(at[0], at[1], at[2]), p.entity_id, seed);
                spawns.push(Spawn { kind: &kiln_data::entities::types::LIGHTNING_BOLT, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(bolt)) });
            }
            // (Blocks, functions and the like happen where a player's own effects do: see `Player::apply_fx`.)
            other => {
                let _ = other;
            }
        }
    }
}

impl Player {
    /// `ServerLevel.playSound(null, x, y, z, sound, PLAYERS, volume, pitch)`: the player and everyone near hear it.
    pub(crate) fn sound_at(&mut self, sound: &str, at: [f64; 3], volume: f32, pitch: f32) {
        use kiln_proto::packets::world_fx;
        let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
        let seed = self.sound_seed.next_long();
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Players, at, volume, pitch, seed);
        self.send(pkt.clone());
        self.pending_sounds.push(pkt);
    }
}

impl crate::Sim {
    /// The modifiers on attribute `attribute` of player `conn` as `(id, amount)`: the equipment's, the effects', the active
    /// `location_changed` effects' and the commands' (for tests).
    pub fn attribute_modifiers(&self, conn: kiln_link::ConnId, attribute: &str) -> Option<Vec<(String, f64)>> {
        let p = self.players.get(&conn)?;
        let attr = crate::combat::Attr::named(kiln_data::live::leak_str(attribute));
        Some(p.attribute_modifiers(attr).into_iter().map(|(id, amount, _)| (id, amount)).collect())
    }
}
