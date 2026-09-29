//! Player melee combat (vanilla `ServerGamePacketListenerImpl.handleAttack`, `Player.attack`):
//! reach, the attack strength cooldown, damage from attributes and the weapon's attribute
//! modifiers, critical hits, sprint knockback, sweeping, knockback and weapon durability.
//!
//! Attributes follow `AttributeInstance`: the player's base value, then the modifiers of the
//! equipment it wore at its last tick (`LivingEntity.collectEquipmentChanges`, enchantment
//! attribute effects such as sweeping edge included) plus the creative reach and sprint speed
//! modifiers.
//!
//! Enchantments act through `EnchantmentHelper` ([`crate::enchant`]): `damage` effects
//! (sharpness, smite and bane of arthropods by the target's entity type tag) on the main
//! target and on swept ones, `knockback`, `post_attack` effects (fire aspect sets the target on
//! fire, thorns hurts the attacker and wears the armor, bane of arthropods' slowness) and
//! unbreaking on the weapon. Strength and weakness change the attack damage attribute.
//!
//! A player can hit the players of its own region (regions are far apart, reach is short), so
//! the outcome does not depend on how the world is split.

use crate::health::{Attacker, DamageCtx, Source};
use crate::{Player, entities};
use kiln_entity::EntityKind;
use kiln_item::component::{AttributeOperation, EquipmentSlot, EquipmentSlotGroup};
use kiln_item::{ItemStack, keys};
use kiln_proto::packets::entity;
use kiln_proto::packets::world_fx;

/// An attribute with the player's base value (`Player.createAttributes`) and its range.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Attr {
    pub(crate) name: &'static str,
    base: f64,
    min: f64,
    max: f64,
}

pub(crate) const ATTACK_DAMAGE: Attr = Attr { name: "minecraft:attack_damage", base: 1.0, min: 0.0, max: 2048.0 };
pub(crate) const ATTACK_SPEED: Attr = Attr { name: "minecraft:attack_speed", base: 4.0, min: 0.0, max: 1024.0 };
pub(crate) const ATTACK_KNOCKBACK: Attr = Attr { name: "minecraft:attack_knockback", base: 0.0, min: 0.0, max: 5.0 };
pub(crate) const ARMOR: Attr = Attr { name: "minecraft:armor", base: 0.0, min: 0.0, max: 30.0 };
pub(crate) const ARMOR_TOUGHNESS: Attr = Attr { name: "minecraft:armor_toughness", base: 0.0, min: 0.0, max: 20.0 };
pub(crate) const KNOCKBACK_RESISTANCE: Attr = Attr { name: "minecraft:knockback_resistance", base: 0.0, min: 0.0, max: 1.0 };
pub(crate) const ENTITY_INTERACTION_RANGE: Attr =
    Attr { name: "minecraft:entity_interaction_range", base: 3.0, min: 0.0, max: 64.0 };
pub(crate) const MOVEMENT_SPEED: Attr = Attr { name: "minecraft:movement_speed", base: 0.10000000149011612, min: 0.0, max: 1024.0 };
pub(crate) const SWEEPING_DAMAGE_RATIO: Attr = Attr { name: "minecraft:sweeping_damage_ratio", base: 0.0, min: 0.0, max: 1.0 };
pub(crate) const MINING_EFFICIENCY: Attr = Attr { name: "minecraft:mining_efficiency", base: 0.0, min: 0.0, max: 1024.0 };
pub(crate) const SUBMERGED_MINING_SPEED: Attr = Attr { name: "minecraft:submerged_mining_speed", base: 0.2, min: 0.0, max: 20.0 };
pub(crate) const BLOCK_BREAK_SPEED: Attr = Attr { name: "minecraft:block_break_speed", base: 1.0, min: 0.0, max: 1024.0 };
pub(crate) const BURNING_TIME: Attr = Attr { name: "minecraft:burning_time", base: 1.0, min: 0.0, max: 1024.0 };
pub(crate) const MAX_HEALTH: Attr = Attr { name: "minecraft:max_health", base: 20.0, min: 1.0, max: 1024.0 };
pub(crate) const MAX_ABSORPTION: Attr = Attr { name: "minecraft:max_absorption", base: 0.0, min: 0.0, max: 2048.0 };
pub(crate) const LUCK: Attr = Attr { name: "minecraft:luck", base: 0.0, min: -1024.0, max: 1024.0 };
pub(crate) const SAFE_FALL_DISTANCE: Attr = Attr { name: "minecraft:safe_fall_distance", base: 3.0, min: -1024.0, max: 1024.0 };
pub(crate) const OXYGEN_BONUS: Attr = Attr { name: "minecraft:oxygen_bonus", base: 0.0, min: 0.0, max: 1024.0 };
pub(crate) const WAYPOINT_TRANSMIT_RANGE: Attr =
    Attr { name: "minecraft:waypoint_transmit_range", base: 6.0e7, min: 0.0, max: 6.0e7 };

/// The attributes effects change that clients are told about (`Attribute.isClientSyncable`).
pub(crate) const EFFECT_SYNCED: [Attr; 6] = [MOVEMENT_SPEED, ATTACK_SPEED, SAFE_FALL_DISTANCE, MAX_HEALTH, MAX_ABSORPTION, LUCK];

impl Attr {
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) const fn new(name: &'static str, base: f64, min: f64, max: f64) -> Attr {
        Attr { name, base, min, max }
    }

    pub(crate) fn default_base(&self) -> f64 {
        self.base
    }
}

