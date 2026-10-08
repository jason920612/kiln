//! Enchantment effect components (`Enchantment.effects`, `EnchantmentEffectComponents`) and
//! the `EnchantmentHelper` aggregations over them.
//!
//! An enchantment's effects are a map of effect components. The ones decoded here:
//!
//! - value components (`ConditionalEffect<EnchantmentValueEffect>`): `damage`, `knockback`,
//!   `armor_effectiveness`, `damage_protection`, `item_damage`, `smash_damage_per_fallen_block`,
//!   `ammo_use`, `block_experience`, `mob_experience`, `repair_with_xp`, `projectile_*`,
//!   `fishing_*`, `trident_return_acceleration`, and the targeted `equipment_drops`;
//! - `damage_immunity`, `attributes`, `post_attack` (with its entity effects);
//! - unconditional values `crossbow_charge_time` and `trident_spin_attack_strength`.
//!
//! Everything else (location changed, tick, hit block, projectile spawned, sounds, the
//! prevent-* markers) is kept by name only: nothing evaluates it yet.
//!
//! Requirements are loot conditions evaluated against a context the caller builds for each
//! enchantment level (vanilla's `damageContext`, `itemContext`...): the helpers take a closure
//! `level -> context`. Iteration follows the `enchantments` component in its stored order;
//! vanilla iterates an identity hash map whose order is not stable across runs, so only the
//! order of float additions between several enchantments with the same component can differ.

use crate::condition::Condition;
use crate::context::{EntityTarget, LootContext};
use crate::data::LootData;
use crate::enchant::Enchantment;
use crate::eval::Eval;
use crate::json::Json;
use crate::number::LevelBasedValue;
use crate::parse::{PResult, Parser, Ref, fail, ident, list, obj, opt, req, value};
use kiln_item::component::{AttributeOperation, EquipmentSlot, EquipmentSlotGroup, keys};
use kiln_item::{Identifier, ItemStack, registry};
use kiln_javamath::random::RandomSource;

/// `EnchantmentValueEffect`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueEffect {
    Add(LevelBasedValue),
    Multiply(LevelBasedValue),
    RemoveBinomial(LevelBasedValue),
    Set(LevelBasedValue),
    Exponential { base: LevelBasedValue, exponent: LevelBasedValue },
    AllOf(Vec<ValueEffect>),
}

impl ValueEffect {
    pub fn parse(j: &Json) -> PResult<ValueEffect> {
        let ty = req(j, "type", ident)?;
        let lbv = |key: &str| req(j, key, LevelBasedValue::parse);
        Ok(match ty.as_str() {
            "minecraft:add" => ValueEffect::Add(lbv("value")?),
            "minecraft:multiply" => ValueEffect::Multiply(lbv("factor")?),
            "minecraft:remove_binomial" => ValueEffect::RemoveBinomial(lbv("chance")?),
            "minecraft:set" => ValueEffect::Set(lbv("value")?),
            "minecraft:exponential" => ValueEffect::Exponential { base: lbv("base")?, exponent: lbv("exponent")? },
            "minecraft:all_of" => ValueEffect::AllOf(req(j, "effects", |v| list(v, ValueEffect::parse))?),
            other => return fail(format!("unknown enchantment value effect type {other}")),
        })
    }

    /// `EnchantmentValueEffect.process(level, random, value)`.
    pub fn process(&self, level: i32, rng: &mut dyn RandomSource, value: f32) -> f32 {
        match self {
            ValueEffect::Add(v) => value + v.calculate(level),
            ValueEffect::Multiply(v) => value * v.calculate(level),
            ValueEffect::Set(v) => v.calculate(level),
            ValueEffect::Exponential { base, exponent } => {
                (value as f64 * kiln_javamath::pow::pow(base.calculate(level) as f64, exponent.calculate(level) as f64)) as f32
            }
            ValueEffect::AllOf(effects) => effects.iter().fold(value, |v, e| e.process(level, rng, v)),
            ValueEffect::RemoveBinomial(chance) => {
                let p = chance.calculate(level);
                let removed = if value <= 128.0 || value * p < 20.0 || value * (1.0 - p) < 20.0 {
                    let mut n = 0;
                    let mut i = 0;
                    while (i as f32) < value {
                        if rng.next_float() < p {
                            n += 1;
                        }
                        i += 1;
                    }
                    n
                } else {
                    // Normal approximation. Vanilla's `nextGaussian` caches the second value of
                    // each Marsaglia pair in the random source; this draws a fresh pair.
                    let mean = (value * p) as f64;
                    let sd = ((value * p * (1.0 - p)) as f64).sqrt();
                    let n = (mean.floor() + gaussian(rng) * sd).round() as i64;
                    n.clamp(0, value as i32 as i64) as i32
                };
                value - removed as f32
            }
        }
    }
}

