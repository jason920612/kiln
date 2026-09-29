//! The creaking heart block entity (`CreakingHeartBlockEntity`) against an abstract
//! [`EntityLevel`]: the heart of a pale oak tree spawns a creaking on its nights (`spawnProtector`),
//! keeps hold of it by UUID, hurts when its creaking is hit (particles and spreading resin),
//! and tears it down when it is gone or the creaking strays too far.
//!
//! The block entity's state ([`HeartBe`]) belongs to whoever owns the level's block entities (the
//! simulation keeps them next to its other block entities; [`crate::memory::MemoryLevel`] keeps
//! them for tests); the functions here take it out of the level, run, and hand it back.
//!
//! The block itself (`CreakingHeartBlock`: state updates from its logs, placement, experience)
//! is the block layer's; here `state` is read from the level.
//!
//! Gaps: the trail and crumble particles are not sent (no particle event; the random draws they
//! make are), `isUnobstructed` for entities that block building is not checked for the spawn.

use super::creaking;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::mob::{self, DamageSource, MobData, MobKind};
use kiln_data::blocks_types::block_of;
use kiln_javamath::random::{LegacyRandom, RandomSource};

/// The block of the heart.
pub const HEART: &str = "minecraft:creaking_heart";

/// `creakingInfo`: the creaking as an entity (already resolved) or by UUID (saved, or not found
/// yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Entity { id: i32, uuid: u128 },
    Uuid(u128),
}

impl Link {
    pub fn uuid(self) -> u128 {
        match self {
            Link::Entity { uuid, .. } | Link::Uuid(uuid) => uuid,
        }
    }
}

/// `CreakingHeartBlockEntity`'s fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeartBe {
    /// `creakingInfo` (`None`: no creaking).
    pub link: Option<Link>,
    pub ticks_existed: i64,
    pub ticker: i32,
    /// `emitter`: ticks of hurt particles left, and where they go.
    pub emitter: i32,
    pub emitter_target: Option<Vec3>,
    pub output_signal: i32,
}

impl HeartBe {
    /// `loadAdditional`: the `creaking` UUID.
    pub fn load(uuid: Option<u128>) -> HeartBe {
        let mut be = HeartBe::default();
        if let Some(u) = uuid {
            be.link = Some(Link::Uuid(u));
            be.ticks_existed = 0;
        }
        be
    }

    /// `saveAdditional`: the creaking's UUID.
    pub fn saved_uuid(&self) -> Option<u128> {
        self.link.map(Link::uuid)
    }

    /// `isProtector` for the creaking `id` with `uuid` (which is being ticked, hence not
    /// looked up among the entities).
    pub fn protects(&self, id: i32, uuid: u128) -> bool {
        match self.link {
            Some(Link::Entity { id: i, uuid: u }) => i == id || (uuid != 0 && u == uuid),
            Some(Link::Uuid(u)) => u == uuid,
            None => false,
        }
    }

    /// `setCreakingInfo(creaking)`.
    pub fn set_creaking(&mut self, id: i32, uuid: u128) {
        self.link = Some(Link::Entity { id, uuid });
    }

    /// `setCreakingInfo(uuid)`.
    pub fn set_uuid(&mut self, uuid: u128) {
        self.link = Some(Link::Uuid(uuid));
        self.ticks_existed = 0;
    }
}

/// Whether `state` is a creaking heart.
pub fn is_heart(state: u16) -> bool {
    block_of(state).name == HEART
}

fn prop(state: u16, name: &str) -> Option<&'static str> {
    block_of(state).property(state, name)
}

/// `CreakingHeartBlock.STATE`: `uprooted`, `dormant` or `awake`.
pub fn heart_state(state: u16) -> &'static str {
    prop(state, "creaking_heart_state").unwrap_or("uprooted")
}

fn in_tag(state: u16, tag: &str) -> bool {
    crate::mob::kinds::wolf::block_in_tag(state, tag)
}

/// `CreakingHeartBlock.hasRequiredLogs`: a `#pale_oak_logs` block on both sides along the
/// heart's axis, standing along the same axis.
pub fn has_required_logs(level: &dyn EntityLevel, state: u16, pos: BlockPos) -> bool {
    let axis = prop(state, "axis").unwrap_or("y");
    let (a, b) = match axis {
        "x" => (Direction::West, Direction::East),
        "z" => (Direction::North, Direction::South),
        _ => (Direction::Down, Direction::Up),
    };
    [a, b].into_iter().all(|d| {
        let n = level.block(pos.relative(d));
        in_tag(n, "minecraft:pale_oak_logs") && prop(n, "axis") == Some(axis)
    })
}

