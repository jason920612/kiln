//! Skeleton horses (`SkeletonHorse`, shared with the horse family in [`super::horse`]) and the
//! skeleton trap: during a thunderstorm the level may spawn a trap horse with the flag set
//! (`SkeletonHorse.setTrap(true)`, which adds the [`SkeletonTrapGoal`]); when a living player
//! comes within 10 blocks the goal calls a visual lightning bolt, tames the horse and adds a
//! skeleton on it and three more skeleton horsemen thrown out around it, their gear
//! enchanted from `minecraft:mob_spawn_equipment`. A trap that nobody approaches vanishes
//! after 18000 ticks.

use super::horse::{self, st, st_mut};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::ext::CustomGoal;
use crate::mob::goals::Goal;
use crate::mob::{self, GroupData, MobData, MobKind, SpawnContext, mth};
use crate::custom_goal_boilerplate;
use kiln_item::ItemStack;

pub use super::horse::SKELETON as KIND;

/// `SkeletonTrapGoal.LIGHTNING_INVULNERABLE_TICKS`.
const INVULNERABLE_TICKS: i32 = 60;

/// `SkeletonHorse.TRAP_MAX_LIFE`.
const TRAP_MAX_LIFE: i32 = 18000;

/// The goal's class name (the parity harness compares it).
const GOAL_NAME: &str = "SkeletonTrapGoal";

/// `SkeletonHorse.setTrap`: adds the trap goal (priority 1), or takes it off.
pub fn set_trap(m: &mut MobData, on: bool) {
    let Some(s) = crate::mob::ext::state_mut::<horse::State>(m) else { return };
    if s.trap == on {
        return;
    }
    s.trap = on;
    if on {
        m.goals.add(1, Goal::Custom(Box::new(SkeletonTrapGoal)));
    } else {
        m.goals.remove_where(|g| g.name() == GOAL_NAME);
    }
}

/// Whether `m` is a skeleton horse whose trap is set.
pub fn is_trap(m: &MobData) -> bool {
    m.kind == MobKind::SkeletonHorse && crate::mob::ext::state::<horse::State>(m).is_some_and(|s| s.trap)
}

/// After the goal selector ran: a goal that just sprung the trap (`setTrap(false)` from its
/// own tick) leaves the selector, running goals and all.
pub(super) fn drop_spent_goal(m: &mut MobData) {
    if !st(m).trap {
        m.goals.remove_where(|g| g.name() == GOAL_NAME);
    }
}

/// `SkeletonHorse.aiStep` after the horse's own: an untamed-by-anyone trap that nobody sprang
/// vanishes after 18000 ticks (`isPersistenceRequired` keeps it).
pub(super) fn ai_step(e: &mut Entity, m: &mut MobData) {
    if m.persistence_required || !st(m).trap {
        return;
    }
    let s = st_mut(m);
    let t = s.trap_time;
    s.trap_time += 1;
    if t >= TRAP_MAX_LIFE {
        e.discard();
    }
}

/// `Level.hasNearbyAlivePlayer(x, y, z, dist)`: a player that is not a spectator and is alive,
/// closer than `dist` (feet position).
fn has_nearby_alive_player(level: &dyn EntityLevel, pos: Vec3, dist: f64) -> bool {
    let area = Aabb::new(pos.x - dist - 1.0, pos.y - dist - 2.0, pos.z - dist - 1.0, pos.x + dist + 1.0, pos.y + dist + 1.0, pos.z + dist + 1.0);
    level.players_in(&area).iter().any(|p| !p.spectator && p.alive && p.pos.distance_to_sqr(pos) < dist * dist)
}

/// `SkeletonTrapGoal`.
#[derive(Clone, Debug)]
pub struct SkeletonTrapGoal;

