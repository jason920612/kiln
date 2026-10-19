//! Entities as enchantment predicates see them (`EntityPredicate` over the facts a level can
//! give of an entity), and the loot context an attack by a mob is evaluated in
//! (`Enchantment.damageContext`), shared by whoever carries out attacks: the simulation for
//! players and the mobs' attacks.

use crate::context::{EntityTarget, LootContext};
use crate::data::LootData;
use crate::effects::{EntityEffect, Target};
use crate::json::Json;
use crate::predicate::{DamageSourcePredicate, EntityPredicate, EntitySubPredicate};
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;

/// What predicates can ask of an entity.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityView {
    /// `minecraft:entity_type` network id.
    pub type_id: i32,
    pub pos: [f64; 3],
    pub on_ground: bool,
    pub on_fire: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub flying: bool,
    /// The entity rides something (`vehicle` predicates with no conditions match).
    pub has_vehicle: bool,
    pub fall_flying: bool,
    pub in_water: bool,
    /// `Entity.fallDistance` (the `movement` predicate's `fall_distance`).
    pub fall_distance: f64,
    /// What `type_specific/player` asks of a player.
    pub player: Option<PlayerFacts>,
    /// `Entity.getKnownMovement()`, blocks per tick (the `movement` predicate scales it to a second).
    pub movement: [f64; 3],
    /// `getBlockPosBelowThatAffectsMyMovement` (the `movement_affected_by` predicate).
    pub movement_block: Option<[i32; 3]>,
    /// `getOnPos` (the `stepping_on` predicate, true only on the ground).
    pub on_pos: Option<[i32; 3]>,
    /// `Entity.tickCount` (the `periodic_tick` predicate).
    pub tick_count: i32,
}

/// A player's game mode and stats, for `type_specific/player` predicates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerFacts {
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: u8,
    pub food: i32,
    pub saturation: f32,
    pub level: i32,
}

impl PlayerFacts {
    /// `PlayerPredicate.matches` for the parts Kiln can answer: `gamemode`, `food` and `level`;
    /// anything else (advancements, recipes, stats, input, looking_at) does not match.
    fn matches(&self, json: &Json) -> bool {
        let Json::Obj(entries) = json else { return false };
        entries.iter().all(|(key, v)| match key.as_str() {
            "gamemode" => {
                let name = ["survival", "creative", "adventure", "spectator"][self.game_mode.min(3) as usize];
                v.as_array().is_some_and(|l| l.iter().any(|m| m.as_str() == Some(name)))
            }
            "level" => bounds(v, self.level as f64),
            "food" => entries_match(v, &[("level", self.food as f64), ("saturation", self.saturation as f64)]),
            _ => false,
        })
    }
}

/// `MinMaxBounds.matches`: a plain number is exact, else the `min` and `max` given.
fn bounds(v: &Json, value: f64) -> bool {
    match v {
        Json::Obj(_) => v.get("min").and_then(|m| m.as_f64()).is_none_or(|m| value >= m) && v.get("max").and_then(|m| m.as_f64()).is_none_or(|m| value <= m),
        other => other.as_f64().is_some_and(|x| x == value),
    }
}

/// `MinMaxBounds.Doubles.matchesSqr`: the bounds squared against a squared value.
fn bounds_sqr(v: &Json, value_sqr: f64) -> bool {
    match v {
        Json::Obj(_) => {
            v.get("min").and_then(|m| m.as_f64()).is_none_or(|m| value_sqr >= m * m) && v.get("max").and_then(|m| m.as_f64()).is_none_or(|m| value_sqr <= m * m)
        }
        other => other.as_f64().is_some_and(|x| x * x == value_sqr),
    }
}

fn entries_match(v: &Json, facts: &[(&str, f64)]) -> bool {
    let Json::Obj(entries) = v else { return false };
    entries.iter().all(|(k, b)| facts.iter().find(|(name, _)| name == k).is_some_and(|(_, value)| bounds(b, *value)))
}

impl EntityView {
    /// `EntityPredicate.matches` for the parts Kiln can answer; anything about vehicles,
    /// passengers, targets, locations, effects or NBT does not match (players ride nothing).
    pub fn matches(&self, p: &EntityPredicate) -> bool {
        self.matches_with(p, &|_, _| false)
    }

