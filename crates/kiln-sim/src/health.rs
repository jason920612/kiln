//! Player health, damage, death and respawn (vanilla `LivingEntity.hurt`/`die`,
//! `ServerPlayer.die`, `PlayerList.respawn`).
//!
//! Food follows `FoodData`: exhaustion from sprinting, jumping and breaking blocks uses up
//! saturation then food; a well-fed player heals, a starving one takes damage. Damage is not
//! reduced by armor, enchantments or effects yet, and eating is not implemented yet.

use crate::{Player, entities};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_proto::packets::entity;

pub(crate) const MAX_HEALTH: f32 = 20.0;
/// `LivingEntity.onBelowWorld`: damage per tick below the world.
const VOID_DAMAGE: f32 = 4.0;
/// Players take void damage this far below the dimension's bottom.
pub(crate) const VOID_DEPTH: f64 = 64.0;
/// `Attributes.SAFE_FALL_DISTANCE` base value.
const SAFE_FALL_DISTANCE: f64 = 3.0;
/// `FoodData.addExhaustion` cap.
const MAX_EXHAUSTION: f32 = 40.0;

/// What hurt a player (a damage type in `minecraft:damage_type`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cause {
    /// `/kill`: bypasses invulnerability.
    Kill,
    OutOfWorld,
    /// Landing after falling this far.
    Fall(f64),
    Starve,
    /// Damage from an entity's behaviour (explosions, falling blocks, ...).
    Entity(kiln_entity::level::DamageKind),
}

impl Cause {
    fn damage_type(self) -> &'static str {
        match self {
            Cause::Kill => "minecraft:generic_kill",
            Cause::OutOfWorld => "minecraft:out_of_world",
            Cause::Fall(_) => "minecraft:fall",
            Cause::Starve => "minecraft:starve",
            Cause::Entity(kind) => entities::damage_type(kind).0,
        }
    }

    /// Both bypass invulnerability (the `bypasses_invulnerability` damage type tag).
    fn bypasses_invulnerability(self) -> bool {
        matches!(self, Cause::Kill | Cause::OutOfWorld)
    }

    /// The death message (`CombatTracker.getDeathMessage` without an attacker).
    fn death_message(self, name: &str) -> Tag {
        let key = match self {
            Cause::Kill => "death.attack.genericKill",
            Cause::OutOfWorld => "death.attack.outOfWorld",
            // A long fall reads "fell from a high place"; a short one "hit the ground too hard".
            Cause::Fall(d) if d > 5.0 => "death.fell.accident.generic",
            Cause::Fall(_) => "death.attack.fall",
            Cause::Starve => "death.attack.starve",
            Cause::Entity(kind) => entities::damage_type(kind).1,
        };
        // Keys in the order vanilla writes them (its compounds are hash maps).
        Tag::Compound(vec![
            ("with".into(), Tag::List(vec![Tag::String(name.into())])),
            ("translate".into(), Tag::String(key.into())),
        ])
    }
}

/// A death to announce in a serial phase (the message goes to everyone).
pub(crate) struct Death {
    pub conn: kiln_link::ConnId,
    pub message: Tag,
}

impl Player {
    /// Creative and spectator players are invulnerable (`Abilities.invulnerable`).
    fn invulnerable(&self) -> bool {
        matches!(self.game_mode, 1 | 3)
    }

    pub(crate) fn health_packet(&self) -> bytes::Bytes {
        packets::player::set_health(self.health, self.food, self.saturation)
    }

    /// Damages the player; returns the death to announce if it died. Dropped items go to
    /// `spawns`.
    pub(crate) fn hurt(&mut self, amount: f32, cause: Cause, spawns: &mut Vec<entities::Spawn>) -> Option<Death> {
        if self.dead || (self.invulnerable() && !cause.bypasses_invulnerability()) || amount <= 0.0 {
            return None;
        }
        self.health = (self.health - amount).max(0.0);
        let damage_type = kiln_data::synced_id("minecraft:damage_type", cause.damage_type()).unwrap_or(0);
        self.send(entity::damage_event(self.entity_id, damage_type, None, None, None));
        self.damaged = Some(damage_type);
        (self.health <= 0.0).then(|| self.die(cause, spawns))
    }

    /// `ServerPlayer.die`: the death screen, the inventory scattered (no `keepInventory` yet),
    /// the death animation for viewers.
    fn die(&mut self, cause: Cause, spawns: &mut Vec<entities::Spawn>) -> Death {
        self.dead = true;
        self.fall_distance = 0.0;
        let message = cause.death_message(&self.name);
        self.send(packets::player::player_combat_kill(self.entity_id, &message));
        self.death_location = Some(self.pos.map(|c| c.floor() as i32));
        for i in 0..self.inv.items.len() {
            let stack = std::mem::replace(&mut self.inv.items[i], kiln_item::ItemStack::empty());
            if !stack.is_empty() {
                spawns.push(self.throw_randomly(stack));
            }
        }
        for i in 0..self.inv.equipment.len() {
            let stack = std::mem::replace(&mut self.inv.equipment[i], kiln_item::ItemStack::empty());
            if !stack.is_empty() {
                spawns.push(self.throw_randomly(stack));
            }
        }
        self.inv.times_changed += 1;
        self.died = true;
        Death { conn: self.conn, message }
    }

    /// Sends Set Health when health, food or whether saturation is zero changed since the last
    /// one (`ServerPlayer.doTick`).
    pub(crate) fn sync_health(&mut self) {
        let now = (self.health.to_bits(), self.food, self.saturation == 0.0);
        if self.sent_health != Some(now) {
            self.sent_health = Some(now);
            self.send(self.health_packet());
        }
    }