/// What `/attribute` changed on a player: base values and permanent modifiers (attribute
/// name, modifier id, amount, operation), kept in the order they were added.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CommandAttributes {
    pub bases: Vec<(&'static str, f64)>,
    pub modifiers: Vec<(&'static str, String, f64, AttributeOperation)>,
}

/// `Player.CREATIVE_ENTITY_INTERACTION_RANGE_MODIFIER_VALUE`.
const CREATIVE_ENTITY_RANGE: f64 = 2.0;
/// `LivingEntity.SPEED_MODIFIER_SPRINTING` (multiplies the total).
const SPRINT_SPEED: f64 = 0.30000001192092896;
/// Extra reach the server allows on top of the attack range (`handleAttack`'s buffer).
const ATTACK_BUFFER: f64 = 3.0;

/// Equipment slots in the order vanilla collects their modifiers.
pub(crate) const SLOTS: [EquipmentSlot; 8] = [
    EquipmentSlot::MainHand,
    EquipmentSlot::OffHand,
    EquipmentSlot::Feet,
    EquipmentSlot::Legs,
    EquipmentSlot::Chest,
    EquipmentSlot::Head,
    EquipmentSlot::Body,
    EquipmentSlot::Saddle,
];

/// `EquipmentSlotGroup.test`.
fn group_has(group: EquipmentSlotGroup, slot: EquipmentSlot) -> bool {
    use EquipmentSlot as S;
    use EquipmentSlotGroup as G;
    match group {
        G::Any => true,
        G::MainHand => slot == S::MainHand,
        G::OffHand => slot == S::OffHand,
        G::Hand => matches!(slot, S::MainHand | S::OffHand),
        G::Feet => slot == S::Feet,
        G::Legs => slot == S::Legs,
        G::Chest => slot == S::Chest,
        G::Head => slot == S::Head,
        G::Armor => matches!(slot, S::Feet | S::Legs | S::Chest | S::Head | S::Body),
        G::Body => slot == S::Body,
        G::Saddle => slot == S::Saddle,
    }
}

/// `Mth.floor`.
pub(crate) fn floor(x: f64) -> i32 {
    let i = x as i32;
    if x < i as f64 { i - 1 } else { i }
}

/// `Mth.floor(float)`.
pub(crate) fn floor_f32(x: f32) -> i32 {
    let i = x as i32;
    if x < i as f32 { i - 1 } else { i }
}

/// `Mth.sin` / `Mth.cos`: the 65536-entry table.
fn sin_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..65536).map(|i| (i as f64 * std::f64::consts::PI * 2.0 / 65536.0).sin() as f32).collect())
}

pub(crate) fn mth_sin(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453) as i64 & 0xffff) as usize]
}

pub(crate) fn mth_cos(v: f64) -> f32 {
    sin_table()[((v * 10430.378350470453 + 16384.0) as i64 & 0xffff) as usize]
}

/// `CombatRules.getDamageAfterAbsorb`: `effectiveness` maps the armor's share of the damage
/// (the weapon's `armor_effectiveness` enchantments, clamped to [0, 1]).
pub(crate) fn damage_after_absorb(damage: f32, armor: f32, toughness: f32, effectiveness: impl FnOnce(f32) -> f32) -> f32 {
    let f = 2.0 + toughness / 4.0;
    let g = (armor - damage / f).clamp(armor * 0.2, 20.0);
    let h = effectiveness(g / 25.0);
    damage * (1.0 - h)
}

/// The modifiers `stack` gives in `slot` (`ItemStack.forEachModifier`: its attribute
/// modifiers, then its enchantments' attribute effects): (attribute network id, modifier id,
/// amount, operation).
pub(crate) fn slot_modifiers(
    loot: Option<&kiln_loot::LootData>,
    stack: &ItemStack,
    slot: EquipmentSlot,
    out: &mut Vec<(i32, String, f64, AttributeOperation)>,
) {
    // `collectEquipmentChanges` skips empty and broken items.
    if stack.is_empty() || (stack.is_damageable_item() && stack.damage() >= stack.max_damage()) {
        return;
    }
    // `addTransientAttributeModifiers` replaces a modifier with the same id.
    let mut add = |attribute: i32, id: String, amount: f64, op: AttributeOperation| {
        out.retain(|(a, i, _, _)| !(*a == attribute && *i == id));
        out.push((attribute, id, amount, op));
    };
    if let Some(mods) = stack.get(keys::ATTRIBUTE_MODIFIERS) {
        for m in mods.0.iter().filter(|m| group_has(m.slot, slot)) {
            add(m.attribute, m.id.as_str().to_owned(), m.amount, m.operation);
        }
    }
    if let Some(loot) = loot {
        loot.enchantment_modifiers(stack, slot, &mut add);
    }
}

/// `String.hashCode`.
fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// Orders modifiers the way `AttributeInstance` iterates them: one fastutil
/// `Object2ObjectOpenHashMap` per operation, keyed by the modifier id (`Identifier.hashCode`,
/// mixed, linear probing in a 32-slot table), walked from the last slot down. Built from the
/// current modifiers in insertion order, which matches vanilla unless removals reshuffled
/// colliding slots.
fn sort_like_open_hash_map(mods: &mut [(String, f64, AttributeOperation)]) {
    const SLOTS: usize = 32;
    let slot_of = |id: &str| -> i32 {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let h = java_string_hash(ns).wrapping_mul(31).wrapping_add(java_string_hash(path));
        let h = h.wrapping_mul(0x9E37_79B9_u32 as i32);
        h ^ ((h as u32) >> 16) as i32
    };
    let mut order = vec![0usize; mods.len()];
    for op in [AttributeOperation::AddValue, AttributeOperation::AddMultipliedBase, AttributeOperation::AddMultipliedTotal] {
        let mut table: [Option<usize>; SLOTS] = [None; SLOTS];
        let members: Vec<usize> = (0..mods.len()).filter(|&i| mods[i].2 == op).collect();
        if members.len() > 24 {
            continue;
        }
        for &i in &members {
            let mut pos = (slot_of(&mods[i].0) as usize) & (SLOTS - 1);
            while table[pos].is_some() {
                pos = (pos + 1) & (SLOTS - 1);
            }
            table[pos] = Some(i);
        }
        for (pos, entry) in table.iter().enumerate() {
            if let Some(i) = entry {
                order[*i] = SLOTS - 1 - pos;
            }
        }
    }
    let mut indexed: Vec<(usize, (String, f64, AttributeOperation))> = mods.iter().cloned().enumerate().collect();
    indexed.sort_by_key(|(i, _)| order[*i]);
    for (slot, (_, m)) in mods.iter_mut().zip(indexed) {
        *slot = m;
    }
}

