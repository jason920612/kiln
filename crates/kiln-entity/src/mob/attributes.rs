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
    Bounciness,
    FollowRange,
    AttackDamage,
    TemptRange,
    SpawnReinforcements,
    FlyingSpeed,
    CameraDistance,
}

impl Attr {
    /// (registry name, default, min, max).
    pub fn info(self) -> (&'static str, f64, f64, f64) {
        use Attr::*;
        match self {
            MaxHealth => ("minecraft:max_health", 20.0, 1.0, 1024.0),
            KnockbackResistance => ("minecraft:knockback_resistance", 0.0, -2.0, 1.0),
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
            Bounciness => ("minecraft:bounciness", 0.0, 0.0, 1.0),
            FollowRange => ("minecraft:follow_range", 32.0, 0.0, 2048.0),
            AttackDamage => ("minecraft:attack_damage", 2.0, 0.0, 2048.0),
            TemptRange => ("minecraft:tempt_range", 10.0, 0.0, 2048.0),
            SpawnReinforcements => ("minecraft:spawn_reinforcements", 0.0, 0.0, 1.0),
            FlyingSpeed => ("minecraft:flying_speed", 0.4, 0.0, 1024.0),
            CameraDistance => ("minecraft:camera_distance", 4.0, 0.0, 32.0),
        }
    }

    pub fn name(self) -> &'static str {
        self.info().0
    }

    /// `Attribute.isClientSyncable`: whether clients are told about the attribute.
    pub fn is_client_syncable(self) -> bool {
        syncable_name(self.name())
    }

    pub fn by_name(name: &str) -> Option<Attr> {
        ALL.iter().copied().find(|a| a.name() == name || a.name().strip_prefix("minecraft:") == Some(name))
    }
}

/// `Attribute.isClientSyncable` by registry name: every attribute except the ones only the
/// server reads (knockback resistance, attack damage and knockback, the AI ranges, the
/// reinforcement chance, the waypoint ranges).
pub fn syncable_name(name: &str) -> bool {
    !matches!(
        name,
        "minecraft:knockback_resistance"
            | "minecraft:attack_knockback"
            | "minecraft:attack_damage"
            | "minecraft:follow_range"
            | "minecraft:tempt_range"
            | "minecraft:spawn_reinforcements"
            | "minecraft:waypoint_transmit_range"
            | "minecraft:waypoint_receive_range"
    )
}

pub const ALL: [Attr; 27] = {
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
        Bounciness,
        FollowRange,
        AttackDamage,
        TemptRange,
        SpawnReinforcements,
        FlyingSpeed,
        CameraDistance,
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
    /// Whether the attribute map has an instance of it (`AttributeMap.getInstance` makes one the first time something asks to change
    /// it; reading a value does not). Only those are in `getSyncableAttributes`, so only those are told to the clients.
    pub created: bool,
    /// The base value the instance was made with (the type's own; a change from it is something to tell the viewers).
    pub initial: f64,
    /// Made by the mob's first step (not by its constructor): a mob paired in the tick it was made, before that step in vanilla's order,
    /// is not told of it.
    pub by_step: bool,
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
            Bounciness,
        ]
        .into_iter()
        .map(|a| Instance { attr: a, base: a.info().1, modifiers: Vec::new(), created: false, initial: a.info().1, by_step: false })
        .collect();
        for &(a, v) in extra {
            match list.iter_mut().find(|i| i.attr == a) {
                Some(i) => {
                    i.base = v;
                    i.initial = v;
                }
                None => list.push(Instance { attr: a, base: v, modifiers: Vec::new(), created: false, initial: v, by_step: false }),
            }
        }
        list.sort_by_key(|i| i.attr);
        Attributes { list }
    }

    pub fn get(&self, a: Attr) -> Option<&Instance> {
        self.list.iter().find(|i| i.attr == a)
    }

    /// `getInstance`: the instance of `a`, made if it was not there.
    pub fn get_mut(&mut self, a: Attr) -> Option<&mut Instance> {
        let i = self.list.iter_mut().find(|i| i.attr == a)?;
        i.created = true;
        Some(i)
    }

    /// The first step of a mob asks for its movement speed (`getAttribute`).
    pub fn touch_by_step(&mut self, a: Attr) {
        if let Some(i) = self.list.iter_mut().find(|i| i.attr == a)
            && !i.created
        {
            i.created = true;
            i.by_step = true;
        }
    }

    /// The instances a new mob of type `type_name` already has (what its constructor and `finalizeSpawn` ask for; read off a 26.3 server
    /// by `tools/AttributeVectors.java`).
    pub fn create_for(&mut self, type_name: &str) {
        use Attr::*;
        let made: &[Attr] = match type_name {
            "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" | "minecraft:zombie_villager" | "minecraft:zombified_piglin" | "minecraft:piglin" | "minecraft:enderman" => &[MovementSpeed],
            "minecraft:slime" | "minecraft:sulfur_cube" => &[MaxHealth, MovementSpeed],
            "minecraft:magma_cube" => &[MaxHealth, MovementSpeed, Armor],
            "minecraft:wolf" => &[MaxHealth],
            _ => &[],
        };
        for &a in made {
            let _ = self.get_mut(a);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn created(a: &Attributes) -> Vec<Attr> {
        a.list.iter().filter(|i| i.created).map(|i| i.attr).collect()
    }

    /// `AttributeMap.getSyncableAttributes` lists the instances that exist, and an instance is made when something asks to change the
    /// attribute (`getInstance`), not when its value is read; a new mob has the few its constructor asks for (read off 26.3 by
    /// `tools/AttributeVectors.java`).
    #[test]
    fn an_attribute_has_an_instance_once_it_is_asked_for() {
        let mut a = Attributes::new(&[(Attr::FollowRange, 16.0)]);
        assert!(created(&a).is_empty());
        let _ = a.value(Attr::MovementSpeed);
        let _ = a.get(Attr::MaxHealth);
        assert!(created(&a).is_empty(), "reading makes none");
        a.set_modifier(Attr::MovementSpeed, "x", 0.1, Op::AddValue);
        assert_eq!(created(&a), [Attr::MovementSpeed]);
        let mut zombie = Attributes::new(&[]);
        zombie.create_for("minecraft:zombie");
        assert_eq!(created(&zombie), [Attr::MovementSpeed]);
        let mut slime = Attributes::new(&[]);
        slime.create_for("minecraft:magma_cube");
        assert_eq!(created(&slime), [Attr::MaxHealth, Attr::MovementSpeed, Attr::Armor]);
        let mut pig = Attributes::new(&[]);
        pig.create_for("minecraft:pig");
        assert!(created(&pig).is_empty());
    }
}
