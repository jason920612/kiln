//! `ProjectileItem.asProjectile` and `Projectile.shoot` for the items that fly (arrows, snowballs, eggs,
//! potions, fire and wind charges, rockets): shared by dispensers and ominous item spawners.

use crate::container::triangle;
use kiln_entity::Entity;
use kiln_entity::math::Vec3;
use kiln_item::ItemStack;
use kiln_javamath::random::LegacyRandom;

/// A projectile made from an item, before it is shot.
pub(crate) struct Shot {
    pub kind: &'static kiln_data::entities::EntityType,
    pub entity: Entity,
    /// Where it starts.
    pub at: [f64; 3],
    /// `DispenseConfig.power` and `uncertainty`.
    pub power: f64,
    pub uncertainty: f64,
    /// `DispenseConfig.overrideDispenseEvent`.
    pub event: Option<i32>,
}

/// Whether the item is a `ProjectileItem` this module makes.
pub(crate) fn is_projectile_item(name: &str) -> bool {
    matches!(
        name,
        "minecraft:arrow"
            | "minecraft:tipped_arrow"
            | "minecraft:spectral_arrow"
            | "minecraft:fire_charge"
            | "minecraft:wind_charge"
            | "minecraft:firework_rocket"
            | "minecraft:snowball"
            | "minecraft:egg"
            | "minecraft:blue_egg"
            | "minecraft:brown_egg"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle"
    )
}

/// `asProjectile(level, origin, stack, direction)` of a projectile item flying along the axis step `step`.
/// Fire and wind charges start at `charge_origin` (a dispenser puts them a whole block out), with their
/// direction spread by the level's random `rng` first; `seed` seeds the new entity's random.
pub(crate) fn as_projectile(stack: &ItemStack, origin: [f64; 3], charge_origin: [f64; 3], step: [i32; 3], seed: i64, rng: &mut LegacyRandom) -> Option<Shot> {
    if !is_projectile_item(stack.item_name()) {
        return None;
    }
    let mut at = origin;
    let (mut uncertainty, mut power) = (6.0f64, 1.1f64);
    let mut event = None;
    let v = Vec3::new(origin[0], origin[1], origin[2]);
    let (kind, entity) = match stack.item_name() {
        "minecraft:fire_charge" | "minecraft:wind_charge" => {
            at = charge_origin;
            let o = Vec3::new(at[0], at[1], at[2]);
            let dir = Vec3::new(triangle(rng, step[0] as f64, 0.11485000000000001), triangle(rng, step[1] as f64, 0.11485000000000001), triangle(rng, step[2] as f64, 0.11485000000000001));
            (uncertainty, power) = (6.6666665f32 as f64, 1.0);
            if stack.item_name() == "minecraft:fire_charge" {
                event = Some(1018);
                let e = kiln_entity::ext_entity::fireball::new_unowned_small(o, dir, seed);
                (&kiln_data::entities::types::SMALL_FIREBALL, e)
            } else {
                event = Some(1051);
                let mut e = kiln_entity::ext_entity::wind_charge::new_thrown(None, o, seed);
                e.delta = dir;
                (&kiln_data::entities::types::WIND_CHARGE, e)
            }
        }
        "minecraft:firework_rocket" => {
            (uncertainty, power) = (1.0, 0.5);
            event = Some(1004);
            let e = kiln_entity::ext_entity::firework::new(v, stack.with_count(1), None, None, true, seed);
            (&kiln_data::entities::types::FIREWORK_ROCKET, e)
        }
        "minecraft:snowball" | "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" | "minecraft:splash_potion" | "minecraft:lingering_potion" | "minecraft:experience_bottle" => {
            use kiln_entity::projectile::Throwable as T;
            let (t, k) = match stack.item_name() {
                "minecraft:snowball" => (T::Snowball, &kiln_data::entities::types::SNOWBALL),
                "minecraft:splash_potion" => (T::SplashPotion, &kiln_data::entities::types::SPLASH_POTION),
                "minecraft:lingering_potion" => (T::LingeringPotion, &kiln_data::entities::types::LINGERING_POTION),
                "minecraft:experience_bottle" => (T::ExperienceBottle, &kiln_data::entities::types::EXPERIENCE_BOTTLE),
                _ => (T::Egg, &kiln_data::entities::types::EGG),
            };
            // (`ThrowablePotionItem`'s config: half the uncertainty, a quarter more power.)
            if matches!(t, T::SplashPotion | T::LingeringPotion | T::ExperienceBottle) {
                (uncertainty, power) = (3.0, 1.375);
            }
            let mut e = kiln_entity::projectile::new(0, 0, t, v, Vec3::new(0.0, 0.0, 0.0), None, seed);
            if let kiln_entity::EntityKind::Throwable(d) = &mut e.kind {
                d.item = Some(stack.with_count(1));
            }
            (k, e)
        }
        name => {
            let (type_name, k) = if name == "minecraft:spectral_arrow" {
                ("minecraft:spectral_arrow", &kiln_data::entities::types::SPECTRAL_ARROW)
            } else {
                ("minecraft:arrow", &kiln_data::entities::types::ARROW)
            };
            let mut e = kiln_entity::arrow::new(0, 0, type_name, v, Vec3::new(0.0, 0.0, 0.0), None, seed);
            // `ArrowItem.asProjectile`: picked up as the item, with a tipped arrow's potion effects.
            let one = stack.with_count(1);
            let effects: Vec<_> = match one.get(kiln_item::keys::POTION_CONTENTS) {
                Some(c) if name != "minecraft:spectral_arrow" => crate::effects::potion_effects(c, 1.0)
                    .into_iter()
                    .filter_map(|fx| kiln_data::builtin_entries("minecraft:mob_effect").and_then(|l| l.get(fx.id as usize).copied()).map(|n| (n, fx.duration, fx.amplifier)))
                    .collect(),
                _ => Vec::new(),
            };
            if let kiln_entity::EntityKind::Arrow(a) = &mut e.kind {
                a.pickup = kiln_entity::arrow::PICKUP_ALLOWED;
                a.pickup_item = Some(one);
                a.effects = effects;
            }
            (k, e)
        }
    };
    Some(Shot { kind, entity, at, power, uncertainty, event })
}

/// `Projectile.shoot`: the entity flies along the axis step `step`, spread by its own random; its motion.
pub(crate) fn shoot(entity: &mut Entity, step: [i32; 3], power: f64, uncertainty: f64) -> [f64; 3] {
    let len = ((step[0] * step[0] + step[1] * step[1] + step[2] * step[2]) as f64).sqrt();
    let spread = 0.0172275 * uncertainty;
    let mut v = [step[0] as f64 / len, step[1] as f64 / len, step[2] as f64 / len];
    for c in &mut v {
        *c += triangle(&mut entity.random, 0.0, spread);
    }
    let v = v.map(|c| c * power);
    entity.delta = Vec3::new(v[0], v[1], v[2]);
    let horizontal = (v[0] * v[0] + v[2] * v[2]).sqrt();
    entity.y_rot = (kiln_javamath::mth::atan2(v[0], v[2]) * 57.2957763671875) as f32;
    entity.x_rot = (kiln_javamath::mth::atan2(v[1], horizontal) * 57.2957763671875) as f32;
    entity.y_rot_o = entity.y_rot;
    entity.x_rot_o = entity.x_rot;
    v
}
