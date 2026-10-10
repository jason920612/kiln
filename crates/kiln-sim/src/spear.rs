//! Spears for players (the 26.x `piercing_weapon` and `kinetic_weapon` components): the stab
//! (`ServerboundPlayerActionPacket` STAB → `PiercingWeapon.attack`) and the charge
//! (`KineticWeapon.damageEntities` every tick of use), both through `Player.stabAttack`, and what
//! a stab leaves the wielder with (`EnchantmentHelper.doPostPiercingAttackEffects`: the lunge
//! enchantment).
//!
//! Victims are the region's other players and its entities (the same ones a plain attack can
//! reach); each is hurt, pushed and dismounted as `stabAttack` says, one after the other.

use crate::blocks::RegionLevel;
use crate::combat::{ATTACK_DAMAGE, EntityClass, classify, dist2, mth_cos, mth_sin};
use crate::entities::{Entities, Spawn};
use crate::health::{Cause, DamageCtx, Death, Source};
use crate::Player;
use kiln_entity::level::DamageKind;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::spear::{self, Candidate, Hit};
use kiln_item::component::{AttackRange, EquipmentSlot, KineticWeapon, SwingAnimation};
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::LegacyRandom;
use kiln_loot::effects::EntityEffect;
use kiln_world::Blocks;
use kiln_proto::packets::entity;
use kiln_proto::packets::world_fx::{self, SoundSource};

/// What `stabAttack` was asked to do (`damage`, `knockback` and `dismount` are its booleans).
#[derive(Debug, Clone, Copy)]
struct Stab {
    amount: f32,
    damage: bool,
    knockback: bool,
    dismount: bool,
}

/// Who a stab reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Victim {
    /// The region's player at this index.
    Player(usize),
    /// A non-player entity, by network id.
    Entity(i32),
}

/// A name for a sound component's event.
fn sound_name(s: &kiln_item::component::SoundEvent) -> Option<String> {
    match s {
        kiln_item::Holder::Reference(id) => kiln_item::registry::SOUND_EVENT.name(*id).map(str::to_owned),
        kiln_item::Holder::Direct(def) => Some(def.sound_id.as_str().to_owned()),
    }
}

/// `Level.playSound(except, x, y, z, sound, source, volume, pitch)`: the players within hearing,
/// except one (the wielder, whose client plays its own).
pub(crate) fn play_sound(players: &mut [&mut Player], at: [f64; 3], sound: &str, source: SoundSource, volume: f32, pitch: f32, except: Option<usize>, game_time: i64) {
    let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
    let mut seed = (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ id as u64;
    for v in [at[0].to_bits(), at[1].to_bits(), at[2].to_bits()] {
        seed = (seed ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        seed ^= seed >> 31;
    }
    let pkt = world_fx::sound(&world_fx::Sound::Registered(id), source, at, volume, pitch, seed as i64);
    let range = if volume > 1.0 { 16.0 * volume as f64 } else { 16.0 };
    for (i, p) in players.iter_mut().enumerate() {
        if Some(i) != except && dist2(p.pos, at) < range * range {
            p.send(pkt.clone());
        }
    }
}

impl Player {
    /// `LivingEntity.swing(MAIN_HAND, animation, false)` with the main hand's `attack_animation`
    /// (`handlePunch`): whether a swing began.
    pub(crate) fn swing_main_hand(&mut self) -> bool {
        let anim = self.inv.selected_item().get(keys::ATTACK_ANIMATION).copied().unwrap_or_default();
        self.swing(anim)
    }

    /// `LivingEntity.swing(hand, animation, false)`: a swing begins unless the current one is in
    /// its first half; viewers are told when it does. The length is shortened by haste and
    /// lengthened by mining fatigue.
    pub(crate) fn swing(&mut self, anim: SwingAnimation) -> bool {
        let wire = anim.duration;
        let duration = self.modified_swing_duration(anim.duration);
        // `SwingState.startIfAble`.
        if self.swing_duration > 0 && self.swing_ticks <= self.swing_duration / 2 && self.swing_ticks > 0 {
            return false;
        }
        self.swing_ticks = -1;
        self.swing_duration = duration.max(1);
        self.swing_kind = match anim.kind {
            kiln_item::component::SwingAnimationType::None => entity::swing::NONE,
            kiln_item::component::SwingAnimationType::Whack => entity::swing::WHACK,
            kiln_item::component::SwingAnimationType::Stab => entity::swing::STAB,
        };
        self.swing_wire_duration = wire;
        self.swung = true;
        true
    }

    /// `LivingEntity.getModifiedSwingDuration`.
    fn modified_swing_duration(&self, duration: i32) -> i32 {
        // `MobEffectUtil.hasDigSpeed` / `getDigSpeedAmplification`: haste or conduit power.
        let (haste, conduit) = (self.effect_amplifier("minecraft:haste"), self.effect_amplifier("minecraft:conduit_power"));
        if haste.is_some() || conduit.is_some() {
            return duration - (1 + haste.unwrap_or(0).max(conduit.unwrap_or(0)));
        }
        if let Some(amp) = self.effect_amplifier("minecraft:mining_fatigue") {
            return duration + (1 + amp) * 2;
        }
        duration
    }

    /// `SwingState.tick`.
    pub(crate) fn tick_swing(&mut self) {
        if self.swing_duration > 0 {
            let old = self.swing_ticks;
            self.swing_ticks += 1;
            if old > self.swing_duration {
                self.swing_duration = 0;
                self.swing_ticks = 0;
            }
        }
    }

    /// `KineticWeapon.makeSound(player)`: the weapon's use sound, heard by everyone near but the
    /// player (whose client plays its own).
    pub(crate) fn make_kinetic_sound(&mut self, sound: &Option<kiln_item::component::SoundEvent>) {
        let Some(name) = sound.as_ref().and_then(sound_name) else { return };
        let Some(id) = kiln_data::builtin_id("minecraft:sound_event", &name) else { return };
        let seed = kiln_javamath::random::RandomSource::next_long(&mut self.sound_seed);
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), SoundSource::Players, self.pos, 1.0, 1.0, seed);
        self.pending_sounds.push(pkt);
    }

    /// The item in a slot of the wielder (hands only).
    fn weapon_in(&self, slot: EquipmentSlot) -> ItemStack {
        self.inv.equipped(slot).clone()
    }
}