/// `MarsagliaPolarGaussian.nextGaussian` without the cached second value.
fn gaussian(rng: &mut dyn RandomSource) -> f64 {
    loop {
        let a = 2.0 * rng.next_double() - 1.0;
        let b = 2.0 * rng.next_double() - 1.0;
        let s = a * a + b * b;
        if s < 1.0 && s != 0.0 {
            return a * (-2.0 * kiln_javamath::pow::log(s) / s).sqrt();
        }
    }
}

/// `EnchantmentTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Attacker,
    DamagingEntity,
    Victim,
}

impl Target {
    fn parse(j: &Json) -> PResult<Target> {
        match crate::parse::string(j)?.as_str() {
            "attacker" => Ok(Target::Attacker),
            "damaging_entity" => Ok(Target::DamagingEntity),
            "victim" => Ok(Target::Victim),
            other => fail(format!("unknown enchantment target {other}")),
        }
    }
}

/// `EnchantmentEntityEffect` (the kinds vanilla's melee enchantments use are decoded; others
/// are kept by type name).
#[derive(Debug, Clone)]
pub enum EntityEffect {
    AllOf(Vec<EntityEffect>),
    /// `apply_mob_effect`: `to_apply` mob effect ids.
    ApplyMobEffect {
        to_apply: Vec<Identifier>,
        min_duration: LevelBasedValue,
        max_duration: LevelBasedValue,
        min_amplifier: LevelBasedValue,
        max_amplifier: LevelBasedValue,
    },
    /// `change_item_damage`: the enchanted item loses `amount` durability.
    ChangeItemDamage(LevelBasedValue),
    /// `damage_entity`: `damage_type` (a `minecraft:damage_type` name) between the bounds.
    DamageEntity { damage_type: Identifier, min_damage: LevelBasedValue, max_damage: LevelBasedValue },
    /// `ignite`: seconds of fire.
    Ignite(LevelBasedValue),
    /// `apply_exhaustion`: a player's food exhaustion.
    ApplyExhaustion(LevelBasedValue),
    /// `apply_impulse`: the entity's motion changes by `direction` turned along its look, scaled
    /// by `coordinate_scale` and `magnitude` (the lunge enchantment).
    ApplyImpulse { direction: [f64; 3], coordinate_scale: [f64; 3], magnitude: LevelBasedValue },
    /// `play_sound`: one of `sounds` (by the enchantment's level), heard by everyone.
    PlaySound { sounds: Vec<Identifier>, volume: crate::provider::Floats, pitch: crate::provider::Floats },
    /// `explode` (the mace's wind burst): an explosion at the enchanted entity.
    Explode {
        /// Whether the enchanted entity is the explosion's source (`attribute_to_user`).
        attribute_to_user: bool,
        /// A damage type makes the explosion hurt entities; none: knockback only.
        has_damage_type: bool,
        knockback_multiplier: Option<LevelBasedValue>,
        /// The `immune_blocks` block tag (`#minecraft:blocks_wind_charge_explosions`).
        immune_blocks: Option<String>,
        offset: [f64; 3],
        radius: LevelBasedValue,
        create_fire: bool,
        /// `block_interaction`: `none`, `block`, `mob`, `tnt`, `trigger`...
        interaction: String,
        small_particle: String,
        large_particle: String,
        sound: Identifier,
    },
    Other(Identifier),
}