impl CustomGoal for SkeletonTrapGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        GOAL_NAME
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        st(m).trap && has_nearby_alive_player(level, e.position(), 10.0)
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let at = e.position();
        let ctx = difficulty_at(level, e.block_position());
        // `setTrap(false)` (the goal leaves the selector after this tick), `setTamed(true)`,
        // `setAge(0)`.
        let s = st_mut(m);
        s.trap = false;
        s.tamed = true;
        mob::set_age(e, m, 0);
        // The visual bolt at the horse.
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        level.add_entity(crate::ext_entity::lightning::new(id, 0, at, true, seed));
        // The skeleton on the trap horse itself.
        let mut skeleton = create_skeleton(level, &ctx, at);
        crate::ride::start_riding(&mut skeleton, e, false);
        // (the trap horse's own mob data is out of `e` while its goal ticks: snap by hand)
        crate::ride::snap_rotation_to_mount(&mut skeleton, e);
        level.add_entity(skeleton);
        // Three more horsemen, thrown about by the trap horse's random.
        for _ in 0..3 {
            let mut horse = create_horse(level, &ctx, at);
            let mut skeleton = create_skeleton(level, &ctx, horse.position());
            crate::ride::start_riding(&mut skeleton, &mut horse, false);
            let x = mth::triangle(&mut e.random, 0.0, 1.1485);
            let z = mth::triangle(&mut e.random, 0.0, 1.1485);
            // `push(x, 0.0, z)`.
            horse.delta = horse.delta.add(x, 0.0, z);
            horse.needs_sync = true;
            level.add_entity(horse);
            level.add_entity(skeleton);
        }
    }
}

/// `ServerLevel.getCurrentDifficultyAt(pos)` for a spawn.
pub fn difficulty_at(level: &dyn EntityLevel, pos: BlockPos) -> SpawnContext {
    let eff = level.effective_difficulty(pos);
    SpawnContext {
        biome: None,
        moon_brightness: 1.0,
        special_multiplier: super::zombie::special_multiplier(eff),
        effective_difficulty: eff,
        hard: level.difficulty() == 3,
        halloween: false,
    }
}

/// `SkeletonTrapGoal.createHorse`: a tamed adult skeleton horse with the spawn protection,
/// after `finalizeSpawn` (the level's random).
fn create_horse(level: &mut dyn EntityLevel, ctx: &SpawnContext, at: Vec3) -> Entity {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut horse = mob::new(MobKind::SkeletonHorse, id, 0, seed);
    mob::finalize_spawn(&mut horse, level.random(), ctx, &mut GroupData::default(), false);
    horse.set_pos(at);
    horse.invulnerable_time = INVULNERABLE_TICKS;
    let mut m = mob::take(&mut horse);
    m.persistence_required = true;
    st_mut(&mut m).tamed = true;
    mob::set_age(&mut horse, &mut m, 0);
    mob::put(&mut horse, m);
    horse
}

/// `SkeletonTrapGoal.createSkeleton`: a skeleton after `finalizeSpawn` with the spawn
/// protection, persistent, an iron helmet unless it got a head item, and both enchanted.
fn create_skeleton(level: &mut dyn EntityLevel, ctx: &SpawnContext, at: Vec3) -> Entity {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut skeleton = mob::new(MobKind::Skeleton, id, 0, seed);
    mob::finalize_spawn(&mut skeleton, level.random(), ctx, &mut GroupData::default(), false);
    skeleton.set_pos(at);
    skeleton.invulnerable_time = INVULNERABLE_TICKS;
    let mut m = mob::take(&mut skeleton);
    m.persistence_required = true;
    if m.equipment[mob::HEAD].is_empty()
        && let Some(helmet) = ItemStack::of("minecraft:iron_helmet", 1)
    {
        m.equipment[mob::HEAD] = helmet;
    }
    for slot in [mob::MAINHAND, mob::HEAD] {
        enchant(level, ctx, &mut skeleton.random, &mut m.equipment[slot]);
    }
    mob::put(&mut skeleton, m);
    skeleton
}

/// `SkeletonTrapGoal.enchant`: the slot's item gets an empty enchantment list, then what the
/// `minecraft:mob_spawn_equipment` provider picks with the skeleton's own random.
fn enchant(level: &mut dyn EntityLevel, ctx: &SpawnContext, random: &mut kiln_javamath::random::LegacyRandom, stack: &mut ItemStack) {
    if stack.is_empty() {
        return;
    }
    stack.insert(kiln_item::component::keys::ENCHANTMENTS, kiln_item::component::Enchantments::default());
    level.enchant_from_provider(stack, "minecraft:mob_spawn_equipment", ctx.special_multiplier, random);
}