/// `Entity.getRootVehicle` (`isPassengerOfSameVehicle` compares them): an entity id of the
/// region, or a player by network id.
type Root = i32;

fn root_of_entity(entities: &Entities, id: i32) -> Root {
    let mut at = id;
    for _ in 0..64 {
        let Ok(i) = entities.list.binary_search_by_key(&at, |e| e.id) else { break };
        match entities.list[i].phys.as_deref().and_then(|p| p.vehicle) {
            Some(v) => at = v,
            None => break,
        }
    }
    at
}

fn root_of_player(entities: &Entities, p: &Player) -> Root {
    match p.vehicle {
        Some(v) => root_of_entity(entities, v),
        None => p.entity_id,
    }
}

/// The wielder's state a stab or charge starts from.
struct Wielder {
    eye: Vec3,
    /// `getLookAngle` (the head's: the same for a player).
    look: Vec3,
    known_movement: Vec3,
    creative: bool,
}

impl Wielder {
    fn of(p: &Player) -> Wielder {
        let eye = p.eye_position();
        Wielder {
            eye: Vec3::new(eye[0], eye[1], eye[2]),
            look: crate::use_item::view_vector(p.rot),
            known_movement: Vec3::new(p.known_movement[0], p.known_movement[1], p.known_movement[2]),
            creative: p.game_mode == 1,
        }
    }
}

/// `LivingEntity.getAttackRangeWith(stack)`: the item's `attack_range`, else the default (up to
/// the entity interaction range, no margin).
fn attack_range_with(p: &Player, stack: &ItemStack) -> AttackRange {
    match stack.get(keys::ATTACK_RANGE) {
        Some(r) => r.clone(),
        None => {
            let r = p.attribute(crate::combat::ENTITY_INTERACTION_RANGE) as f32;
            AttackRange { min_reach: 0.0, max_reach: r, min_creative_reach: 0.0, max_creative_reach: r, hitbox_margin: 0.0, mob_factor: 1.0 }
        }
    }
}

/// `StabWork`: the region the stab happens in, with the level random the stab's enchantment
/// effects draw from lent by the wielder.
struct Work<'a, 'l, 'p> {
    entities: &'a mut Entities,
    level: &'a mut RegionLevel<'l>,
    players: &'a mut [&'p mut Player],
    spawns: &'a mut Vec<Spawn>,
    deaths: &'a mut Vec<Death>,
    rng: Option<LegacyRandom>,
}