    /// [`EntityView::matches`] with `location` answering a `LocationPredicate` at a position (for the
    /// `location`, `stepping_on` and `movement_affected_by` parts).
    pub fn matches_with(&self, p: &EntityPredicate, location: &dyn Fn(&crate::predicate::world::LocationPredicate, [f64; 3]) -> bool) -> bool {
        let center = |p: [i32; 3]| [p[0] as f64 + 0.5, p[1] as f64 + 0.5, p[2] as f64 + 0.5];
        p.parts.iter().all(|part| match part {
            EntitySubPredicate::Location(l) => location(l, self.pos),
            EntitySubPredicate::SteppingOn(l) => self.on_ground && self.on_pos.is_some_and(|at| location(l, center(at))),
            EntitySubPredicate::MovementAffectedBy(l) => self.movement_block.is_some_and(|at| location(l, center(at))),
            EntitySubPredicate::EntityType(set) => set.contains(self.type_id),
            EntitySubPredicate::Flags(f) => {
                let ok = |want: Option<bool>, have: bool| want.is_none_or(|w| w == have);
                ok(f.is_on_ground, self.on_ground)
                    && ok(f.is_on_fire, self.on_fire)
                    && ok(f.is_sneaking, self.sneaking)
                    && ok(f.is_sprinting, self.sprinting)
                    && ok(f.is_flying, self.flying || self.fall_flying)
                    && ok(f.is_baby, false)
                    && ok(f.is_swimming, false)
                    && ok(f.is_fall_flying, self.fall_flying)
                    && ok(f.is_in_water, self.in_water)
            }
            // A vehicle with no conditions: whether there is one.
            EntitySubPredicate::Vehicle(v) => v.parts.is_empty() && self.has_vehicle,
            EntitySubPredicate::Other(id, json) if id.as_str() == "minecraft:type_specific/player" => {
                self.player.as_ref().is_some_and(|p| p.matches(json))
            }
            // `PeriodicEntityTickPredicate.matches`: every `ticks` ticks of the entity's life.
            EntitySubPredicate::Other(id, json) if id.as_str() == "minecraft:periodic_tick" => {
                json.as_i32().is_some_and(|period| period != 0 && self.tick_count % period == 0)
            }
            // `MovementPredicate.matches(entity)`: the known movement scaled to a second.
            EntitySubPredicate::Other(id, json) if id.as_str() == "minecraft:movement" => {
                let Json::Obj(entries) = json else { return false };
                let v = [self.movement[0] * 20.0, self.movement[1] * 20.0, self.movement[2] * 20.0];
                entries.iter().all(|(key, b)| match key.as_str() {
                    "x" => bounds(b, v[0]),
                    "y" => bounds(b, v[1]),
                    "z" => bounds(b, v[2]),
                    "speed" => bounds_sqr(b, v[0] * v[0] + v[1] * v[1] + v[2] * v[2]),
                    "horizontal_speed" => bounds_sqr(b, v[0] * v[0] + v[2] * v[2]),
                    "vertical_speed" => bounds(b, v[1].abs()),
                    "fall_distance" => bounds(b, self.fall_distance),
                    _ => false,
                })
            }
            _ => false,
        })
    }
}

/// Whether a `minecraft:damage_type` network id is in a damage type tag.
pub fn damage_type_in_tag(id: i32, tag: &str) -> bool {
    kiln_data::live::tags()
        .iter()
        .find(|(r, _)| *r == "minecraft:damage_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

/// `Enchantment.damageContext(level, victim, source)` of a hit by a mob: `this_entity` (the
/// victim), `enchantment_level`, `origin` (the victim's position), `damage_source` and the
/// attacker (also the direct attacker: a melee or spear blow).
pub struct MobDamageContext<'a> {
    pub level: i32,
    pub this: &'a EntityView,
    pub attacker: &'a EntityView,
    /// `minecraft:damage_type` network id of the source.
    pub damage_type: i32,
}

impl LootContext for MobDamageContext<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        matches!(target, EntityTarget::This | EntityTarget::Attacker | EntityTarget::DirectAttacker)
    }

    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.this.pos)
    }

    fn has_damage_source(&self) -> bool {
        true
    }

    fn enchantment_level(&self) -> Option<i32> {
        Some(self.level)
    }

    fn entity_matches(&self, target: EntityTarget, predicate: &EntityPredicate) -> bool {
        match target {
            EntityTarget::This => self.this.matches(predicate),
            EntityTarget::Attacker | EntityTarget::DirectAttacker => self.attacker.matches(predicate),
            _ => false,
        }
    }

    fn damage_source_matches(&self, p: &DamageSourcePredicate) -> bool {
        if !p.tags.iter().all(|t| damage_type_in_tag(self.damage_type, t.tag.as_str()) == t.expected) {
            return false;
        }
        if let Some(d) = &p.direct_entity
            && !self.attacker.matches(d)
        {
            return false;
        }
        if let Some(s) = &p.source_entity
            && !self.attacker.matches(s)
        {
            return false;
        }
        // `isDirect`: the attacker is the direct entity (a mob's blow).
        p.is_direct.is_none_or(|want| want)
    }
}

