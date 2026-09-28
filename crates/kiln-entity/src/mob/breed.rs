//! Animals: love mode, breeding, babies growing up (`Animal`, `AgeableMob`).

use super::{MobData, MobKind, Species};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use kiln_javamath::random::RandomSource;

/// `Animal.DEFAULT_IN_LOVE_TIME`.
pub const IN_LOVE_TIME: i32 = 600;
/// `AgeableMob.BABY_START_AGE`.
pub const BABY_START_AGE: i32 = -24000;
/// `Animal.PARENT_AGE_AFTER_BREEDING`.
pub const PARENT_AGE_AFTER_BREEDING: i32 = 6000;

/// `AgeableMob.aiStep` and `Animal.aiStep` after `Mob.aiStep`: the forced-age particles, growing
/// up (or the breeding cooldown running out), the age lock particles, then love mode counting
/// down with its heart particles (only their random draws matter here).
pub fn ai_step(e: &mut Entity, m: &mut MobData) {
    if m.forced_age_timer > 0 {
        if m.forced_age_timer % 4 == 0 {
            super::random_point(e, 1.0);
        }
        m.forced_age_timer -= 1;
    }
    if super::is_alive(e, m) {
        let age = m.age;
        if age < 0 && !m.age_locked {
            super::set_age(e, m, age + 1);
        } else if age > 0 {
            super::set_age(e, m, age - 1);
        }
    }
    if m.age_lock_timer > 0 {
        if m.age_lock_timer % 2 == 0 {
            // `getRandomX(1.0)`, `getRandomY(0.2)`, `getRandomZ(1.0)`.
            e.random.next_double();
            e.random.next_double();
            e.random.next_double();
        }
        m.age_lock_timer -= 1;
    }
    if !is_animal(m.kind) {
        return;
    }
    if m.age != 0 {
        m.in_love = 0;
    }
    if m.in_love > 0 {
        m.in_love -= 1;
        if m.in_love % 10 == 0 {
            e.random.next_gaussian();
            e.random.next_gaussian();
            e.random.next_gaussian();
            super::random_point(e, 1.0);
        }
    }
}

/// Types that extend `AgeableMob` (an age, growing up).
pub fn is_ageable(kind: MobKind) -> bool {
    match kind.ext() {
        Some(k) => k.info().ageable,
        None => is_animal(kind),
    }
}

/// Types that extend `Animal` (love mode, breeding food).
pub fn is_animal(kind: MobKind) -> bool {
    match kind.ext() {
        Some(k) => k.info().animal,
        None => matches!(kind, MobKind::Pig | MobKind::Cow | MobKind::Sheep | MobKind::Chicken),
    }
}

/// `Animal.customServerAiStep`: out of love once the age is not zero.
pub fn custom_server_ai_step(m: &mut MobData) {
    if is_animal(m.kind) && m.age != 0 {
        m.in_love = 0;
    }
}

/// `Animal.setInLove`: love for 600 ticks, remembering the player; the hearts for viewers.
pub fn set_in_love(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: Option<i32>) {
    m.in_love = IN_LOVE_TIME;
    if player.is_some() {
        m.love_cause = player;
    }
    level.emit(Event::EntityEvent { entity: e.id, event: 18 });
}

/// `AgeableMob.getSpeedUpSecondsWhenFeeding`.
pub fn speed_up_seconds_when_feeding(ticks_until_adult: i32) -> i32 {
    ((ticks_until_adult / 20) as f32 * 0.1) as i32
}

/// `Animal.spawnChildFromBreeding` with `partner` (another animal of the type, in love): the
/// baby between them, both parents' breeding cooldown, the hearts and an experience orb.
pub fn spawn_child(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, partner: i32) {
    let Some(p) = level.entity(partner) else {
        return;
    };
    let Some(pm) = super::data(p) else { return };
    let partner_variant = pm.variant;
    let partner_color = match pm.species {
        Species::Sheep { color, .. } => Some(color),
        _ => None,
    };
    // `getBreedOffspring`.
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut child = super::new(m.kind, id, 0, seed);
    {
        let cm = super::data_mut(&mut child).expect("a mob");
        match m.kind {
            MobKind::Pig | MobKind::Cow | MobKind::Chicken => {
                cm.variant = if e.random.next_bool() { m.variant } else { partner_variant };
            }
            MobKind::Sheep => {
                let mine = match m.species {
                    Species::Sheep { color, .. } => color,
                    _ => 0,
                };
                let color = offspring_color(level, mine, partner_color.unwrap_or(0));
                if let Species::Sheep { color: c, .. } = &mut cm.species {
                    *c = color;
                }
            }
            _ => {
                if let Some(k) = m.kind.ext()
                    && let Some(p) = level.entity(partner).and_then(super::data).cloned()
                {
                    k.breed_offspring(e, m, &p, cm, level);
                }
            }
        }
    }
    // `setBaby(true)`, then `snapTo(x, y, z, 0, 0)`.
    let mut cm = super::take(&mut child);
    super::set_age(&mut child, &mut cm, BABY_START_AGE);
    super::put(&mut child, cm);
    child.set_pos(e.position());
    child.y_rot = 0.0;
    child.x_rot = 0.0;
    child.set_old_pos_and_rot();
    // `finalizeSpawnChildFromBreeding`.
    super::set_age(e, m, PARENT_AGE_AFTER_BREEDING);
    m.in_love = 0;
    if let Some(p) = level.entity_mut(partner) {
        let mut pm = super::take(p);
        super::set_age(p, &mut pm, PARENT_AGE_AFTER_BREEDING);
        pm.in_love = 0;
        super::put(p, pm);
    }
    level.emit(Event::EntityEvent { entity: e.id, event: 18 });
    if level.mob_drops() {
        let value = e.random.next_int_bounded(7) + 1;
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut orb = crate::xp_orb::new_at(id, 0, e.position(), value, seed);
        orb.set_old_pos_and_rot();
        level.add_entity(orb);
    }
    level.add_entity(child);
}

