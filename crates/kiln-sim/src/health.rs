//! Player health, damage, death and respawn (vanilla `LivingEntity.hurt`/`die`,
//! `ServerPlayer.die`, `PlayerList.respawn`).
//!
//! Damage is not reduced by armor, enchantments or effects yet, and food does not deplete or
//! regenerate health yet.

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

/// What hurt a player (a damage type in `minecraft:damage_type`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cause {
    /// `/kill`: bypasses invulnerability.
    Kill,
    OutOfWorld,
    /// Landing after falling this far.
    Fall(f64),
}

impl Cause {
    fn damage_type(self) -> &'static str {
        match self {
            Cause::Kill => "minecraft:generic_kill",
            Cause::OutOfWorld => "minecraft:out_of_world",
            Cause::Fall(_) => "minecraft:fall",
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
        self.send(self.health_packet());
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