/// `CreakingHeartBlock.updateState` for the block at `pos` (a scheduled tick, or placement):
/// the state an uprooted heart with logs takes, if it changes.
pub fn update_state(level: &dyn EntityLevel, state: u16, pos: BlockPos) -> u16 {
    if has_required_logs(level, state, pos) && heart_state(state) == "uprooted" {
        return with_state(state, if level.creaking_active(pos) { "awake" } else { "dormant" });
    }
    state
}

fn with_state(state: u16, value: &str) -> u16 {
    block_of(state).with_property(state, "creaking_heart_state", value).unwrap_or(state)
}

/// A draw of the level's random: vanilla's own stream when replaying it, otherwise a stream
/// seeded by the heart and the game time (so the outcome does not depend on the region split).
struct Draw {
    r: LegacyRandom,
    shared: bool,
}

fn draw(level: &mut dyn EntityLevel, pos: BlockPos, salt: i64) -> Draw {
    match level.shared_ai_random() {
        Some(r) => Draw { r: r.clone(), shared: true },
        None => Draw { r: level.pos_random(pos, salt), shared: false },
    }
}

fn finish(level: &mut dyn EntityLevel, d: Draw) {
    if d.shared
        && let Some(r) = level.shared_ai_random()
    {
        *r = d.r;
    }
}

fn center(p: BlockPos) -> Vec3 {
    Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5)
}

fn sound(level: &mut dyn EntityLevel, at: Vec3, name: &'static str, volume: f32, pitch: f32) {
    level.emit(Event::Sound { pos: at, sound: name, source: "block", volume, pitch });
}