impl<'a, 'l, 'p> Work<'a, 'l, 'p> {
    /// Runs `f` with a [`DamageCtx`] holding the lent random.
    fn with_ctx<R>(&mut self, f: impl FnOnce(&mut [&'p mut Player], &mut DamageCtx<'_>) -> R) -> R {
        let env = self.level.env;
        let mut ctx = DamageCtx { rules: env.damage, game_time: env.game_time, spawns: &mut *self.spawns, deaths: &mut *self.deaths, level_rng: self.rng.take() };
        let r = f(&mut *self.players, &mut ctx);
        self.rng = ctx.level_rng.take();
        r
    }

    /// `Level.clip` with colliders, no fluids: the first block hit.
    fn clip(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let cells = &*self.level.cells;
        kiln_entity::clip::traverse_blocks(from, to, |p| {
            let s = cells.get_block(p.x, p.y, p.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR);
            let (shape, _) = kiln_entity::collision::collision_shape(s, p, &kiln_entity::collision::CollisionContext::EMPTY);
            kiln_entity::clip::shape_clip(&shape, from, to, p).map(|(l, _)| l)
        })
    }

    /// `PiercingWeapon.canHitEntity` for each entity the wielder `a` could reach, as victims with
    /// their boxes, in id order.
    fn victims(&self, a: usize) -> Vec<(Victim, Candidate)> {
        let attacker = &*self.players[a];
        let root = root_of_player(self.entities, attacker);
        let mut out: Vec<(Victim, Candidate)> = Vec::new();
        for (i, p) in self.players.iter().enumerate() {
            if i == a || p.dead || p.disconnected || p.game_mode == 3 {
                continue;
            }
            // `Player.canHarmPlayer`: pvp.
            if !self.level.env.damage.pvp {
                continue;
            }
            if root_of_player(self.entities, p) == root {
                continue;
            }
            out.push((Victim::Player(i), Candidate { id: p.entity_id, bb: p.bounding_box() }));
        }
        for e in self.entities.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            // `Entity.isInvulnerableToPiercingWeapon`, `canBeHitByProjectile`.
            if phys.invulnerable || phys.invulnerable_time > 0 || !phys.is_alive() {
                continue;
            }
            // (`canBeHitByProjectile`: also what `deflectProjectile` turns around.)
            if !matches!(classify(phys), EntityClass::Mob | EntityClass::Redirectable) {
                continue;
            }
            if root_of_entity(self.entities, e.id) == root {
                continue;
            }
            out.push((Victim::Entity(e.id), Candidate { id: e.id, bb: phys.bounding_box() }));
        }
        out.sort_by_key(|(_, c)| c.id);
        out
    }

    /// `ProjectileUtil.getHitEntitiesAlong` for wielder `a` with `range`: who is hit, in order.
    fn reach(&self, a: usize, range: &AttackRange) -> Vec<(Victim, Hit)> {
        let w = Wielder::of(self.players[a]);
        let (min, max) = spear::effective_range(range, true, w.creative);
        let victims = self.victims(a);
        let candidates: Vec<Candidate> = victims.iter().map(|(_, c)| *c).collect();
        let clip = |from: Vec3, to: Vec3| self.clip(from, to);
        let hits = spear::hit_entities_along(w.eye, w.look, min, max, w.known_movement, range.hitbox_margin, &clip, &candidates);
        hits.into_iter().filter_map(|h| victims.iter().find(|(_, c)| c.id == h.id).map(|(v, _)| (*v, h))).collect()
    }