impl EntityEffect {
    pub fn parse(j: &Json) -> PResult<EntityEffect> {
        let ty = req(j, "type", ident)?;
        let lbv = |key: &str| req(j, key, LevelBasedValue::parse);
        Ok(match ty.as_str() {
            "minecraft:all_of" => EntityEffect::AllOf(req(j, "effects", |v| list(v, EntityEffect::parse))?),
            "minecraft:apply_mob_effect" => EntityEffect::ApplyMobEffect {
                to_apply: req(j, "to_apply", |v| match v {
                    Json::Arr(_) => list(v, ident),
                    _ => Ok(vec![ident(v)?]),
                })?,
                min_duration: lbv("min_duration")?,
                max_duration: lbv("max_duration")?,
                min_amplifier: lbv("min_amplifier")?,
                max_amplifier: lbv("max_amplifier")?,
            },
            "minecraft:change_item_damage" => EntityEffect::ChangeItemDamage(lbv("amount")?),
            "minecraft:damage_entity" => EntityEffect::DamageEntity {
                damage_type: req(j, "damage_type", ident)?,
                min_damage: lbv("min_damage")?,
                max_damage: lbv("max_damage")?,
            },
            "minecraft:ignite" => EntityEffect::Ignite(lbv("duration")?),
            "minecraft:apply_exhaustion" => EntityEffect::ApplyExhaustion(lbv("amount")?),
            "minecraft:apply_impulse" => {
                let vec3 = |key: &str| -> PResult<[f64; 3]> {
                    let v = req(j, key, |v| list(v, crate::parse::float))?;
                    match v[..] {
                        [x, y, z] => Ok([x as f64, y as f64, z as f64]),
                        _ => fail(format!("{key} needs three numbers")),
                    }
                };
                EntityEffect::ApplyImpulse { direction: vec3("direction")?, coordinate_scale: vec3("coordinate_scale")?, magnitude: lbv("magnitude")? }
            }
            "minecraft:play_sound" => EntityEffect::PlaySound {
                sounds: req(j, "sound", |v| match v {
                    Json::Arr(_) => list(v, ident),
                    _ => Ok(vec![ident(v)?]),
                })?,
                volume: opt(j, "volume", crate::provider::Floats::parse)?.unwrap_or(crate::provider::Floats::Constant(1.0)),
                pitch: opt(j, "pitch", crate::provider::Floats::parse)?.unwrap_or(crate::provider::Floats::Constant(1.0)),
            },
            "minecraft:explode" => {
                let particle = |key: &str| -> PResult<String> { req(j, key, |v| req(v, "type", |t| ident(t).map(|i| i.as_str().to_owned()))) };
                EntityEffect::Explode {
                    attribute_to_user: opt(j, "attribute_to_user", crate::parse::boolean)?.unwrap_or(false),
                    has_damage_type: j.get("damage_type").is_some(),
                    knockback_multiplier: opt(j, "knockback_multiplier", LevelBasedValue::parse)?,
                    immune_blocks: opt(j, "immune_blocks", crate::parse::string)?,
                    offset: match opt(j, "offset", |v| list(v, crate::parse::float))? {
                        Some(v) if v.len() == 3 => [v[0] as f64, v[1] as f64, v[2] as f64],
                        _ => [0.0; 3],
                    },
                    radius: lbv("radius")?,
                    create_fire: opt(j, "create_fire", crate::parse::boolean)?.unwrap_or(false),
                    interaction: opt(j, "block_interaction", crate::parse::string)?.unwrap_or_else(|| "none".to_owned()),
                    small_particle: particle("small_particle")?,
                    large_particle: particle("large_particle")?,
                    sound: req(j, "sound", ident)?,
                }
            }
            _ => EntityEffect::Other(ty),
        })
    }
}

/// `ConditionalEffect`: an effect and the loot condition that must hold.
#[derive(Debug, Clone)]
pub struct Conditional<T> {
    pub effect: T,
    pub requirements: Option<Ref<Condition>>,
}

impl<T> Conditional<T> {
    fn parse(p: &Parser, j: &Json, effect: impl FnOnce(&Json) -> PResult<T>) -> PResult<Conditional<T>> {
        Ok(Conditional { effect: req(j, "effect", effect)?, requirements: opt(j, "requirements", |v| Condition::parse_ref(p, v))? })
    }

    /// `ConditionalEffect.matches`.
    pub fn matches(&self, data: &LootData, ctx: &dyn LootContext, rng: &mut dyn RandomSource) -> bool {
        match &self.requirements {
            None => true,
            Some(c) => Eval::new(data, ctx, rng).test(c),
        }
    }
}