/// Runs `f` on the mob entity `id` taken out of the level (the way mobs hurt each other).
fn with_mob<R>(level: &mut dyn EntityLevel, id: i32, f: impl FnOnce(&mut Entity, &mut MobData, &mut dyn EntityLevel) -> R) -> Option<R> {
    let slot = level.entity_mut(id)?;
    if !matches!(slot.kind, EntityKind::Mob(_)) {
        return None;
    }
    let mut e = std::mem::replace(slot, Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
    let mut m = mob::take(&mut e);
    let r = f(&mut e, &mut m, level);
    mob::put(&mut e, m);
    if let Some(slot) = level.entity_mut(id) {
        *slot = e;
    }
    Some(r)
}

/// `getCreakingProtector`: the creaking entity's id, resolving (and dropping) the link on the way.
pub fn protector(level: &mut dyn EntityLevel, be: &mut HeartBe) -> Option<i32> {
    let uuid = match be.link? {
        Link::Entity { id, uuid } => {
            if level.entity(id).is_some_and(|e| !e.is_removed()) {
                return Some(id);
            }
            be.set_uuid(uuid);
            uuid
        }
        Link::Uuid(u) => u,
    };
    match level.entity_by_uuid(uuid).filter(|e| e.type_name == "minecraft:creaking" && !e.is_removed()).map(|e| e.id) {
        Some(id) => {
            be.set_creaking(id, uuid);
            Some(id)
        }
        None => {
            if be.ticks_existed >= 30 {
                be.link = None;
            }
            None
        }
    }
}

/// `distanceToCreaking`: from the creaking to the bottom centre of the heart.
fn distance_to_creaking(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut HeartBe) -> f64 {
    let Some(id) = protector(level, be) else { return 0.0 };
    let Some(e) = level.entity(id) else { return 0.0 };
    let (dx, dy, dz) = (e.x() - (pos.x as f64 + 0.5), e.y() - pos.y as f64, e.z() - (pos.z as f64 + 0.5));
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// `computeAnalogOutputSignal`: 15 with the creaking at the heart, less the further it is (32
/// blocks: none).
pub fn compute_signal(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut HeartBe) -> i32 {
    if be.link.is_none() || protector(level, be).is_none() {
        return 0;
    }
    let d = distance_to_creaking(level, pos, be);
    let f = d.clamp(0.0, 32.0) / 32.0;
    15 - (f * 15.0).floor() as i32
}

/// `serverTick` (the block ticks its entity unless the heart is uprooted).
pub fn tick(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut HeartBe) {
    let state = level.block(pos);
    if !is_heart(state) || heart_state(state) == "uprooted" {
        return;
    }
    be.ticks_existed += 1;
    let signal = compute_signal(level, pos, be);
    if be.output_signal != signal {
        be.output_signal = signal;
        level.update_neighbours_for_output_signal(pos);
    }
    let mut rng = draw(level, pos, 0x4352_4541);
    if be.emitter > 0 {
        if be.emitter > 50 {
            emit_particles(level, &mut rng.r, pos, be, 1, true);
            emit_particles(level, &mut rng.r, pos, be, 1, false);
        }
        if be.emitter % 10 == 0 && be.emitter_target.is_some() {
            if let Some(id) = protector(level, be)
                && let Some(e) = level.entity(id)
            {
                be.emitter_target = Some(e.bounding_box().center());
            }
            let target = be.emitter_target.unwrap();
            let f = 0.2f32 + 0.8f32 * (100 - be.emitter) as f32 / 100.0;
            let c = center(pos);
            let v = c.subtract(target.x, target.y, target.z).scale(f as f64).add(target.x, target.y, target.z);
            let at = center(BlockPos::containing(v.x, v.y, v.z));
            let volume = (be.emitter as f32 / 2.0) / 100.0 + 0.5;
            sound(level, at, "minecraft:block.creaking_heart.hurt", volume, 1.0);
        }
        be.emitter -= 1;
    }
    // `if (ticker-- < 0)`: the first run is on the second tick.
    let due = be.ticker < 0;
    be.ticker -= 1;
    if !due {
        finish(level, rng);
        return;
    }
    be.ticker = rng.r.next_int_bounded(5) + 20;
    // `updateCreakingState`.
    let new = if !has_required_logs(level, state, pos) && be.link.is_none() {
        with_state(state, "uprooted")
    } else if level.creaking_active(pos) {
        with_state(state, "awake")
    } else {
        with_state(state, "dormant")
    };
    let mut state = state;
    if new != state {
        level.set_block(pos, new, 3);
        state = new;
        if heart_state(state) == "uprooted" {
            finish(level, rng);
            return;
        }
    }
    if be.link.is_some() {
        if let Some(id) = protector(level, be) {
            let persistent = level.entity(id).and_then(mob::data).is_some_and(|m| m.persistence_required);
            let far = distance_to_creaking(level, pos, be) > 34.0;
            let active = level.creaking_active(pos);
            let mut stuck = || with_mob(level, id, |e, m, l| creaking::player_is_stuck_in_you(e, m, &*l)).unwrap_or(false);
            let remove = if !active && !persistent { true } else { far || stuck() };
            if remove {
                remove_protector(level, pos, be, None);
            }
        }
        finish(level, rng);
        return;
    }
    if heart_state(state) != "awake" || !level.spawning_monsters() || level.difficulty() == 0 {
        finish(level, rng);
        return;
    }
    // `getNearestPlayer(x, y, z, 32, false)`: any non-spectator within 32 blocks.
    let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
    let near = level.players_in(&Aabb::new(px - 32.0, py - 32.0 - 2.0, pz - 32.0, px + 32.0, py + 32.0 + 2.0, pz + 32.0)).iter().any(|p| !p.spectator && p.pos.distance_to_sqr(Vec3::new(px, py, pz)) < 32.0 * 32.0);
    if near
        && let Some((id, uuid)) = spawn_protector(level, &mut rng.r, pos)
    {
        be.set_creaking(id, uuid);
        sound(level, center(pos), "minecraft:block.creaking_heart.spawn", 1.0, 1.0);
    }
    finish(level, rng);
}

/// `SpawnUtil.trySpawnMob(CREAKING, SPAWNER, level, pos, 5, 16, 8, ON_TOP_OF_COLLIDER_NO_LEAVES, true)`
/// and what `spawnProtector` adds. Returns the new creaking's id (or the placeholder the level
/// gave it) and UUID.
fn spawn_protector(level: &mut dyn EntityLevel, r: &mut LegacyRandom, pos: BlockPos) -> Option<(i32, u128)> {
    for _ in 0..5 {
        let dx = r.next_int_bounded(33) - 16;
        let dz = r.next_int_bounded(33) - 16;
        let mut p = pos.offset(dx, 8, dz);
        if !move_to_possible_spawn_position(level, 8, &mut p) {
            continue;
        }
        // `noCollision(getSpawnAABB(x + 0.5, y, z + 0.5))`.
        let (w, h) = (0.66f32, 2.7f32);
        let half = (w / 2.0) as f64;
        let spawn_box = Aabb::new(p.x as f64 + 0.5 - half, p.y as f64, p.z as f64 + 0.5 - half, p.x as f64 + 0.5 + half, p.y as f64 + h as f64, p.z as f64 + 0.5 + half);
        if !crate::collision::no_collision(level, &crate::collision::CollisionContext::EMPTY, i32::MIN, &spawn_box) {
            continue;
        }
        // `EntityType.create`: the position of the block, a random yaw, `finalizeSpawn`.
        let id = level.next_entity_id();
        let uuid = level.fresh_uuid_at(pos);
        let seed = level.fresh_seed();
        let mut e = mob::new(MobKind::Creaking, id, uuid, seed);
        e.set_pos(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5));
        let yaw = mob::mth::wrap_degrees(r.next_float() * 360.0);
        e.y_rot = yaw;
        e.x_rot = 0.0;
        e.set_old_pos_and_rot();
        if let Some(m) = mob::data_mut(&mut e) {
            m.y_head_rot = yaw;
            m.y_body_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot_o = yaw;
        }
        let eff = level.effective_difficulty(p);
        let ctx = mob::SpawnContext { biome: None, moon_brightness: 1.0, special_multiplier: 0.0, effective_difficulty: eff, hard: level.difficulty() == 3, halloween: false };
        mob::finalize_spawn(&mut e, r, &ctx, &mut mob::GroupData::default(), false);
        // `checkSpawnObstruction`: no liquid in its box.
        let bb = e.bounding_box();
        let (x0, x1) = (crate::math::floor(bb.min_x), crate::math::ceil(bb.max_x));
        let (y0, y1) = (crate::math::floor(bb.min_y), crate::math::ceil(bb.max_y));
        let (z0, z1) = (crate::math::floor(bb.min_z), crate::math::ceil(bb.max_z));
        let liquid = (x0..x1).any(|x| (y0..y1).any(|y| (z0..z1).any(|z| kiln_data::blocks_types::has_fluid(level.block(BlockPos::new(x, y, z))))));
        if liquid {
            continue;
        }
        // `playAmbientSound` of the new creaking.
        if let Some(m) = mob::data_mut(&mut e) {
            creaking::set_transient(m, pos);
        }
        let position = e.position();
        {
            // `playAmbientSound`, and the `makeSound(CREAKING_SPAWN)` `serverTick` does once it
            // holds the creaking (the new mob is not in the level yet).
            let m = mob::take(&mut e);
            mob::make_sound(&mut e, &m, level, "minecraft:entity.creaking.ambient");
            mob::make_sound(&mut e, &m, level, "minecraft:entity.creaking.spawn");
            mob::put(&mut e, m);
        }
        level.add_entity_with_uuid(e);
        level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: position, entity: None });
        level.emit(Event::EntityEvent { entity: id, event: 60 });
        return Some((id, uuid));
    }
    None
}

