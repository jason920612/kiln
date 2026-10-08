//! `Player.attack` (vanilla `ServerGamePacketListenerImpl.handleAttack` → `Player.attack`): one
//! swing of a player at a player or at the region's entities, in vanilla's order: the damage
//! (attribute, enchantments, the attack strength, the mace's smash bonus, a critical hit), the
//! hurt, the knockback, the sweeping attack on the victims around, the effects on the clients,
//! the weapon (`hurtEnemy`: the mace's smash and its blast; the post-attack enchantment effects:
//! fire aspect, bane of arthropods, thorns, the wind burst; `postHurtEnemy`: the wear), the
//! damage statistic and the exhaustion.
//!
//! Victims are the region's other players and its entities (mobs are hurt through
//! [`entities::with_mob`], so they take damage, knockback, fire and effects as they would from
//! any other source). Players sweep and blast players and mobs alike.

use crate::blocks::RegionLevel;
use crate::combat::{
    ATTACK_DAMAGE, BURNING_TIME, EntityClass, KNOCKBACK_RESISTANCE, SWEEPING_DAMAGE_RATIO, Spin, Target, dist2, feet_state, mth_cos, mth_sin, play_sounds, send_particles,
    send_to_trackers_and_self,
};
use crate::enchant::{DamageContext, EntityView};
use crate::entities::{self, Entities, Spawn};
use crate::health::{Attacker, Cause, DamageCtx, Death, Source};
use crate::Player;
use kiln_entity::level::DamageKind;
use kiln_entity::math::{Aabb, Vec3};
use kiln_entity::mob::attributes::Attr;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::LegacyRandom;
use kiln_loot::effects::{EntityEffect, Target as EffectTarget, Targeted};
use kiln_proto::packets::entity;
use kiln_proto::packets::world_fx;
use kiln_world::Blocks;

/// Who an attack, a sweep or a blast reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Victim {
    /// The region's player at this index.
    Player(usize),
    /// An entity by network id, and the ender dragon part hit (the id is the dragon's).
    Entity(i32, Option<usize>),
}

/// What the attack needs to know of a victim.
struct Facts {
    id: i32,
    pos: [f64; 3],
    bb: Aabb,
    view: EntityView,
    /// A `LivingEntity` (players, mobs).
    living: bool,
    on_ground: bool,
    /// `Entity.bbHeight`.
    height: f32,
    /// `Attributes.KNOCKBACK_RESISTANCE`.
    knockback_resistance: f64,
    spectator: bool,
    creative_flying: bool,
    /// The entity type (`minecraft:...`).
    type_name: &'static str,
}

/// The mob entity `e` (not a player) as an attack sees it.
fn mob_facts(e: &kiln_entity::Entity, id: i32) -> Facts {
    let pos = e.position();
    let m = kiln_entity::mob::data(e);
    let view = EntityView {
        type_id: kiln_item::registry::ENTITY_TYPE.id(e.type_name).unwrap_or(-1),
        pos: [pos.x, pos.y, pos.z],
        on_ground: e.on_ground,
        on_fire: e.remaining_fire_ticks > 0,
        has_vehicle: e.vehicle.is_some(),
        in_water: e.was_touching_water,
        fall_distance: e.fall_distance,
        ..Default::default()
    };
    Facts {
        id,
        pos: [pos.x, pos.y, pos.z],
        bb: e.bounding_box(),
        view,
        living: m.is_some(),
        on_ground: e.on_ground,
        height: e.height,
        knockback_resistance: m.map_or(0.0, |m| m.attrs.value(Attr::KnockbackResistance)),
        spectator: false,
        creative_flying: false,
        type_name: e.type_name,
    }
}

/// `LivingEntity.setDeltaMovement` of a player's velocity for the Y axis (`Vec3.with(Axis.Y, y)`).
fn with_y(v: [f64; 3], y: f64) -> [f64; 3] {
    [v[0], y, v[2]]
}

/// The region an attack happens in, with the level random it draws from lent by the attacker
/// (the attack's enchantment effects use the level's random; the attacker's own random is the
/// players' entity random).
pub(crate) struct Work<'a, 'l, 'p> {
    pub(crate) entities: &'a mut Entities,
    pub(crate) level: &'a mut RegionLevel<'l>,
    pub(crate) players: &'a mut [&'p mut Player],
    pub(crate) spawns: &'a mut Vec<Spawn>,
    pub(crate) deaths: &'a mut Vec<Death>,
    pub(crate) rng: Option<LegacyRandom>,
}

impl<'a, 'l, 'p> Work<'a, 'l, 'p> {
    fn game_time(&self) -> i64 {
        self.level.env.game_time
    }

    fn rng(&mut self) -> &mut LegacyRandom {
        self.rng.as_mut().expect("attack random")
    }