/// `AttributeInstance.calculateValue` for `base` and `mods` (in iteration order).
fn attribute_value(attr: Attr, mods: impl Iterator<Item = (f64, AttributeOperation)> + Clone) -> f64 {
    let mut base = attr.base;
    for (amount, op) in mods.clone() {
        if op == AttributeOperation::AddValue {
            base += amount;
        }
    }
    let mut d = base;
    for (amount, op) in mods.clone() {
        if op == AttributeOperation::AddMultipliedBase {
            d += base * amount;
        }
    }
    for (amount, op) in mods {
        if op == AttributeOperation::AddMultipliedTotal {
            d *= 1.0 + amount;
        }
    }
    if d.is_nan() { attr.min } else { d.clamp(attr.min, attr.max) }
}

/// A projection of the target of an attack.
enum Target {
    Player(usize),
    /// A kiln-entity entity: its bounding box, what it is and its entity type id.
    /// `part`: an ender dragon part (the entity is its dragon).
    Entity { bb: kiln_entity::math::Aabb, kind: EntityClass, type_id: i32, pos: [f64; 3], part: Option<usize> },
}

impl Target {
    fn part(&self) -> Option<usize> {
        match self {
            Target::Entity { part, .. } => *part,
            Target::Player(_) => None,
        }
    }
}