/// `SpawnUtil.moveToPossibleSpawnPosition` with `ON_TOP_OF_COLLIDER_NO_LEAVES`.
fn move_to_possible_spawn_position(level: &dyn EntityLevel, range: i32, p: &mut BlockPos) -> bool {
    let mut above = level.block(*p);
    for _ in (-range..=range).rev() {
        *p = p.below();
        let above_pos = p.above();
        let ground = level.block(*p);
        let no_collision_above = {
            let (shape, _) = crate::collision::collision_shape(above, above_pos, &crate::collision::CollisionContext::EMPTY);
            shape.is_empty()
        };
        if no_collision_above && !in_tag(ground, "minecraft:leaves") && kiln_data::block_logic::face_sturdy(ground, Direction::Up as u8, kiln_data::block_logic::Support::Full) {
            *p = p.above();
            return true;
        }
        above = ground;
    }
    false
}

/// `emitParticles(level, count, reverse)`: the trail particles from the creaking to the heart
/// (or back); only their random draws are simulated (six doubles and an int a particle).
fn emit_particles(level: &mut dyn EntityLevel, r: &mut LegacyRandom, pos: BlockPos, be: &mut HeartBe, count: i32, _reverse: bool) {
    let _ = pos;
    if protector(level, be).is_none() {
        return;
    }
    for _ in 0..count {
        for _ in 0..6 {
            r.next_double();
        }
        r.next_int_bounded(40);
    }
}

