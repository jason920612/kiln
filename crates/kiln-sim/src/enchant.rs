//! The world side of enchantment effects: the loot contexts enchantment requirements are
//! tested in (`Enchantment.damageContext`), with the entity and damage source predicates they
//! use evaluated against players and entities, and the post-attack entity effects carried out
//! on players.
//!
//! Enchantment definitions come from the datapack's loot data ([`kiln_loot::LootData`]); the
//! aggregations (`EnchantmentHelper`) live in `kiln_loot::effects`.
//!
//! Randomness: vanilla tests requirements and runs `item_damage` with the level's random and
//! draws `damage_entity` amounts from the affected entity's random. Kiln has no level-wide random
//! that is independent of how the world is split into regions, so each player carries a
//! Java-exact "level random" of its own; during an attack every draw uses the attacker's, the
//! way a whole attack draws from one level random in vanilla.

use crate::Player;
use crate::health::Source;
use kiln_javamath::random::LegacyRandom;
use kiln_loot::context::EntityTarget;
use kiln_loot::effects::{EntityEffect, Target, Targeted};
use kiln_loot::predicate::{DamageSourcePredicate, EntityPredicate, EntitySubPredicate};
use kiln_loot::LootContext;
use kiln_item::component::EquipmentSlot;
use kiln_item::ItemStack;

/// `EnchantmentHelper.enchantItemFromProvider`: the datapack's provider `provider` (an
/// `enchantment_provider` id such as `minecraft:mob_spawn_equipment`) applied to `stack`.
pub(crate) fn enchant_from_provider(loot: &kiln_loot::LootData, stack: &mut ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn kiln_javamath::random::RandomSource) {
    loot.enchant_from_provider(provider, stack, special_multiplier, random);
}

/// The [`kiln_entity::enchanting::Enchanter`] of a datapack.
struct LootEnchanter(std::sync::Arc<kiln_loot::LootData>);

impl kiln_entity::enchanting::Enchanter for LootEnchanter {
    fn enchant(&self, stack: &mut ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn kiln_javamath::random::RandomSource) {
        self.0.enchant_from_provider(provider, stack, special_multiplier, random);
    }
}

/// Lets the mobs this thread runs enchant their spawn equipment from `loot`'s providers until
/// the result is dropped.
pub(crate) fn install_enchanter(loot: Option<&std::sync::Arc<kiln_loot::LootData>>) -> kiln_entity::enchanting::Installed {
    kiln_entity::enchanting::install(loot.map(|l| std::rc::Rc::new(LootEnchanter(l.clone())) as std::rc::Rc<dyn kiln_entity::enchanting::Enchanter>))
}

/// What predicates can ask of an entity.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct EntityView {
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
pub(crate) struct PlayerFacts {
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: u8,
    pub food: i32,
    pub saturation: f32,
    pub level: i32,
}