/// `TargetedConditionalEffect`.
#[derive(Debug, Clone)]
pub struct Targeted<T> {
    pub enchanted: Target,
    pub affected: Target,
    pub effect: T,
    pub requirements: Option<Ref<Condition>>,
}

impl<T> Targeted<T> {
    fn parse(p: &Parser, j: &Json, effect: impl FnOnce(&Json) -> PResult<T>) -> PResult<Targeted<T>> {
        let enchanted = req(j, "enchanted", Target::parse)?;
        Ok(Targeted {
            enchanted,
            affected: opt(j, "affected", Target::parse)?.unwrap_or(enchanted),
            effect: req(j, "effect", effect)?,
            requirements: opt(j, "requirements", |v| Condition::parse_ref(p, v))?,
        })
    }

    pub fn matches(&self, data: &LootData, ctx: &dyn LootContext, rng: &mut dyn RandomSource) -> bool {
        match &self.requirements {
            None => true,
            Some(c) => Eval::new(data, ctx, rng).test(c),
        }
    }
}

/// `EnchantmentAttributeEffect`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeEffect {
    pub id: Identifier,
    /// `minecraft:attribute` network id.
    pub attribute: i32,
    pub amount: LevelBasedValue,
    pub operation: AttributeOperation,
}

impl AttributeEffect {
    fn parse(p: &Parser, j: &Json) -> PResult<AttributeEffect> {
        Ok(AttributeEffect {
            id: req(j, "id", ident)?,
            attribute: req(j, "attribute", |v| p.id(v, registry::ATTRIBUTE))?,
            amount: req(j, "amount", LevelBasedValue::parse)?,
            operation: req(j, "operation", |v| value(v, AttributeOperation::from_value))?,
        })
    }

    /// `getModifier(level, slot)`: the id gets `/<slot>` appended, the amount is the float
    /// value widened.
    pub fn modifier(&self, level: i32, slot: &str) -> (String, f64) {
        (format!("{}/{slot}", self.id.as_str()), self.amount.calculate(level) as f64)
    }
}

/// The value effect components (`ConditionalEffect<EnchantmentValueEffect>` lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueComponent {
    DamageProtection,
    ItemDamage,
    Damage,
    SmashDamagePerFallenBlock,
    Knockback,
    ArmorEffectiveness,
    AmmoUse,
    ProjectilePiercing,
    ProjectileSpread,
    ProjectileCount,
    TridentReturnAcceleration,
    FishingTimeReduction,
    FishingLuckBonus,
    BlockExperience,
    MobExperience,
    RepairWithXp,
}

