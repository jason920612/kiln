//! What each mob type adds to the shared mob tick: the creeper's fuse, the chicken's eggs and
//! slow fall, the sheep's grazing, the skeleton's arrows, the spider's climbing.

use super::goals::Living;
use super::{MobData, MobKind, Species};
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::Vec3;
use kiln_javamath::random::RandomSource;

/// Before `LivingEntity.tick` (`Creeper.tick` swells first).
pub fn pre_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let alive = super::is_alive(e, m);
    if let Species::Creeper { swell, old_swell, swell_dir, max_swell, radius, powered, ignited } = &mut m.species {
        if !alive {
            return;
        }
        *old_swell = *swell;
        if *ignited {
            *swell_dir = 1;
        }
        if *swell_dir > 0 && *swell == 0 {
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.creeper.primed", source: "hostile", volume: 1.0, pitch: 0.5 });
            }
            level.emit(Event::GameEvent { event: "minecraft:prime_fuse", pos: e.position(), entity: Some(e.id) });
        }
        *swell += *swell_dir;
        if *swell < 0 {
            *swell = 0;
        }
        if *swell >= *max_swell {
            *swell = *max_swell;
            // `explodeCreeper`.
            let power = *radius as f32 * if *powered { 2.0 } else { 1.0 };
            m.dead = true;
            let pos = e.position();
            let interaction = if level.mob_griefing() { crate::explosion::Interaction::DestroyWithDecay } else { crate::explosion::Interaction::Keep };
            crate::explosion::explode(level, Some(e.id), pos, power, false, interaction);
            e.discard();
        }
    }
}

/// After `LivingEntity.tick` (`Spider.tick` records its climbing).
pub fn post_tick(e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
    if let Species::Spider { climbing } = &mut m.species {
        *climbing = e.horizontal_collision;
    }
}

/// The types' `aiStep` additions (after `Mob.aiStep`).
pub fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    // `AgeableMob.aiStep`: babies grow up.
    if m.kind.is_animal() && super::is_alive(e, m) && m.age != 0 {
        m.age += if m.age < 0 { 1 } else { -1 };
    }
    let alive = super::is_alive(e, m);
    let baby = m.baby();
    if let Species::Chicken { egg_time } = &mut m.species {
        if !e.on_ground && e.delta.y < 0.0 {
            e.delta = e.delta.multiply(1.0, 0.6, 1.0);
        }
        if alive && !baby {
            *egg_time -= 1;
            if *egg_time <= 0 {
                level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/chicken_lay", pos: e.position() });
                let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.chicken.egg", source: "neutral", volume: 1.0, pitch });
                }
                level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: e.position(), entity: Some(e.id) });
                *egg_time = e.random.next_int_bounded(6000) + 6000;
            }
        }
    }
}

/// `Sheep.ate`: the wool grows back.
pub fn ate(_e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
    if let Species::Sheep { sheared, .. } = &mut m.species {
        *sheared = false;
    }
    if m.age < 0 {
        // `ageUp(60)`: 60 seconds closer to adulthood.
        m.age = (m.age + 60 * 20).min(0);
    }
}

/// `AbstractSkeleton.performRangedAttack`: an arrow from the eyes toward the target.
pub fn perform_ranged_attack(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living, power: f32) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
    let mut arrow = crate::arrow::new(id, 0, "minecraft:arrow", pos, Vec3::ZERO, Some(e.id), seed);
    let difficulty = level.difficulty() as f64;
    let damage = (power * 2.0) as f64 + super::mth::triangle(&mut arrow.random, difficulty * 0.11, 0.57425);
    if let crate::entity::EntityKind::Arrow(a) = &mut arrow.kind {
        a.base_damage = damage;
    }
    let dx = t.pos.x - e.x();
    let dy = t.pos.y + (t.bb.max_y - t.bb.min_y) * 0.3333333333333333 - arrow.y();
    let dz = t.pos.z - e.z();
    let d = (dx * dx + dz * dz).sqrt();
    let uncertainty = (14 - level.difficulty() as i32 * 4) as f32;
    shoot(&mut arrow, dx, dy + d * 0.20000000298023224, dz, 1.6, uncertainty);
    arrow.set_old_pos_and_rot();
    level.add_entity(arrow);
    let pitch = 1.0 / (e.random.next_float() * 0.4 + 0.8);
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.skeleton.shoot", source: "hostile", volume: 1.0, pitch });
    }
    let _ = m;
}

/// `Projectile.shoot`.
pub fn shoot(p: &mut Entity, x: f64, y: f64, z: f64, velocity: f32, inaccuracy: f32) {
    let spread = 0.0172275 * inaccuracy as f64;
    let n = Vec3::new(x, y, z).normalize();
    let a = super::mth::triangle(&mut p.random, 0.0, spread);
    let b = super::mth::triangle(&mut p.random, 0.0, spread);
    let c = super::mth::triangle(&mut p.random, 0.0, spread);
    let v = n.add(a, b, c).scale(velocity as f64);
    p.delta = v;
    p.needs_sync = true;
    let h = v.horizontal_distance();
    p.y_rot = (super::mth::atan2(v.x, v.z) * 57.2957763671875) as f32;
    p.x_rot = (super::mth::atan2(v.y, h) * 57.2957763671875) as f32;
    p.y_rot_o = p.y_rot;
    p.x_rot_o = p.x_rot;
}

/// `SheepColorSpawnRules` for a temperate biome: black, gray, light gray, brown or the common
/// white (with a rare pink).
pub fn sheep_color(r: &mut dyn RandomSource) -> u8 {
    const WHITE: u8 = 0;
    const PINK: u8 = 6;
    const GRAY: u8 = 7;
    const LIGHT_GRAY: u8 = 8;
    const BROWN: u8 = 12;
    const BLACK: u8 = 15;
    let i = r.next_int_bounded(100);
    match i {
        0..5 => BLACK,
        5..10 => GRAY,
        10..15 => LIGHT_GRAY,
        15..18 => BROWN,
        _ => {
            if r.next_int_bounded(500) < 499 {
                WHITE
            } else {
                PINK
            }
        }
    }
}

/// `SpiderEffectsGroupData.setRandomEffect`.
pub fn spider_effect(r: &mut dyn RandomSource) -> &'static str {
    match r.next_int_bounded(5) {
        0 | 1 => "minecraft:speed",
        2 => "minecraft:strength",
        3 => "minecraft:regeneration",
        _ => "minecraft:invisibility",
    }
}

/// Shearing a sheep (`Sheep.shear`): the wool drops come from the shearing loot table.
pub fn shear(e: &mut Entity, level: &mut dyn EntityLevel) -> bool {
    let Some(m) = super::data_mut(e) else { return false };
    let Species::Sheep { color, sheared } = &mut m.species else { return false };
    if *sheared || m.health <= 0.0 || m.age < 0 {
        return false;
    }
    *sheared = true;
    let color = *color;
    let names = [
        "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green",
        "red", "black",
    ];
    let table = format!("minecraft:shearing/sheep/{}", names[color as usize & 15]);
    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.sheep.shear", source: "players", volume: 1.0, pitch: 1.0 });
    level.emit(Event::ShearLoot { entity: e.id, table, pos: e.position() });
    true
}

/// Whether `kind` is hurt by `damage` at all (`fireImmune` types are not simulated).
pub fn immune(kind: MobKind, damage: DamageKind) -> bool {
    let _ = (kind, damage);
    false
}