/// `Sheep.getOffspringColor`: the dye two wool colors mix into (vanilla looks the pair up in
/// the crafting recipes), else one of the parents' colors at random (the level's random).
fn offspring_color(level: &mut dyn EntityLevel, a: u8, b: u8) -> u8 {
    if let Some(c) = mixed_dye(a, b) {
        return c;
    }
    if level.random().next_bool() { a } else { b }
}

/// Dye colors (`DyeColor` ids) whose mix has a crafting recipe (two dyes → two dyes).
pub fn mixed_dye(a: u8, b: u8) -> Option<u8> {
    const WHITE: u8 = 0;
    const ORANGE: u8 = 1;
    const MAGENTA: u8 = 2;
    const LIGHT_BLUE: u8 = 3;
    const YELLOW: u8 = 4;
    const LIME: u8 = 5;
    const PINK: u8 = 6;
    const GRAY: u8 = 7;
    const LIGHT_GRAY: u8 = 8;
    const CYAN: u8 = 9;
    const PURPLE: u8 = 10;
    const BLUE: u8 = 11;
    const GREEN: u8 = 13;
    const RED: u8 = 14;
    const BLACK: u8 = 15;
    let pair = |x: u8, y: u8| (a == x && b == y) || (a == y && b == x);
    Some(if pair(RED, YELLOW) {
        ORANGE
    } else if pair(BLUE, WHITE) {
        LIGHT_BLUE
    } else if pair(GREEN, WHITE) {
        LIME
    } else if pair(RED, WHITE) {
        PINK
    } else if pair(BLACK, WHITE) {
        GRAY
    } else if pair(GRAY, WHITE) {
        LIGHT_GRAY
    } else if pair(BLUE, GREEN) {
        CYAN
    } else if pair(BLUE, RED) {
        PURPLE
    } else if pair(PURPLE, PINK) {
        MAGENTA
    } else {
        return None;
    })
}

/// Whether the mob type is fed `item` (`Animal.isFood`: the type's food tag).
pub fn is_food(kind: MobKind, item: i32) -> bool {
    let tag = match kind {
        MobKind::Pig => "minecraft:pig_food",
        MobKind::Cow => "minecraft:cow_food",
        MobKind::Sheep => "minecraft:sheep_food",
        MobKind::Chicken => "minecraft:chicken_food",
        _ => return kind.ext().is_some_and(|k| k.is_food(item)),
    };
    super::item_tag(item, tag)
}

#[cfg(test)]
mod tests {
    use super::super::{GroupData, MobKind, SpawnContext, Species};
    use kiln_javamath::random::LegacyRandom;

    fn ctx(biome: &str) -> SpawnContext {
        let biome = kiln_data::synced_id("minecraft:worldgen/biome", biome);
        SpawnContext { special_multiplier: 0.0, effective_difficulty: 1.5, hard: false, halloween: false, biome, moon_brightness: 1.0 }
    }

    #[test]
    fn farm_animals_take_the_climate_variant() {
        for (biome, want) in [("minecraft:desert", "minecraft:warm"), ("minecraft:snowy_plains", "minecraft:cold"), ("minecraft:plains", "minecraft:temperate")] {
            for kind in [MobKind::Pig, MobKind::Cow, MobKind::Chicken] {
                let mut e = super::super::new(kind, 1, 0, 5);
                super::super::finalize_spawn(&mut e, &mut LegacyRandom::new(9), &ctx(biome), &mut GroupData::default(), true);
                let v = super::super::data(&e).unwrap().variant;
                assert_eq!(Some(v), kiln_data::synced_id(&format!("{}_variant", kind.type_name()), want), "{kind:?} in {biome}");
            }
        }
        // Cold biomes' common sheep are black.
        let blacks = (0..200)
            .filter(|&s| {
                let mut e = super::super::new(MobKind::Sheep, 1, 0, 5);
                super::super::finalize_spawn(&mut e, &mut LegacyRandom::new(s), &ctx("minecraft:snowy_plains"), &mut GroupData::default(), true);
                matches!(super::super::data(&e).unwrap().species, Species::Sheep { color: 15, .. })
            })
            .count();
        assert!(blacks > 150, "{blacks} of 200 cold sheep are black");
    }
}