    /// `Player.stabAttack(slot, target, amount, damage, knockback, dismount)`.
    fn stab_attack(&mut self, a: usize, victim: Victim, slot: EquipmentSlot, s: Stab) -> bool {
        let p = &*self.players[a];
        let weapon = p.weapon_in(slot);
        // `ItemStack.getDamageSource(attacker)`: the item's damage type, else a player attack.
        let cause = match weapon.get(keys::DAMAGE_TYPE).and_then(|t| kiln_item::registry::DAMAGE_TYPE.name(t.0)) {
            Some(name) => Cause::Other(crate::health::static_damage_type(name)),
            None => Cause::PlayerAttack,
        };
        let source = Source { cause, attacker: Some(p.as_attacker()), direct: None, weapon: Some(weapon.clone()), position: None };
        // `isUsingItem() && getUsedItemHand().asEquipmentSlot() == slot`: charging with the weapon.
        let charging = p.using.is_some_and(|u| u.off_hand == (slot == EquipmentSlot::OffHand));
        let (view, victim_id) = match victim {
            Victim::Player(t) => (self.players[t].view(), self.players[t].entity_id),
            Victim::Entity(id) => {
                let e = self.entities.list.binary_search_by_key(&id, |e| e.id).ok().and_then(|i| self.entities.list[i].phys.as_deref());
                let (type_id, pos) = e.map_or((-1, [0.0; 3]), |e| {
                    let v = e.position();
                    (kiln_item::registry::ENTITY_TYPE.id(e.type_name).unwrap_or(-1), [v.x, v.y, v.z])
                });
                (crate::enchant::EntityView { type_id, pos, ..Default::default() }, id)
            }
        };
        // `getEnchantedDamage(target, amount, source) - amount`, scaled by the attack strength
        // unless the weapon is in use; the amount itself scales (`baseDamageScaleFactor`) too.
        let mut amount = s.amount;
        let mut enchant_bonus = {
            let mut rng = self.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
            let v = self.players[a].enchanted_damage(&view, amount, &source, &mut rng) - amount;
            self.rng = Some(rng);
            v
        };
        if !charging {
            enchant_bonus *= self.players[a].attack_strength_scale(0.5);
            amount *= 0.2 + {
                let scale = self.players[a].attack_strength_scale(0.5);
                scale * scale * 0.8
            };
        }
        // `if (dealsKnockback && deflectProjectile(target)) return true`: a fireball or wind
        // charge hit by a stab with knockback flies on along the wielder's look. Without the
        // knockback the stab hurts nothing (`AbstractHurtingProjectile.hurtServer` is false).
        if let Victim::Entity(id) = victim
            && let Ok(i) = self.entities.list.binary_search_by_key(&id, |e| e.id)
            && self.entities.list[i].phys.as_deref().is_some_and(|e| matches!(classify(e), EntityClass::Redirectable))
        {
            if !s.knockback {
                return false;
            }
            let (by, at, rot) = ((self.players[a].entity_id, self.players[a].uuid.as_u128()), self.players[a].pos, self.players[a].rot);
            crate::entities::aim_deflect(self.entities, id, by, rot);
            play_sound(self.players, at, "minecraft:entity.player.attack.nodamage", SoundSource::Players, 1.0, 1.0, None, self.level.env.game_time);
            return true;
        }
        let dealt = if s.damage { amount + enchant_bonus } else { 0.0 };
        match victim {
            Victim::Player(t) => self.stab_player(a, t, slot, &weapon, &source, s, dealt, enchant_bonus),
            Victim::Entity(id) => self.stab_entity(a, id, victim_id, slot, &weapon, &source, s, dealt, enchant_bonus, &view),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stab_player(&mut self, a: usize, t: usize, slot: EquipmentSlot, weapon: &ItemStack, source: &Source, s: Stab, dealt: f32, enchant_bonus: f32) -> bool {
        let health_before = self.players[t].health;
        let old_vel = self.players[t].vel;
        let hurt = s.damage && self.with_ctx(|players, ctx| players[t].hurt(dealt, source, ctx));
        if s.knockback {
            let view = self.players[t].view();
            let rng = &mut self.rng;
            let strength = {
                let mut r = rng.take().unwrap_or_else(|| LegacyRandom::new(0));
                let k = self.players[a].attack_knockback(&view, source, &mut r);
                *rng = Some(r);
                k
            };
            self.extra_knockback_player(a, t, 0.4, old_vel);
            self.extra_knockback_player(a, t, strength, old_vel);
        }
        let mut dismounted = false;
        if s.dismount && self.players[t].vehicle.is_some() {
            dismounted = true;
            self.stop_riding_player(t);
        }
        if !hurt && !s.knockback && !dismounted {
            return false;
        }
        self.finish_stab(a, Victim::Player(t), slot, weapon, source, hurt, health_before, enchant_bonus);
        true
    }

    /// `Player.causeExtraKnockback` on a player victim.
    fn extra_knockback_player(&mut self, a: usize, t: usize, strength: f32, old_vel: [f64; 3]) {
        if strength <= 0.0 {
            return;
        }
        let rad = (self.players[a].rot[0] * 0.017453292) as f64;
        self.players[t].knockback(strength as f64, mth_sin(rad) as f64, -mth_cos(rad) as f64);
        let p = &mut *self.players[a];
        p.vel = [p.vel[0] * 0.6, p.vel[1], p.vel[2] * 0.6];
        if p.sprinting {
            p.sprinting = false;
            p.meta_dirty = true;
        }
        let victim = &mut *self.players[t];
        if victim.sync_velocity {
            victim.send(entity::set_entity_motion(victim.entity_id, victim.vel));
            victim.sync_velocity = false;
            victim.vel = old_vel;
        }
    }

    /// `Entity.stopRiding` for a player.
    fn stop_riding_player(&mut self, t: usize) {
        let id = self.players[t].entity_id;
        if let Some(v) = self.players[t].vehicle.take() {
            if let Ok(i) = self.entities.list.binary_search_by_key(&v, |e| e.id)
                && let Some(phys) = self.entities.list[i].phys.as_deref_mut()
            {
                kiln_entity::ride::remove_passenger(phys, id);
            }
            self.players[t].vehicle_type = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stab_entity(
        &mut self,
        a: usize,
        id: i32,
        victim_id: i32,
        slot: EquipmentSlot,
        weapon: &ItemStack,
        source: &Source,
        s: Stab,
        dealt: f32,
        enchant_bonus: f32,
        view: &crate::enchant::EntityView,
    ) -> bool {
        let strengths = if s.knockback {
            let mut r = self.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
            let k = self.players[a].attack_knockback(view, source, &mut r);
            self.rng = Some(r);
            [0.4, k]
        } else {
            [0.0, 0.0]
        };
        let fire = weapon
            .get(keys::ENCHANTMENTS)
            .map_or(0, |e| e.level(kiln_item::registry::ENCHANTMENT.id("minecraft:fire_aspect").unwrap_or(-1)));
        let p = &*self.players[a];
        let stab = crate::entities::MobStab {
            target: id,
            attacker: p.entity_id,
            attacker_pos: p.pos,
            yaw: p.rot[0],
            kind: DamageKind::of_type(source.cause.damage_type()),
            amount: dealt,
            damage: s.damage,
            knockbacks: strengths,
            dismount: s.dismount,
            fire_seconds: if s.damage { 4.0 * fire as f32 } else { 0.0 },
        };
        let Some(out) = crate::entities::stab_mob(self.entities, self.level, self.players, self.spawns, self.deaths, &stab) else { return false };
        // The attacker slows down for each knockback (`Player.causeExtraKnockback`).
        for k in strengths {
            if k > 0.0 {
                let p = &mut *self.players[a];
                p.vel = [p.vel[0] * 0.6, p.vel[1], p.vel[2] * 0.6];
                if p.sprinting {
                    p.sprinting = false;
                    p.meta_dirty = true;
                }
            }
        }
        if !out.hurt && !s.knockback && !out.dismounted {
            return false;
        }
        let health_before = out.health_before.unwrap_or(0.0);
        let _ = victim_id;
        self.finish_stab(a, Victim::Entity(id), slot, weapon, source, out.hurt, health_before, enchant_bonus);
        true
    }

    /// The end of `Player.stabAttack`: the magic critical particles, the last mob hurt, the
    /// weapon's wear and the effects of its enchantments, the damage statistic and hearts, and
    /// the exhaustion.
    #[allow(clippy::too_many_arguments)]
    fn finish_stab(&mut self, a: usize, victim: Victim, slot: EquipmentSlot, weapon: &ItemStack, source: &Source, hurt: bool, health_before: f32, enchant_bonus: f32) {
        let (victim_id, living) = match victim {
            Victim::Player(t) => (self.players[t].entity_id, true),
            Victim::Entity(id) => {
                let living = self.entities.list.binary_search_by_key(&id, |e| e.id).ok().and_then(|i| self.entities.list[i].phys.as_deref()).is_some_and(|e| kiln_entity::mob::data(e).is_some());
                (id, living)
            }
        };
        // `attackVisualEffects(target, false, false, damage, true, enchantBonus)`: no sounds, the
        // enchanted hit particles for a bonus.
        if enchant_bonus > 0.0 {
            let pkt = entity::animate(victim_id, entity::animation::MAGIC_CRITICAL_HIT);
            crate::combat::send_to_trackers_and_self(self.players, a, &pkt);
        }
        let game_time = self.level.env.game_time;
        self.players[a].last_hurt_mob = Some((victim_id, game_time));
        // `itemAttackInteraction`: `hurtEnemy` wears a weapon (a living victim), then the
        // post-attack enchantment effects of a hurt victim.
        if living {
            let per_attack = weapon.get(keys::WEAPON).map(|w| w.item_damage_per_attack);
            if let Some(n) = per_attack
                && !weapon.is_empty()
            {
                self.players[a].award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, weapon.item()), 1);
                if self.players[a].inv.equipped(slot).item() == weapon.item() {
                    let mut rng = self.rng.take();
                    self.players[a].hurt_and_break(slot, n, rng.as_mut());
                    self.rng = rng;
                }
            }
        }
        if hurt && let Victim::Player(t) = victim {
            self.with_ctx(|players, ctx| crate::combat::post_attack(players, a, t, source, ctx));
        }
        // `damageStatsAndHearts`.
        let health_now = match victim {
            Victim::Player(t) => Some(self.players[t].health),
            Victim::Entity(id) => self.entities.list.binary_search_by_key(&id, |e| e.id).ok().and_then(|i| self.entities.list[i].phys.as_deref()).and_then(|e| kiln_entity::mob::data(e)).map(|m| m.health),
        };
        if let Some(now) = health_now {
            let dealt = health_before - now;
            self.players[a].award_stat(*crate::player_stats::stat::DAMAGE_DEALT, (dealt * 10.0).round() as i32);
            if dealt > 2.0 {
                let count = (dealt as f64 * 0.5) as i32;
                let at = match victim {
                    Victim::Player(t) => [self.players[t].pos[0], self.players[t].pos[1] + 0.9, self.players[t].pos[2]],
                    Victim::Entity(id) => {
                        let e = self.entities.list.binary_search_by_key(&id, |e| e.id).ok().and_then(|i| self.entities.list[i].phys.as_deref());
                        e.map_or([0.0; 3], |e| [e.x(), e.y() + e.height as f64 * 0.5, e.z()])
                    }
                };
                crate::combat::send_particles(self.players, "minecraft:damage_indicator", at, count, [0.1, 0.0, 0.1], 0.2);
            }
        }
        self.players[a].exhaust(0.1);
    }

    /// The attacker's own sound: `PiercingWeapon.makeSound` / `KineticWeapon.makeSound`.
    fn sound_of_wielder(&mut self, a: usize, sound: &Option<kiln_item::component::SoundEvent>, skip_wielder: bool) {
        let Some(name) = sound.as_ref().and_then(sound_name) else { return };
        let at = self.players[a].pos;
        let game_time = self.level.env.game_time;
        play_sound(self.players, at, &name, SoundSource::Players, 1.0, 1.0, skip_wielder.then_some(a), game_time);
    }

    /// `EnchantmentHelper.doPostPiercingAttackEffects` for the wielder's main hand.
    fn post_piercing_attack(&mut self, a: usize) {
        let Some(loot) = self.players[a].loot.clone() else { return };
        let weapon = self.players[a].inv.equipped(EquipmentSlot::MainHand).clone();
        let cells = &*self.level.cells;
        let block = |pos: BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0);
        let view = self.players[a].view_in(&block);
        let mut rng = self.rng.take().unwrap_or_else(|| LegacyRandom::new(0));
        let mut effects: Vec<(&EntityEffect, i32)> = Vec::new();
        loot.post_piercing_effects(&weapon, EquipmentSlot::MainHand, &mut rng, |level| WielderContext { level, view: &view }, |e, level| effects.push((e, level)));
        self.rng = Some(rng);
        for (effect, level) in effects {
            self.apply_wielder_effect(a, effect, level);
        }
    }

    fn apply_wielder_effect(&mut self, a: usize, effect: &EntityEffect, level: i32) {
        match effect {
            EntityEffect::AllOf(list) => {
                for e in list {
                    self.apply_wielder_effect(a, e, level);
                }
            }
            EntityEffect::ChangeItemDamage(amount) => {
                let stack = self.players[a].inv.equipped(EquipmentSlot::MainHand);
                if stack.get(keys::MAX_DAMAGE).is_some() && stack.get(keys::DAMAGE).is_some() {
                    let mut rng = self.rng.take();
                    self.players[a].hurt_and_break(EquipmentSlot::MainHand, amount.calculate(level) as i32, rng.as_mut());
                    self.rng = rng;
                }
            }
            EntityEffect::ApplyExhaustion(amount) => self.players[a].exhaust(amount.calculate(level)),
            EntityEffect::ApplyImpulse { direction, coordinate_scale, magnitude } => {
                let p = &mut *self.players[a];
                let look = look_rotate(p.rot, *direction);
                let m = magnitude.calculate(level) as f64;
                let impulse = [look[0] * coordinate_scale[0] * m, look[1] * coordinate_scale[1] * m, look[2] * coordinate_scale[2] * m];
                p.vel = [p.vel[0] + impulse[0], p.vel[1] + impulse[1], p.vel[2] + impulse[2]];
                p.send(entity::set_entity_motion(p.entity_id, p.vel));
                p.sync_velocity = false;
            }
            EntityEffect::PlaySound { sounds, volume, pitch } => {
                let p = &mut *self.players[a];
                if sounds.is_empty() {
                    return;
                }
                let index = (level - 1).clamp(0, sounds.len() as i32 - 1) as usize;
                let (v, pi) = (volume.sample(&mut p.entity_rng), pitch.sample(&mut p.entity_rng));
                let (at, game_time) = (p.pos, self.level.env.game_time);
                play_sound(self.players, at, sounds[index].as_str(), SoundSource::Players, v, pi, None, game_time);
            }
            _ => {}
        }
    }
}

/// `Entity.getLookQuaternion().transform(direction)`: `direction` turned by the player's
/// rotation (yaw about y, then pitch about x), in the floats `Quaternionf` works in.
fn look_rotate(rot: [f32; 2], direction: [f64; 3]) -> [f64; 3] {
    // `Quaternionf.rotationYXZ(-yRot * (PI / 180), xRot * (PI / 180), 0)`.
    let (ay, ax) = (-rot[0] * 0.017453292, rot[1] * 0.017453292);
    let sx = kiln_javamath::trig::sin((ax * 0.5) as f64) as f32;
    let cx = cos_from_sin(sx, ax * 0.5);
    let sy = kiln_javamath::trig::sin((ay * 0.5) as f64) as f32;
    let cy = cos_from_sin(sy, ay * 0.5);
    let (x, y, z, w) = (cy * sx, sy * cx, -sy * sx, cy * cx);
    // `Quaternionf.transform(x, y, z, dest)` in `double`s from the float components.
    let (qx, qy, qz, qw) = (x as f64, y as f64, z as f64, w as f64);
    let (d, e) = (direction, 2.0);
    let _ = e;
    let w2 = qw * qw;
    let x2 = qx * qx;
    let y2 = qy * qy;
    let z2 = qz * qz;
    let zw = qz * qw;
    let xy = qx * qy;
    let xz = qx * qz;
    let yw = qy * qw;
    let yz = qy * qz;
    let xw = qx * qw;
    let (dx, dy, dz) = (d[0], d[1], d[2]);
    [
        (w2 + x2 - z2 - y2) * dx + (-zw + xy - zw + xy) * dy + (yw + xz + xz + yw) * dz,
        (xy + zw + zw + xy) * dx + (y2 - z2 + w2 - x2) * dy + (yz + yz - xw - xw) * dz,
        (xz - yw + xz - yw) * dx + (yz + yz + xw + xw) * dy + (z2 - y2 - x2 + w2) * dz,
    ]
}

/// `Math.cosFromSin(sin, angle)` of JOML (the non-fast variant): the cosine from the sine,
/// signed by where the angle falls.
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0f32 - sin * sin).sqrt();
    let a = angle + std::f32::consts::FRAC_PI_2;
    let b = a - ((a / std::f32::consts::TAU) as i32) as f32 * std::f32::consts::TAU;
    let b = if b < 0.0 { std::f32::consts::TAU + b } else { b };
    if b >= std::f32::consts::PI { -cos } else { cos }
}