impl ValueComponent {
    pub const ALL: [ValueComponent; 16] = [
        ValueComponent::DamageProtection,
        ValueComponent::ItemDamage,
        ValueComponent::Damage,
        ValueComponent::SmashDamagePerFallenBlock,
        ValueComponent::Knockback,
        ValueComponent::ArmorEffectiveness,
        ValueComponent::AmmoUse,
        ValueComponent::ProjectilePiercing,
        ValueComponent::ProjectileSpread,
        ValueComponent::ProjectileCount,
        ValueComponent::TridentReturnAcceleration,
        ValueComponent::FishingTimeReduction,
        ValueComponent::FishingLuckBonus,
        ValueComponent::BlockExperience,
        ValueComponent::MobExperience,
        ValueComponent::RepairWithXp,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ValueComponent::DamageProtection => "minecraft:damage_protection",
            ValueComponent::ItemDamage => "minecraft:item_damage",
            ValueComponent::Damage => "minecraft:damage",
            ValueComponent::SmashDamagePerFallenBlock => "minecraft:smash_damage_per_fallen_block",
            ValueComponent::Knockback => "minecraft:knockback",
            ValueComponent::ArmorEffectiveness => "minecraft:armor_effectiveness",
            ValueComponent::AmmoUse => "minecraft:ammo_use",
            ValueComponent::ProjectilePiercing => "minecraft:projectile_piercing",
            ValueComponent::ProjectileSpread => "minecraft:projectile_spread",
            ValueComponent::ProjectileCount => "minecraft:projectile_count",
            ValueComponent::TridentReturnAcceleration => "minecraft:trident_return_acceleration",
            ValueComponent::FishingTimeReduction => "minecraft:fishing_time_reduction",
            ValueComponent::FishingLuckBonus => "minecraft:fishing_luck_bonus",
            ValueComponent::BlockExperience => "minecraft:block_experience",
            ValueComponent::MobExperience => "minecraft:mob_experience",
            ValueComponent::RepairWithXp => "minecraft:repair_with_xp",
        }
    }

    fn by_name(name: &str) -> Option<ValueComponent> {
        ValueComponent::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// An enchantment's decoded effect components.
#[derive(Debug, Clone, Default)]
pub struct Effects {
    pub values: Vec<(ValueComponent, Vec<Conditional<ValueEffect>>)>,
    pub damage_immunity: Vec<Conditional<()>>,
    pub attributes: Vec<AttributeEffect>,
    pub post_attack: Vec<Targeted<EntityEffect>>,
    /// `post_piercing_attack`: what a spear's stab does to its wielder (lunge).
    pub post_piercing_attack: Vec<Conditional<EntityEffect>>,
    /// `equipment_drops`: targeted value effects (`enchanted` only).
    pub equipment_drops: Vec<Targeted<ValueEffect>>,
    pub crossbow_charge_time: Option<ValueEffect>,
    pub trident_spin_attack_strength: Option<ValueEffect>,
    /// Components kept by name only.
    pub other: Vec<Identifier>,
}

impl Effects {
    /// `DataComponentMap` of `EnchantmentEffectComponents`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<Effects> {
        let mut out = Effects::default();
        for (key, v) in obj(j)? {
            let id = Identifier::parse(key).ok_or_else(|| crate::parse::ParseError::new(format!("invalid key {key:?}")))?;
            let at = |e: crate::parse::ParseError| e.at(key);
            if let Some(c) = ValueComponent::by_name(id.as_str()) {
                let list = list(v, |e| Conditional::parse(p, e, ValueEffect::parse)).map_err(at)?;
                out.values.push((c, list));
                continue;
            }
            match id.as_str() {
                "minecraft:damage_immunity" => {
                    out.damage_immunity = list(v, |e| Conditional::parse(p, e, |_| Ok(()))).map_err(at)?;
                }
                "minecraft:attributes" => out.attributes = list(v, |e| AttributeEffect::parse(p, e)).map_err(at)?,
                "minecraft:post_attack" => {
                    out.post_attack = list(v, |e| Targeted::parse(p, e, EntityEffect::parse)).map_err(at)?;
                }
                "minecraft:post_piercing_attack" => {
                    out.post_piercing_attack = list(v, |e| Conditional::parse(p, e, EntityEffect::parse)).map_err(at)?;
                }
                "minecraft:equipment_drops" => {
                    out.equipment_drops = list(v, |e| Targeted::parse(p, e, ValueEffect::parse)).map_err(at)?;
                }
                "minecraft:crossbow_charge_time" => out.crossbow_charge_time = Some(ValueEffect::parse(v).map_err(at)?),
                "minecraft:trident_spin_attack_strength" => {
                    out.trident_spin_attack_strength = Some(ValueEffect::parse(v).map_err(at)?);
                }
                _ => out.other.push(id),
            }
        }
        Ok(out)
    }

    /// `getEffects(component)` for a value component.
    pub fn value(&self, c: ValueComponent) -> &[Conditional<ValueEffect>] {
        self.values.iter().find(|(k, _)| *k == c).map_or(&[], |(_, l)| l.as_slice())
    }
}

// ---- EnchantmentHelper ------------------------------------------------------------------------

