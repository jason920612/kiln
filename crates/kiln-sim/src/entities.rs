//! Non-player entities: storage that follows region merges and splits, spawning with network
//! ids that do not depend on how regions split the world, ticking inside their region with
//! kiln-entity's vanilla behaviour, and tracking to the region's players (design §6).

use crate::Player;
use crate::blocks::{self, RegionLevel};
use crate::health;
use bytes::Bytes;
use kiln_blocks::{Effect, Level};
use kiln_data::entities::{EntityType, data};
use kiln_entity::level::{DamageKind, PlayerView};
use kiln_entity::math::{Aabb, BlockPos, Vec3};
use kiln_entity::{EntityFilter, EntityKind, EntityLevel, Event};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_link::ConnId;
use kiln_proto::packets::entity::{self, DataValue, EntityData, MoveState, MovementTracker};
use kiln_proto::packets::world_fx;
use kiln_region::{CellPos, RegionPart};
use kiln_world::{CellStore, ChunkPos};
use smallvec::SmallVec;
use uuid::Uuid;

/// Ticks an item waits before it can be picked up after a player drops it.
pub(crate) const DROP_PICKUP_DELAY: i32 = 40;

/// What a spawn becomes.
pub(crate) enum Body {
    Item { stack: kiln_item::ItemStack, pickup_delay: i32 },
    /// `FallingBlockEntity.fall` of `state` from the block at the spawn position.
    FallingBlock { state: u16 },
    /// `TntBlock.prime`: a primed TNT with vanilla's random hop.
    Tnt,
    /// An entity kiln-entity built during a tick (its id is replaced by the assigned one).
    Ready(Box<kiln_entity::Entity>),
    /// An entity loaded from its chunk's saved data; keeps its UUID (unless it had none).
    Loaded(Box<kiln_entity::Entity>),
}

pub(crate) struct Entity {
    pub id: i32,
    pub uuid: Uuid,
    pub kind: &'static EntityType,
    /// The owned cell the entity is routed by (where it last was inside its region).
    pub cell: CellPos,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub on_ground: bool,
    /// Ticks since it was added (`ServerEntity.tickCount`, which paces velocity updates).
    pub age: i32,
    pub removed: bool,
    /// The vanilla state; `None` only while the entity is being ticked.
    pub phys: Option<kiln_entity::Entity>,
    tracker: MovementTracker,
    /// Velocity the viewers last got.
    sent_vel: [f64; 3],
    /// Players tracking this entity (sorted).
    pub seen_by: Vec<ConnId>,
    /// Section at the last tracking update; `None` forces a re-evaluation.
    section: Option<[i32; 3]>,
}

/// A spawn requested during a phase; ids are handed out afterwards in canonical order.
pub(crate) struct Spawn {
    pub kind: &'static EntityType,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub body: Body,
}

impl Spawn {
    /// Order of spawns from different regions: by position, then type, then the UUID of a
    /// loaded entity (never by region or load order).
    fn key(&self) -> ([u64; 3], i32, u128) {
        let uuid = match &self.body {
            Body::Loaded(e) => e.uuid,
            _ => 0,
        };
        (self.pos.map(f64::to_bits), self.kind.id, uuid)
    }

    /// A spawn for an entity loaded from its chunk.
    pub fn loaded(e: kiln_entity::Entity) -> Option<Spawn> {
        let kind = kiln_data::entities::by_name(e.type_name)?;
        Some(Spawn { kind, pos: arr(e.position()), vel: arr(e.delta), body: Body::Loaded(Box::new(e)) })
    }
}

/// A fresh entity UUID (version 4 layout), from the world seed, the world age and the network
/// id: unique across restarts of the same world, and the same however regions split it.
pub(crate) fn fresh_uuid(world_seed: i64, game_time: i64, id: i32) -> Uuid {
    let mix = |mut h: u64, v: u64| {
        h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^ (h >> 31)
    };
    let a = mix(mix(mix(0x6b69_6c6e_656e_7469, world_seed as u64), game_time as u64), id as u64);
    let b = mix(mix(a, 0x9E37_79B9_7F4A_7C15), id as u64);
    let hi = (a & !0xF000) | 0x4000;
    let lo = (b & !(0xC000 << 48)) | (0x8000 << 48);
    Uuid::from_u64_pair(hi, lo)
}