/// `Enchantment.damageContext(level, victim, source)` of a hit by a projectile the weapon fired
/// (`AbstractArrow.onHitEntity`: `getWeaponItem` is the bow): the `direct_attacker` is the
/// projectile, the `attacker` its owner (when there is one).
pub struct ProjectileDamageContext<'a> {
    pub level: i32,
    pub this: &'a EntityView,
    pub attacker: Option<&'a EntityView>,
    pub direct: &'a EntityView,
    /// `minecraft:damage_type` network id of the source.
    pub damage_type: i32,
}

impl LootContext for ProjectileDamageContext<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        match target {
            EntityTarget::This | EntityTarget::DirectAttacker => true,
            EntityTarget::Attacker => self.attacker.is_some(),
            _ => false,
        }
    }

    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.this.pos)
    }

    fn has_damage_source(&self) -> bool {
        true
    }

    fn enchantment_level(&self) -> Option<i32> {
        Some(self.level)
    }

    fn entity_matches(&self, target: EntityTarget, predicate: &EntityPredicate) -> bool {
        match target {
            EntityTarget::This => self.this.matches(predicate),
            EntityTarget::Attacker => self.attacker.is_some_and(|a| a.matches(predicate)),
            EntityTarget::DirectAttacker => self.direct.matches(predicate),
            _ => false,
        }
    }

    fn damage_source_matches(&self, p: &DamageSourcePredicate) -> bool {
        if !p.tags.iter().all(|t| damage_type_in_tag(self.damage_type, t.tag.as_str()) == t.expected) {
            return false;
        }
        if let Some(d) = &p.direct_entity
            && !self.direct.matches(d)
        {
            return false;
        }
        if let Some(s) = &p.source_entity
            && !self.attacker.is_some_and(|a| a.matches(s))
        {
            return false;
        }
        // `isDirect`: never, the projectile is not the attacker.
        p.is_direct.is_none_or(|want| !want)
    }
}

/// What a weapon's `post_attack` enchantments do to the victim of a mob's blow.
#[derive(Debug, Clone, PartialEq)]
pub enum MobPostAttack {
    /// `ignite` (fire aspect): the victim burns for `seconds`.
    Ignite { seconds: f32 },
    /// `apply_mob_effect` (bane of arthropods): `effect` with the duration (ticks) and amplifier
    /// as rolled.
    MobEffect { effect: String, duration: i32, amplifier: i32 },
    /// `explode` (the mace's wind burst): an explosion at the wielder (`on_victim` false) or at the victim.
    Explode { effect: EffectRef, level: i32, on_victim: bool },
}

/// An entity effect handed on whole (never equal to another).
#[derive(Debug, Clone)]
pub struct EffectRef(pub std::sync::Arc<EntityEffect>);