/// `creakingHurt`: the heart feels its creaking hit: particles, and resin over the logs of an
/// awake heart.
pub fn creaking_hurt(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut HeartBe) {
    let Some(id) = protector(level, be) else { return };
    if be.emitter > 0 {
        return;
    }
    let mut rng = draw(level, pos, 0x4352_4855);
    emit_particles(level, &mut rng.r, pos, be, 20, false);
    if heart_state(level.block(pos)) == "awake" {
        let n = rng.r.next_int_bounded(2) + 2;
        for _ in 0..n {
            if let Some(at) = spread_resin(level, &mut rng.r, pos) {
                sound(level, center(at), "minecraft:block.resin.place", 1.0, 1.0);
                let s = level.block(pos);
                level.block_game_event("minecraft:block_place", center(at), None, s);
            }
        }
    }
    be.emitter = 100;
    be.emitter_target = level.entity(id).map(|e| e.bounding_box().center());
    finish(level, rng);
}

/// `Util.shuffledCopy(Direction.values(), random)`.
fn shuffled_directions(r: &mut LegacyRandom) -> [Direction; 6] {
    let mut d = Direction::ALL;
    let mut i = d.len();
    while i > 1 {
        let k = r.next_int_bounded(i as i32) as usize;
        d.swap(i - 1, k);
        i -= 1;
    }
    d
}

fn face_name(d: Direction) -> &'static str {
    match d {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}

fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}

/// `spreadResin`: a breadth first walk over the pale oak logs around the heart (2 deep, 64 at
/// most, neighbours in a shuffled order) puts resin clump on the first free face.
fn spread_resin(level: &mut dyn EntityLevel, r: &mut LegacyRandom, pos: BlockPos) -> Option<BlockPos> {
    let mut queue: std::collections::VecDeque<(BlockPos, i32)> = std::collections::VecDeque::new();
    let mut visited: std::collections::HashSet<(i32, i32, i32)> = std::collections::HashSet::new();
    queue.push_back((pos, 0));
    let mut count = 0;
    let resin = kiln_data::blocks_types::block_by_name("minecraft:resin_clump")?;
    while let Some((p, depth)) = queue.pop_front() {
        if !visited.insert((p.x, p.y, p.z)) {
            continue;
        }
        // The node visitor.
        let s = level.block(p);
        if in_tag(s, "minecraft:pale_oak_logs") {
            for d in shuffled_directions(r) {
                let rel = p.relative(d);
                let mut st = level.block(rel);
                let face = opposite(d);
                if kiln_data::blocks_types::is_air(st) {
                    st = resin.default;
                } else if block_of(st).name == "minecraft:water" && prop(st, "level") == Some("0") {
                    st = resin.with_property(resin.default, "waterlogged", "true").unwrap_or(resin.default);
                }
                if block_of(st).name == "minecraft:resin_clump" && prop(st, face_name(face)) != Some("true") {
                    let new = resin.with_property(st, face_name(face), "true").unwrap_or(st);
                    level.set_block(rel, new, 3);
                    return Some(rel);
                }
            }
        }
        count += 1;
        if count >= 64 {
            return None;
        }
        if depth < 2 {
            // The neighbour provider: the logs around, in a shuffled order.
            for d in shuffled_directions(r) {
                let rel = p.relative(d);
                if in_tag(level.block(rel), "minecraft:pale_oak_logs") {
                    queue.push_back((rel, depth + 1));
                }
            }
        }
    }
    None
}

/// `removeProtector(source)`: the creaking crumbles (no source: the heart went away) or dies
/// twitching (broken by a player or an explosion), and the heart lets go of it.
pub fn remove_protector(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut HeartBe, source: Option<DamageSource>) {
    let _ = pos;
    let Some(id) = protector(level, be) else { return };
    with_mob(level, id, |e, m, l| match source {
        None => creaking::tear_down(e, m, l),
        Some(src) => {
            creaking::death_effects(e, m, l, &src);
            creaking::set_tearing_down(m);
            m.set_health(0.0);
        }
    });
    be.link = None;
}