/// An ender dragon part by its id (the dragon's id plus 1 to 8): the part's box, the dragon.
fn dragon_part(entities: &entities::Entities, id: i32) -> Option<Target> {
    let i = entities.list.partition_point(|e| e.id < id).checked_sub(1)?;
    let e = &entities.list[i];
    let part = (id - e.id - 1) as usize;
    if e.removed || part >= kiln_entity::mob::kinds::ender_dragon::PARTS.len() {
        return None;
    }
    let phys = e.phys.as_ref()?;
    let s = kiln_entity::mob::kinds::ender_dragon::state_of(phys)?;
    let p = s.parts[part];
    Some(Target::Entity {
        bb: s.part_box(part),
        kind: classify(phys),
        type_id: kiln_item::registry::ENTITY_TYPE.id(phys.type_name).unwrap_or(-1),
        pos: [p.x, p.y, p.z],
        part: Some(part),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EntityClass {
    /// Items and experience orbs: attacking them is a protocol violation.
    Invalid,
    /// `isAttackable` false: nothing happens (falling blocks).
    NotAttackable,
    /// Attackable, but hurting it does nothing (primed TNT, thrown items).
    Unhurtable,
    /// A mob: a living target the simulation hurts after the attack (see [`MobHit`]).
    Mob,
}

/// A player's hit on a mob, carried out against the region's entities by
/// [`crate::entities::hit_mob`].
#[derive(Debug, Clone)]
pub(crate) struct MobHit {
    pub target: i32,
    /// The ender dragon part hit (`target` is the part's id, the dragon's plus one plus this).
    pub part: Option<usize>,
    pub attacker: i32,
    pub attacker_pos: [f64; 3],
    pub amount: f32,
    /// `causeExtraKnockback` strength (enchantments and sprinting), along `yaw`.
    pub knockback: f32,
    pub yaw: f32,
    /// Fire aspect: seconds the mob burns.
    pub fire_seconds: f32,
}

fn classify(e: &kiln_entity::Entity) -> EntityClass {
    match &e.kind {
        EntityKind::Item(_) | EntityKind::ExperienceOrb(_) => EntityClass::Invalid,
        // `AbstractArrow.isAttackable`: only redirectable projectiles (none of Kiln's arrows).
        EntityKind::Arrow(_) => EntityClass::Invalid,
        EntityKind::FallingBlock(_) => EntityClass::NotAttackable,
        EntityKind::Mob(m) if m.health > 0.0 => EntityClass::Mob,
        // `EndCrystal.hurtServer`: an attack breaks it.
        EntityKind::Ext(_) if e.type_name == "minecraft:end_crystal" => EntityClass::Mob,
        EntityKind::Mob(_) => EntityClass::NotAttackable,
        EntityKind::Ext(x) if x.attackable() => EntityClass::Mob,
        _ => EntityClass::Unhurtable,
    }
}

impl Player {
    /// The modifiers of the equipment seen at the last tick.
    fn equipment_modifiers(&self) -> Vec<(i32, String, f64, AttributeOperation)> {
        let mut out = Vec::new();
        for (stack, slot) in self.equipment_seen.iter().zip(SLOTS) {
            slot_modifiers(self.loot.as_deref(), stack, slot, &mut out);
        }
        out
    }

    /// The modifiers on `attr`: the equipment's, the creative reach and sprint speed ones, then
    /// the active effects' (id, amount, operation).
    pub(crate) fn attribute_modifiers(&self, attr: Attr) -> Vec<(String, f64, AttributeOperation)> {
        let Some(id) = kiln_item::registry::ATTRIBUTE.id(attr.name) else { return Vec::new() };
        let mut mods: Vec<(String, f64, AttributeOperation)> =
            self.equipment_modifiers().into_iter().filter(|m| m.0 == id).map(|m| (m.1, m.2, m.3)).collect();
        if attr.name == ENTITY_INTERACTION_RANGE.name && self.game_mode == 1 {
            mods.push(("minecraft:creative_mode_entity_range".into(), CREATIVE_ENTITY_RANGE, AttributeOperation::AddValue));
        }
        if attr.name == MOVEMENT_SPEED.name && self.sprinting {
            mods.push(("minecraft:sprinting".into(), SPRINT_SPEED, AttributeOperation::AddMultipliedTotal));
        }
        // `ServerPlayer.updatePlayerAttributes`: crouching hides the player's waypoint.
        if attr.name == WAYPOINT_TRANSMIT_RANGE.name && self.sneaking {
            mods.push(("minecraft:waypoint_transmit_range_crouch".into(), -1.0, AttributeOperation::AddMultipliedTotal));
        }
        for (id, amount, op) in self.effect_modifiers(attr.name) {
            mods.retain(|m| m.0 != id);
            mods.push((id.to_owned(), amount, op));
        }
        for (a, id, amount, op) in &self.command_attributes.modifiers {
            if *a == attr.name {
                mods.retain(|m| m.0 != *id);
                mods.push((id.clone(), *amount, *op));
            }
        }
        mods
    }

    /// The attribute with the base value `/attribute` set, if any.
    pub(crate) fn with_base(&self, attr: Attr) -> Attr {
        match self.command_attributes.bases.iter().find(|(a, _)| *a == attr.name) {
            Some((_, b)) => Attr { base: *b, ..attr },
            None => attr,
        }
    }

    /// `getAttributeValue`: the modifiers of each operation in the order vanilla's per-operation
    /// hash maps hold them.
    pub(crate) fn attribute(&self, attr: Attr) -> f64 {
        let attr = self.with_base(attr);
        let mut mods = self.attribute_modifiers(attr);
        sort_like_open_hash_map(&mut mods);
        attribute_value(attr, mods.iter().map(|m| (m.1, m.2)))
    }

    /// `ClientboundUpdateAttributesPacket` for the attributes effects change.
    pub(crate) fn effect_attributes_packet(&self) -> bytes::Bytes {
        use kiln_proto::packets::entity::{AttributeModifier, AttributeSnapshot, ModifierOperation};
        type Listed = (i32, f64, Vec<(String, f64, AttributeOperation)>);
        let lists: Vec<Listed> = EFFECT_SYNCED
            .iter()
            .filter_map(|a| Some((kiln_data::builtin_id("minecraft:attribute", a.name)?, self.with_base(*a).base, self.attribute_modifiers(*a))))
            .collect();
        let op = |o: AttributeOperation| match o {
            AttributeOperation::AddValue => ModifierOperation::AddValue,
            AttributeOperation::AddMultipliedBase => ModifierOperation::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => ModifierOperation::AddMultipliedTotal,
        };
        let mods: Vec<Vec<AttributeModifier>> = lists
            .iter()
            .map(|(_, _, l)| l.iter().map(|(id, amount, o)| AttributeModifier { id, amount: *amount, operation: op(*o) }).collect())
            .collect();
        let snapshots: Vec<AttributeSnapshot> = lists
            .iter()
            .zip(&mods)
            .map(|((attribute, base, _), modifiers)| AttributeSnapshot { attribute: *attribute, base: *base, modifiers })
            .collect();
        kiln_proto::packets::entity::update_attributes(self.entity_id, &snapshots)
    }

    /// Remembers the equipment for attributes and resets the attack strength when the main
    /// hand holds a different item (`Player.tick`, `LivingEntity.detectEquipmentUpdates`).
    pub(crate) fn tick_combat(&mut self) {
        self.attack_ticker += 1;
        let main = self.inv.selected_item().clone();
        if !main.is_same_item(&self.equipment_seen[0]) {
            self.attack_ticker = 0;
        }
        for (i, slot) in SLOTS.iter().enumerate() {
            let stack = self.inv.equipped(*slot);
            if self.equipment_seen[i] != *stack {
                self.equipment_seen[i] = stack.clone();
            }
        }
    }

    /// `getCurrentItemAttackStrengthDelay`: ticks to a full-strength attack.
    pub(crate) fn attack_strength_delay(&self) -> f32 {
        (1.0 / self.attribute(ATTACK_SPEED) * 20.0) as f32
    }

    /// `getAttackStrengthScale`.
    pub(crate) fn attack_strength_scale(&self, partial: f32) -> f32 {
        ((self.attack_ticker as f32 + partial) / self.attack_strength_delay()).clamp(0.0, 1.0)
    }

    /// `cannotAttackWithItem`: items with a minimum charge need that much attack strength.
    fn cannot_attack_with_item(&self, stack: &ItemStack, extra_ticks: i32) -> bool {
        let min = stack.get(keys::MINIMUM_ATTACK_CHARGE).copied().unwrap_or(0.0);
        let charge = (self.attack_ticker + extra_ticks) as f32 / self.attack_strength_delay();
        min > 0.0 && charge < min
    }

    pub(crate) fn eye_position(&self) -> [f64; 3] {
        let eye = if self.fall_flying {
            0.4
        } else if self.sneaking {
            1.27
        } else {
            1.62
        };
        [self.pos[0], self.pos[1] + eye, self.pos[2]]
    }

    /// The player's bounding box (standing or crouching).
    pub(crate) fn bounding_box(&self) -> kiln_entity::math::Aabb {
        let h = if self.fall_flying {
            0.6
        } else if self.sneaking {
            1.5
        } else {
            1.8
        };
        kiln_entity::math::Aabb::new(self.pos[0] - 0.3, self.pos[1], self.pos[2] - 0.3, self.pos[0] + 0.3, self.pos[1] + h, self.pos[2] + 0.3)
    }

    /// `isWithinAttackRange` with the server's buffer: the weapon's `attack_range`, or the
    /// entity interaction range.
    fn within_attack_range(&self, bb: &kiln_entity::math::Aabb) -> bool {
        let creative = self.game_mode == 1;
        let (min, max, margin) = match self.inv.selected_item().get(keys::ATTACK_RANGE) {
            Some(r) if creative => (r.min_creative_reach, r.max_creative_reach, r.hitbox_margin),
            Some(r) => (r.min_reach, r.max_reach, r.hitbox_margin),
            None => {
                let range = self.attribute(ENTITY_INTERACTION_RANGE) as f32;
                (0.0, range, 0.0)
            }
        };
        let d = aabb_distance_sqr(bb, self.eye_position()).sqrt();
        let lo = (min - margin) as f64 - ATTACK_BUFFER;
        let hi = (max + margin) as f64 + ATTACK_BUFFER;
        d >= lo && d <= hi
    }

    /// `LivingEntity.knockback`: pushed away from (`dx`, `dz`) with `strength`, less the
    /// knockback resistance. The velocity is the server's view (the client gets it in a
    /// motion packet).
    pub(crate) fn knockback(&mut self, strength: f64, dx: f64, dz: f64) {
        let strength = strength * (1.0 - self.attribute(KNOCKBACK_RESISTANCE));
        if strength <= 0.0 {
            return;
        }
        let (mut dx, mut dz) = (dx, dz);
        while dx * dx + dz * dz < 9.999999747378752e-6 {
            dx = (self.rng.next_f64() - self.rng.next_f64()) * 0.01;
            dz = (self.rng.next_f64() - self.rng.next_f64()) * 0.01;
        }
        let len = (dx * dx + dz * dz).sqrt();
        let (kx, kz) = if len < 9.999999747378752e-6 { (0.0, 0.0) } else { (dx / len * strength, dz / len * strength) };
        let v = self.vel;
        let y = if self.on_ground { 0.4f64.min(v[1] / 2.0 + strength) } else { v[1] };
        self.vel = [v[0] / 2.0 - kx, y, v[2] / 2.0 - kz];
    }

    /// `LivingEntity.aiStep` for a player the server does not simulate: its velocity decays.
    pub(crate) fn decay_velocity(&mut self) {
        let v = self.vel.map(|c| c * 0.98);
        let horizontal = v[0] * v[0] + v[2] * v[2];
        let (x, z) = if horizontal < 9.0e-6 { (0.0, 0.0) } else { (v[0], v[2]) };
        let y = if v[1].abs() < 0.003 { 0.0 } else { v[1] };
        self.vel = [x, y, z];
    }

    /// `ItemStack.hurtAndBreak` on an equipped item: loses `amount` durability less what its
    /// `item_damage` enchantments (unbreaking) take off (`processDurabilityChange`; none for
    /// creative players), breaking with the break animation. `level_rng` is the random the
    /// enchantments draw from (the player's own when `None`).
    pub(crate) fn hurt_and_break(&mut self, slot: EquipmentSlot, amount: i32, level_rng: Option<&mut kiln_javamath::random::LegacyRandom>) {
        let index = kiln_inventory::inventory::equipment_index(slot, self.inv.selected);
        let stack = kiln_inventory::Container::item(&self.inv, index);
        if !stack.is_damageable_item() || self.game_mode == 1 {
            return;
        }
        let amount = match (&self.loot, amount > 0) {
            (Some(loot), true) => {
                let rng = level_rng.unwrap_or(&mut self.level_rng);
                loot.process_durability_change(stack, rng, amount)
            }
            _ => amount,
        };
        if amount == 0 {
            return;
        }
        // `ItemStack.applyDamage`: `item_durability_changed` with the stack before the change.
        let before = kiln_inventory::Container::item(&self.inv, index).clone();
        let new_damage = before.damage() + amount;
        self.fire_conds("minecraft:item_durability_changed", None, |c, _, loot| {
            c.item("item").is_none_or(|p| kiln_loot::predicate::item_matches(&loot.tags, p, &before))
                && kiln_loot::predicate::item::int_bounds(&c.ints("durability"), before.max_damage() - new_damage)
                && kiln_loot::predicate::item::int_bounds(&c.ints("delta"), before.damage() - new_damage)
        });
        let stack = kiln_inventory::Container::item_mut(&mut self.inv, index);
        let damage = stack.damage() + amount;
        stack.insert(keys::DAMAGE, damage.clamp(0, stack.max_damage()));
        if damage >= stack.max_damage() {
            let broken = stack.item();
            stack.shrink(1);
            self.award_stat(crate::player_stats::Stat::item(crate::player_stats::BROKEN, broken), 1);
            // `LivingEntity.onEquippedItemBroken`: `entityEventForEquipmentBreak`.
            let event = match slot {
                EquipmentSlot::MainHand => 47,
                EquipmentSlot::OffHand => 48,
                EquipmentSlot::Head => 49,
                EquipmentSlot::Chest => 50,
                EquipmentSlot::Feet => 52,
                EquipmentSlot::Legs => 51,
                EquipmentSlot::Body => 65,
                EquipmentSlot::Saddle => 68,
            };
            self.entity_events.push(event);
            self.send(entity::entity_event(self.entity_id, event));
            // `stopLocationBasedEffects`: the broken item's modifiers go at once.
            if let Some(i) = SLOTS.iter().position(|s| *s == slot) {
                self.equipment_seen[i] = ItemStack::empty();
            }
        }
        self.inv.times_changed += 1;
    }

    /// The attacker as its victims see it.
    pub(crate) fn as_attacker(&self) -> Attacker {
        let held = self.inv.selected_item();
        let weapon = held.get(keys::CUSTOM_NAME).map(|name| item_display_name(held, name.nbt().clone()));
        Attacker { id: self.entity_id, name: self.name.clone(), pos: self.pos, creative: self.game_mode == 1, weapon, view: self.view(), mob: None }
    }

    /// `Player.canCriticalAttack`: falling, in the air, not climbing, in water, riding or
    /// sprinting, against a living target (blindness does not exist yet).
    fn can_critical_attack(&self, climbing: bool, in_water: bool) -> bool {
        self.fall_distance > 0.0 && !self.on_ground && !climbing && !in_water && !self.sprinting
    }

    /// `Player.isSweepAttack`: a full-strength, grounded, slow, non-critical, non-sprinting hit
    /// with a sword.
    fn is_sweep_attack(&self, full: bool, crit: bool, sprint_knockback: bool) -> bool {
        if !full || crit || sprint_knockback || !self.on_ground {
            return false;
        }
        let m = self.known_movement;
        let speed = self.attribute(MOVEMENT_SPEED) as f32 as f64 * 2.5;
        m[0] * m[0] + m[2] * m[2] < speed * speed && item_in_tag(self.inv.selected_item(), "minecraft:swords")
    }
}

/// `ItemStack.getDisplayName` for a custom-named item: the name in brackets, italic, in the
/// item's rarity color (without the item hover event).
fn item_display_name(stack: &ItemStack, name: kiln_proto::nbt::Tag) -> kiln_proto::nbt::Tag {
    use kiln_proto::nbt::Tag;
    let color = match stack.get(keys::RARITY).map(|r| r.name()) {
        Some("uncommon") => "yellow",
        Some("rare") => "aqua",
        Some("epic") => "light_purple",
        _ => "white",
    };
    let inner = Tag::Compound(vec![
        ("italic".into(), Tag::Byte(1)),
        ("extra".into(), Tag::List(vec![name])),
        ("text".into(), Tag::String(String::new())),
    ]);
    Tag::Compound(vec![
        ("color".into(), Tag::String(color.into())),
        ("with".into(), Tag::List(vec![inner])),
        ("translate".into(), Tag::String("chat.square_brackets".into())),
    ])
}

/// Whether an item is in a `minecraft:item` tag.
pub(crate) fn item_in_tag(stack: &ItemStack, tag: &str) -> bool {
    !stack.is_empty()
        && kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:item")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&stack.item()))
}

/// `AABB.distanceToSqr(Vec3)`.
fn aabb_distance_sqr(bb: &kiln_entity::math::Aabb, p: [f64; 3]) -> f64 {
    let d = |lo: f64, hi: f64, v: f64| (lo - v).max(v - hi).max(0.0);
    let (x, y, z) = (d(bb.min_x, bb.max_x, p[0]), d(bb.min_y, bb.max_y, p[1]), d(bb.min_z, bb.max_z, p[2]));
    x * x + y * y + z * z
}

/// The world around an attack: blocks for the critical hit checks, sounds and particles.
pub(crate) struct AttackEnv<'a> {
    pub cells: &'a kiln_region::CellSet<kiln_world::Cell>,
    pub game_time: i64,
    pub seed: i64,
}

/// `ServerGamePacketListenerImpl.handleAttack` then `Player.attack`, for player `a` of a
/// region's players (sorted by connection) hitting the entity with network id `target_id`.
pub(crate) fn handle_attack(
    players: &mut [&mut Player],
    a: usize,
    target_id: i32,
    entities: &entities::Entities,
    env: &AttackEnv,
    ctx: &mut DamageCtx,
    mob_hits: &mut Vec<MobHit>,
) {
    let attacker = &*players[a];
    if !attacker.client_loaded() || attacker.game_mode == 3 {
        return;
    }
    let target = if target_id == attacker.entity_id {
        None
    } else if let Some(t) = players.iter().position(|p| p.entity_id == target_id && !p.dead) {
        Some(Target::Player(t))
    } else {
        entities
            .list
            .iter()
            .find(|e| e.id == target_id && !e.removed)
            .and_then(|e| e.phys.as_ref())
            .map(|e| Target::Entity {
                bb: e.bounding_box(),
                kind: classify(e),
                type_id: kiln_item::registry::ENTITY_TYPE.id(e.type_name).unwrap_or(-1),
                pos: { let v = e.position(); [v.x, v.y, v.z] },
                part: None,
            })
            .or_else(|| dragon_part(entities, target_id))
    };
    // `handleAttack` disconnects for attacking itself.
    if target_id == attacker.entity_id {
        players[a].disconnect_text(translatable("multiplayer.disconnect.invalid_entity_attacked"));
        return;
    }
    let Some(target) = target else { return };
    let bb = match &target {
        Target::Player(t) => players[*t].bounding_box(),
        Target::Entity { bb, .. } => *bb,
    };
    let attacker = &*players[a];
    if !attacker.within_attack_range(&bb) {
        return;
    }
    let held = attacker.inv.selected_item();
    // Spears stab instead (`stabAttack`, not modelled): a plain attack does nothing.
    if held.has(kiln_item::component::ids::PIERCING_WEAPON) {
        return;
    }
    if let Target::Entity { kind: EntityClass::Invalid, .. } = target {
        tracing::warn!("Player {} tried to attack an invalid entity", attacker.name);
        players[a].disconnect_text(translatable("multiplayer.disconnect.invalid_entity_attacked"));
        return;
    }
    if attacker.cannot_attack_with_item(held, 5) {
        return;
    }
    // The whole attack draws enchantment randomness from the attacker's level random.
    let lent = std::mem::replace(&mut players[a].level_rng, kiln_javamath::random::LegacyRandom::new(0));
    let outer = ctx.level_rng.replace(lent);
    attack(players, a, target, target_id, env, ctx, mob_hits);
    if let Some(r) = std::mem::replace(&mut ctx.level_rng, outer) {
        players[a].level_rng = r;
    }
}

impl Player {
    /// `ServerPlayer.getEnchantedDamage`: `EnchantmentHelper.modifyDamage` with the main hand
    /// item against `target`.
    fn enchanted_damage(&self, target: &crate::enchant::EntityView, damage: f32, source: &Source, rng: &mut kiln_javamath::random::LegacyRandom) -> f32 {
        let (Some(loot), Some(weapon)) = (&self.loot, &source.weapon) else { return damage };
        loot.modify_damage(weapon, rng, damage, |level| crate::enchant::DamageContext { level, this: target, source })
    }

    /// `LivingEntity.getKnockback`: the attack knockback attribute through the weapon's
    /// `knockback` enchantments, halved.
    fn attack_knockback(&self, target: &crate::enchant::EntityView, source: &Source, rng: &mut kiln_javamath::random::LegacyRandom) -> f32 {
        let base = self.attribute(ATTACK_KNOCKBACK) as f32;
        let value = match (&self.loot, &source.weapon) {
            (Some(loot), Some(weapon)) => {
                loot.modify_knockback(weapon, rng, base, |level| crate::enchant::DamageContext { level, this: target, source })
            }
            _ => base,
        };
        value / 2.0
    }
}

/// The level random lent to an attack (see [`handle_attack`]).
fn attack_rng<'c>(ctx: &'c mut DamageCtx) -> &'c mut kiln_javamath::random::LegacyRandom {
    ctx.level_rng.as_mut().expect("attack random")
}

