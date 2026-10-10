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
pub fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn crate::level::EntityLevel) {
    if m.forced_age_timer > 0 {
        if m.forced_age_timer % 4 == 0 {
            super::random_point(e, 1.0);
        }
        m.forced_age_timer -= 1;
    }
    if super::is_alive(e, m) {
        let age = m.age;
        if age < 0 && !m.age_locked {
            super::set_age_in(e, m, age + 1, Some(&mut *level));
        } else if age > 0 {
            super::set_age_in(e, m, age - 1, Some(&mut *level));
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
    // `Frog.spawnChildFromBreeding`: `finalizeSpawnChildFromBreeding` with no baby (the caller
    // makes the mother pregnant).
    if m.kind.ext().is_some_and(|k| k.breed_as_pregnancy()) {
        let cause = m.love_cause.or(pm.love_cause);
        if let Some(player) = cause.filter(|&c| level.player(c).is_some()) {
            let partner_seen = crate::level::Seen::of(p);
            let criterion = crate::level::Criterion::BredAnimals { parent: crate::level::Seen::of_mob(e, m), partner: partner_seen, child: None };
            level.emit(Event::Criterion { player, criterion });
        }
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
        return;
    }
    let pm = pm.clone();
    // `getBreedOffspring`.
    let mut child = breed_offspring(e, m, &pm, level);
    // `setBaby(true)`, then `snapTo(x, y, z, 0, 0)`.
    let mut cm = super::take(&mut child);
    super::set_age(&mut child, &mut cm, BABY_START_AGE);
    super::put(&mut child, cm);
    child.set_pos(e.position());
    child.y_rot = 0.0;
    child.x_rot = 0.0;
    child.set_old_pos_and_rot();
    // `finalizeSpawnChildFromBreeding`: the player who made either parent fall in love bred
    // them (the `animals_bred` statistic and `bred_animals`).
    let cause = m.love_cause.or_else(|| level.entity(partner).and_then(super::data).and_then(|p| p.love_cause));
    if let Some(player) = cause.filter(|&c| level.player(c).is_some()) {
        let partner_seen = level.entity(partner).map(crate::level::Seen::of);
        if let Some(partner_seen) = partner_seen {
            let criterion = crate::level::Criterion::BredAnimals {
                parent: crate::level::Seen::of_mob(e, m),
                partner: partner_seen,
                child: Some(crate::level::Seen::of(&child)),
            };
            level.emit(Event::Criterion { player, criterion });
        }
    }
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
    // Sniffers lay an egg (`Sniffer.spawnChildFromBreeding`) with its plop.
    if let Some(item) = m.kind.ext().and_then(|k| k.breed_as_item()) {
        let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 0.5;
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.sniffer.egg_plop", source: "neutral", volume: 1.0, pitch });
        }
        if let Some(stack) = kiln_item::ItemStack::of(item, 1) {
            super::spawn_at_location(e, level, stack);
        }
        return;
    }
    level.add_entity(child);
}

/// `AgeableMob.getBreedOffspring(level, partner)` for the mob `e` (its data `m`) and `partner`'s data: the child with its
/// variant or colour from the parents, a grown one until the caller makes it a baby.
pub fn breed_offspring(e: &mut Entity, m: &mut MobData, partner: &MobData, level: &mut dyn EntityLevel) -> Entity {
    let partner_variant = partner.variant;
    let partner_color = match partner.species {
        Species::Sheep { color, .. } => Some(color),
        _ => None,
    };
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let child_kind = m.kind.ext().map_or(m.kind, |k| k.offspring_kind(m, partner));
    let mut child = super::new(child_kind, id, 0, seed);
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
                if let Some(k) = m.kind.ext() {
                    k.breed_offspring(e, m, partner, cm, level);
                }
            }
        }
    }
    child
}

/// `SpawnEggItem.spawnOffspringFromSpawnEgg` for a spawn egg of the mob's own type used on it: a baby (`getBreedOffspring` for an
/// ageable mob, a fresh mob for the others) at its place, or `None` when the type has no babies.
pub fn offspring_from_egg(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> Option<Entity> {
    if is_ageable(m.kind) && m.kind.ext().is_some_and(|k| k.no_offspring()) {
        // (`getBreedOffspring` returned null: the mob was never made.)
        return None;
    }
    let mut child = if is_ageable(m.kind) {
        let me = m.clone();
        breed_offspring(e, m, &me, level)
    } else {
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        super::new(m.kind, id, 0, seed)
    };
    let mut cm = super::take(&mut child);
    super::convert::set_baby(&mut child, &mut cm, true);
    let baby = cm.baby();
    super::put(&mut child, cm);
    if !baby {
        return None;
    }
    child.set_pos(e.position());
    child.y_rot = 0.0;
    child.x_rot = 0.0;
    child.set_old_pos_and_rot();
    Some(child)
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