impl PartialEq for EffectRef {
    fn eq(&self, other: &EffectRef) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl LootData {
    /// `EnchantmentHelper.modifyDamage(level, weapon, victim, source, damage)` for a weapon a
    /// mob wields (`attacker` is also the direct entity of the blow).
    pub fn mob_modify_damage(
        &self,
        weapon: &ItemStack,
        attacker: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        damage: f32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.modify_damage(weapon, rng, damage, |level| MobDamageContext { level, this: victim, attacker, damage_type })
    }

    /// `EnchantmentHelper.modifyDamage(level, weapon, victim, source, damage)` for a hit by a
    /// projectile `weapon` fired (power on an arrow, impaling on a trident).
    pub fn projectile_modify_damage(
        &self,
        weapon: &ItemStack,
        attacker: Option<&EntityView>,
        direct: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        damage: f32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.modify_damage(weapon, rng, damage, |level| ProjectileDamageContext { level, this: victim, attacker, direct, damage_type })
    }

    /// `EnchantmentHelper.modifyKnockback` for a hit by a projectile `weapon` fired (punch).
    pub fn projectile_modify_knockback(
        &self,
        weapon: &ItemStack,
        attacker: Option<&EntityView>,
        direct: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        value: f32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.modify_knockback(weapon, rng, value, |level| ProjectileDamageContext { level, this: victim, attacker, direct, damage_type })
    }

    /// `EnchantmentHelper.modifyArmorEffectiveness` of `weapon` against a victim (a mob wearing
    /// armor).
    pub fn mob_armor_effectiveness(
        &self,
        weapon: &ItemStack,
        attacker: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        value: f32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.modify_armor_effectiveness(weapon, rng, value, |level| MobDamageContext { level, this: victim, attacker, damage_type })
    }

    /// `EnchantmentHelper.getDamageProtection(level, victim, source)` for a mob victim: its
    /// equipment's `damage_protection` effects.
    pub fn mob_damage_protection(
        &self,
        equipment: &[(EquipmentSlot, &ItemStack)],
        victim: &EntityView,
        attacker: &EntityView,
        damage_type: i32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.damage_protection(equipment, rng, |level| MobDamageContext { level, this: victim, attacker, damage_type })
    }

    /// `EnchantmentHelper.isImmuneToDamage(level, victim, source)` for a mob victim: its equipment's `damage_immunity` effects (frost walker and
    /// the damage of stepping on magma).
    pub fn mob_damage_immunity(
        &self,
        equipment: &[(EquipmentSlot, &ItemStack)],
        victim: &EntityView,
        attacker: &EntityView,
        damage_type: i32,
        rng: &mut dyn RandomSource,
    ) -> bool {
        self.is_immune_to_damage(equipment, rng, |level| MobDamageContext { level, this: victim, attacker, damage_type })
    }

    /// `EnchantmentHelper.modifyKnockback(level, weapon, victim, source, value)` for a mob's weapon.
    pub fn mob_modify_knockback(
        &self,
        weapon: &ItemStack,
        attacker: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        value: f32,
        rng: &mut dyn RandomSource,
    ) -> f32 {
        self.modify_knockback(weapon, rng, value, |level| MobDamageContext { level, this: victim, attacker, damage_type })
    }

    /// `EnchantmentHelper.doPostAttackEffectsWithItemSource` for a mob's weapon: what its
    /// enchantments do to the victim (the effects on the attacker itself, thorns' damage, are not
    /// among them). Rolls (`apply_mob_effect`) draw from `rng`.
    pub fn mob_post_attack(
        &self,
        weapon: &ItemStack,
        attacker: &EntityView,
        victim: &EntityView,
        damage_type: i32,
        rng: &mut dyn RandomSource,
    ) -> Vec<MobPostAttack> {
        let mut found: Vec<(&crate::effects::Targeted<EntityEffect>, i32)> = Vec::new();
        self.post_attack_effects(weapon, EquipmentSlot::MainHand, Target::Attacker, rng, |level| MobDamageContext { level, this: victim, attacker, damage_type }, |t, level| {
            found.push((t, level))
        });
        let mut out = Vec::new();
        for (t, level) in found {
            collect_post_attack(&t.effect, level, rng, t.affected == Target::Victim, &mut out);
        }
        out
    }
}

fn collect_post_attack(effect: &EntityEffect, level: i32, rng: &mut dyn RandomSource, on_victim: bool, out: &mut Vec<MobPostAttack>) {
    match effect {
        EntityEffect::AllOf(list) => list.iter().for_each(|e| collect_post_attack(e, level, rng, on_victim, out)),
        EntityEffect::Explode { .. } => out.push(MobPostAttack::Explode { effect: EffectRef(std::sync::Arc::new(effect.clone())), level, on_victim }),
        // (What the wielder itself undergoes besides the explosion is not carried out for a mob.)
        _ if !on_victim => {}
        EntityEffect::Ignite(seconds) => out.push(MobPostAttack::Ignite { seconds: seconds.calculate(level) }),
        // `ApplyMobEffect.apply`: a random entry of the list, duration and amplifier rolled.
        EntityEffect::ApplyMobEffect { to_apply, min_duration, max_duration, min_amplifier, max_amplifier } => {
            if to_apply.is_empty() {
                return;
            }
            let pick = &to_apply[rng.next_int_bounded(to_apply.len() as i32) as usize];
            let mut between = |min: f32, max: f32| rng.next_float() * (max - min) + min;
            let round = |v: f32| (v as f64 + 0.5).floor() as i32;
            let duration = round(between(min_duration.calculate(level), max_duration.calculate(level)) * 20.0);
            let amplifier = round(between(min_amplifier.calculate(level), max_amplifier.calculate(level))).max(0);
            out.push(MobPostAttack::MobEffect { effect: pick.as_str().to_owned(), duration, amplifier });
        }
        _ => {}
    }
}

/// The context of an enchantment effect that is not a plain hit (`Enchantment.entityContext`,
/// `locationContext`, `blockHitContext`, and `damageContext` of a hit with the level around):
/// `this_entity` with the enchantment's level at `origin`, and whichever of the damage source's
/// entities, the hit block's state and the active flag the trigger has.
pub struct FxContext<'a> {
    pub level: i32,
    pub this: &'a EntityView,
    /// A damage context's attacker and direct attacker, and the source's damage type.
    pub attacker: Option<&'a EntityView>,
    pub direct: Option<&'a EntityView>,
    pub damage_type: Option<i32>,
    /// A hit-block context's block state.
    pub block_state: Option<u16>,
    pub origin: [f64; 3],
    /// A location-based effect's `enchantment_active_check`.
    pub active: Option<bool>,
    pub world: Option<crate::probe::WorldFacts<'a>>,
}