/// `EnchantmentHelper.doPostAttackEffectsWithItemSource` for player `victim` hit by player
/// `a`, then the effects carried out.
fn post_attack(players: &mut [&mut Player], a: usize, victim: usize, source: &Source, ctx: &mut DamageCtx) {
    let Some(loot) = players[victim].loot.clone() else { return };
    let effects = crate::enchant::post_attack_effects(&loot, players, victim, Some(a), source, attack_rng(ctx));
    for e in &effects {
        crate::enchant::apply_post_attack(players, victim, Some(a), e, ctx);
    }
}

fn translatable(key: &str) -> kiln_proto::nbt::Tag {
    kiln_proto::nbt::Tag::Compound(vec![("translate".into(), kiln_proto::nbt::Tag::String(key.into()))])
}

/// `Player.attack`.
fn attack(players: &mut [&mut Player], a: usize, target: Target, target_id: i32, env: &AttackEnv, ctx: &mut DamageCtx, mob_hits: &mut Vec<MobHit>) {
    let living = match target {
        Target::Player(_) => true,
        Target::Entity { kind: EntityClass::NotAttackable, .. } => return,
        Target::Entity { kind: EntityClass::Mob, .. } => true,
        Target::Entity { .. } => false,
    };
    let target_view = match &target {
        Target::Player(t) => players[*t].view(),
        Target::Entity { type_id, pos, .. } => crate::enchant::EntityView {
            type_id: *type_id,
            pos: *pos,
            on_ground: false,
            on_fire: false,
            sneaking: false,
            sprinting: false,
            flying: false,
        },
    };
    let p = &mut *players[a];
    let mut damage = p.attribute(ATTACK_DAMAGE) as f32;
    let source = Source::melee(p.as_attacker(), p.inv.selected_item().clone());
    let scale = p.attack_strength_scale(0.5);
    // `scale * (getEnchantedDamage(target, damage, source) - damage)`.
    let enchant_bonus = scale * (p.enchanted_damage(&target_view, damage, &source, attack_rng(ctx)) - damage);
    damage *= 0.2 + scale * scale * 0.8;
    p.attack_ticker = 0;
    if !(damage > 0.0 || enchant_bonus > 0.0) {
        return;
    }
    let full = scale > 0.9;
    let sprint_knockback = p.sprinting && full;
    let mut sounds: Vec<&'static str> = Vec::new();
    if sprint_knockback {
        sounds.push("minecraft:entity.player.attack.knockback");
    }
    let (climbing, in_water) = feet_state(p, env.cells);
    let crit = full && living && p.can_critical_attack(climbing, in_water);
    if crit {
        damage *= 1.5;
    }
    let total = damage + enchant_bonus;
    let sweep = p.is_sweep_attack(full, crit, sprint_knockback);
    let yaw = p.rot[0];
    let t = match target {
        Target::Player(t) => t,
        Target::Entity { kind: EntityClass::Mob, .. } => {
            // The mob is hurt against the region's entities afterwards; the attacker's side of
            // the attack (knockback strength, sounds, durability, exhaustion) happens here.
            let strength =
                players[a].attack_knockback(&target_view, &source, attack_rng(ctx)) + if sprint_knockback { 0.5 } else { 0.0 };
            let fire = source
                .weapon
                .as_ref()
                .and_then(kiln_loot::predicate::item::enchantments)
                .map_or(0, |e| e.level(kiln_item::registry::ENCHANTMENT.id("minecraft:fire_aspect").unwrap_or(-1)));
            mob_hits.push(MobHit {
                target: target_id,
                part: target.part(),
                attacker: players[a].entity_id,
                attacker_pos: players[a].pos,
                amount: total,
                knockback: strength,
                yaw,
                fire_seconds: 4.0 * fire as f32,
            });
            if strength > 0.0 {
                let p = &mut *players[a];
                p.vel = [p.vel[0] * 0.6, p.vel[1], p.vel[2] * 0.6];
                if p.sprinting {
                    p.sprinting = false;
                    p.meta_dirty = true;
                }
            }
            if crit {
                sounds.push("minecraft:entity.player.attack.crit");
                let pkt = entity::animate(target_id, entity::animation::CRITICAL_HIT);
                send_to_trackers_and_self(players, a, &pkt);
            } else {
                sounds.push(if full { "minecraft:entity.player.attack.strong" } else { "minecraft:entity.player.attack.weak" });
            }
            let per_attack = players[a].inv.selected_item().get(keys::WEAPON).map(|w| w.item_damage_per_attack);
            if let Some(n) = per_attack
                && !players[a].inv.selected_item().is_empty()
            {
                let item = players[a].inv.selected_item().item();
                players[a].award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
                players[a].hurt_and_break(EquipmentSlot::MainHand, n, ctx.level_rng.as_mut());
            }
            players[a].exhaust(0.1);
            play_sounds(players, a, &sounds, env);
            return;
        }
        Target::Entity { .. } => {
            // `hurtOrSimulate` is false for TNT and thrown items.
            sounds.push("minecraft:entity.player.attack.nodamage");
            play_sounds(players, a, &sounds, env);
            return;
        }
    };
    let health_before = players[t].health;
    let old_vel = players[t].vel;
    let hurt = players[t].hurt(total, &source, ctx);
    if !hurt {
        sounds.push("minecraft:entity.player.attack.nodamage");
        play_sounds(players, a, &sounds, env);
        return;
    }
    // `causeExtraKnockback` with `getKnockback(target, source)`.
    let strength =
        players[a].attack_knockback(&target_view, &source, attack_rng(ctx)) + if sprint_knockback { 0.5 } else { 0.0 };
    let rad = (yaw * 0.017453292) as f64;
    if strength > 0.0 {
        players[t].knockback(strength as f64, mth_sin(rad) as f64, -mth_cos(rad) as f64);
        let p = &mut *players[a];
        p.vel = [p.vel[0] * 0.6, p.vel[1], p.vel[2] * 0.6];
        if p.sprinting {
            p.sprinting = false;
            p.meta_dirty = true;
        }
    }
    let victim = &mut *players[t];
    if victim.sync_velocity {
        victim.send(entity::set_entity_motion(victim.entity_id, victim.vel));
        victim.sync_velocity = false;
        victim.vel = old_vel;
    }
    if sweep {
        sweep_attack(players, a, t, damage, &source, scale, env, ctx);
    }
    // `attackVisualEffects`.
    if crit {
        sounds.push("minecraft:entity.player.attack.crit");
        let pkt = entity::animate(target_id, entity::animation::CRITICAL_HIT);
        send_to_trackers_and_self(players, a, &pkt);
    }
    if !crit && !sweep {
        sounds.push(if full { "minecraft:entity.player.attack.strong" } else { "minecraft:entity.player.attack.weak" });
    }
    // `itemAttackInteraction`: the post-attack enchantment effects (any held item, even
    // none), then a weapon (the `weapon` component, `hurtEnemy`) loses durability.
    let per_attack = players[a].inv.selected_item().get(keys::WEAPON).map(|w| w.item_damage_per_attack);
    post_attack(players, a, t, &source, ctx);
    if let Some(n) = per_attack
        && !players[a].inv.selected_item().is_empty()
    {
        // `ItemStack.hurtEnemy`: a weapon counts as used.
        let item = players[a].inv.selected_item().item();
        players[a].award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
        players[a].hurt_and_break(EquipmentSlot::MainHand, n, ctx.level_rng.as_mut());
    }
    // `damageStatsAndHearts`: the damage statistic, heart particles for more than a heart.
    let dealt = health_before - players[t].health;
    players[a].award_stat(*crate::player_stats::stat::DAMAGE_DEALT, (dealt * 10.0).round() as i32);
    if dealt > 2.0 {
        let count = (dealt as f64 * 0.5) as i32;
        let at = [players[t].pos[0], players[t].pos[1] + 0.9, players[t].pos[2]];
        send_particles(players, "minecraft:damage_indicator", at, count, [0.1, 0.0, 0.1], 0.2);
    }
    players[a].exhaust(0.1);
    play_sounds(players, a, &sounds, env);
}