/// `EquipmentSlotGroup.test`.
pub fn slot_group_has(group: EquipmentSlotGroup, slot: EquipmentSlot) -> bool {
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

impl Enchantment {
    /// `matchingSlot`.
    pub fn matching_slot(&self, slot: EquipmentSlot) -> bool {
        self.slots.iter().any(|g| slot_group_has(*g, slot))
    }
}

/// `EquipmentSlot.VALUES` order, which `runIterationOnEquipment` walks.
pub const EQUIPMENT_ORDER: [EquipmentSlot; 8] = [
    EquipmentSlot::MainHand,
    EquipmentSlot::OffHand,
    EquipmentSlot::Feet,
    EquipmentSlot::Legs,
    EquipmentSlot::Chest,
    EquipmentSlot::Head,
    EquipmentSlot::Body,
    EquipmentSlot::Saddle,
];

impl LootData {
    /// `runIterationOnItem(stack, visitor)`: each enchantment of the `enchantments` component
    /// (definitions that failed to load are skipped).
    pub fn for_each_enchantment<'a>(&'a self, stack: &ItemStack, mut f: impl FnMut(&'a Enchantment, i32)) {
        let Some(list) = stack.get(keys::ENCHANTMENTS) else { return };
        for &(id, level) in &list.0 {
            if let Some(e) = self.enchantment(id) {
                f(e, level);
            }
        }
    }

    /// `runIterationOnItem(stack, slot, owner, visitor)`: a non-empty stack's enchantments that
    /// apply in `slot`.
    pub fn for_each_enchantment_in_slot<'a>(&'a self, stack: &ItemStack, slot: EquipmentSlot, mut f: impl FnMut(&'a Enchantment, i32)) {
        if stack.is_empty() {
            return;
        }
        self.for_each_enchantment(stack, |e, level| {
            if e.matching_slot(slot) {
                f(e, level);
            }
        });
    }

    /// `runIterationOnEquipment`: `equipment` holds the entity's item per slot (any order; walked
    /// in [`EQUIPMENT_ORDER`]).
    pub fn for_each_equipped_enchantment<'a, 's>(
        &'a self,
        equipment: &[(EquipmentSlot, &'s ItemStack)],
        mut f: impl FnMut(&'a Enchantment, i32, EquipmentSlot, &'s ItemStack),
    ) {
        for slot in EQUIPMENT_ORDER {
            for &(s, stack) in equipment.iter().filter(|(s, _)| *s == slot) {
                self.for_each_enchantment_in_slot(stack, s, |e, level| f(e, level, s, stack));
            }
        }
    }

    /// Runs one enchantment's value effects of component `c` on `value`, testing requirements in
    /// the context built for `level`.
    pub fn apply_value_effects<C: LootContext>(
        &self,
        e: &Enchantment,
        c: ValueComponent,
        level: i32,
        ctx: &C,
        rng: &mut dyn RandomSource,
        value: f32,
    ) -> f32 {
        let mut value = value;
        for effect in e.effects.value(c) {
            if effect.matches(self, ctx, rng) {
                value = effect.effect.process(level, rng, value);
            }
        }
        value
    }

    /// The `modifyDamage`-style helpers: the weapon's enchantments' value effects of `c`, each
    /// with its own context (`damageContext(level, target, source)`).
    pub fn modify_item_value<C: LootContext>(
        &self,
        stack: &ItemStack,
        c: ValueComponent,
        rng: &mut dyn RandomSource,
        value: f32,
        ctx: impl Fn(i32) -> C,
    ) -> f32 {
        let mut value = value;
        self.for_each_enchantment(stack, |e, level| {
            value = self.apply_value_effects(e, c, level, &ctx(level), rng, value);
        });
        value
    }

    /// `EnchantmentHelper.modifyDamage`.
    pub fn modify_damage<C: LootContext>(&self, weapon: &ItemStack, rng: &mut dyn RandomSource, damage: f32, ctx: impl Fn(i32) -> C) -> f32 {
        self.modify_item_value(weapon, ValueComponent::Damage, rng, damage, ctx)
    }

    /// `EnchantmentHelper.modifyKnockback`.
    pub fn modify_knockback<C: LootContext>(&self, weapon: &ItemStack, rng: &mut dyn RandomSource, value: f32, ctx: impl Fn(i32) -> C) -> f32 {
        self.modify_item_value(weapon, ValueComponent::Knockback, rng, value, ctx)
    }

    /// `EnchantmentHelper.modifyArmorEffectiveness`.
    pub fn modify_armor_effectiveness<C: LootContext>(
        &self,
        weapon: &ItemStack,
        rng: &mut dyn RandomSource,
        value: f32,
        ctx: impl Fn(i32) -> C,
    ) -> f32 {
        self.modify_item_value(weapon, ValueComponent::ArmorEffectiveness, rng, value, ctx)
    }

    /// `EnchantmentHelper.modifyFallBasedDamage` (`smash_damage_per_fallen_block`).
    pub fn modify_fall_based_damage<C: LootContext>(
        &self,
        weapon: &ItemStack,
        rng: &mut dyn RandomSource,
        value: f32,
        ctx: impl Fn(i32) -> C,
    ) -> f32 {
        self.modify_item_value(weapon, ValueComponent::SmashDamagePerFallenBlock, rng, value, ctx)
    }

    /// `EnchantmentHelper.getDamageProtection`: the equipment's `damage_protection` from 0,
    /// each enchantment with the victim's `damageContext`.
    pub fn damage_protection<C: LootContext>(
        &self,
        equipment: &[(EquipmentSlot, &ItemStack)],
        rng: &mut dyn RandomSource,
        ctx: impl Fn(i32) -> C,
    ) -> f32 {
        let mut value = 0.0;
        self.for_each_equipped_enchantment(equipment, |e, level, _, _| {
            value = self.apply_value_effects(e, ValueComponent::DamageProtection, level, &ctx(level), rng, value);
        });
        value
    }

    /// `EnchantmentHelper.isImmuneToDamage`.
    pub fn is_immune_to_damage<C: LootContext>(
        &self,
        equipment: &[(EquipmentSlot, &ItemStack)],
        rng: &mut dyn RandomSource,
        ctx: impl Fn(i32) -> C,
    ) -> bool {
        let mut immune = false;
        self.for_each_equipped_enchantment(equipment, |e, level, _, _| {
            if immune || e.effects.damage_immunity.is_empty() {
                return;
            }
            let c = ctx(level);
            immune = e.effects.damage_immunity.iter().any(|d| d.matches(self, &c, rng));
        });
        immune
    }

    /// `EnchantmentHelper.processDurabilityChange`: `item_damage` effects on `amount`, each
    /// enchantment in `itemContext(level, stack)`; the result truncates toward zero.
    pub fn process_durability_change(&self, stack: &ItemStack, rng: &mut dyn RandomSource, amount: i32) -> i32 {
        let mut value = amount as f32;
        self.for_each_enchantment(stack, |e, level| {
            let ctx = ItemContext { tool: stack, level };
            value = self.apply_value_effects(e, ValueComponent::ItemDamage, level, &ctx, rng, value);
        });
        value as i32
    }

    /// `EnchantmentHelper.getRandomItemWith(component, entity, filter)`'s candidates: every
    /// (slot, stack) of `equipment` in [`EQUIPMENT_ORDER`] that passes `filter`, once per
    /// enchantment on it that has component `c` and applies in that slot. The caller picks one
    /// with `nextInt(len)` of the entity's random.
    pub fn items_with<'s>(
        &self,
        c: ValueComponent,
        equipment: &[(EquipmentSlot, &'s ItemStack)],
        filter: impl Fn(&ItemStack) -> bool,
    ) -> Vec<(EquipmentSlot, &'s ItemStack)> {
        let mut out = Vec::new();
        for slot in EQUIPMENT_ORDER {
            for &(s, stack) in equipment.iter().filter(|(s, _)| *s == slot) {
                if !filter(stack) {
                    continue;
                }
                self.for_each_enchantment(stack, |e, _| {
                    if e.effects.values.iter().any(|(k, _)| *k == c) && e.matching_slot(s) {
                        out.push((s, stack));
                    }
                });
            }
        }
        out
    }

    /// `EnchantmentHelper.modifyDurabilityToRepairFromXp`: `repair_with_xp` effects on
    /// `amount` in `itemContext(level, stack)`, truncated, at least 0.
    pub fn durability_from_xp(&self, stack: &ItemStack, rng: &mut dyn RandomSource, amount: i32) -> i32 {
        let mut value = amount as f32;
        self.for_each_enchantment(stack, |e, level| {
            let ctx = ItemContext { tool: stack, level };
            value = self.apply_value_effects(e, ValueComponent::RepairWithXp, level, &ctx, rng, value);
        });
        (value as i32).max(0)
    }

    /// `EnchantmentHelper.forEachModifier(stack, slot, ...)`: attribute modifiers of the
    /// enchantments that apply in `slot`, as (attribute id, modifier id, amount, operation).
    pub fn enchantment_modifiers(&self, stack: &ItemStack, slot: EquipmentSlot, mut f: impl FnMut(i32, String, f64, AttributeOperation)) {
        self.for_each_enchantment(stack, |e, level| {
            for a in &e.effects.attributes {
                if e.matching_slot(slot) {
                    let (id, amount) = a.modifier(level, slot.name());
                    f(a.attribute, id, amount, a.operation);
                }
            }
        });
    }

    /// `Enchantment.doPostAttack` over one item: the `post_attack` effects whose `enchanted`
    /// is `target` and whose requirements hold in `damageContext(level, victim, source)`.
    pub fn post_attack_effects<'a, C: LootContext>(
        &'a self,
        stack: &ItemStack,
        slot: EquipmentSlot,
        target: Target,
        rng: &mut dyn RandomSource,
        ctx: impl Fn(i32) -> C,
        mut f: impl FnMut(&'a Targeted<EntityEffect>, i32),
    ) {
        self.for_each_enchantment_in_slot(stack, slot, |e, level| {
            for t in e.effects.post_attack.iter().filter(|t| t.enchanted == target) {
                if t.matches(self, &ctx(level), rng) {
                    f(t, level);
                }
            }
        });
    }
}

impl LootData {
    /// `EnchantmentHelper.doPostPiercingAttackEffects` over the weapon in `slot`: each enchantment's
    /// `post_piercing_attack` effects whose requirements hold in `entityContext(level, wielder)`,
    /// with the enchantment's level, in the stored enchantment order.
    pub fn post_piercing_effects<'a, C: LootContext>(
        &'a self,
        stack: &ItemStack,
        slot: EquipmentSlot,
        rng: &mut dyn RandomSource,
        ctx: impl Fn(i32) -> C,
        mut f: impl FnMut(&'a EntityEffect, i32),
    ) {
        self.for_each_enchantment_in_slot(stack, slot, |e, level| {
            for c in &e.effects.post_piercing_attack {
                if c.matches(self, &ctx(level), rng) {
                    f(&c.effect, level);
                }
            }
        });
    }
}

/// `Enchantment.itemContext`: the `tool` and `enchantment_level` parameters.
pub struct ItemContext<'a> {
    pub tool: &'a ItemStack,
    pub level: i32,
}

impl LootContext for ItemContext<'_> {
    fn tool(&self) -> Option<&ItemStack> {
        Some(self.tool)
    }

    fn enchantment_level(&self) -> Option<i32> {
        Some(self.level)
    }
}

/// `EntityTarget`s a damage context provides (`LootContextParamSets.ENCHANTED_DAMAGE`).
pub const DAMAGE_CONTEXT_TARGETS: [EntityTarget; 3] = [EntityTarget::This, EntityTarget::Attacker, EntityTarget::DirectAttacker];

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_javamath::random::LegacyRandom;

    #[test]
    fn value_effects() {
        let mut rng = LegacyRandom::new(0);
        let add = ValueEffect::Add(LevelBasedValue::Linear { base: 1.0, per_level_above_first: 0.5 });
        assert_eq!(add.process(5, &mut rng, 7.0), 10.0);
        let all = ValueEffect::AllOf(vec![add.clone(), ValueEffect::Multiply(LevelBasedValue::Constant(2.0))]);
        assert_eq!(all.process(1, &mut rng, 1.0), 4.0);
        // remove_binomial with chance 1 removes everything, 0 nothing.
        let one = ValueEffect::RemoveBinomial(LevelBasedValue::Constant(1.0));
        assert_eq!(one.process(1, &mut rng, 3.0), 0.0);
        let zero = ValueEffect::RemoveBinomial(LevelBasedValue::Constant(0.0));
        assert_eq!(zero.process(1, &mut rng, 3.0), 3.0);
    }

    #[test]
    fn remove_binomial_draws_one_float_per_point() {
        // Unbreaking III on a tool: chance 3/4 each; `nextFloat() < 0.75` removes a point.
        let chance = LevelBasedValue::Fraction {
            numerator: Box::new(LevelBasedValue::Linear { base: 1.0, per_level_above_first: 1.0 }),
            denominator: Box::new(LevelBasedValue::Linear { base: 2.0, per_level_above_first: 1.0 }),
        };
        let e = ValueEffect::RemoveBinomial(chance);
        let mut a = LegacyRandom::new(42);
        let mut b = LegacyRandom::new(42);
        let expected = 2.0 - [b.next_float(), b.next_float()].iter().filter(|f| **f < 0.75).count() as f32;
        assert_eq!(e.process(3, &mut a, 2.0), expected);
    }
}