    /// `LivingEntity.heal`.
    fn heal(&mut self, amount: f32) {
        if self.health > 0.0 {
            self.health = (self.health + amount).min(MAX_HEALTH);
        }
    }

    /// `Player.causeFoodExhaustion`: nothing for invulnerable (creative, spectator) players.
    pub(crate) fn exhaust(&mut self, amount: f32) {
        if !self.invulnerable() {
            self.add_exhaustion(amount);
        }
    }

    /// `FoodData.addExhaustion`.
    fn add_exhaustion(&mut self, amount: f32) {
        self.exhaustion = (self.exhaustion + amount).min(MAX_EXHAUSTION);
    }

    /// `FoodData.tick` and the peaceful regeneration of `Player.aiStep`; returns a starvation
    /// death. `difficulty` is 0 (peaceful) to 3 (hard).
    pub(crate) fn tick_food(&mut self, difficulty: u8, natural_regen: bool, game_time: i64, spawns: &mut Vec<entities::Spawn>) -> Option<Death> {
        if self.dead {
            return None;
        }
        if difficulty == 0 && natural_regen {
            if self.health < MAX_HEALTH && game_time % 20 == 0 {
                self.heal(1.0);
            }
            if self.food < 20 && game_time % 10 == 0 {
                self.food += 1;
            }
        }
        if self.exhaustion > 4.0 {
            self.exhaustion -= 4.0;
            if self.saturation > 0.0 {
                self.saturation = (self.saturation - 1.0).max(0.0);
            } else if difficulty != 0 {
                self.food = (self.food - 1).max(0);
            }
        }
        let hurt = self.health > 0.0 && self.health < MAX_HEALTH;
        if natural_regen && self.saturation > 0.0 && hurt && self.food >= 20 {
            self.food_timer += 1;
            if self.food_timer >= 10 {
                let f = self.saturation.min(6.0);
                self.heal(f / 6.0);
                self.add_exhaustion(f);
                self.food_timer = 0;
            }
        } else if natural_regen && self.food >= 18 && hurt {
            self.food_timer += 1;
            if self.food_timer >= 80 {
                self.heal(1.0);
                self.add_exhaustion(6.0);
                self.food_timer = 0;
            }
        } else if self.food <= 0 {
            self.food_timer += 1;
            if self.food_timer >= 80 {
                self.food_timer = 0;
                if self.health > 10.0 || difficulty == 3 || (self.health > 1.0 && difficulty == 2) {
                    return self.hurt(1.0, Cause::Starve, spawns);
                }
            }
        } else {
            self.food_timer = 0;
        }
        None
    }

    /// Exhaustion from a move by `d` (`Player.checkMovementStatistics`) and from jumping
    /// (`jumpFromGround`: left the ground going up).
    pub(crate) fn exhaust_for_move(&mut self, d: [f64; 3], was_on_ground: bool, in_water: bool) {
        if was_on_ground && !self.on_ground && d[1] > 0.0 {
            self.exhaust(if self.sprinting { 0.2 } else { 0.05 });
        }
        let horizontal = ((d[0] * d[0] + d[2] * d[2]).sqrt() as f32 * 100.0).round();
        if horizontal <= 0.0 {
            return;
        }
        if in_water {
            self.exhaust(0.01 * horizontal * 0.01);
        } else if self.on_ground && self.sprinting {
            self.exhaust(0.1 * horizontal * 0.01);
        }
    }

    /// Vanilla `Entity.checkFallDamage` for a reported move by `dy` ending `on_ground`.
    pub(crate) fn check_fall(&mut self, dy: f64, on_ground: bool, in_fluid: bool, spawns: &mut Vec<entities::Spawn>) -> Option<Death> {
        if in_fluid || self.game_mode == 3 || self.flying {
            self.fall_distance = 0.0;
            return None;
        }
        // The move itself counts, landing included.
        if dy < 0.0 {
            self.fall_distance -= dy;
        }
        if on_ground {
            let fell = std::mem::take(&mut self.fall_distance);
            // `LivingEntity.calculateFallDamage`; creative players (`mayfly`) take none.
            let damage = (fell - SAFE_FALL_DISTANCE).floor();
            if damage > 0.0 && self.game_mode != 1 {
                return self.hurt(damage as f32, Cause::Fall(fell), spawns);
            }
        }
        None
    }

    /// Void damage every tick below the world (`Entity.checkBelowWorld`).
    pub(crate) fn check_void(&mut self, min_y: i32, spawns: &mut Vec<entities::Spawn>) -> Option<Death> {
        (self.pos[1] < min_y as f64 - VOID_DEPTH).then(|| self.hurt(VOID_DAMAGE, Cause::OutOfWorld, spawns)).flatten()
    }

    /// An item flung in a random direction (`Player.drop(stack, throwRandomly = true)`).
    fn throw_randomly(&mut self, stack: kiln_item::ItemStack) -> entities::Spawn {
        let f = self.rng.next_f32() * 0.5;
        let a = self.rng.next_f32() * std::f32::consts::TAU;
        let vel = [(-a.sin() * f) as f64, 0.2, (a.cos() * f) as f64];
        entities::Spawn {
            kind: &kiln_data::entities::types::ITEM,
            pos: [self.pos[0], self.pos[1] + 1.62 - 0.3, self.pos[2]],
            vel,
            body: entities::Body::Item { stack, pickup_delay: entities::DROP_PICKUP_DELAY },
        }
    }
}