/// `Player.doSweepAttack`: other living entities near the target take `1 + ratio * damage`
/// (times the attack strength) and a small knockback.
#[allow(clippy::too_many_arguments)]
fn sweep_attack(
    players: &mut [&mut Player],
    a: usize,
    t: usize,
    damage: f32,
    source: &Source,
    scale: f32,
    env: &AttackEnv,
    ctx: &mut DamageCtx,
) {
    let sweep = 1.0 + players[a].attribute(SWEEPING_DAMAGE_RATIO) as f32 * damage;
    let bb = players[t].bounding_box().inflate(1.0, 0.25, 1.0);
    let (attacker_pos, yaw) = (players[a].pos, players[a].rot[0]);
    let rad = (yaw * 0.017453292) as f64;
    // `getEntitiesOfClass(LivingEntity, ...)` skips spectators.
    let hit: Vec<usize> = (0..players.len())
        .filter(|&i| i != a && i != t)
        .filter(|&i| !players[i].dead && players[i].game_mode != 3 && !players[i].disconnected)
        .filter(|&i| players[i].bounding_box().intersects(&bb))
        .collect();
    for i in hit {
        let p = &players[i].pos;
        let d2 = (p[0] - attacker_pos[0]).powi(2) + (p[1] - attacker_pos[1]).powi(2) + (p[2] - attacker_pos[2]).powi(2);
        if d2 >= 9.0 {
            continue;
        }
        // `getEnchantedDamage(entity, sweep, source) * scale`.
        let view = players[i].view();
        let amount = players[a].enchanted_damage(&view, sweep, source, attack_rng(ctx)) * scale;
        if players[i].hurt(amount, source, ctx) {
            players[i].knockback(0.4000000059604645, mth_sin(rad) as f64, -mth_cos(rad) as f64);
            // `doPostAttackEffects`: the source's attacker is a living entity, so its weapon too.
            post_attack(players, a, i, source, ctx);
        }
    }
    let (dx, dz) = (-mth_sin(rad) as f64, mth_cos(rad) as f64);
    let at = [attacker_pos[0] + dx, attacker_pos[1] + 0.9, attacker_pos[2] + dz];
    send_particles(players, "minecraft:sweep_attack", at, 0, [dx as f32, 0.0, dz as f32], 0.0);
    play_sounds(players, a, &["minecraft:entity.player.attack.sweep"], env);
}

