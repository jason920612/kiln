//! `DispenserBlock.dispenseFrom` and `DropperBlock.dispenseFrom`: a random non-empty slot's
//! item is dropped in front (`DefaultDispenseItemBehavior`) or, for a dropper facing a
//! container, moved into it. Dispensers use the default behaviour for every item, except
//! arrows, spectral arrows, snowballs and eggs (shot), water and lava buckets (placed) and
//! empty buckets (filled from a source) — the other vanilla dispense behaviours (tipped arrows,
//! potions, fire charges, armor, bone meal, shulker boxes, boats, ...) are not simulated yet.

use super::hopper::{View, add_item, container_at, with_target};
use super::triangle;
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_inventory::stack::StackExt;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};

/// `DispenserBlockEntity.getRandomSlot`: each non-empty slot replaces the pick with chance
/// 1/(its rank).
fn random_slot(items: &[ItemStack], rng: &mut dyn RandomSource) -> Option<usize> {
    let mut slot = None;
    let mut j = 1;
    for (i, s) in items.iter().enumerate() {
        if !s.is_empty() {
            let r = rng.next_int_bounded(j);
            j += 1;
            if r == 0 {
                slot = Some(i);
            }
        }
    }
    slot
}

/// `DispenserBlock.getDispensePosition`: 0.7 blocks out of the front face's centre.
pub(super) fn dispense_position(pos: BlockPos, facing: Direction) -> [f64; 3] {
    let st = facing.step();
    [pos.x as f64 + 0.5 + 0.7 * st[0] as f64, pos.y as f64 + 0.5 + 0.7 * st[1] as f64, pos.z as f64 + 0.5 + 0.7 * st[2] as f64]
}

/// `DefaultDispenseItemBehavior.spawnItem`.
pub(crate) fn spawn_item(level: &mut RegionLevel, rng: &mut LegacyRandom, stack: ItemStack, accuracy: i32, facing: Direction, at: [f64; 3]) {
    let y = at[1] - if facing.axis() == kiln_blocks::Axis::Y { 0.125 } else { 0.15625 };
    // The `ItemEntity` constructor's random throw, replaced below.
    rng.next_double();
    rng.next_double();
    let speed = rng.next_double() * 0.1 + 0.2;
    let st = facing.step();
    let spread = 0.0172275 * accuracy as f64;
    let vel = [triangle(rng, st[0] as f64 * speed, spread), triangle(rng, 0.2, spread), triangle(rng, st[2] as f64 * speed, spread)];
    level.out.spawns.push(crate::entities::Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos: [at[0], y, at[2]],
        vel,
        body: crate::entities::Body::Item { stack, pickup_delay: 0, thrower: None },
    });
}

/// `DefaultDispenseItemBehavior.dispense`: one item flies out, with the click sound (level
/// event 1000) and smoke (2000).
pub(super) fn default_dispense(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    let one = stack.split_count(1);
    spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
    level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

/// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
pub(crate) fn dispense_from(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    use kiln_data::block_logic::{self as logic, BlockClass as C};
    let dropper = logic::block_class(s) == C::DropperBlock;
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    super::unpack_loot(c, pos, loot.as_deref(), false, game_time, seed);
    let items = c.items.clone();
    let mut rng = super::pos_random(level, pos, 4);
    let Some(slot) = random_slot(&items, &mut rng) else {
        level.effect(Effect::LevelEvent { id: 1001, pos, data: 0 });
        if !dropper {
            level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
        }
        return;
    };
    let stack = items[slot].clone();
    let facing = state::get_dir(level.block(pos), "facing").unwrap_or(Direction::North);
    let result = if dropper {
        if stack.is_empty() {
            return;
        }
        match container_at(level, pos.relative(facing)) {
            Some(target) => {
                let left = with_target(level, &target, |dest: &mut View| add_item(dest, stack.copy_with_count(1), Some(facing.opposite()), None));
                for p in target.positions() {
                    crate::jukebox::settle(level, p);
                }
                match left {
                    Some(left) if left.is_empty() => {
                        for p in target.positions() {
                            let st = level.block(p);
                            kiln_blocks::update::update_neighbour_for_output_signal(level, p, kiln_blocks::BlockId::of(st));
                        }
                        let mut r = stack.copy();
                        r.shrink_count(1);
                        r
                    }
                    _ => stack.copy(),
                }
            }
            None => default_dispense(level, &mut rng, pos, facing, stack),
        }
    } else {
        dispense_behaviour(level, &mut rng, pos, facing, stack)
    };
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        kiln_inventory::Container::set_item(c, slot, result);
    }
    let st = level.block(pos);
    kiln_blocks::update::update_neighbour_for_output_signal(level, pos, kiln_blocks::BlockId::of(st));
}

