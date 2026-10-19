//! Carrying out enchantment entity effects (`EnchantmentEntityEffect.apply`) in two steps: the
//! effect is *resolved* here against the entity it affects (the level-based values, the entity's
//! random draws in vanilla's order, the look vector of an impulse) into [`Action`]s, which whoever
//! holds the world then applies: a player, a mob, an arrow, a block in the level.
//!
//! Which effects run when, and with which context, is the caller's: `tick` and `location_changed`
//! for the equipment of a living entity every tick / when its block position changes, `hit_block`
//! and `projectile_spawned` for projectiles, `post_attack` and `post_piercing_attack` for blows.

use crate::effects::{EntityEffect, SpawnParticles};
use kiln_item::Identifier;
use kiln_javamath::random::RandomSource;

/// What the effect's entity looks like to it: where, which way, how big, how it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Subject {
    /// `Entity.position()` (the effect's position can be another, see [`resolve`]).
    pub pos: [f64; 3],
    /// `[yRot, xRot]` in degrees.
    pub rot: [f32; 2],
    pub width: f32,
    pub height: f32,
    /// `Entity.getKnownMovement()`.
    pub movement: [f64; 3],
    /// `Entity.isSilent()`.
    pub silent: bool,
    /// A `LivingEntity` (mob effects apply to those only).
    pub living: bool,
}

/// One thing an effect does, with every number decided.
#[derive(Debug, Clone)]
pub enum Action {
    /// `Entity.igniteForSeconds`.
    Ignite { seconds: f32 },
    /// `LivingEntity.addEffect(new MobEffectInstance(effect, duration, amplifier))`.
    MobEffect { effect: Identifier, duration: i32, amplifier: i32 },
    /// `Entity.hurtServer` with a damage source of `damage_type` caused by the item's owner.
    Damage { damage_type: Identifier, amount: f32 },
    /// The enchanted item loses durability (`EnchantedItemInUse.onBreak` when it breaks).
    ItemDamage { amount: i32 },
    /// `Player.causeFoodExhaustion`.
    Exhaustion { amount: f32 },
    /// `Entity.addDeltaMovement` (and the motion packet to a player).
    Impulse { delta: [f64; 3] },
    /// `ServerLevel.playSound` at the effect's position, in the entity's sound source.
    Sound { sound: Identifier, volume: f32, pitch: f32, at: [f64; 3] },
    /// `ServerLevel.sendParticles` with a count of 0: `velocity` is the particle's direction.
    Particles { particle: Identifier, has_options: bool, pos: [f64; 3], velocity: [f64; 3], speed: f64 },
    /// `EntityType.spawn` at the position, one of the types picked by the level's random.
    Summon { entity: Identifier, at: [f64; 3], join_team: bool },
    /// `run_function`: the function as the entity, at the position.
    Function { function: Identifier, at: [f64; 3] },
    /// An effect that needs the level's blocks (`replace_block`, `replace_disk`,
    /// `set_block_properties`), or the level to explode; `at` is the effect's position.
    Level { effect: EntityEffect, level: i32, at: [f64; 3] },
}

/// `Mth.randomBetween`.
fn between(rng: &mut dyn RandomSource, min: f32, max: f32) -> f32 {
    rng.next_float() * (max - min) + min
}

/// `Math.round(float)`.
fn round(v: f32) -> i32 {
    (v as f64 + 0.5).floor() as i32
}