/// Whether the player's feet are in a climbable block or in water (`onClimbable`,
/// `isInWater` approximated by the block at the feet).
fn feet_state(p: &Player, cells: &kiln_region::CellSet<kiln_world::Cell>) -> (bool, bool) {
    use kiln_world::Blocks;
    let f = p.pos.map(|c| c.floor() as i32);
    let Some(state) = cells.get_block(f[0], f[1], f[2]) else { return (false, false) };
    let climbing = p.game_mode != 3 && kiln_entity::blocks::has_tag(state, kiln_entity::blocks::Tag::Climbable);
    let water = kiln_entity::blocks::kind(state) == kiln_entity::blocks::Kind::Water;
    (climbing, water)
}

/// `Level.playSound(null, attacker position, ...)`: heard by players within 16 blocks.
fn play_sounds(players: &mut [&mut Player], a: usize, sounds: &[&str], env: &AttackEnv) {
    let at = players[a].pos;
    for (n, sound) in sounds.iter().enumerate() {
        let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { continue };
        let mut seed = (env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.seed as u64;
        for v in [players[a].entity_id as u64, n as u64, at[0].to_bits(), at[2].to_bits()] {
            seed = (seed ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            seed ^= seed >> 31;
        }
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Players, at, 1.0, 1.0, seed as i64);
        for p in players.iter_mut().filter(|p| dist2(p.pos, at) < 16.0 * 16.0) {
            p.send(pkt.clone());
        }
    }
}

/// `ServerLevel.sendParticles`: to players within 32 blocks.
fn send_particles(players: &mut [&mut Player], particle: &str, at: [f64; 3], count: i32, offset: [f32; 3], speed: f32) {
    let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", particle) else { return };
    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
        particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
        override_limiter: false,
        always_show: false,
        pos: at,
        offset,
        max_speed: [speed; 3],
        count,
        randomization: world_fx::ParticleRandomization::Default,
    });
    for p in players.iter_mut().filter(|p| dist2(p.pos, at) < 32.0 * 32.0) {
        p.send(pkt.clone());
    }
}