/// The dispenser's behaviour for an item (`DispenserBlock.getDispenseMethod`).
/// `ProjectileDispenseBehavior` with the default `DispenseConfig` (power 1.1, uncertainty 6):
/// the projectile flies out of the front face (`Projectile.shoot`), click sound 1002.
fn dispense_projectile(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    use kiln_entity::math::Vec3;
    let mut at = dispense_position(pos, facing);
    let (mut uncertainty, mut power) = (6.0f64, 1.1f64);
    // `DispenseConfig.overrideDispenseEvent`.
    let mut event = 1002;
    let (kind, entity) = {
        let seed = rng.next_long();
        let mut origin = Vec3::new(at[0], at[1], at[2]);
        match stack.item_name() {
            // (`FireChargeItem` and `WindChargeItem`: a whole block out, the direction spread by the level's random first.)
            "minecraft:fire_charge" | "minecraft:wind_charge" => {
                let st = facing.step();
                at = [pos.x as f64 + 0.5 + st[0] as f64, pos.y as f64 + 0.5 + st[1] as f64, pos.z as f64 + 0.5 + st[2] as f64];
                origin = Vec3::new(at[0], at[1], at[2]);
                let dir = Vec3::new(triangle(rng, st[0] as f64, 0.11485000000000001), triangle(rng, st[1] as f64, 0.11485000000000001), triangle(rng, st[2] as f64, 0.11485000000000001));
                (uncertainty, power) = (6.6666665f32 as f64, 1.0);
                if stack.item_name() == "minecraft:fire_charge" {
                    event = 1018;
                    let e = kiln_entity::ext_entity::fireball::new_unowned_small(origin, dir, seed);
                    (&kiln_data::entities::types::SMALL_FIREBALL, e)
                } else {
                    event = 1051;
                    let mut e = kiln_entity::ext_entity::wind_charge::new_thrown(None, origin, seed);
                    e.delta = dir;
                    (&kiln_data::entities::types::WIND_CHARGE, e)
                }
            }
            "minecraft:firework_rocket" => {
                (uncertainty, power) = (1.0, 0.5);
                event = 1004;
                let e = kiln_entity::ext_entity::firework::new(origin, stack.with_count(1), None, None, true, seed);
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
                let mut e = kiln_entity::projectile::new(0, 0, t, origin, Vec3::new(0.0, 0.0, 0.0), None, seed);
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
                let mut e = kiln_entity::arrow::new(0, 0, type_name, origin, Vec3::new(0.0, 0.0, 0.0), None, seed);
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
        }
    };
    let mut entity = entity;
    // `Projectile.getMovementToShoot` and `shoot`: the facing, spread by the entity's random.
    let st = facing.step();
    let len = ((st[0] * st[0] + st[1] * st[1] + st[2] * st[2]) as f64).sqrt();
    let spread = 0.0172275 * uncertainty;
    let mut v = [st[0] as f64 / len, st[1] as f64 / len, st[2] as f64 / len];
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
    level.out.spawns.push(crate::entities::Spawn { kind, pos: at, vel: v, body: crate::entities::Body::Ready(Box::new(entity)) });
    stack.shrink_count(1);
    level.effect(Effect::LevelEvent { id: event, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

/// `MinecartDispenseItemBehavior.execute`: the minecart lands on the rail in front (or on
/// the one below the empty block in front); anything else is dispensed as an item.
fn dispense_minecart(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    use kiln_entity::ext_entity::minecart::rail_shape;
    let st = facing.step();
    let (x, y, z) = (pos.x as f64 + 0.5 + st[0] as f64 * 1.125, (pos.y as f64 + 0.5).floor() + st[1] as f64, pos.z as f64 + 0.5 + st[2] as f64 * 1.125);
    let front = pos.relative(facing);
    let state = level.block(front);
    let y_offset = if let Some(shape) = rail_shape(state) {
        if shape.is_slope() { 0.6 } else { 0.1 }
    } else if kiln_data::blocks_types::is_air(state)
        && let Some(below) = rail_shape(level.block(front.below()))
    {
        if facing == Direction::Down || !below.is_slope() { -0.9 } else { -0.4 }
    } else {
        // `DefaultDispenseItemBehavior.dispense`, then this behaviour's own sound and animation.
        let out = default_dispense(level, rng, pos, facing, stack);
        level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
        level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
        return out;
    };
    if let Some(kind) = kiln_data::entities::by_name(stack.item_name()) {
        let cart = crate::boats::new_cart(kind.name, kiln_entity::math::Vec3::new(x, y + y_offset, z), rng.next_long(), &stack);
        level.out.spawns.push(crate::entities::Spawn {
            kind,
            pos: [x, y + y_offset, z],
            vel: [0.0; 3],
            body: crate::entities::Body::Ready(Box::new(cart)),
        });
        stack.shrink_count(1);
    }
    level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

fn dispense_behaviour(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, stack: ItemStack) -> ItemStack {
    // The behaviours of `dispense_items`: `DefaultDispenseItemBehavior.dispense` follows them with its sound and animation.
    let stack = match super::dispense_items::behaviour(level, rng, pos, facing, stack) {
        Ok(done) => {
            let id = if done.success == Some(false) { 1001 } else { 1000 };
            level.effect(Effect::LevelEvent { id, pos, data: 0 });
            level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
            return done.stack;
        }
        Err(stack) => stack,
    };
    let target = pos.relative(facing);
    match stack.item_name() {
        name if kiln_entity::ext_entity::minecart::is_minecart(name) => dispense_minecart(level, rng, pos, facing, stack),
        "minecraft:arrow" | "minecraft:tipped_arrow" | "minecraft:spectral_arrow" | "minecraft:fire_charge" | "minecraft:wind_charge" | "minecraft:firework_rocket" | "minecraft:snowball" | "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" | "minecraft:splash_potion" | "minecraft:lingering_potion" | "minecraft:experience_bottle" => dispense_projectile(level, rng, pos, facing, stack),
        "minecraft:bucket" => {
            // Picks up a fluid source in front (`BucketPickup`).
            let t = level.block(target);
            let f = kiln_data::block_logic::fluid(t);
            if f.source && kiln_blocks::state::is(t, kiln_data::blocks::default_state::WATER) || f.source && kiln_blocks::state::is(t, kiln_data::blocks::default_state::LAVA) {
                let filled = if kiln_blocks::state::is(t, kiln_data::blocks::default_state::WATER) { "minecraft:water_bucket" } else { "minecraft:lava_bucket" };
                kiln_blocks::set_block(level, target, kiln_data::blocks::default_state::AIR, kiln_blocks::flags::ALL_IMMEDIATE);
                let mut rest = stack.clone();
                rest.shrink_count(1);
                let bucket = ItemStack::of(filled, 1).unwrap_or_default();
                if rest.is_empty() {
                    return bucket;
                }
                // `consumeWithRemainder`: into the dispenser, else dropped.
                if let Some(c) = level.blocks.containers.get_mut(pos) {
                    let mut left = bucket;
                    let mut view = View::one(c);
                    left = add_item(&mut view, left, None, None);
                    if !left.is_empty() {
                        spawn_item(level, rng, left, 6, facing, dispense_position(pos, facing));
                        level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
                        level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
                    }
                }
                return rest;
            }
            default_dispense(level, rng, pos, facing, stack)
        }
        _ => default_dispense(level, rng, pos, facing, stack),
    }
}