/// Resolves `effect` at enchantment level `level` for the entity `subject`, at `at` (the entity's
/// position for tick, location, projectile and attack effects; the hit position for `hit_block`).
/// `rng` is the entity's random (`Entity.getRandom()`), `level_rng` the level's (what a summon
/// picks its type with).
pub fn resolve(effect: &EntityEffect, level: i32, at: [f64; 3], subject: &Subject, rng: &mut dyn RandomSource, level_rng: &mut dyn RandomSource, out: &mut Vec<Action>) {
    match effect {
        EntityEffect::AllOf(list) => {
            for e in list {
                resolve(e, level, at, subject, rng, level_rng, out);
            }
        }
        EntityEffect::Ignite(seconds) => out.push(Action::Ignite { seconds: seconds.calculate(level) }),
        EntityEffect::ApplyMobEffect { to_apply, min_duration, max_duration, min_amplifier, max_amplifier } => {
            if !subject.living || to_apply.is_empty() {
                return;
            }
            let pick = &to_apply[rng.next_int_bounded(to_apply.len() as i32) as usize];
            let duration = round(between(rng, min_duration.calculate(level), max_duration.calculate(level)) * 20.0);
            let amplifier = round(between(rng, min_amplifier.calculate(level), max_amplifier.calculate(level))).max(0);
            out.push(Action::MobEffect { effect: pick.clone(), duration, amplifier });
        }
        EntityEffect::DamageEntity { damage_type, min_damage, max_damage } => {
            let amount = between(rng, min_damage.calculate(level), max_damage.calculate(level));
            out.push(Action::Damage { damage_type: damage_type.clone(), amount });
        }
        EntityEffect::ChangeItemDamage(amount) => out.push(Action::ItemDamage { amount: amount.calculate(level) as i32 }),
        EntityEffect::ApplyExhaustion(amount) => out.push(Action::Exhaustion { amount: amount.calculate(level) }),
        EntityEffect::ApplyImpulse { direction, coordinate_scale, magnitude } => {
            let look = look_rotate(subject.rot, *direction);
            let m = magnitude.calculate(level) as f64;
            out.push(Action::Impulse { delta: [look[0] * coordinate_scale[0] * m, look[1] * coordinate_scale[1] * m, look[2] * coordinate_scale[2] * m] });
        }
        EntityEffect::PlaySound { sounds, volume, pitch } => {
            if subject.silent || sounds.is_empty() {
                return;
            }
            let index = (level - 1).clamp(0, sounds.len() as i32 - 1) as usize;
            let (volume, pitch) = (volume.sample(rng), pitch.sample(rng));
            out.push(Action::Sound { sound: sounds[index].clone(), volume, pitch, at });
        }
        EntityEffect::SpawnParticles(p) => out.push(particles(p, at, subject, rng)),
        EntityEffect::SummonEntity { entity, join_team } => {
            let mut candidates: Vec<Identifier> = Vec::new();
            for name in entity {
                match name.strip_prefix('#') {
                    Some(tag) => {
                        let Some(ids) = kiln_data::live::tag("minecraft:entity_type", tag) else { continue };
                        candidates.extend(ids.iter().filter_map(|&id| kiln_item::registry::ENTITY_TYPE.name(id)).filter_map(Identifier::parse));
                    }
                    None => candidates.extend(Identifier::parse(name)),
                }
            }
            if candidates.is_empty() {
                return;
            }
            let pick = candidates.swap_remove(level_rng.next_int_bounded(candidates.len() as i32) as usize);
            out.push(Action::Summon { entity: pick, at, join_team: *join_team });
        }
        EntityEffect::RunFunction(function) => out.push(Action::Function { function: function.clone(), at }),
        EntityEffect::ReplaceBlock { .. } | EntityEffect::ReplaceDisk { .. } | EntityEffect::SetBlockProperties { .. } | EntityEffect::Explode { .. } => {
            out.push(Action::Level { effect: effect.clone(), level, at });
        }
        // `attribute` is the location logic's (modifiers while active); the rest is unknown.
        EntityEffect::Attribute(_) | EntityEffect::Other(_) => {}
    }
}

/// `SpawnParticlesEffect.apply`: the draws go x, y, z of the position, then the velocity's x, y, z,
/// then the speed (the order the arguments of `sendParticles` are evaluated in).
fn particles(p: &SpawnParticles, at: [f64; 3], subject: &Subject, rng: &mut dyn RandomSource) -> Action {
    let coordinate = |source: &crate::effects::ParticlePosition, entity: f64, center: f64, size: f32, rng: &mut dyn RandomSource| -> f64 {
        let size = size * source.scale;
        let base = if source.in_bounding_box { center + (rng.next_double() - 0.5) * size as f64 } else { entity };
        base + source.offset as f64
    };
    let velocity = |source: &crate::effects::ParticleVelocity, movement: f64, rng: &mut dyn RandomSource| -> f64 {
        movement * source.movement_scale as f64 + source.base.sample(rng) as f64
    };
    let (w, h) = (subject.width, subject.height);
    let x = coordinate(&p.horizontal_position, at[0], at[0], w, rng);
    let y = coordinate(&p.vertical_position, at[1], at[1] + (h / 2.0) as f64, h, rng);
    let z = coordinate(&p.horizontal_position, at[2], at[2], w, rng);
    let vx = velocity(&p.horizontal_velocity, subject.movement[0], rng);
    let vy = velocity(&p.vertical_velocity, subject.movement[1], rng);
    let vz = velocity(&p.horizontal_velocity, subject.movement[2], rng);
    let speed = p.speed.sample(rng) as f64;
    Action::Particles { particle: p.particle.clone(), has_options: p.has_options, pos: [x, y, z], velocity: [vx, vy, vz], speed }
}