impl<'a> FxContext<'a> {
    /// `Enchantment.entityContext(level, entity, origin)`.
    pub fn entity(level: i32, this: &'a EntityView, origin: [f64; 3], world: Option<crate::probe::WorldFacts<'a>>) -> FxContext<'a> {
        FxContext { level, this, attacker: None, direct: None, damage_type: None, block_state: None, origin, active: None, world }
    }

    fn view(&self, target: EntityTarget) -> Option<&EntityView> {
        match target {
            EntityTarget::This => Some(self.this),
            EntityTarget::Attacker => self.attacker,
            EntityTarget::DirectAttacker => self.direct.or(self.attacker),
            _ => None,
        }
    }
}

impl LootContext for FxContext<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        self.view(target).is_some()
    }

    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }

    fn block_state(&self) -> Option<u16> {
        self.block_state
    }

    fn has_damage_source(&self) -> bool {
        self.damage_type.is_some()
    }

    fn enchantment_level(&self) -> Option<i32> {
        Some(self.level)
    }

    fn enchantment_active(&self) -> Option<bool> {
        self.active
    }

    fn is_raining(&self) -> bool {
        self.world.is_some_and(|w| w.raining)
    }

    fn is_thundering(&self) -> bool {
        self.world.is_some_and(|w| w.thundering)
    }

    fn entity_matches(&self, target: EntityTarget, predicate: &EntityPredicate) -> bool {
        self.view(target).is_some_and(|v| v.matches_with(predicate, &|l, pos| self.location_matches(l, pos)))
    }

    fn location_matches(&self, predicate: &crate::predicate::world::LocationPredicate, pos: [f64; 3]) -> bool {
        match &self.world {
            Some(w) => crate::probe::location_matches(predicate, pos, w.dim, Some(w.probe)),
            None => false,
        }
    }

    fn damage_source_matches(&self, p: &DamageSourcePredicate) -> bool {
        let Some(damage_type) = self.damage_type else { return false };
        if !p.tags.iter().all(|t| damage_type_in_tag(damage_type, t.tag.as_str()) == t.expected) {
            return false;
        }
        if let Some(d) = &p.direct_entity
            && !self.direct.or(self.attacker).is_some_and(|e| e.matches(d))
        {
            return false;
        }
        if let Some(s) = &p.source_entity
            && !self.attacker.is_some_and(|e| e.matches(s))
        {
            return false;
        }
        // `isDirect`: no separate direct entity (a melee blow).
        p.is_direct.is_none_or(|want| want == self.direct.is_none())
    }
}
