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
    /// What `type_specific/player` asks of a player.
    pub player: Option<PlayerFacts>,
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

fn entries_match(v: &Json, facts: &[(&str, f64)]) -> bool {
    let Json::Obj(entries) = v else { return false };
    entries.iter().all(|(k, b)| facts.iter().find(|(name, _)| name == k).is_some_and(|(_, value)| bounds(b, *value)))
}

impl EntityView {
    /// `EntityPredicate.matches` for the parts Kiln can answer; anything about vehicles,
    /// passengers, targets, locations, effects or NBT does not match (players ride nothing).
    pub fn matches(&self, p: &EntityPredicate) -> bool {
        p.parts.iter().all(|part| match part {
            EntitySubPredicate::EntityType(set) => set.contains(self.type_id),
            EntitySubPredicate::Flags(f) => {
                let ok = |want: Option<bool>, have: bool| want.is_none_or(|w| w == have);
                ok(f.is_on_ground, self.on_ground)
                    && ok(f.is_on_fire, self.on_fire)
                    && ok(f.is_sneaking, self.sneaking)
                    && ok(f.is_sprinting, self.sprinting)
                    && ok(f.is_flying, self.flying)
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
            _ => false,
        })
    }
}

/// Whether a `minecraft:damage_type` network id is in a damage type tag.
pub fn damage_type_in_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
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

/// What a weapon's `post_attack` enchantments do to the victim of a mob's blow.
#[derive(Debug, Clone, PartialEq)]
pub enum MobPostAttack {
    /// `ignite` (fire aspect): the victim burns for `seconds`.
    Ignite { seconds: f32 },
    /// `apply_mob_effect` (bane of arthropods): `effect` with the duration (ticks) and amplifier
    /// as rolled.
    MobEffect { effect: String, duration: i32, amplifier: i32 },
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
            if t.affected == Target::Victim {
                collect_post_attack(&t.effect, level, rng, &mut out);
            }
        }
        out
    }
}

fn collect_post_attack(effect: &EntityEffect, level: i32, rng: &mut dyn RandomSource, out: &mut Vec<MobPostAttack>) {
    match effect {
        EntityEffect::AllOf(list) => list.iter().for_each(|e| collect_post_attack(e, level, rng, out)),
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