/// `sendToTrackingPlayersAndSelf` for player `a`.
fn send_to_trackers_and_self(players: &mut [&mut Player], a: usize, pkt: &bytes::Bytes) {
    let viewers = players[a].seen_by.clone();
    players[a].send(pkt.clone());
    for v in viewers {
        if let Ok(i) = players.binary_search_by_key(&v, |p| p.conn) {
            players[i].send(pkt.clone());
        }
    }
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armor_formula() {
        // Full diamond (20 armor, 8 toughness) against a 7-damage hit.
        let d = damage_after_absorb(7.0, 20.0, 8.0, |h| h);
        assert_eq!(d, 7.0 * (1.0 - ((20.0f32 - 7.0 / 4.0).clamp(4.0, 20.0) / 25.0)));
        assert_eq!(damage_after_absorb(5.0, 0.0, 0.0, |h| h), 5.0);
    }

    #[test]
    fn attribute_math() {
        let mods = [(6.0, AttributeOperation::AddValue), (0.5, AttributeOperation::AddMultipliedBase), (0.1, AttributeOperation::AddMultipliedTotal)];
        // (1 + 6) + 7 * 0.5 = 10.5, then * 1.1.
        assert_eq!(attribute_value(ATTACK_DAMAGE, mods.iter().copied()), 10.5 * 1.1);
        assert_eq!(attribute_value(KNOCKBACK_RESISTANCE, [(3.0, AttributeOperation::AddValue)].into_iter()), 1.0);
    }
}