/// `Entity.getLookQuaternion().transform(direction)`: `direction` turned by the entity's
/// rotation (yaw about y, then pitch about x), in the floats `Quaternionf` works in.
pub fn look_rotate(rot: [f32; 2], direction: [f64; 3]) -> [f64; 3] {
    // `Quaternionf.rotationYXZ(-yRot * (PI / 180), xRot * (PI / 180), 0)`.
    let (ay, ax) = (-rot[0] * 0.017453292, rot[1] * 0.017453292);
    let sx = kiln_javamath::trig::sin((ax * 0.5) as f64) as f32;
    let cx = cos_from_sin(sx, ax * 0.5);
    let sy = kiln_javamath::trig::sin((ay * 0.5) as f64) as f32;
    let cy = cos_from_sin(sy, ay * 0.5);
    let (x, y, z, w) = (cy * sx, sy * cx, -sy * sx, cy * cx);
    // `Quaternionf.transform(x, y, z, dest)` in `double`s from the float components.
    let (qx, qy, qz, qw) = (x as f64, y as f64, z as f64, w as f64);
    let d = direction;
    let w2 = qw * qw;
    let x2 = qx * qx;
    let y2 = qy * qy;
    let z2 = qz * qz;
    let zw = qz * qw;
    let xy = qx * qy;
    let xz = qx * qz;
    let yw = qy * qw;
    let yz = qy * qz;
    let xw = qx * qw;
    let (dx, dy, dz) = (d[0], d[1], d[2]);
    [
        (w2 + x2 - z2 - y2) * dx + (-zw + xy - zw + xy) * dy + (yw + xz + xz + yw) * dz,
        (xy + zw + zw + xy) * dx + (y2 - z2 + w2 - x2) * dy + (yz + yz - xw - xw) * dz,
        (xz - yw + xz - yw) * dx + (yz + yz + xw + xw) * dy + (z2 - y2 - x2 + w2) * dz,
    ]
}

/// `Math.cosFromSin(sin, angle)` of JOML (the non-fast variant): the cosine from the sine,
/// signed by where the angle falls.
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0f32 - sin * sin).sqrt();
    let a = angle + std::f32::consts::FRAC_PI_2;
    let b = a - ((a / std::f32::consts::TAU) as i32) as f32 * std::f32::consts::TAU;
    let b = if b < 0.0 { std::f32::consts::TAU + b } else { b };
    if b >= std::f32::consts::PI { -cos } else { cos }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{ParticlePosition, ParticleVelocity};
    use crate::number::LevelBasedValue;
    use crate::provider::Floats;
    use kiln_javamath::random::LegacyRandom;

    fn subject() -> Subject {
        Subject { pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0], width: 0.6, height: 1.8, movement: [0.5, 0.0, -0.25], silent: false, living: true }
    }

    #[test]
    fn particles_draw_in_the_order_vanilla_evaluates_them() {
        let p = SpawnParticles {
            particle: Identifier::parse("minecraft:soul").unwrap(),
            has_options: false,
            horizontal_position: ParticlePosition { in_bounding_box: true, offset: 0.0, scale: 1.0 },
            vertical_position: ParticlePosition { in_bounding_box: false, offset: 0.1, scale: 1.0 },
            horizontal_velocity: ParticleVelocity { movement_scale: -0.2, base: Floats::Constant(0.0) },
            vertical_velocity: ParticleVelocity { movement_scale: 0.0, base: Floats::Constant(0.1) },
            speed: Floats::Constant(1.0),
        };
        let mut a = LegacyRandom::new(7);
        let mut b = LegacyRandom::new(7);
        let mut out = Vec::new();
        resolve(&EntityEffect::SpawnParticles(p), 1, [1.0, 2.0, 3.0], &subject(), &mut a, &mut LegacyRandom::new(0), &mut out);
        let (x, z) = (1.0 + (b.next_double() - 0.5) * 0.6000000238418579, 3.0 + (b.next_double() - 0.5) * 0.6000000238418579);
        match &out[..] {
            [Action::Particles { pos, velocity, speed, .. }] => {
                assert_eq!(*pos, [x, 2.0 + 0.1f32 as f64, z]);
                assert_eq!(velocity[0], 0.5 * -0.2f32 as f64);
                assert_eq!(velocity[1], 0.1f32 as f64);
                assert_eq!(*speed, 1.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn sounds_follow_the_level_and_the_silent_flag() {
        let sounds = vec![Identifier::parse("a:one").unwrap(), Identifier::parse("a:two").unwrap()];
        let e = EntityEffect::PlaySound { sounds, volume: Floats::Constant(1.0), pitch: Floats::Constant(1.0) };
        let mut out = Vec::new();
        resolve(&e, 5, [0.0; 3], &subject(), &mut LegacyRandom::new(1), &mut LegacyRandom::new(1), &mut out);
        assert!(matches!(&out[..], [Action::Sound { sound, .. }] if sound.as_str() == "a:two"));
        out.clear();
        let silent = Subject { silent: true, ..subject() };
        resolve(&e, 1, [0.0; 3], &silent, &mut LegacyRandom::new(1), &mut LegacyRandom::new(1), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn an_impulse_follows_the_look() {
        let e = EntityEffect::ApplyImpulse { direction: [0.0, 0.0, 1.0], coordinate_scale: [1.0, 0.0, 1.0], magnitude: LevelBasedValue::Constant(2.0) };
        let mut out = Vec::new();
        // Facing south (yaw 0): +z.
        resolve(&e, 1, [0.0; 3], &subject(), &mut LegacyRandom::new(1), &mut LegacyRandom::new(1), &mut out);
        match &out[..] {
            [Action::Impulse { delta }] => assert!((delta[2] - 2.0).abs() < 1e-6 && delta[0].abs() < 1e-6 && delta[1] == 0.0, "{delta:?}"),
            other => panic!("{other:?}"),
        }
    }
}