impl PlayerFacts {
    /// `PlayerPredicate.matches` for the parts Kiln can answer: `gamemode`, `food` and `level`;
    /// anything else (advancements, recipes, stats, input, looking_at) does not match.
    fn matches(&self, json: &kiln_loot::json::Json) -> bool {
        use kiln_loot::json::Json;
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
fn bounds(v: &kiln_loot::json::Json, value: f64) -> bool {
    use kiln_loot::json::Json;
    match v {
        Json::Obj(_) => v.get("min").and_then(|m| m.as_f64()).is_none_or(|m| value >= m) && v.get("max").and_then(|m| m.as_f64()).is_none_or(|m| value <= m),
        other => other.as_f64().is_some_and(|x| x == value),
    }
}

fn entries_match(v: &kiln_loot::json::Json, facts: &[(&str, f64)]) -> bool {
    let kiln_loot::json::Json::Obj(entries) = v else { return false };
    entries.iter().all(|(k, b)| facts.iter().find(|(name, _)| name == k).is_some_and(|(_, value)| bounds(b, *value)))
}

/// `minecraft:player`'s entity type id.
pub(crate) fn player_type() -> i32 {
    kiln_item::registry::ENTITY_TYPE.id("minecraft:player").unwrap_or(-1)
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

impl Player {
    pub(crate) fn view(&self) -> EntityView {
        EntityView {
            type_id: player_type(),
            pos: self.pos,
            on_ground: self.on_ground,
            on_fire: self.fire_ticks > 0,
            sneaking: self.sneaking,
            sprinting: self.sprinting,
            flying: self.flying,
            has_vehicle: self.vehicle.is_some(),
            fall_flying: self.fall_flying,
            // (Where the water is needs the blocks: [`Player::view_in`].)
            in_water: false,
            player: Some(PlayerFacts { game_mode: self.game_mode, food: self.food, saturation: self.saturation, level: self.xp_level }),
        }
    }

    /// [`Player::view`] with the fluids the player stands in.
    pub(crate) fn view_in(&self, block: crate::hazards::BlockAt) -> EntityView {
        EntityView { in_water: self.fluids(block).in_water, ..self.view() }
    }

    /// The player's equipment per slot, for `runIterationOnEquipment`.
    pub(crate) fn equipment(&self) -> Vec<(EquipmentSlot, &ItemStack)> {
        crate::combat::SLOTS.iter().map(|s| (*s, self.inv.equipped(*s))).collect()
    }
}

/// `Enchantment.damageContext(level, entity, source)`: `this_entity`, `enchantment_level`,
/// `origin` (the entity's position), `damage_source`, and the source's attacker and direct
/// attacker when there are.
pub(crate) struct DamageContext<'a> {
    pub level: i32,
    pub this: &'a EntityView,
    pub source: &'a Source,
}

impl DamageContext<'_> {
    fn attacker(&self) -> Option<&EntityView> {
        self.source.attacker.as_ref().map(|a| &a.view)
    }

    /// `getDirectEntity`: the attacker unless something else dealt the damage (Kiln's direct
    /// entities other than the attacker are projectiles it cannot describe).
    fn direct(&self) -> Option<&EntityView> {
        if self.source.direct.is_some() { None } else { self.attacker() }
    }

    fn entity(&self, target: EntityTarget) -> Option<&EntityView> {
        match target {
            EntityTarget::This => Some(self.this),
            EntityTarget::Attacker => self.attacker(),
            EntityTarget::DirectAttacker => self.direct(),
            _ => None,
        }
    }
}

impl LootContext for DamageContext<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        match target {
            EntityTarget::This => true,
            EntityTarget::Attacker => self.source.attacker.is_some(),
            EntityTarget::DirectAttacker => self.source.attacker.is_some() || self.source.direct.is_some(),
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
        self.entity(target).is_some_and(|e| e.matches(predicate))
    }

    fn damage_source_matches(&self, p: &DamageSourcePredicate) -> bool {
        if !p.tags.iter().all(|t| self.source.is(t.tag.as_str()) == t.expected) {
            return false;
        }
        if let Some(d) = &p.direct_entity
            && !self.direct().is_some_and(|e| e.matches(d))
        {
            return false;
        }
        if let Some(s) = &p.source_entity
            && !self.attacker().is_some_and(|e| e.matches(s))
        {
            return false;
        }
        // `isDirect`: no separate direct entity (melee, thorns).
        p.is_direct.is_none_or(|want| want == (self.source.direct.is_none()))
    }
}

/// A post-attack effect to carry out: who is affected, the item that holds the enchantment
/// (for `change_item_damage`) and the level.
pub(crate) struct PostAttack<'a> {
    pub effect: &'a Targeted<EntityEffect>,
    pub level: i32,
    /// Player index (in the region's player list) that owns the enchanted item, and its slot.
    pub owner: usize,
    pub slot: EquipmentSlot,
}

/// `EnchantmentHelper.doPostAttackEffectsWithItemSource(victim, source, weapon)` for player
/// `victim` hit by player `attacker`: the victim's equipment effects enchanted `victim`, then
/// the weapon's (`attacker`). Requirements draw from `rng`.
pub(crate) fn post_attack_effects<'a>(
    loot: &'a kiln_loot::LootData,
    players: &[&mut Player],
    victim: usize,
    attacker: Option<usize>,
    source: &Source,
    rng: &mut LegacyRandom,
) -> Vec<PostAttack<'a>> {
    let mut out = Vec::new();
    let view = players[victim].view();
    let ctx = |level: i32| DamageContext { level, this: &view, source };
    for (slot, stack) in players[victim].equipment() {
        loot.post_attack_effects(stack, slot, Target::Victim, rng, ctx, |effect, level| {
            out.push(PostAttack { effect, level, owner: victim, slot });
        });
    }
    if let (Some(a), Some(weapon)) = (attacker, source.weapon.as_ref()) {
        loot.post_attack_effects(weapon, EquipmentSlot::MainHand, Target::Attacker, rng, ctx, |effect, level| {
            out.push(PostAttack { effect, level, owner: a, slot: EquipmentSlot::MainHand });
        });
    }
    out
}