/// `Enchantment.entityContext`: the wielder as `this_entity`, at its position, with the
/// enchantment's level.
struct WielderContext<'a> {
    level: i32,
    view: &'a crate::enchant::EntityView,
}

impl kiln_loot::LootContext for WielderContext<'_> {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        target == kiln_loot::EntityTarget::This
    }

    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.view.pos)
    }

    fn enchantment_level(&self) -> Option<i32> {
        Some(self.level)
    }

    fn entity_matches(&self, target: kiln_loot::EntityTarget, predicate: &kiln_loot::predicate::EntityPredicate) -> bool {
        target == kiln_loot::EntityTarget::This && self.view.matches(predicate)
    }
}

/// `ServerboundPlayerActionPacket.Action.STAB` → `PiercingWeapon.attack(player, MAINHAND)`.
pub(crate) fn piercing_attack(
    entities: &mut Entities,
    level: &mut RegionLevel<'_>,
    players: &mut [&mut Player],
    a: usize,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<Death>,
) {
    let p = &*players[a];
    if p.game_mode == 3 || p.dead || !p.client_loaded() {
        return;
    }
    let held = p.in_hand(false).clone();
    if p.cannot_attack_with_item(&held, 5) {
        return;
    }
    let Some(piercing) = held.get(keys::PIERCING_WEAPON).cloned() else { return };
    // The attack draws enchantment randomness from the wielder's level random.
    let lent = std::mem::replace(&mut players[a].level_rng, LegacyRandom::new(0));
    let mut w = Work { entities, level, players, spawns, deaths, rng: Some(lent) };
    let damage = w.players[a].attribute(ATTACK_DAMAGE) as f32;
    let anim = held.get(keys::ATTACK_ANIMATION).copied().unwrap_or_default();
    let range = attack_range_with(w.players[a], &held);
    let mut any = false;
    for (victim, _) in w.reach(a, &range) {
        any |= w.stab_attack(a, victim, EquipmentSlot::MainHand, Stab { amount: damage, damage: true, knockback: piercing.deals_knockback, dismount: piercing.dismounts });
    }
    // `LivingEntity.onAttack` (nothing), `postPiercingAttack`, the sounds, the swing.
    w.post_piercing_attack(a);
    if any {
        w.sound_of_wielder(a, &piercing.hit_sound, false);
    }
    w.sound_of_wielder(a, &piercing.sound, true);
    let started = w.players[a].swing(anim);
    if started {
        w.players[a].attack_ticker = 0;
    }
    let rng = w.rng.take();
    if let Some(r) = rng {
        w.players[a].level_rng = r;
    }
}