    /// Runs `f` with a [`DamageCtx`] holding the lent random.
    fn with_ctx<R>(&mut self, f: impl FnOnce(&mut [&'p mut Player], &mut DamageCtx<'_>) -> R) -> R {
        let env = self.level.env;
        let mut ctx = DamageCtx { rules: env.damage, game_time: env.game_time, spawns: &mut *self.spawns, deaths: &mut *self.deaths, level_rng: self.rng.take() };
        let r = f(&mut *self.players, &mut ctx);
        self.rng = ctx.level_rng.take();
        r
    }

    /// Runs `f` on entity `id` with the level around it ([`entities::with_mob`]).
    fn with_entity<R>(&mut self, id: i32, salt: u64, f: impl FnOnce(&mut kiln_entity::Entity, &mut entities::SimLevel<'_, '_, '_>) -> R) -> Option<R> {
        entities::with_mob(self.entities, self.level, self.players, self.spawns, self.deaths, id, salt, f)
    }

    fn entity(&self, id: i32) -> Option<&kiln_entity::Entity> {
        let i = self.entities.list.binary_search_by_key(&id, |e| e.id).ok()?;
        let e = &self.entities.list[i];
        if e.removed { None } else { e.phys.as_deref() }
    }

    fn facts(&self, v: Victim) -> Option<Facts> {
        match v {
            Victim::Player(t) => {
                let p = &*self.players[t];
                let cells = &*self.level.cells;
                let block = |pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0);
                let mut view = p.view_in(&block);
                view.fall_distance = p.fall_distance;
                Some(Facts {
                    id: p.entity_id,
                    pos: p.pos,
                    bb: p.bounding_box(),
                    view,
                    living: true,
                    on_ground: p.on_ground,
                    height: if p.sneaking { 1.5 } else { 1.8 },
                    knockback_resistance: p.attribute(KNOCKBACK_RESISTANCE),
                    spectator: p.game_mode == 3,
                    creative_flying: p.game_mode == 1 && p.flying,
                    type_name: "minecraft:player",
                })
            }
            Victim::Entity(id, _) => self.entity(id).map(|e| mob_facts(e, id)),
        }
    }

    /// The victim's equipment as `runIterationOnEquipment` walks it.
    fn equipment_of(&self, v: Victim) -> Vec<(EquipmentSlot, ItemStack)> {
        match v {
            Victim::Player(t) => self.players[t].equipment().into_iter().map(|(s, st)| (s, st.clone())).collect(),
            Victim::Entity(id, _) => {
                let Some(m) = self.entity(id).and_then(kiln_entity::mob::data) else { return Vec::new() };
                use kiln_entity::mob::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
                [
                    (EquipmentSlot::MainHand, MAINHAND),
                    (EquipmentSlot::OffHand, OFFHAND),
                    (EquipmentSlot::Feet, FEET),
                    (EquipmentSlot::Legs, LEGS),
                    (EquipmentSlot::Chest, CHEST),
                    (EquipmentSlot::Head, HEAD),
                ]
                .into_iter()
                .map(|(slot, i)| (slot, m.equipment[i].clone()))
                .collect()
            }
        }
    }

    fn health_of(&self, v: Victim) -> Option<f32> {
        match v {
            Victim::Player(t) => Some(self.players[t].health),
            Victim::Entity(id, _) => self.entity(id).and_then(kiln_entity::mob::data).map(|m| m.health),
        }
    }

    /// `Entity.hurtServer` / `hurtOrSimulate` of the victim.
    fn hurt(&mut self, a: usize, v: Victim, amount: f32, source: &Source) -> bool {
        match v {
            Victim::Player(t) => self.with_ctx(|players, ctx| players[t].hurt(amount, source, ctx)),
            Victim::Entity(id, part) => {
                let attacker = &*self.players[a];
                let kind = match source.cause {
                    Cause::Other(name) => DamageKind::of_type(name),
                    _ => DamageKind::PlayerAttack,
                };
                let dsource = kiln_entity::mob::DamageSource {
                    kind,
                    attacker: Some(attacker.entity_id),
                    direct: Some(attacker.entity_id),
                    pos: Some(Vec3::new(attacker.pos[0], attacker.pos[1], attacker.pos[2])),
                    attacker_is_player: true,
                };
                let attacker_id = attacker.entity_id;
                let weapon = source.weapon.clone().unwrap_or_else(ItemStack::empty);
                let attacker_view = attacker.view();
                // `EnderDragonPart.hurtServer` → `EnderDragon.hurt(part)`; an end crystal explodes.
                let hit = self.with_entity(id, 0x6869_74, |phys, sim| {
                    let _scope = kiln_entity::enchanting::attack_scope(weapon, attacker_view);
                    match part {
                    Some(part) => kiln_entity::mob::kinds::ender_dragon::hurt_entity_part(phys, sim, part, dsource, amount),
                    None if kiln_entity::mob::data(phys).is_none() => phys.hurt(sim, kind, amount, Some(attacker_id)),
                    None => kiln_entity::mob::hurt_entity(phys, sim, dsource, amount),
                    }
                });
                hit.unwrap_or(false)
            }
        }
    }

    /// `LivingEntity.knockback(strength, dx, dz)` on the victim (the server's view of the velocity;
    /// a player's client gets its motion from [`Work::send_motion`]).
    fn knockback(&mut self, v: Victim, strength: f64, dx: f64, dz: f64) {
        match v {
            Victim::Player(t) => self.players[t].knockback(strength, dx, dz),
            Victim::Entity(id, _) => {
                self.with_entity(id, 0x6b6e_6f63, |phys, _| kiln_entity::mob::knockback_entity(phys, strength, dx, dz));
            }
        }
    }

    /// `Entity.push(x, y, z)`.
    fn push(&mut self, v: Victim, x: f64, y: f64, z: f64) {
        match v {
            Victim::Player(t) => {
                let p = &mut *self.players[t];
                p.vel = [p.vel[0] + x, p.vel[1] + y, p.vel[2] + z];
            }
            Victim::Entity(id, _) => {
                self.with_entity(id, 0x7075_7368, |phys, _| {
                    phys.delta = phys.delta.add(x, y, z);
                    phys.needs_sync = true;
                });
            }
        }
    }

    /// `connection.send(new ClientboundSetEntityMotionPacket(player))` for a player victim.
    fn send_motion(&mut self, v: Victim) {
        if let Victim::Player(t) = v {
            let p = &mut *self.players[t];
            p.send(entity::set_entity_motion(p.entity_id, p.vel));
        }
    }

    /// `Player.causeExtraKnockback(target, strength, oldDelta, source, damage, true)`: the victim
    /// is pushed away along the attacker's view, the attacker slows down and stops sprinting.
    fn cause_extra_knockback(&mut self, a: usize, v: Victim, strength: f32, old_vel: [f64; 3]) {
        if strength <= 0.0 {
            return;
        }
        let rad = (self.players[a].rot[0] * 0.017453292) as f64;
        let living = self.facts(v).is_some_and(|f| f.living);
        if living {
            self.knockback(v, strength as f64, mth_sin(rad) as f64, -mth_cos(rad) as f64);
        } else {
            // `Entity.push(-sin(yaw) * strength, 0.1, cos(yaw) * strength)`.
            self.push(v, -mth_sin(rad) as f64 * strength as f64, 0.1, mth_cos(rad) as f64 * strength as f64);
        }
        let p = &mut *self.players[a];
        p.vel = [p.vel[0] * 0.6, p.vel[1], p.vel[2] * 0.6];
        if p.sprinting {
            p.sprinting = false;
            p.meta_dirty = true;
        }
        if let Victim::Player(t) = v {
            let victim = &mut *self.players[t];
            if victim.sync_velocity {
                victim.send(entity::set_entity_motion(victim.entity_id, victim.vel));
                victim.sync_velocity = false;
                victim.vel = old_vel;
            }
        }
    }

    /// `EntityPredicate`s see this entity: the target of the weapon's enchantments.
    fn view_of(&self, v: Victim) -> EntityView {
        self.facts(v).map(|f| f.view).unwrap_or_default()
    }

    /// `ItemStack.hurtAndBreak` on an equipped item of the attacker (the weapon).
    fn wear_weapon(&mut self, a: usize, slot: EquipmentSlot, amount: i32) {
        let mut rng = self.rng.take();
        self.players[a].hurt_and_break(slot, amount, rng.as_mut());
        self.rng = rng;
    }

    /// `Item.getAttackDamageBonus(target, damage, source)`: the mace's smash.
    fn attack_damage_bonus(&mut self, a: usize, weapon: &ItemStack, target: &EntityView, source: &Source) -> f32 {
        if !is_mace(weapon) {
            return 0.0;
        }
        let p = &*self.players[a];
        if !can_smash_attack(p) {
            return 0.0;
        }
        let fall = p.fall_distance;
        let bonus = if fall <= 3.0 {
            4.0 * fall
        } else if fall <= 8.0 {
            12.0 + 2.0 * (fall - 3.0)
        } else {
            22.0 + fall - 8.0
        };
        let Some(loot) = p.loot.clone() else { return bonus as f32 };
        let per_block = loot.modify_fall_based_damage(weapon, self.rng(), 0.0, |level| DamageContext { level, this: target, source });
        (bonus + per_block as f64 * fall) as f32
    }

    /// `MaceItem.hurtEnemy`: a smash throws the attacker up a little, sounds, and blasts the
    /// entities around the target.
    fn mace_hurt_enemy(&mut self, a: usize, target: Victim, target_facts: &Facts) {
        let p = &mut *self.players[a];
        if !can_smash_attack(p) {
            return;
        }
        p.vel = with_y(p.vel, 0.009999999776482582);
        p.send(entity::set_entity_motion(p.entity_id, p.vel));
        p.sync_velocity = false;
        let (at, fall) = (p.pos, p.fall_distance);
        let sound = if target_facts.on_ground {
            if fall > 5.0 { "minecraft:item.mace.smash_ground_heavy" } else { "minecraft:item.mace.smash_ground" }
        } else {
            "minecraft:item.mace.smash_air"
        };
        let (game_time, seed) = (self.game_time(), self.level.env.seed);
        play_sounds(self.players, a, &[sound], game_time, seed);
        let _ = at;
        self.smash_blast(a, target, target_facts);
    }

    /// `MaceItem.knockback(level, attacker, target)`: the level event of the smash and the push of
    /// everything living within 3.5 blocks of the attacker (around the target's box).
    fn smash_blast(&mut self, a: usize, target: Victim, target_facts: &Facts) {
        // `Entity.getOnPos()`.
        let on = [kiln_entity::math::floor(target_facts.pos[0]), kiln_entity::math::floor(target_facts.pos[1] - 1.0E-5), kiln_entity::math::floor(target_facts.pos[2])];
        let pkt = world_fx::level_event(2013, on, 750, false);
        let at = [on[0] as f64 + 0.5, on[1] as f64 + 0.5, on[2] as f64 + 0.5];
        for p in self.players.iter_mut().filter(|p| dist2(p.pos, at) < 64.0 * 64.0) {
            p.send(pkt.clone());
        }
        let (attacker_pos, fall) = (self.players[a].pos, self.players[a].fall_distance);
        let area = target_facts.bb.inflate_all(3.5);
        let mut candidates: Vec<(Victim, Facts)> = Vec::new();
        for t in 0..self.players.len() {
            if t == a || self.players[t].dead || self.players[t].disconnected {
                continue;
            }
            if let Some(f) = self.facts(Victim::Player(t)).filter(|f| f.bb.intersects(&area)) {
                candidates.push((Victim::Player(t), f));
            }
        }
        for e in self.entities.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            if kiln_entity::mob::data(phys).is_none() || e.id == target_facts.id {
                continue;
            }
            let f = mob_facts(phys, e.id);
            if f.bb.intersects(&area) {
                candidates.push((Victim::Entity(e.id, None), f));
            }
        }
        candidates.sort_by_key(|(_, f)| f.id);
        for (v, f) in candidates {
            // `knockbackPredicate`: not a spectator, not the target, within 3.5 of the attacker,
            // not a flying creative player.
            if f.spectator || f.id == target_facts.id || f.creative_flying || dist2(attacker_pos, f.pos) > 3.5f64.powi(2) {
                continue;
            }
            let delta = Vec3::new(f.pos[0] - target_facts.pos[0], f.pos[1] - target_facts.pos[1], f.pos[2] - target_facts.pos[2]);
            // `getKnockbackPower`.
            let power = (3.5 - delta.length()) * 0.699999988079071 * if fall > 5.0 { 2.0 } else { 1.0 } * (1.0 - f.knockback_resistance);
            let push = delta.normalize().scale(power);
            if power > 0.0 {
                self.push(v, push.x, 0.699999988079071, push.z);
                self.send_motion(v);
            }
        }
        let _ = target;
    }

