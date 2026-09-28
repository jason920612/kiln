//! `AttributeMap` for mobs: the type's defaults (`DefaultAttributes`) and modifiers.

/// The attributes mobs read (`Attributes`), with vanilla's default value and range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attr {
    MaxHealth,
    KnockbackResistance,
    MovementSpeed,
    Armor,
    ArmorToughness,
    MaxAbsorption,
    StepHeight,
    Scale,
    Gravity,
    SafeFallDistance,
    FallDamageMultiplier,
    JumpStrength,
    OxygenBonus,
    BurningTime,
    ExplosionKnockbackResistance,
    WaterMovementEfficiency,
    MovementEfficiency,
    AttackKnockback,
    AirDragModifier,
    FrictionModifier,
    FollowRange,
    AttackDamage,
    TemptRange,
    SpawnReinforcements,
}

impl Attr {
    /// (registry name, default, min, max).
    pub fn info(self) -> (&'static str, f64, f64, f64) {
        use Attr::*;
        match self {
            MaxHealth => ("minecraft:max_health", 20.0, 1.0, 1024.0),
            KnockbackResistance => ("minecraft:knockback_resistance", 0.0, 0.0, 1.0),
            MovementSpeed => ("minecraft:movement_speed", 0.7, 0.0, 1024.0),
            Armor => ("minecraft:armor", 0.0, 0.0, 30.0),
            ArmorToughness => ("minecraft:armor_toughness", 0.0, 0.0, 20.0),
            MaxAbsorption => ("minecraft:max_absorption", 0.0, 0.0, 2048.0),
            StepHeight => ("minecraft:step_height", 0.6, 0.0, 10.0),
            Scale => ("minecraft:scale", 1.0, 0.0625, 16.0),
            Gravity => ("minecraft:gravity", 0.08, -1.0, 1.0),
            SafeFallDistance => ("minecraft:safe_fall_distance", 3.0, -1024.0, 1024.0),
            FallDamageMultiplier => ("minecraft:fall_damage_multiplier", 1.0, 0.0, 100.0),
            JumpStrength => ("minecraft:jump_strength", 0.41999998688697815, 0.0, 32.0),
            OxygenBonus => ("minecraft:oxygen_bonus", 0.0, 0.0, 1024.0),
            BurningTime => ("minecraft:burning_time", 1.0, 0.0, 1024.0),
            ExplosionKnockbackResistance => ("minecraft:explosion_knockback_resistance", 0.0, 0.0, 1.0),
            WaterMovementEfficiency => ("minecraft:water_movement_efficiency", 0.0, 0.0, 1.0),
            MovementEfficiency => ("minecraft:movement_efficiency", 0.0, 0.0, 1.0),
            AttackKnockback => ("minecraft:attack_knockback", 0.0, 0.0, 5.0),
            AirDragModifier => ("minecraft:air_drag_modifier", 1.0, 0.0, 2048.0),
            FrictionModifier => ("minecraft:friction_modifier", 1.0, 0.0, 2048.0),
            FollowRange => ("minecraft:follow_range", 32.0, 0.0, 2048.0),
            AttackDamage => ("minecraft:attack_damage", 2.0, 0.0, 2048.0),
            TemptRange => ("minecraft:tempt_range", 10.0, 0.0, 2048.0),
            SpawnReinforcements => ("minecraft:spawn_reinforcements", 0.0, 0.0, 1.0),
        }
    }

    pub fn name(self) -> &'static str {
        self.info().0
    }

    pub fn by_name(name: &str) -> Option<Attr> {
        ALL.iter().copied().find(|a| a.name() == name || a.name().strip_prefix("minecraft:") == Some(name))
    }
}