/// `KineticWeapon.damageEntities` for player `a` (whose weapon has been in use for `ticks_used`
/// ticks, the use counted from the start).
pub(crate) fn kinetic_attack(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    a: usize,
    ticks_used: i32,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<Death>,
) {
    let p = &*players[a];
    let Some(using) = p.using else { return };
    let slot = if using.off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let held = p.weapon_in(slot);
    let Some(kinetic) = held.get(keys::KINETIC_WEAPON).cloned() else { return };
    let mut ticks = ticks_used;
    if ticks < kinetic.delay_ticks {
        return;
    }
    ticks -= kinetic.delay_ticks;
    let lent = std::mem::replace(&mut players[a].level_rng, LegacyRandom::new(0));
    let mut w = Work { entities, level, players, spawns, deaths, rng: Some(lent) };
    kinetic_hits(&mut w, a, slot, &held, &kinetic, ticks);
    if let Some(r) = w.rng.take() {
        w.players[a].level_rng = r;
    }
}

fn kinetic_hits(w: &mut Work<'_, '_, '_>, a: usize, slot: EquipmentSlot, held: &ItemStack, kinetic: &KineticWeapon, ticks: i32) {
    let wielder = Wielder::of(w.players[a]);
    let my_speed = wielder.look.dot(motion_of_player(w, a));
    // (A player's contacts count the way a player's do: `mobFactor` 1.)
    let mob_factor = 1.0f64;
    let range = attack_range_with(w.players[a], held);
    let base_damage = w.players[a].with_base(ATTACK_DAMAGE).default_base();
    let mut any = false;
    let mut stabbed = 0;
    for (victim, _) in w.reach(a, &range) {
        let id = match victim {
            Victim::Player(t) => w.players[t].entity_id,
            Victim::Entity(id) => id,
        };
        // `wasRecentlyStabbed` / `rememberStabbedEntity`.
        let now = w.level.env.game_time;
        if let Some(&(_, at)) = w.players[a].recent_stabs.iter().find(|(e, _)| *e == id)
            && now - at < kinetic.contact_cooldown_ticks as i64
        {
            continue;
        }
        w.players[a].recent_stabs.retain(|(e, _)| *e != id);
        w.players[a].recent_stabs.push((id, now));
        let their = wielder.look.dot(motion_of_victim(w, victim));
        let Some(hit) = spear::kinetic_hit(kinetic, ticks, my_speed * 1.0, their, mob_factor, base_damage) else { continue };
        stabbed += 1;
        any |= w.stab_attack(a, victim, slot, Stab { amount: hit.amount, damage: hit.damage, knockback: hit.knockback, dismount: hit.dismount });
    }
    let _ = stabbed;
    if any {
        // `broadcastEntityEvent(attacker, 2)`: viewers (and the wielder) hear the hit.
        let pkt = entity::entity_event(w.players[a].entity_id, 2);
        crate::combat::send_to_trackers_and_self(w.players, a, &pkt);
        // `SpearMobsTrigger`: the living things in the weapon's memory of recent stabs.
        let living = w.players[a]
            .recent_stabs
            .iter()
            .filter(|(id, _)| {
                w.players.iter().any(|q| q.entity_id == *id) || w.entities.list.binary_search_by_key(id, |e| e.id).ok().and_then(|i| w.entities.list[i].phys.as_deref()).is_some_and(|e| kiln_entity::mob::data(e).is_some())
            })
            .count() as i32;
        w.players[a].spear_mobs(living);
    }
}

/// `KineticWeapon.getMotion(entity)` for a player: the known speed per second.
fn motion_of_player(w: &Work<'_, '_, '_>, a: usize) -> Vec3 {
    let k = w.players[a].known_movement;
    Vec3::new(k[0], k[1], k[2]).scale(spear::MOTION_SCALE)
}

/// `KineticWeapon.getMotion(entity)` for a victim: a player's known speed, a mob's (or its root
/// vehicle's, for a rider) last known speed, per second.
fn motion_of_victim(w: &Work<'_, '_, '_>, victim: Victim) -> Vec3 {
    match victim {
        Victim::Player(t) => {
            let k = w.players[t].known_movement;
            Vec3::new(k[0], k[1], k[2]).scale(spear::MOTION_SCALE)
        }
        Victim::Entity(id) => {
            let root = root_of_entity(w.entities, id);
            w.entities
                .list
                .binary_search_by_key(&root, |e| e.id)
                .ok()
                .and_then(|i| w.entities.list[i].phys.as_deref())
                .map_or(Vec3::ZERO, |e| e.last_known_speed.scale(spear::MOTION_SCALE))
        }
    }
}