    /// `Player.doSweepAttack`: the other living entities near the target take `1 + ratio * damage`
    /// (times the attack strength) and a small knockback.
    fn do_sweep_attack(&mut self, a: usize, target: Victim, target_bb: &Aabb, damage: f32, source: &Source, scale: f32) {
        let (game_time, seed) = (self.game_time(), self.level.env.seed);
        play_sounds(self.players, a, &["minecraft:entity.player.attack.sweep"], game_time, seed);
        let sweep = 1.0 + self.players[a].attribute(SWEEPING_DAMAGE_RATIO) as f32 * damage;
        let bb = target_bb.inflate(1.0, 0.25, 1.0);
        let (attacker_pos, yaw) = (self.players[a].pos, self.players[a].rot[0]);
        let rad = (yaw * 0.017453292) as f64;
        // `getEntitiesOfClass(LivingEntity, box)` skips spectators.
        let mut hit: Vec<(Victim, Facts)> = Vec::new();
        for i in 0..self.players.len() {
            if i == a || Victim::Player(i) == target || self.players[i].dead || self.players[i].disconnected {
                continue;
            }
            if let Some(f) = self.facts(Victim::Player(i)).filter(|f| !f.spectator && f.bb.intersects(&bb)) {
                hit.push((Victim::Player(i), f));
            }
        }
        for e in self.entities.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            if kiln_entity::mob::data(phys).is_none() || Victim::Entity(e.id, None) == target_entity_of(target) {
                continue;
            }
            let f = mob_facts(phys, e.id);
            if f.bb.intersects(&bb) {
                hit.push((Victim::Entity(e.id, None), f));
            }
        }
        hit.sort_by_key(|(_, f)| f.id);
        for (v, f) in hit {
            if dist2(attacker_pos, f.pos) >= 9.0 {
                continue;
            }
            // `getEnchantedDamage(entity, sweep, source) * scale`.
            let amount = {
                let mut rng = self.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
                let d = self.players[a].enchanted_damage(&f.view, sweep, source, &mut rng) * scale;
                self.rng = Some(rng);
                d
            };
            if self.hurt(a, v, amount, source) {
                self.knockback(v, 0.4000000059604645, mth_sin(rad) as f64, -mth_cos(rad) as f64);
                // `doPostAttackEffects`: the source's attacker is a living entity, so its weapon too.
                self.post_attack(a, v, source);
            }
        }
        let (dx, dz) = (-mth_sin(rad) as f64, mth_cos(rad) as f64);
        let at = [attacker_pos[0] + dx, attacker_pos[1] + 0.9, attacker_pos[2] + dz];
        send_particles(self.players, "minecraft:sweep_attack", at, 0, [dx as f32, 0.0, dz as f32], 0.0);
    }

    /// `EnchantmentHelper.doPostAttackEffectsWithItemSource(victim, source, weapon)`: the victim's
    /// equipment effects enchanted `victim`, then the weapon's (`attacker`), each carried out on
    /// who it affects.
    fn post_attack(&mut self, a: usize, victim: Victim, source: &Source) {
        let Some(loot) = self.players[a].loot.clone() else { return };
        let view = self.view_of(victim);
        let equipment = self.equipment_of(victim);
        let mut found: Vec<(&Targeted<EntityEffect>, i32, bool, EquipmentSlot)> = Vec::new();
        let mut rng = self.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
        let ctx = |level: i32| DamageContext { level, this: &view, source };
        for (slot, stack) in &equipment {
            loot.post_attack_effects(stack, *slot, EffectTarget::Victim, &mut rng, ctx, |effect, level| found.push((effect, level, true, *slot)));
        }
        if let Some(weapon) = source.weapon.as_ref() {
            loot.post_attack_effects(weapon, EquipmentSlot::MainHand, EffectTarget::Attacker, &mut rng, ctx, |effect, level| found.push((effect, level, false, EquipmentSlot::MainHand)));
        }
        self.rng = Some(rng);
        for (effect, level, owner_is_victim, slot) in found {
            let affected = match effect.affected {
                EffectTarget::Attacker | EffectTarget::DamagingEntity => Victim::Player(a),
                EffectTarget::Victim => victim,
            };
            let owner = if owner_is_victim { victim } else { Victim::Player(a) };
            self.apply_effect(a, affected, owner, slot, &effect.effect, level, victim);
        }
    }

    /// `Enchantment.doPostAttack`'s application of one entity effect.
    #[allow(clippy::too_many_arguments)]
    fn apply_effect(&mut self, a: usize, affected: Victim, owner: Victim, slot: EquipmentSlot, effect: &EntityEffect, level: i32, victim: Victim) {
        match effect {
            EntityEffect::AllOf(list) => {
                for inner in list {
                    self.apply_effect(a, affected, owner, slot, inner, level, victim);
                }
            }
            EntityEffect::Ignite(seconds) => self.ignite(affected, seconds.calculate(level)),
            EntityEffect::DamageEntity { damage_type, min_damage, max_damage } => {
                let Victim::Player(t) = affected else { return };
                // `Mth.randomBetween(entity.getRandom(), min, max)`.
                let (min, max) = (min_damage.calculate(level), max_damage.calculate(level));
                let amount = kiln_javamath::random::RandomSource::next_float(&mut self.players[t].entity_rng) * (max - min) + min;
                let attacker = self.attacker_of(owner);
                let source = Source { cause: Cause::Other(crate::health::static_damage_type(damage_type.as_str())), attacker, direct: None, weapon: None, position: None };
                self.with_ctx(|players, ctx| players[t].hurt(amount, &source, ctx));
            }
            EntityEffect::ChangeItemDamage(amount) => match owner {
                Victim::Player(o) => {
                    let stack = self.players[o].inv.equipped(slot);
                    if stack.get(keys::MAX_DAMAGE).is_some() && stack.get(keys::DAMAGE).is_some() {
                        let mut rng = self.rng.take();
                        self.players[o].hurt_and_break(slot, amount.calculate(level) as i32, rng.as_mut());
                        self.rng = rng;
                    }
                }
                Victim::Entity(id, _) => self.wear_mob_item(id, slot, amount.calculate(level) as i32),
            },
            EntityEffect::ApplyMobEffect { to_apply, min_duration, max_duration, min_amplifier, max_amplifier } => {
                use kiln_javamath::random::RandomSource;
                if to_apply.is_empty() {
                    return;
                }
                let roll = |r: &mut LegacyRandom| {
                    let pick = to_apply[r.next_int_bounded(to_apply.len() as i32) as usize].as_str().to_owned();
                    let between = |r: &mut LegacyRandom, min: f32, max: f32| r.next_float() * (max - min) + min;
                    let round = |v: f32| (v as f64 + 0.5).floor() as i32;
                    let (lo, hi) = (min_duration.calculate(level), max_duration.calculate(level));
                    let duration = round(between(r, lo, hi) * 20.0);
                    let (lo, hi) = (min_amplifier.calculate(level), max_amplifier.calculate(level));
                    let amplifier = round(between(r, lo, hi)).max(0);
                    (pick, duration, amplifier)
                };
                match affected {
                    Victim::Player(t) => {
                        let (pick, duration, amplifier) = roll(&mut self.players[t].entity_rng);
                        if let Some(id) = crate::effects::effect_id(&pick) {
                            self.players[t].add_effect(crate::effects::Effect::simple(id, duration, amplifier));
                        }
                    }
                    Victim::Entity(id, _) => {
                        let attacker = self.players[a].entity_id;
                        self.with_entity(id, 0x6566_6665, |phys, sim| {
                            let mut r = std::mem::replace(&mut phys.random, LegacyRandom::new(0));
                            let (pick, duration, amplifier) = roll(&mut r);
                            phys.random = r;
                            if let Some(effect) = crate::effects::effect_id(&pick) {
                                kiln_entity::mob::add_effect_entity(phys, sim, crate::effects::Effect::simple(effect, duration, amplifier), Some(attacker));
                            }
                        });
                    }
                }
            }
            EntityEffect::Explode { knockback_multiplier, immune_blocks, offset, radius, interaction, small_particle, large_particle, sound, has_damage_type, .. } => {
                let Some(f) = self.facts(affected) else { return };
                let center = Vec3::new(f.pos[0] + offset[0], f.pos[1] + offset[1], f.pos[2] + offset[2]);
                let blocks = interaction != "none";
                let burst = Burst {
                    center,
                    radius: radius.calculate(level).max(0.0),
                    knockback_multiplier: knockback_multiplier.as_ref().map_or(1.0, |m| m.calculate(level)),
                    immune_blocks: immune_blocks.as_deref().map(|t| t.trim_start_matches('#').to_owned()),
                    blocks,
                    particle: if (radius.calculate(level).max(0.0) < 2.0) || !blocks { small_particle.clone() } else { large_particle.clone() },
                    sound: sound.as_str().to_owned(),
                    damages: *has_damage_type,
                };
                self.explode(&burst);
            }
            _ => {}
        }
    }

    /// `ServerLevel.explode` with a `SimpleExplosionDamageCalculator` (the wind burst of the mace):
    /// knockback by exposure and distance for everything around, the explosion packet for the
    /// players near, the level random drawn from the attack's.
    fn explode(&mut self, b: &Burst) {
        if b.damages {
            // (No Kiln enchantment explodes with a damage type yet.)
        }
        let center = b.center;
        let (min_y, height) = (self.level.env.min_y, self.level.env.height);
        let count = {
            let cells = &*self.level.cells;
            let block = |p: kiln_entity::math::BlockPos| cells.get_block(p.x, p.y, p.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR);
            let immune = |state: u16| b.immune_blocks.as_deref().is_some_and(|t| kiln_entity::mob::kinds::wolf::block_in_tag(state, &format!("minecraft:{}", t.trim_start_matches("minecraft:"))));
            let rng = self.rng.as_mut().expect("attack random");
            kiln_entity::explosion::burst_positions(&block, rng, (min_y, min_y + height - 1), center, b.radius, &immune, b.blocks).len() as i32
        };
        // `hurtEntities`: everything in the box (spectators excepted) within twice the radius.
        let diameter = b.radius * 2.0;
        let area = Aabb::new(
            kiln_entity::math::floor(center.x - diameter as f64 - 1.0) as f64,
            kiln_entity::math::floor(center.y - diameter as f64 - 1.0) as f64,
            kiln_entity::math::floor(center.z - diameter as f64 - 1.0) as f64,
            kiln_entity::math::floor(center.x + diameter as f64 + 1.0) as f64,
            kiln_entity::math::floor(center.y + diameter as f64 + 1.0) as f64,
            kiln_entity::math::floor(center.z + diameter as f64 + 1.0) as f64,
        );
        let mut hit_players: Vec<(usize, Vec3)> = Vec::new();
        let mut pushes: Vec<(i32, Vec3)> = Vec::new();
        if b.radius >= 1.0e-5 {
            let cells = &*self.level.cells;
            let block = |p: kiln_entity::math::BlockPos| cells.get_block(p.x, p.y, p.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR);
            let power_of = |pos: [f64; 3], eye_y: f64, bb: &Aabb, ctx: &kiln_entity::collision::CollisionContext, resistance: f64| -> Option<Vec3> {
                let d = Vec3::new(pos[0] - center.x, pos[1] - center.y, pos[2] - center.z);
                let dist = d.length_sqr().sqrt() / diameter as f64;
                if dist > 1.0 {
                    return None;
                }
                let dir = Vec3::new(pos[0], eye_y, pos[2]).subtract(center.x, center.y, center.z).normalize();
                let seen = if b.knockback_multiplier != 0.0 { kiln_entity::explosion::seen_percent_with(&block, center, bb, ctx) } else { 0.0 };
                let power = (1.0 - dist) * seen as f64 * b.knockback_multiplier as f64 * (1.0 - resistance);
                Some(dir.scale(power))
            };
            for t in 0..self.players.len() {
                let p = &*self.players[t];
                if p.dead || p.disconnected || p.game_mode == 3 || !p.bounding_box().intersects(&area) {
                    continue;
                }
                if let Some(push) = power_of(p.pos, p.eye_position()[1], &p.bounding_box(), &kiln_entity::collision::CollisionContext::EMPTY, 0.0) {
                    hit_players.push((t, push));
                }
            }
            for e in self.entities.list.iter().filter(|e| !e.removed) {
                let Some(phys) = e.phys.as_deref() else { continue };
                let Some(m) = kiln_entity::mob::data(phys) else { continue };
                if !phys.bounding_box().intersects(&area) {
                    continue;
                }
                let pos = phys.position();
                let resistance = m.attrs.value(Attr::ExplosionKnockbackResistance);
                if let Some(push) = power_of([pos.x, pos.y, pos.z], phys.eye_y(), &phys.bounding_box(), &phys.collision_context(), resistance) {
                    pushes.push((e.id, push));
                }
            }
        }
        for (id, push) in pushes {
            if push.x.is_finite() && push.y.is_finite() && push.z.is_finite() {
                self.push(Victim::Entity(id, None), push.x, push.y, push.z);
            }
        }
        let mut knockbacks: Vec<(usize, Option<[f64; 3]>)> = Vec::new();
        for &(t, push) in &hit_players {
            if push.x.is_finite() && push.y.is_finite() && push.z.is_finite() {
                self.push(Victim::Player(t), push.x, push.y, push.z);
            }
            let p = &*self.players[t];
            if !(p.game_mode == 1 && p.flying) {
                knockbacks.push((t, Some([push.x, push.y, push.z])));
            }
        }
        // `ClientboundExplodePacket` to the players within 64 blocks.
        let (Some(particle), Some(sound)) = (
            kiln_data::builtin_id("minecraft:particle_type", &b.particle),
            kiln_data::builtin_id("minecraft:sound_event", &b.sound),
        ) else {
            return;
        };
        for (i, p) in self.players.iter_mut().enumerate() {
            if dist2(p.pos, [center.x, center.y, center.z]) >= 4096.0 {
                continue;
            }
            let knockback = knockbacks.iter().find(|(t, _)| *t == i).and_then(|(_, k)| *k);
            let pkt = world_fx::explode(
                [center.x, center.y, center.z],
                b.radius,
                count,
                knockback,
                &world_fx::Particle { kind: particle, options: world_fx::ParticleOptions::None },
                &world_fx::Sound::Registered(sound),
                true,
            );
            p.send(pkt);
        }
    }

    /// The owner of an enchanted item as the damage source `thorns` names its attacker.
    fn attacker_of(&self, owner: Victim) -> Option<Attacker> {
        match owner {
            Victim::Player(o) => Some(self.players[o].as_attacker()),
            Victim::Entity(id, _) => {
                let f = self.facts(owner)?;
                Some(Attacker { id, name: String::new(), pos: f.pos, creative: false, weapon: None, view: f.view, mob: Some(f.type_name) })
            }
        }
    }

    /// `Entity.igniteForSeconds`: scaled by the `burning_time` attribute of living entities.
    fn ignite(&mut self, v: Victim, seconds: f32) {
        match v {
            Victim::Player(t) => self.players[t].ignite_for_seconds(seconds),
            Victim::Entity(id, _) => {
                self.with_entity(id, 0x6669_7265, |phys, _| phys.ignite_for_seconds(seconds));
            }
        }
    }

    /// `ItemStack.hurtAndBreak` on a mob's equipped item (thorns on its armor).
    fn wear_mob_item(&mut self, id: i32, slot: EquipmentSlot, amount: i32) {
        use kiln_entity::mob::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
        let index = match slot {
            EquipmentSlot::MainHand => MAINHAND,
            EquipmentSlot::OffHand => OFFHAND,
            EquipmentSlot::Feet => FEET,
            EquipmentSlot::Legs => LEGS,
            EquipmentSlot::Chest => CHEST,
            EquipmentSlot::Head => HEAD,
            _ => return,
        };
        let loot = self.players.iter().find_map(|p| p.loot.clone());
        let mut rng = self.rng.take();
        self.with_entity(id, 0x7765_6172, |phys, sim| {
            let Some(m) = kiln_entity::mob::data_mut(phys) else { return };
            let stack = &mut m.equipment[index];
            if !stack.is_damageable_item() {
                return;
            }
            let amount = match (&loot, rng.as_mut()) {
                (Some(loot), Some(r)) if amount > 0 => loot.process_durability_change(stack, r, amount),
                _ => amount,
            };
            if amount <= 0 {
                return;
            }
            let damage = stack.damage() + amount;
            stack.insert(keys::DAMAGE, damage.clamp(0, stack.max_damage()));
            if damage >= stack.max_damage() {
                stack.shrink(1);
                // `entityEventForEquipmentBreak`.
                let event = match slot {
                    EquipmentSlot::MainHand => 47,
                    EquipmentSlot::OffHand => 48,
                    EquipmentSlot::Head => 49,
                    EquipmentSlot::Chest => 50,
                    EquipmentSlot::Legs => 51,
                    _ => 52,
                };
                kiln_entity::level::EntityLevel::emit(sim, kiln_entity::level::Event::EntityEvent { entity: id, event });
            }
        });
        self.rng = rng;
    }
}

