//! The sulfur cube archetypes (`data/minecraft/sulfur_cube_archetype/*.json` of the 26.3 data pack,
//! in the registry's order): what a sulfur cube turns into by the item it swallowed. The data
//! pack's own archetypes are not read (the table is the vanilla data).

use crate::mob::attributes::{Attr, Op};
use kiln_item::ItemStack;

/// `SulfurCubeArchetype.ExplosionData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explosion {
    pub power: i32,
    pub fuse: i32,
    pub causes_fire: bool,
}

/// `SulfurCubeArchetype.ContactDamage` (the amount is a constant float provider).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactDamage {
    pub amount: f32,
    pub attribute_to_source: bool,
    pub damage_type: &'static str,
}

/// One archetype.
#[derive(Debug)]
pub struct Archetype {
    pub name: &'static str,
    /// The item tag its items are in.
    pub items: &'static str,
    pub buoyant: bool,
    pub explosion: Option<Explosion>,
    pub contact_damage: Option<ContactDamage>,
    pub horizontal_power: f32,
    pub vertical_power: f32,
    pub hit_sound: &'static str,
    pub push_sound: &'static str,
    pub push_sound_cooldown: f32,
    pub push_sound_impulse_threshold: f32,
    /// (attribute, modifier id, amount, operation).
    pub attributes: &'static [(&'static str, &'static str, f64, &'static str)],
}

impl Archetype {
    /// The attribute modifiers as (attribute, id, amount, operation), skipping what Kiln has no attribute for.
    pub fn modifiers(&self) -> impl Iterator<Item = (Attr, &'static str, f64, Op)> + '_ {
        self.attributes.iter().filter_map(|&(attr, id, amount, op)| {
            let op = match op {
                "add_value" => Op::AddValue,
                "add_multiplied_base" => Op::AddMultipliedBase,
                _ => Op::AddMultipliedTotal,
            };
            Some((Attr::by_name(attr)?, id, amount, op))
        })
    }
}

/// `SulfurCubeArchetype.DEFAULT_KNOCKBACK_MODIFIERS`.
pub const DEFAULT_KNOCKBACK: (f32, f32) = (0.33, 0.06);

/// `SulfurCubeArchetype.DEFAULT_SOUND_SETTINGS`: (hit, push, cooldown, impulse threshold).
pub const DEFAULT_SOUND: (&str, &str, f32, f32) = ("minecraft:entity.sulfur_cube.regular.hit", "minecraft:entity.sulfur_cube.regular.push", 0.5, 0.2);

/// `SulfurCube.matchingArchetypes`: the archetypes whose items include the stack, in registry order.
pub fn matching(stack: &ItemStack) -> Vec<&'static Archetype> {
    if stack.is_empty() {
        return Vec::new();
    }
    ARCHETYPES.iter().filter(|a| crate::mob::item_tag(stack.item(), a.items)).collect()
}