/// `Enchantment.doPostAttack`'s application: the affected entity (attacker, direct attacker or
/// victim) gets the effect, from its position.
pub(crate) fn apply_post_attack(
    players: &mut [&mut Player],
    victim: usize,
    attacker: Option<usize>,
    e: &PostAttack<'_>,
    ctx: &mut crate::health::DamageCtx,
) {
    let affected = match e.effect.affected {
        Target::Attacker | Target::DamagingEntity => attacker,
        Target::Victim => Some(victim),
    };
    let Some(affected) = affected else { return };
    apply_entity_effect(players, affected, e, &e.effect.effect, ctx);
}

fn apply_entity_effect(
    players: &mut [&mut Player],
    affected: usize,
    e: &PostAttack<'_>,
    effect: &EntityEffect,
    ctx: &mut crate::health::DamageCtx,
) {
    match effect {
        EntityEffect::AllOf(list) => {
            for inner in list {
                apply_entity_effect(players, affected, e, inner, ctx);
            }
        }
        EntityEffect::Ignite(seconds) => players[affected].ignite_for_seconds(seconds.calculate(e.level)),
        EntityEffect::DamageEntity { damage_type, min_damage, max_damage } => {
            let p = &mut *players[affected];
            // `Mth.randomBetween(entity.getRandom(), min, max)`.
            let (min, max) = (min_damage.calculate(e.level), max_damage.calculate(e.level));
            let amount = kiln_javamath::random::RandomSource::next_float(&mut p.entity_rng) * (max - min) + min;
            let owner = players[e.owner].as_attacker();
            let source = Source {
                cause: crate::health::Cause::Other(crate::health::static_damage_type(damage_type.as_str())),
                attacker: Some(owner),
                direct: None,
                weapon: None,
                position: None,
            };
            players[affected].hurt(amount, &source, ctx);
        }
        EntityEffect::ChangeItemDamage(amount) => {
            let owner = &mut *players[e.owner];
            let stack = owner.inv.equipped(e.slot);
            if stack.get(kiln_item::keys::MAX_DAMAGE).is_some() && stack.get(kiln_item::keys::DAMAGE).is_some() {
                owner.hurt_and_break(e.slot, amount.calculate(e.level) as i32, ctx.level_rng.as_mut());
            }
        }
        // `ApplyMobEffect.apply` (bane of arthropods' slowness): a random entry of the list,
        // duration and amplifier from the affected entity's random.
        EntityEffect::ApplyMobEffect { to_apply, min_duration, max_duration, min_amplifier, max_amplifier } => {
            use kiln_javamath::random::RandomSource;
            let p = &mut *players[affected];
            if to_apply.is_empty() {
                return;
            }
            let pick = &to_apply[p.entity_rng.next_int_bounded(to_apply.len() as i32) as usize];
            let between = |r: &mut kiln_javamath::random::LegacyRandom, min: f32, max: f32| r.next_float() * (max - min) + min;
            let round = |v: f32| (v as f64 + 0.5).floor() as i32;
            let (lo, hi) = (min_duration.calculate(e.level), max_duration.calculate(e.level));
            let duration = round(between(&mut p.entity_rng, lo, hi) * 20.0);
            let (lo, hi) = (min_amplifier.calculate(e.level), max_amplifier.calculate(e.level));
            let amplifier = round(between(&mut p.entity_rng, lo, hi)).max(0);
            if let Some(id) = crate::effects::effect_id(pick.as_str()) {
                p.add_effect(crate::effects::Effect::simple(id, duration, amplifier));
            }
        }
        EntityEffect::Other(_) | EntityEffect::ApplyExhaustion(_) | EntityEffect::ApplyImpulse { .. } | EntityEffect::PlaySound { .. } => {}
    }
}

impl Player {
    /// `Entity.igniteForSeconds`: `igniteForTicks(floor(seconds * 20))`, scaled by the
    /// `burning_time` attribute (fire protection), capped at 1 tick for invulnerable players
    /// (see [`crate::hazards`] for the burning).
    pub(crate) fn ignite_for_seconds(&mut self, seconds: f32) {
        let ticks = crate::combat::floor_f32(seconds * 20.0);
        let scaled = (ticks as f64 * self.attribute(crate::combat::BURNING_TIME)).ceil() as i32;
        if self.fire_ticks < scaled {
            self.fire_ticks = if self.invulnerable() { scaled.min(1) } else { scaled };
        }
    }
}