/// An explosion with a `SimpleExplosionDamageCalculator` (`ExplodeEffect`).
struct Burst {
    center: Vec3,
    radius: f32,
    knockback_multiplier: f32,
    /// The immune blocks' tag (`blocks_wind_charge_explosions`).
    immune_blocks: Option<String>,
    /// Whether the explosion interacts with blocks (shuffles the exploded positions).
    blocks: bool,
    particle: String,
    sound: String,
    damages: bool,
}

/// The `Victim` of the target itself (its dragon part dropped).
fn target_entity_of(v: Victim) -> Victim {
    match v {
        Victim::Entity(id, _) => Victim::Entity(id, None),
        other => other,
    }
}

/// `MaceItem.canSmashAttack`: falling more than 1.5 blocks and not gliding.
fn can_smash_attack(p: &Player) -> bool {
    p.fall_distance > 1.5 && !p.fall_flying
}

fn is_mace(stack: &ItemStack) -> bool {
    !stack.is_empty() && stack.item_name() == "minecraft:mace"
}

/// `ServerGamePacketListenerImpl.handleAttack` after its checks, then `Player.attack`.
pub(crate) fn attack(w: &mut Work<'_, '_, '_>, a: usize, target: Target, target_id: i32, spin: Option<&Spin>) {
    let victim = match &target {
        Target::Player(t) => Victim::Player(*t),
        Target::Entity { part, .. } => Victim::Entity(target_id - part.map_or(0, |p| p as i32 + 1), *part),
    };
    let (living, target_view, target_bb) = match &target {
        Target::Player(t) => {
            let f = w.facts(Victim::Player(*t)).expect("player target");
            (true, f.view, f.bb)
        }
        Target::Entity { kind: EntityClass::NotAttackable, .. } => return,
        Target::Entity { kind, bb, type_id, pos, part } => {
            let live = w.facts(victim);
            let mut view = live.as_ref().map(|f| f.view.clone()).unwrap_or_default();
            view.type_id = *type_id;
            view.pos = *pos;
            let living = *kind == EntityClass::Mob && live.as_ref().is_some_and(|f| f.living || part.is_some());
            (living, view, *bb)
        }
    };
    let p = &mut *w.players[a];
    // `isAutoSpinAttack ? autoSpinAttackDmg : ATTACK_DAMAGE`, and `getWeaponItem` (the trident
    // of the spin, else what is held).
    let mut damage = spin.map_or_else(|| p.attribute(ATTACK_DAMAGE) as f32, |s| s.damage);
    let weapon = spin.map_or_else(|| p.inv.selected_item().clone(), |s| s.item.clone());
    let slot = if spin.is_some_and(|s| s.off_hand) { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    // `ItemStack.getDamageSource`: a smashing mace hurts as `mace_smash`.
    let smash = is_mace(&weapon) && can_smash_attack(p);
    let cause = if smash { Cause::Other("minecraft:mace_smash") } else { Cause::PlayerAttack };
    let source = Source { cause, attacker: Some(p.as_attacker()), direct: None, weapon: Some(weapon.clone()), position: None };
    let scale = p.attack_strength_scale(0.5);
    // `scale * (getEnchantedDamage(target, damage, source) - damage)`.
    let enchant_bonus = {
        let mut rng = w.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
        let d = scale * (w.players[a].enchanted_damage(&target_view, damage, &source, &mut rng) - damage);
        w.rng = Some(rng);
        d
    };
    let p = &mut *w.players[a];
    damage *= 0.2 + scale * scale * 0.8;
    // `onAttack`.
    p.attack_ticker = 0;
    // `deflectProjectile` (after `onAttack`, before any damage): a fireball or wind charge flies
    // on along the player's look, the player its owner; the exhaustion and wear are skipped.
    if let Target::Entity { kind: EntityClass::Redirectable, .. } = target {
        let (attacker, uuid, rot) = (p.entity_id, p.uuid.as_u128(), p.rot);
        entities::aim_deflect(w.entities, target_id, (attacker, uuid), rot);
        let (game_time, seed) = (w.game_time(), w.level.env.seed);
        play_sounds(w.players, a, &["minecraft:entity.player.attack.nodamage"], game_time, seed);
        return;
    }
    if !(damage > 0.0 || enchant_bonus > 0.0) {
        return;
    }
    let full = scale > 0.9;
    let sprint_knockback = p.sprinting && full;
    let (game_time, seed) = (w.game_time(), w.level.env.seed);
    if sprint_knockback {
        play_sounds(w.players, a, &["minecraft:entity.player.attack.knockback"], game_time, seed);
    }
    // `Item.getAttackDamageBonus`.
    damage += w.attack_damage_bonus(a, &weapon, &target_view, &source);
    let (climbing, in_water) = feet_state(&*w.players[a], &*w.level.cells);
    let p = &*w.players[a];
    let crit = full && living && p.can_critical_attack(climbing, in_water);
    if crit {
        damage *= 1.5;
    }
    let total = damage + enchant_bonus;
    let sweep = p.is_sweep_attack(full, crit, sprint_knockback);
    let health_before = w.health_of(victim);
    let old_vel = match victim {
        Victim::Player(t) => w.players[t].vel,
        _ => [0.0; 3],
    };
    let hurt = w.hurt(a, victim, total, &source);
    if !hurt {
        play_sounds(w.players, a, &["minecraft:entity.player.attack.nodamage"], game_time, seed);
        return;
    }
    // `causeExtraKnockback` with `getKnockback(target, source)`.
    let strength = {
        let mut rng = w.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
        let k = w.players[a].attack_knockback(&target_view, &source, &mut rng);
        w.rng = Some(rng);
        k + if sprint_knockback { 0.5 } else { 0.0 }
    };
    w.cause_extra_knockback(a, victim, strength, old_vel);
    if sweep {
        w.do_sweep_attack(a, victim, &target_bb, damage, &source, scale);
    }
    // `attackVisualEffects`.
    if crit {
        play_sounds(w.players, a, &["minecraft:entity.player.attack.crit"], game_time, seed);
        let pkt = entity::animate(target_id - 0, entity::animation::CRITICAL_HIT);
        send_to_trackers_and_self(w.players, a, &pkt);
    }
    if !crit && !sweep {
        play_sounds(w.players, a, &[if full { "minecraft:entity.player.attack.strong" } else { "minecraft:entity.player.attack.weak" }], game_time, seed);
    }
    if enchant_bonus > 0.0 {
        let pkt = entity::animate(target_id, entity::animation::MAGIC_CRITICAL_HIT);
        send_to_trackers_and_self(w.players, a, &pkt);
    }
    w.players[a].last_hurt_mob = Some((target_id, game_time));
    // `itemAttackInteraction`.
    item_attack_interaction(w, a, victim, living, &weapon, slot, &source);
    // `damageStatsAndHearts`: the damage statistic, heart particles for more than a heart.
    if let (Some(before), Some(after)) = (health_before, w.health_of(victim)) {
        let dealt = before - after;
        w.players[a].award_stat(*crate::player_stats::stat::DAMAGE_DEALT, (dealt * 10.0).round() as i32);
        if dealt > 2.0
            && let Some(f) = w.facts(victim)
        {
            let count = (dealt as f64 * 0.5) as i32;
            let at = [f.pos[0], f.pos[1] + f.height as f64 * 0.5, f.pos[2]];
            send_particles(w.players, "minecraft:damage_indicator", at, count, [0.1, 0.0, 0.1], 0.2);
        }
    }
    w.players[a].exhaust(0.1);
}

/// `Player.itemAttackInteraction(target, weapon, source, true)`.
fn item_attack_interaction(w: &mut Work<'_, '_, '_>, a: usize, victim: Victim, living: bool, weapon: &ItemStack, slot: EquipmentSlot, source: &Source) {
    let per_attack = weapon.get(keys::WEAPON).map(|x| x.item_damage_per_attack);
    let mut hurt_enemy = false;
    if living && !weapon.is_empty() {
        // `ItemStack.hurtEnemy`: `Item.hurtEnemy` (the mace's smash), and a weapon counts as used.
        if is_mace(weapon)
            && let Some(f) = w.facts(victim)
        {
            w.mace_hurt_enemy(a, victim, &f);
        }
        if per_attack.is_some() {
            w.players[a].award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, weapon.item()), 1);
            hurt_enemy = true;
        }
    }
    w.post_attack(a, victim, source);
    if living && !weapon.is_empty() && hurt_enemy {
        // `ItemStack.postHurtEnemy`: the mace forgets its fall; the weapon wears.
        if is_mace(weapon) && can_smash_attack(&*w.players[a]) {
            w.players[a].fall_distance = 0.0;
        }
        if let Some(n) = per_attack
            && w.players[a].inv.equipped(slot).item() == weapon.item()
        {
            w.wear_weapon(a, slot, n);
        }
    }
}

/// Attack from `handle_attack` / the riptide spin, with the level random lent by the attacker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_attack(
    entities: &mut Entities,
    level: &mut RegionLevel<'_>,
    players: &mut [&mut Player],
    a: usize,
    target: Target,
    target_id: i32,
    spin: Option<&Spin>,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<Death>,
) {
    let lent = std::mem::replace(&mut players[a].level_rng, LegacyRandom::new(0));
    let mut w = Work { entities, level, players, spawns, deaths, rng: Some(lent) };
    attack(&mut w, a, target, target_id, spin);
    if let Some(r) = w.rng.take() {
        w.players[a].level_rng = r;
    }
}

// `BURNING_TIME` is read by `Player::ignite_for_seconds`.
const _: () = {
    let _ = BURNING_TIME;
};