pub static ARCHETYPES: [Archetype; 12] = [
    Archetype {
        name: "minecraft:bouncy",
        items: "minecraft:sulfur_cube_archetype/bouncy",
        buoyant: true,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.105,
        hit_sound: "minecraft:entity.sulfur_cube.bouncy.hit",
        push_sound: "minecraft:entity.sulfur_cube.bouncy.push",
        push_sound_cooldown: 0.7,
        push_sound_impulse_threshold: 0.3,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:bouncy_add_knockback_resistance", -2.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:bouncy_add_explosion_knockback_resistance", -2.0, "add_value"),
            ("minecraft:bounciness", "minecraft:bouncy_add_bounciness", 0.8999999761581421, "add_value"),
            ("minecraft:friction_modifier", "minecraft:bouncy_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:bouncy_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:explosive",
        items: "minecraft:sulfur_cube_archetype/explosive",
        buoyant: true,
        explosion: Some(Explosion { power: 3, fuse: 120, causes_fire: false }),
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.explosive.hit",
        push_sound: "minecraft:entity.sulfur_cube.explosive.push",
        push_sound_cooldown: 0.7,
        push_sound_impulse_threshold: 0.1,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:explosive_add_knockback_resistance", -1.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:explosive_add_explosion_knockback_resistance", -1.0, "add_value"),
            ("minecraft:bounciness", "minecraft:explosive_add_bounciness", 0.5, "add_value"),
            ("minecraft:friction_modifier", "minecraft:explosive_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:explosive_mul_air_drag_modifier", -0.699999988079071, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:fast_flat",
        items: "minecraft:sulfur_cube_archetype/fast_flat",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.9125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.fast_flat.hit",
        push_sound: "minecraft:entity.sulfur_cube.fast_flat.push",
        push_sound_cooldown: 0.9,
        push_sound_impulse_threshold: 0.03,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:fast_flat_add_knockback_resistance", -1.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:fast_flat_add_explosion_knockback_resistance", -1.0, "add_value"),
            ("minecraft:bounciness", "minecraft:fast_flat_add_bounciness", 0.5, "add_value"),
            ("minecraft:friction_modifier", "minecraft:fast_flat_mul_friction_modifier", -0.7999999970197678, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:fast_flat_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:fast_sliding",
        items: "minecraft:sulfur_cube_archetype/fast_sliding",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.6625,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.fast_sliding.hit",
        push_sound: "minecraft:entity.sulfur_cube.fast_sliding.push",
        push_sound_cooldown: 1.0,
        push_sound_impulse_threshold: 0.05,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:fast_sliding_add_knockback_resistance", 0.5, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:fast_sliding_add_explosion_knockback_resistance", 0.5, "add_value"),
            ("minecraft:bounciness", "minecraft:fast_sliding_add_bounciness", 0.10000000149011612, "add_value"),
            ("minecraft:friction_modifier", "minecraft:fast_sliding_mul_friction_modifier", -0.9499999992549419, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:fast_sliding_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:high_resistance",
        items: "minecraft:sulfur_cube_archetype/high_resistance",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.high_resistance.hit",
        push_sound: "minecraft:entity.sulfur_cube.high_resistance.push",
        push_sound_cooldown: 0.7,
        push_sound_impulse_threshold: 0.03,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:high_resistance_add_knockback_resistance", 0.699999988079071, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:high_resistance_add_explosion_knockback_resistance", 0.699999988079071, "add_value"),
            ("minecraft:bounciness", "minecraft:high_resistance_add_bounciness", 0.20000000298023224, "add_value"),
            ("minecraft:friction_modifier", "minecraft:high_resistance_mul_friction_modifier", 0.0, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:high_resistance_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:hot",
        items: "minecraft:sulfur_cube_archetype/hot",
        buoyant: true,
        explosion: None,
        contact_damage: Some(ContactDamage { amount: 1.0, attribute_to_source: false, damage_type: "minecraft:sulfur_cube_hot" }),
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.hot.hit",
        push_sound: "minecraft:entity.sulfur_cube.hot.push",
        push_sound_cooldown: 0.7,
        push_sound_impulse_threshold: 0.2,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:hot_add_knockback_resistance", -1.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:hot_add_explosion_knockback_resistance", -1.0, "add_value"),
            ("minecraft:bounciness", "minecraft:hot_add_bounciness", 0.5, "add_value"),
            ("minecraft:friction_modifier", "minecraft:hot_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:hot_mul_air_drag_modifier", -0.8999999985098839, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:light",
        items: "minecraft:sulfur_cube_archetype/light",
        buoyant: true,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.18,
        hit_sound: "minecraft:entity.sulfur_cube.light.hit",
        push_sound: "minecraft:entity.sulfur_cube.light.push",
        push_sound_cooldown: 0.7,
        push_sound_impulse_threshold: 0.2,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:light_add_knockback_resistance", -1.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:light_add_explosion_knockback_resistance", -1.0, "add_value"),
            ("minecraft:bounciness", "minecraft:light_add_bounciness", 1.0, "add_value"),
            ("minecraft:friction_modifier", "minecraft:light_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:light_mul_air_drag_modifier", 0.7999999523162842, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:regular",
        items: "minecraft:sulfur_cube_archetype/regular",
        buoyant: true,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.regular.hit",
        push_sound: "minecraft:entity.sulfur_cube.regular.push",
        push_sound_cooldown: 0.5,
        push_sound_impulse_threshold: 0.2,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:regular_add_knockback_resistance", -1.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:regular_add_explosion_knockback_resistance", -1.0, "add_value"),
            ("minecraft:bounciness", "minecraft:regular_add_bounciness", 0.5, "add_value"),
            ("minecraft:friction_modifier", "minecraft:regular_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:regular_mul_air_drag_modifier", -0.8999999985098839, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:slow_bouncy",
        items: "minecraft:sulfur_cube_archetype/slow_bouncy",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.24,
        hit_sound: "minecraft:entity.sulfur_cube.slow_bouncy.hit",
        push_sound: "minecraft:entity.sulfur_cube.slow_bouncy.push",
        push_sound_cooldown: 0.5,
        push_sound_impulse_threshold: 0.05,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:slow_bouncy_add_knockback_resistance", 0.4000000059604645, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:slow_bouncy_add_explosion_knockback_resistance", 0.4000000059604645, "add_value"),
            ("minecraft:bounciness", "minecraft:slow_bouncy_add_bounciness", 0.6000000238418579, "add_value"),
            ("minecraft:friction_modifier", "minecraft:slow_bouncy_mul_friction_modifier", -0.699999988079071, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:slow_bouncy_mul_air_drag_modifier", -0.9499999992549419, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:slow_flat",
        items: "minecraft:sulfur_cube_archetype/slow_flat",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.105,
        hit_sound: "minecraft:entity.sulfur_cube.slow_flat.hit",
        push_sound: "minecraft:entity.sulfur_cube.slow_flat.push",
        push_sound_cooldown: 0.9,
        push_sound_impulse_threshold: 0.03,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:slow_flat_add_knockback_resistance", 0.5, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:slow_flat_add_explosion_knockback_resistance", 0.5, "add_value"),
            ("minecraft:bounciness", "minecraft:slow_flat_add_bounciness", 0.4000000059604645, "add_value"),
            ("minecraft:friction_modifier", "minecraft:slow_flat_mul_friction_modifier", -0.5999999940395355, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:slow_flat_mul_air_drag_modifier", -0.8999999985098839, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:slow_sliding",
        items: "minecraft:sulfur_cube_archetype/slow_sliding",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.slow_sliding.hit",
        push_sound: "minecraft:entity.sulfur_cube.slow_sliding.push",
        push_sound_cooldown: 1.0,
        push_sound_impulse_threshold: 0.02,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:slow_sliding_add_knockback_resistance", 0.800000011920929, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:slow_sliding_add_explosion_knockback_resistance", 0.800000011920929, "add_value"),
            ("minecraft:bounciness", "minecraft:slow_sliding_add_bounciness", 0.10000000149011612, "add_value"),
            ("minecraft:friction_modifier", "minecraft:slow_sliding_mul_friction_modifier", -0.9499999992549419, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:slow_sliding_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
    Archetype {
        name: "minecraft:sticky",
        items: "minecraft:sulfur_cube_archetype/sticky",
        buoyant: false,
        explosion: None,
        contact_damage: None,
        horizontal_power: 0.4125,
        vertical_power: 0.09,
        hit_sound: "minecraft:entity.sulfur_cube.sticky.hit",
        push_sound: "minecraft:entity.sulfur_cube.sticky.push",
        push_sound_cooldown: 0.5,
        push_sound_impulse_threshold: 0.05,
        attributes: &[
            ("minecraft:knockback_resistance", "minecraft:sticky_add_knockback_resistance", -2.0, "add_value"),
            ("minecraft:explosion_knockback_resistance", "minecraft:sticky_add_explosion_knockback_resistance", -2.0, "add_value"),
            ("minecraft:bounciness", "minecraft:sticky_add_bounciness", 0.0, "add_value"),
            ("minecraft:friction_modifier", "minecraft:sticky_mul_friction_modifier", 1.0, "add_multiplied_total"),
            ("minecraft:air_drag_modifier", "minecraft:sticky_mul_air_drag_modifier", -0.9900000002235174, "add_multiplied_total"),
        ],
    },
];