pub const ALL: [Attr; 24] = {
    use Attr::*;
    [
        MaxHealth,
        KnockbackResistance,
        MovementSpeed,
        Armor,
        ArmorToughness,
        MaxAbsorption,
        StepHeight,
        Scale,
        Gravity,
        SafeFallDistance,
        FallDamageMultiplier,
        JumpStrength,
        OxygenBonus,
        BurningTime,
        ExplosionKnockbackResistance,
        WaterMovementEfficiency,
        MovementEfficiency,
        AttackKnockback,
        AirDragModifier,
        FrictionModifier,
        FollowRange,
        AttackDamage,
        TemptRange,
        SpawnReinforcements,
    ]
};

/// `AttributeModifier.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub id: String,
    pub amount: f64,
    pub op: Op,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub attr: Attr,
    pub base: f64,
    pub modifiers: Vec<Modifier>,
}

impl Instance {
    /// `AttributeInstance.calculateValue`.
    pub fn value(&self) -> f64 {
        let (_, _, min, max) = self.attr.info();
        let mut base = self.base;
        for m in self.modifiers.iter().filter(|m| m.op == Op::AddValue) {
            base += m.amount;
        }
        let mut d = base;
        for m in self.modifiers.iter().filter(|m| m.op == Op::AddMultipliedBase) {
            d += base * m.amount;
        }
        for m in self.modifiers.iter().filter(|m| m.op == Op::AddMultipliedTotal) {
            d *= 1.0 + m.amount;
        }
        if d.is_nan() { min } else { d.clamp(min, max) }
    }

    pub fn has_modifier(&self, id: &str) -> bool {
        self.modifiers.iter().any(|m| m.id == id)
    }
}

/// A mob's attributes: the ones its type supports, in `Attr` order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub list: Vec<Instance>,
}

impl Attributes {
    /// Defaults of the living attributes (`LivingEntity.createLivingAttributes`), then `extra`.
    pub fn new(extra: &[(Attr, f64)]) -> Attributes {
        use Attr::*;
        let mut list: Vec<Instance> = [
            MaxHealth,
            KnockbackResistance,
            MovementSpeed,
            Armor,
            ArmorToughness,
            MaxAbsorption,
            StepHeight,
            Scale,
            Gravity,
            SafeFallDistance,
            FallDamageMultiplier,
            JumpStrength,
            OxygenBonus,
            BurningTime,
            ExplosionKnockbackResistance,
            WaterMovementEfficiency,
            MovementEfficiency,
            AttackKnockback,
            AirDragModifier,
            FrictionModifier,
        ]
        .into_iter()
        .map(|a| Instance { attr: a, base: a.info().1, modifiers: Vec::new() })
        .collect();
        for &(a, v) in extra {
            match list.iter_mut().find(|i| i.attr == a) {
                Some(i) => i.base = v,
                None => list.push(Instance { attr: a, base: v, modifiers: Vec::new() }),
            }
        }
        list.sort_by_key(|i| i.attr);
        Attributes { list }
    }

    pub fn get(&self, a: Attr) -> Option<&Instance> {
        self.list.iter().find(|i| i.attr == a)
    }

    pub fn get_mut(&mut self, a: Attr) -> Option<&mut Instance> {
        self.list.iter_mut().find(|i| i.attr == a)
    }

    /// `getAttributeValue` (0 for an attribute the mob does not have, as vanilla's map
    /// throws; callers only ask for supported ones).
    pub fn value(&self, a: Attr) -> f64 {
        self.get(a).map_or(0.0, Instance::value)
    }

    pub fn base(&self, a: Attr) -> f64 {
        self.get(a).map_or(0.0, |i| i.base)
    }

    /// `addOrReplacePermanentModifier` / `addTransientModifier`.
    pub fn set_modifier(&mut self, a: Attr, id: &str, amount: f64, op: Op) {
        if let Some(i) = self.get_mut(a) {
            i.modifiers.retain(|m| m.id != id);
            i.modifiers.push(Modifier { id: id.to_owned(), amount, op });
        }
    }

    pub fn remove_modifier(&mut self, a: Attr, id: &str) {
        if let Some(i) = self.get_mut(a) {
            i.modifiers.retain(|m| m.id != id);
        }
    }
}