/// A seed for a loaded entity's own random, from its UUID.
pub(crate) fn seed_for_uuid(uuid: u128) -> i64 {
    seed_for((uuid as u64 ^ (uuid >> 64) as u64) as i32) ^ (uuid >> 64) as i64
}

/// Puts spawns in the order ids are assigned in.
pub(crate) fn canonical(mut spawns: Vec<Spawn>) -> Vec<Spawn> {
    spawns.sort_by_key(Spawn::key);
    spawns
}

/// A seed for the entity's own random, from its id (so it does not depend on the regions).
fn seed_for(id: i32) -> i64 {
    let mut h = (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x6b69_6c6e_5eed;
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (h ^ (h >> 29)) as i64
}

fn vec3(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}

fn arr(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

impl Entity {
    /// The entity of `spawn` with network id `id`; `uuid` unless it was loaded with one.
    pub fn new(id: i32, uuid: Uuid, spawn: Spawn) -> Self {
        let uuid = match &spawn.body {
            Body::Loaded(e) if e.uuid != 0 => Uuid::from_u128(e.uuid),
            _ => uuid,
        };
        let (u, seed, pos) = (uuid.as_u128(), seed_for(id), vec3(spawn.pos));
        let phys = match spawn.body {
            Body::Item { stack, pickup_delay } => {
                let mut e = kiln_entity::item::new(id, u, stack, seed);
                e.set_pos(pos);
                e.set_old_pos_and_rot();
                e.delta = vec3(spawn.vel);
                if let EntityKind::Item(d) = &mut e.kind {
                    d.pickup_delay = pickup_delay;
                }
                e
            }
            Body::FallingBlock { state } => {
                kiln_entity::falling_block::fall(id, u, BlockPos::containing(pos.x, pos.y, pos.z), state, seed)
            }
            Body::Tnt => kiln_entity::tnt::ignite(id, u, pos, None, seed),
            Body::Ready(e) | Body::Loaded(e) => {
                let mut e = *e;
                e.id = id;
                e.uuid = u;
                e
            }
        };
        let (pos, vel, on_ground) = (arr(phys.position()), arr(phys.delta), phys.on_ground);
        let state = MoveState { pos, yaw: phys.y_rot, pitch: phys.x_rot, head_yaw: phys.y_rot, on_ground };
        Self {
            id,
            uuid,
            kind: spawn.kind,
            cell: chunk_of(pos).cell(),
            pos,
            vel,
            on_ground,
            age: 0,
            removed: false,
            phys: Some(phys),
            tracker: MovementTracker::new(id, spawn.kind.update_interval, &state),
            sent_vel: vel,
            seen_by: Vec::new(),
            section: None,
        }
    }

    /// The entity as saved in its chunk (`Entity.save`); `owners` resolves the network ids
    /// of owners to UUIDs.
    pub fn save(&self, owners: &dyn Fn(i32) -> Option<u128>) -> kiln_proto::nbt::Tag {
        kiln_entity::persist::save(self.phys(), owners)
    }

    fn phys(&self) -> &kiln_entity::Entity {
        self.phys.as_ref().expect("entity state is back after its tick")
    }

    /// Copies what the rest of the simulation reads from the vanilla state.
    fn sync(&mut self) {
        let p = self.phys();
        let (pos, vel, on_ground, removed) = (arr(p.position()), arr(p.delta), p.on_ground, p.is_removed());
        self.pos = pos;
        self.vel = vel;
        self.on_ground = on_ground;
        self.removed |= removed;
    }

    fn move_state(&self) -> MoveState {
        let p = self.phys();
        MoveState { pos: self.pos, yaw: p.y_rot, pitch: p.x_rot, head_yaw: p.y_rot, on_ground: self.on_ground }
    }

    fn metadata(&self) -> EntityData {
        let mut d = EntityData::new();
        match &self.phys().kind {
            EntityKind::Item(item) => {
                let mut bytes = bytes::BytesMut::new();
                item.stack.write_optional(&mut bytes);
                d.set(data::item_entity::ITEM, &DataValue::EncodedItemStack(bytes.freeze()));
            }
            EntityKind::Tnt(t) => {
                d.set(data::primed_tnt::FUSE, &DataValue::Int(t.fuse));
                d.set(data::primed_tnt::BLOCK_STATE, &DataValue::BlockState(t.block_state as i32));
            }
            EntityKind::FallingBlock(_) => {
                // `FallingBlockEntity.setStartPos`: where it fell from.
                let p = self.phys().old_pos;
                let start = BlockPos::containing(p.x, p.y, p.z);
                d.set(data::falling_block_entity::START_POS, &DataValue::BlockPos([start.x, start.y, start.z]));
            }
            EntityKind::ExperienceOrb(o) => {
                d.set(data::experience_orb::VALUE, &DataValue::Int(o.value));
            }
            _ => {}
        }
        d
    }

    /// Bundle that makes a new viewer see the entity.
    fn spawn_packets(&self) -> [Bytes; 4] {
        // A falling block's spawn data is its block state (`Block.getId`).
        let spawn_data = match &self.phys().kind {
            EntityKind::FallingBlock(f) => f.state as i32,
            _ => 0,
        };
        [
            entity::bundle_delimiter(),
            self.tracker.spawn(self.uuid, self.kind.id, self.vel, spawn_data),
            entity::set_entity_data(self.id, &self.metadata()),
            entity::bundle_delimiter(),
        ]
    }

    /// The bounding box, and whether it keeps blocks from being placed into it
    /// (`Entity.blocksBuilding`: primed TNT and falling blocks).
    pub fn body(&self) -> ([f64; 3], [f64; 3], bool) {
        let p = self.phys();
        let bb = p.bounding_box();
        let blocks_building = matches!(p.kind, EntityKind::Tnt(_) | EntityKind::FallingBlock(_));
        ([bb.min_x, bb.min_y, bb.min_z], [bb.max_x, bb.max_y, bb.max_z], blocks_building)
    }
}

/// A region's entities in id order, which is spawn order (vanilla's tick order is insertion
/// order); merges interleave by id, splits go by each entity's cell.
#[derive(Default)]
pub(crate) struct Entities {
    pub list: Vec<Entity>,
}

impl RegionPart for Entities {
    fn merge(into: &mut Self, from: Self) {
        let a = std::mem::take(&mut into.list);
        let mut out = Vec::with_capacity(a.len() + from.list.len());
        let (mut a, mut b) = (a.into_iter().peekable(), from.list.into_iter().peekable());
        loop {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.id < y.id,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            out.push(if take_a { a.next() } else { b.next() }.unwrap());
        }
        into.list = out;
    }

    fn split(self, owner_of: &dyn Fn(CellPos) -> usize, n: usize) -> SmallVec<[Self; 4]> {
        let mut parts: SmallVec<[Self; 4]> = (0..n).map(|_| Self::default()).collect();
        for e in self.list {
            parts[owner_of(e.cell)].list.push(e);
        }
        parts
    }

    fn count(&self) -> usize {
        self.list.len()
    }

    fn for_each_cell(&self, f: &mut dyn FnMut(CellPos)) {
        self.list.iter().for_each(|e| f(e.cell));
    }
}

pub(crate) fn chunk_of(pos: [f64; 3]) -> ChunkPos {
    ChunkPos::of_block(pos[0].floor() as i32, pos[2].floor() as i32)
}

fn kb(p: BlockPos) -> kiln_blocks::BlockPos {
    kiln_blocks::BlockPos::new(p.x, p.y, p.z)
}

/// The region as kiln-entity's world: blocks through kiln-blocks (so landing falling blocks
/// and explosions update their neighbours), the region's entities by id, and its players as
/// stand-ins that explosions can hurt and push.
struct SimLevel<'a, 'l> {
    level: &'a mut RegionLevel<'l>,
    list: &'a mut Vec<Entity>,
    /// Players as `Other` entities, in connection order.
    proxies: Vec<kiln_entity::Entity>,
    views: Vec<PlayerView>,
    spawns: &'a mut Vec<Spawn>,
    events: Vec<Event>,
    /// Ids for entities spawned during the tick until they get their real one.
    next_placeholder: i32,
    /// The entity being ticked and how many seeds it drew, for partition-independent seeds.
    current: i32,
    seeds: u64,
}

impl SimLevel<'_, '_> {
    fn index(&self, id: i32) -> Option<usize> {
        self.list.binary_search_by_key(&id, |e| e.id).ok()
    }
}

/// Vanilla iterates entity sections by x, then by the packed (z, y) section key.
fn section_key(e: &kiln_entity::Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl EntityLevel for SimLevel<'_, '_> {
    fn block(&self, pos: BlockPos) -> u16 {
        self.level.block(kb(pos))
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.level.is_loaded(kb(pos))
    }

    fn set_block(&mut self, pos: BlockPos, state: u16, flags: u32) -> bool {
        kiln_blocks::set_block(self.level, kb(pos), state, flags)
    }

    fn destroy_block(&mut self, pos: BlockPos, drop: bool) -> bool {
        kiln_blocks::destroy_block(self.level, kb(pos), drop, 512)
    }

    fn random(&mut self) -> &mut LegacyRandom {
        self.level.random()
    }

    fn game_time(&self) -> i64 {
        self.level.env.game_time
    }

    fn min_y(&self) -> i32 {
        self.level.env.min_y
    }

    fn max_y(&self) -> i32 {
        self.level.env.min_y + self.level.env.height - 1
    }

    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        let wanted = |e: &kiln_entity::Entity| {
            let kind = match filter {
                EntityFilter::Any => true,
                EntityFilter::Item => matches!(e.kind, EntityKind::Item(_)),
                EntityFilter::ExperienceOrb => matches!(e.kind, EntityKind::ExperienceOrb(_)),
                EntityFilter::Living => matches!(e.kind, EntityKind::Other { .. } | EntityKind::Player(_)),
            };
            kind && e.id != exclude && e.is_alive() && e.bounding_box().intersects(area)
        };
        // Within a section vanilla keeps insertion order, which id order follows here.
        let mut found: Vec<((i32, i64), i32)> = self
            .list
            .iter()
            .filter_map(|e| e.phys.as_ref())
            .chain(self.proxies.iter())
            .filter(|e| wanted(e))
            .map(|e| (section_key(e), e.id))
            .collect();
        found.sort_unstable();
        found.into_iter().map(|(_, id)| id).collect()
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut kiln_entity::Entity> {
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_mut();
        }
        self.proxies.iter_mut().find(|e| e.id == id)
    }

    fn entity(&self, id: i32) -> Option<&kiln_entity::Entity> {
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_ref();
        }
        self.proxies.iter().find(|e| e.id == id)
    }

    fn add_entity(&mut self, entity: kiln_entity::Entity) {
        let Some(kind) = kiln_data::entities::by_name(entity.type_name) else { return };
        self.spawns.push(Spawn { kind, pos: arr(entity.position()), vel: arr(entity.delta), body: Body::Ready(Box::new(entity)) });
    }

    fn next_entity_id(&mut self) -> i32 {
        self.next_placeholder -= 1;
        self.next_placeholder
    }

    fn fresh_seed(&mut self) -> i64 {
        self.seeds += 1;
        let mut h = (self.level.env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.level.env.seed as u64;
        for v in [self.current as u64, self.seeds] {
            h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        h as i64
    }

    fn players(&self) -> Vec<PlayerView> {
        self.views.clone()
    }

    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}

/// A player as the entities see it (`minecraft:player`, standing or sneaking).
fn proxy(p: &Player) -> kiln_entity::Entity {
    let mut e = kiln_entity::Entity::new("minecraft:player", p.entity_id, p.uuid.as_u128(), EntityKind::Other { type_name: "minecraft:player" }, 0);
    if p.sneaking {
        e.height = 1.5;
        e.eye_height = 1.27;
    }
    e.set_pos(vec3(p.pos));
    e.invulnerable = matches!(p.game_mode, 1 | 3);
    e
}

/// L8: ticks the region's entities in order (`commonTick` then `tick`, as `ServerLevel`
/// does), in chunks within simulation distance of a player; then carries out what they did:
/// sounds, particles, damage and knockback to players, explosion drops. Block changes go
/// through `level`, spawns to `spawns`.
pub(crate) fn tick(
    entities: &mut Entities,
    level: &mut RegionLevel,
    ticking: &blocks::Ticking,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
) {
    if entities.list.is_empty() {
        return;
    }
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views = players
        .iter()
        .filter(|p| live(p))
        .map(|p| PlayerView { id: p.entity_id, pos: vec3(p.pos), eye_height: if p.sneaking { 1.27 } else { 1.62 }, spectator: p.game_mode == 3 })
        .collect();
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: 0,
        seeds: 0,
    };
    for i in 0..sim.list.len() {
        let e = &mut sim.list[i];
        if e.removed || !ticking.contains(chunk_of(e.pos)) {
            continue;
        }
        e.age += 1;
        let Some(mut phys) = e.phys.take() else { continue };
        (sim.current, sim.seeds) = (phys.id, 0);
        if !phys.is_removed() {
            phys.common_tick();
            phys.tick(&mut sim);
        }
        let e = &mut sim.list[i];
        e.phys = Some(phys);
        e.sync();
        let cell = chunk_of(e.pos).cell();
        if sim.level.cells.cell(cell).is_some() {
            e.cell = cell;
        }
    }
    let SimLevel { level, list, proxies, events, spawns, .. } = sim;
    // Explosion knockback reaches the pushed player's client (it owns its movement).
    for pr in proxies.iter().filter(|e| e.delta != Vec3::ZERO) {
        if let Some(p) = players.iter_mut().find(|p| p.entity_id == pr.id)
            && !(p.game_mode == 1 && p.flying)
        {
            p.send(entity::set_entity_motion(p.entity_id, arr(pr.delta)));
        }
    }
    for (n, event) in events.into_iter().enumerate() {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
}

fn carry_out(
    event: Event,
    n: usize,
    level: &mut RegionLevel,
    list: &[Entity],
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
) {
    let env = level.env;
    match event {
        Event::Sound { pos, sound, source, volume, pitch } => {
            send_sound(players, env, n, arr(pos), sound, source_of(source), volume, pitch);
        }
        Event::LevelEvent { event, pos, data } => level.effect(Effect::LevelEvent { id: event, pos: kb(pos), data }),
        Event::BlockExploded { pos, state, .. } => level.effect(Effect::Drop { pos: kb(pos), state }),
        Event::Hurt { target, amount, kind, .. } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == target)
                && let Some(death) = p.hurt(amount, health::Cause::Entity(kind), spawns)
            {
                deaths.push(death);
            }
        }
        Event::EntityEvent { entity: id, event } => {
            if let Ok(i) = list.binary_search_by_key(&id, |e| e.id) {
                let pkt = entity::entity_event(id, event);
                for p in players.iter_mut().filter(|p| list[i].seen_by.binary_search(&p.conn).is_ok()) {
                    p.send(pkt.clone());
                }
            }
        }
        Event::Explosion { pos, power, .. } => {
            // `ServerLevel.explode` sends the explosion (sound and particle) to players within 64
            // blocks; the removed blocks reach clients as block updates.
            let at = arr(pos);
            let particle = if power < 2.0 { "minecraft:explosion" } else { "minecraft:explosion_emitter" };
            if let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", particle) {
                let pkt = world_fx::level_particles(&world_fx::LevelParticles {
                    particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
                    override_limiter: true,
                    always_show: false,
                    pos: at,
                    offset: [0.0; 3],
                    max_speed: [1.0, 0.0, 0.0],
                    count: 0,
                    randomization: world_fx::ParticleRandomization::Default,
                });
                for p in players.iter_mut().filter(|p| dist2(p.pos, at) < 64.0 * 64.0) {
                    p.send(pkt.clone());
                }
            }
            let r = level.random();
            let pitch = (1.0 + (r.next_float() - r.next_float()) * 0.2) * 0.7;
            send_sound(players, env, n, at, "minecraft:entity.generic.explode", world_fx::SoundSource::Blocks, 4.0, pitch);
        }
        // Vibrations, projectiles and the block effects of entities inside blocks (pressure
        // plates are pressed through the entity boxes) are not simulated yet.
        Event::GameEvent { .. } | Event::EntityInsideBlock { .. } | Event::ProjectileHit { .. } => {}
    }
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

fn source_of(name: &str) -> world_fx::SoundSource {
    use world_fx::SoundSource as S;
    match name {
        "master" => S::Master,
        "music" => S::Music,
        "record" | "records" => S::Records,
        "weather" => S::Weather,
        "hostile" => S::Hostile,
        "neutral" => S::Neutral,
        "player" | "players" => S::Players,
        "ambient" => S::Ambient,
        "voice" => S::Voice,
        "ui" => S::Ui,
        _ => S::Blocks,
    }
}

#[allow(clippy::too_many_arguments)]
fn send_sound(
    players: &mut [&mut Player],
    env: &blocks::BlockEnv,
    n: usize,
    at: [f64; 3],
    sound: &str,
    source: world_fx::SoundSource,
    volume: f32,
    pitch: f32,
) {
    let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
    let mut seed = (env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.seed as u64;
    for v in [at[0].to_bits(), at[1].to_bits(), at[2].to_bits(), n as u64] {
        seed = (seed ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        seed ^= seed >> 31;
    }
    let pkt = world_fx::sound(&world_fx::Sound::Registered(id), source, at, volume, pitch, seed as i64);
    let range = 16.0 * volume.max(1.0) as f64;
    for p in players.iter_mut().filter(|p| dist2(p.pos, at) < range * range) {
        p.send(pkt.clone());
    }
}

/// Players touching items that can be picked up take them into their inventory. `players`
/// is the region's, sorted by connection.
pub(crate) fn pickups(entities: &mut Entities, players: &mut [&mut Player]) {
    for e in &mut entities.list {
        if e.removed {
            continue;
        }
        let (lo, hi, _) = e.body();
        let Some(EntityKind::Item(item)) = e.phys.as_mut().map(|p| &mut p.kind) else { continue };
        if item.pickup_delay > 0 {
            continue;
        }
        // The player's box inflated by (1, 0.5, 1), as in vanilla's player `aiStep`.
        let touching = |p: &Player| {
            let pmin = [p.pos[0] - 1.3, p.pos[1] - 0.5, p.pos[2] - 1.3];
            let pmax = [p.pos[0] + 1.3, p.pos[1] + 2.3, p.pos[2] + 1.3];
            (0..3).all(|i| pmin[i] < hi[i] && pmax[i] > lo[i])
        };
        let Some(i) = players.iter().position(|p| !p.disconnected && !p.dead && p.game_mode != 3 && touching(p)) else {
            continue;
        };
        let taken = players[i].add_to_inventory(&mut item.stack);
        if taken == 0 {
            continue;
        }
        let pkt = entity::take_item_entity(e.id, players[i].entity_id, taken);
        players[i].send(pkt.clone());
        for v in &e.seen_by {
            if let Ok(j) = players.binary_search_by_key(v, |q| q.conn)
                && j != i
            {
                players[j].send(pkt.clone());
            }
        }
        if item.stack.is_empty() {
            e.removed = true;
            if let Some(p) = e.phys.as_mut() {
                p.discard();
            }
        } else {
            // The remaining count reaches the viewers.
            let pkt = entity::set_entity_data(e.id, &e.metadata());
            for v in &e.seen_by {
                if let Ok(j) = players.binary_search_by_key(v, |q| q.conn) {
                    players[j].send(pkt.clone());
                }
            }
        }
    }
}

/// Tracks the region's entities to its players (sorted by connection) with vanilla's
/// triggers: an entity whose section changed is re-evaluated against every player, and every
/// entity against the players in `movers` (whose section changed). Then sends movement and
/// velocity, and removes dead entities from their viewers and the region.
pub(crate) fn track(entities: &mut Entities, players: &mut [&mut Player], movers: &[ConnId]) {
    let sees = |p: &Player, e: &Entity| {
        let range = (e.kind.tracking_range as f64 * 16.0).min(p.view_distance as f64 * 16.0);
        let (dx, dz) = (p.pos[0] - e.pos[0], p.pos[2] - e.pos[2]);
        let (pc, ec) = (chunk_of(p.pos), chunk_of(e.pos));
        !e.removed
            && dx * dx + dz * dz <= range * range
            && (pc.x - ec.x).abs() <= p.view_distance
            && (pc.z - ec.z).abs() <= p.view_distance
    };
    let index = |players: &[&mut Player], conn: ConnId| players.binary_search_by_key(&conn, |p| p.conn).ok();
    for e in &mut entities.list {
        let section = e.pos.map(|c| c.floor() as i32 >> 4);
        let moved = e.section != Some(section);
        e.section = Some(section);
        let (mut added, mut removed) = (Vec::new(), Vec::new());
        let mut check = |p: &Player, seen: bool| match (sees(p, e), seen) {
            (true, false) => added.push(p.conn),
            (false, true) => removed.push(p.conn),
            _ => {}
        };
        if moved || e.removed {
            for p in players.iter() {
                check(p, e.seen_by.binary_search(&p.conn).is_ok());
            }
        } else {
            for &m in movers {
                if let Some(i) = index(players, m) {
                    check(players[i], e.seen_by.binary_search(&m).is_ok());
                }
            }
        }
        if !added.is_empty() {
            let spawn = e.spawn_packets();
            for &c in &added {
                if let Some(i) = index(players, c) {
                    spawn.iter().for_each(|pkt| players[i].send(pkt.clone()));
                }
            }
        }
        if !removed.is_empty() {
            let despawn = entity::remove_entities(&[e.id]);
            for &c in &removed {
                if let Some(i) = index(players, c) {
                    players[i].send(despawn.clone());
                }
            }
        }
        e.seen_by.retain(|v| removed.binary_search(v).is_err());
        e.seen_by.extend(added);
        e.seen_by.sort_unstable();
        // Players that left the region or the game are no longer viewers.
        e.seen_by.retain(|&v| index(players, v).is_some());

        if e.removed {
            continue;
        }
        let mut packets = e.tracker.tick(&e.move_state());
        // `ServerEntity.sendChanges`: velocity on update ticks when it changed, or at once
        // after an impulse (explosion knockback).
        let impulse = e.phys.as_mut().is_some_and(|p| std::mem::take(&mut p.needs_sync));
        if e.kind.track_deltas && (impulse || e.age % e.kind.update_interval.max(1) == 0) {
            let d: f64 = (0..3).map(|i| (e.vel[i] - e.sent_vel[i]).powi(2)).sum();
            let still = e.vel.iter().all(|&v| v == 0.0);
            if impulse || d > 1.0e-7 || (d > 0.0 && still) {
                e.sent_vel = e.vel;
                packets.push(entity::set_entity_motion(e.id, e.vel));
            }
        }
        for v in &e.seen_by {
            if let Some(i) = index(players, *v) {
                packets.iter().for_each(|pkt| players[i].send(pkt.clone()));
            }
        }
    }
    entities.list.retain(|e| !e.removed);
}

/// Health damage cause for an entity damage kind.
pub(crate) fn damage_type(kind: DamageKind) -> (&'static str, &'static str) {
    match kind {
        DamageKind::OnFire => ("minecraft:on_fire", "death.attack.onFire"),
        DamageKind::InFire => ("minecraft:in_fire", "death.attack.inFire"),
        DamageKind::Lava => ("minecraft:lava", "death.attack.lava"),
        DamageKind::FallingBlock => ("minecraft:falling_block", "death.attack.fallingBlock"),
        DamageKind::FallingAnvil => ("minecraft:falling_anvil", "death.attack.anvil"),
        DamageKind::FallingStalactite => ("minecraft:falling_stalactite", "death.attack.fallingStalactite"),
        DamageKind::Explosion => ("minecraft:explosion", "death.attack.explosion"),
        DamageKind::Cactus => ("minecraft:cactus", "death.attack.cactus"),
        DamageKind::SweetBerryBush => ("minecraft:sweet_berry_bush", "death.attack.sweetBerryBush"),
        DamageKind::HotFloor => ("minecraft:hot_floor", "death.attack.hotFloor"),
        DamageKind::Freeze => ("minecraft:freeze", "death.attack.freeze"),
        DamageKind::Arrow => ("minecraft:arrow", "death.attack.arrow"),
        DamageKind::Thrown => ("minecraft:thrown", "death.attack.thrown"),
        DamageKind::Generic => ("minecraft:generic", "death.attack.generic"),
    }
}
