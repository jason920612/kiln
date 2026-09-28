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
use std::collections::HashMap;
use uuid::Uuid;

/// Ticks an item waits before it can be picked up after a player drops it.
pub(crate) const DROP_PICKUP_DELAY: i32 = 40;

/// What a spawn becomes.
pub(crate) enum Body {
    /// `thrower`: the player who dropped it with the drop key (`ItemEntity.setThrower`).
    Item { stack: kiln_item::ItemStack, pickup_delay: i32, thrower: Option<u128> },
    /// `FallingBlockEntity.fall` of `state` from the block at the spawn position.
    FallingBlock { state: u16 },
    /// `TntBlock.prime`: a primed TNT with vanilla's random hop.
    Tnt,
    /// An entity kiln-entity built during a tick (its id is replaced by the assigned one).
    Ready(Box<kiln_entity::Entity>),
    /// An entity loaded from its chunk's saved data; keeps its UUID (unless it had none).
    Loaded(Box<kiln_entity::Entity>),
    /// A new mob facing `yaw`; `finalize` runs its `finalizeSpawn`.
    /// `yaw`: `None` keeps the constructor's random yaw.
    Mob { kind: kiln_entity::mob::MobKind, yaw: Option<f32>, finalize: Option<crate::mobs::Finalize> },
    /// A lightning bolt (`visual_only`: a skeleton trap's, which hurts nothing).
    Lightning { visual_only: bool },
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
    /// A mob's entity data and equipment as its viewers last got them.
    meta_sent: Vec<u8>,
    /// Passengers as viewers last got them (Set Passengers).
    passengers_sent: Vec<i32>,
    equipment_sent: Vec<(u8, kiln_item::ItemStack)>,
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
            Body::Item { stack, pickup_delay, thrower } => {
                let mut e = kiln_entity::item::new(id, u, stack, seed);
                e.set_pos(pos);
                e.set_old_pos_and_rot();
                e.delta = vec3(spawn.vel);
                if let EntityKind::Item(d) = &mut e.kind {
                    d.pickup_delay = pickup_delay;
                    d.thrower = thrower;
                }
                e
            }
            Body::FallingBlock { state } => {
                kiln_entity::falling_block::fall(id, u, BlockPos::containing(pos.x, pos.y, pos.z), state, seed)
            }
            Body::Tnt => kiln_entity::tnt::ignite(id, u, pos, None, seed),
            Body::Lightning { visual_only } => kiln_entity::ext_entity::lightning::new(id, u, pos, visual_only, seed),
            Body::Mob { kind, yaw, finalize } => {
                let mut e = kiln_entity::mob::new(kind, id, u, seed);
                e.set_pos(pos);
                let yaw = yaw.unwrap_or(e.y_rot);
                e.y_rot = yaw;
                e.set_old_pos_and_rot();
                if let Some(m) = kiln_entity::mob::data_mut(&mut e) {
                    m.y_head_rot = yaw;
                    m.y_body_rot = yaw;
                    m.y_head_rot_o = yaw;
                    m.y_body_rot_o = yaw;
                }
                if let Some(f) = finalize {
                    let mut r = LegacyRandom::new(f.seed);
                    kiln_entity::mob::finalize_spawn(&mut e, &mut r, &f.ctx, &mut kiln_entity::mob::GroupData::default(), true);
                    if f.persistent
                        && let Some(m) = kiln_entity::mob::data_mut(&mut e)
                    {
                        m.persistence_required = true;
                    }
                }
                e
            }
            Body::Ready(e) | Body::Loaded(e) => {
                let mut e = *e;
                e.id = id;
                e.uuid = u;
                e
            }
        };
        let (pos, vel, on_ground) = (arr(phys.position()), arr(phys.delta), phys.on_ground);
        let head_yaw = kiln_entity::mob::data(&phys).map_or(phys.y_rot, |m| m.y_head_rot);
        let state = MoveState { pos, yaw: phys.y_rot, pitch: phys.x_rot, head_yaw, on_ground };
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
            meta_sent: Vec::new(),
            passengers_sent: Vec::new(),
            equipment_sent: Vec::new(),
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
    pub(crate) fn sync(&mut self) {
        let p = self.phys();
        let (pos, vel, on_ground, removed) = (arr(p.position()), arr(p.delta), p.on_ground, p.is_removed());
        self.pos = pos;
        self.vel = vel;
        self.on_ground = on_ground;
        self.removed |= removed;
    }

    fn move_state(&self) -> MoveState {
        let p = self.phys();
        let head_yaw = kiln_entity::mob::data(p).map_or(p.y_rot, |m| m.y_head_rot);
        MoveState { pos: self.pos, yaw: p.y_rot, pitch: p.x_rot, head_yaw, on_ground: self.on_ground }
    }

    pub(crate) fn metadata(&self) -> EntityData {
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
            EntityKind::Mob(m) => return crate::mobs::metadata(self.phys(), m),
            EntityKind::Ext(x) => x.entity_data(self.phys(), &mut d),
            _ => {}
        }
        d
    }

    /// Bundle that makes a new viewer see the entity.
    fn spawn_packets(&self) -> Vec<Bytes> {
        // A falling block's spawn data is its block state (`Block.getId`).
        let spawn_data = match &self.phys().kind {
            EntityKind::FallingBlock(f) => f.state as i32,
            EntityKind::Ext(x) => x.spawn_data(),
            _ => 0,
        };
        let mut out = vec![
            entity::bundle_delimiter(),
            self.tracker.spawn(self.uuid, self.kind.id, self.vel, spawn_data),
            entity::set_entity_data(self.id, &self.metadata()),
        ];
        // `ServerEntity.sendPairingData`: a mob's equipment.
        if let EntityKind::Mob(m) = &self.phys().kind {
            let worn = crate::mobs::shown_equipment(m);
            if !worn.is_empty() {
                let slots: Vec<(u8, &kiln_item::ItemStack)> = worn.iter().map(|(i, s)| (*i, s)).collect();
                out.push(crate::players::set_equipment(self.id, &slots));
            }
        }
        if !self.phys().passengers.is_empty() {
            out.push(entity::set_passengers(self.id, &self.phys().passengers));
        }
        out.push(entity::bundle_delimiter());
        out
    }

    /// The bounding box, and whether it keeps blocks from being placed into it
    /// (`Entity.blocksBuilding`: primed TNT and falling blocks).
    /// `Monster.isPreventingPlayerRest`: monsters do, zombified piglins only while angry.
    pub fn prevents_rest(&self) -> bool {
        let Some(m) = self.phys.as_ref().and_then(kiln_entity::mob::data) else { return false };
        if m.health <= 0.0 || m.kind.category() != kiln_entity::mob::Category::Monster {
            return false;
        }
        m.kind != kiln_entity::mob::MobKind::ZombifiedPiglin || m.target.is_some()
    }

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
struct SimLevel<'a, 'l, 'p> {
    level: &'a mut RegionLevel<'l>,
    list: &'a mut Vec<Entity>,
    /// The region's players, which mobs hurt directly.
    players: &'a mut [&'p mut Player],
    deaths: &'a mut Vec<health::Death>,
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
    /// The level random as the ticking entity sees it: seeded per entity and tick, so what
    /// one entity draws (explosions, experience orbs) does not depend on the others in its
    /// region (vanilla shares one random per level).
    rng: LegacyRandom,
    /// Entity sections (16³) → indices in `list`, for area queries.
    grid: Grid,
    /// Player entity id → index in `proxies` / `views` ([`SimLevel::index_players`]; mobs look
    /// players up several times a tick, and a crowd has a thousand).
    proxy_at: HashMap<i32, usize>,
    view_at: HashMap<i32, usize>,
    /// Player stand-ins by entity section (like `grid`), so area queries skip far players.
    proxy_grid: HashMap<(i32, i32, i32), Vec<usize>>,
}

/// Entities by section, like vanilla's `EntitySectionStorage`.
#[derive(Default)]
struct Grid {
    cells: std::collections::HashMap<(i32, i32, i32), Vec<usize>>,
    at: Vec<(i32, i32, i32)>,
}

fn section_of(p: [f64; 3]) -> (i32, i32, i32) {
    ((p[0].floor() as i32) >> 4, (p[1].floor() as i32) >> 4, (p[2].floor() as i32) >> 4)
}

impl Grid {
    fn build(list: &[Entity]) -> Grid {
        let mut g = Grid { cells: Default::default(), at: Vec::with_capacity(list.len()) };
        for (i, e) in list.iter().enumerate() {
            let s = section_of(e.pos);
            g.cells.entry(s).or_default().push(i);
            g.at.push(s);
        }
        g
    }

    /// Entity `i` is now at `pos`.
    fn moved(&mut self, i: usize, pos: [f64; 3]) {
        let s = section_of(pos);
        let old = self.at[i];
        if s != old {
            if let Some(v) = self.cells.get_mut(&old) {
                v.retain(|&j| j != i);
            }
            self.cells.entry(s).or_default().push(i);
            self.at[i] = s;
        }
    }
}

/// The level random stand-in for entity `id` this tick.
fn entity_level_random(seed: i64, game_time: i64, id: i32) -> LegacyRandom {
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (game_time as u64) ^ 0x6c65_7665_6c;
    h = (h ^ id as u32 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    LegacyRandom::new(h as i64)
}

impl SimLevel<'_, '_, '_> {
    fn index(&self, id: i32) -> Option<usize> {
        self.list.binary_search_by_key(&id, |e| e.id).ok()
    }

    fn index_players(&mut self) {
        self.proxy_at = self.proxies.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        self.view_at = self.views.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
        self.proxy_grid.clear();
        for (i, e) in self.proxies.iter().enumerate() {
            let p = e.position();
            self.proxy_grid.entry(section_of([p.x, p.y, p.z])).or_default().push(i);
        }
    }

    fn proxy_index(&self, id: i32) -> Option<usize> {
        if self.proxy_at.is_empty() { self.proxies.iter().position(|e| e.id == id) } else { self.proxy_at.get(&id).copied() }
    }
}

/// Vanilla iterates entity sections by x, then by the packed (z, y) section key.
fn section_key(e: &kiln_entity::Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl EntityLevel for SimLevel<'_, '_, '_> {
    fn piglins_zombify(&self) -> bool {
        !self.level.env.rules.fast_lava
    }

    fn trade_offers(&mut self, set: &str, merchant: &kiln_entity::level::TradeMerchant) -> Vec<kiln_item::trading::MerchantOffer> {
        let env = self.level.env;
        crate::trading::roll_offers(env.loot.as_deref(), env.seed, env.game_time, set, merchant)
    }

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
        &mut self.rng
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
                EntityFilter::Living => matches!(e.kind, EntityKind::Other { .. } | EntityKind::Player(_) | EntityKind::Mob(_)),
            };
            kind && e.id != exclude && e.is_alive() && e.bounding_box().intersects(area)
        };
        // Within a section vanilla keeps insertion order, which id order follows here. The
        // sections within 2 blocks of the area hold every entity whose box can touch it.
        let lo = section_of([area.min_x - 2.0, area.min_y - 2.0, area.min_z - 2.0]);
        let hi = section_of([area.max_x + 2.0, area.max_y + 2.0, area.max_z + 2.0]);
        let mut found: Vec<((i32, i64), i32)> = Vec::new();
        let span = (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64 * (hi.2 - lo.2 + 1) as i64;
        if span > self.grid.cells.len() as i64 * 4 {
            found.extend(self.list.iter().filter_map(|e| e.phys.as_ref()).filter(|e| wanted(e)).map(|e| (section_key(e), e.id)));
        } else {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        let Some(v) = self.grid.cells.get(&(x, y, z)) else { continue };
                        for &i in v {
                            if let Some(e) = self.list.get(i).and_then(|e| e.phys.as_ref())
                                && wanted(e)
                            {
                                found.push((section_key(e), e.id));
                            }
                        }
                    }
                }
            }
        }
        if self.proxy_grid.is_empty() || span > self.proxy_grid.len() as i64 * 4 {
            found.extend(self.proxies.iter().filter(|e| wanted(e)).map(|e| (section_key(e), e.id)));
        } else {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        let Some(v) = self.proxy_grid.get(&(x, y, z)) else { continue };
                        found.extend(v.iter().map(|&i| &self.proxies[i]).filter(|e| wanted(e)).map(|e| (section_key(e), e.id)));
                    }
                }
            }
        }
        found.sort_unstable();
        found.into_iter().map(|(_, id)| id).collect()
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut kiln_entity::Entity> {
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_mut();
        }
        let i = self.proxy_index(id)?;
        self.proxies.get_mut(i)
    }

    fn entity(&self, id: i32) -> Option<&kiln_entity::Entity> {
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_ref();
        }
        let i = self.proxy_index(id)?;
        self.proxies.get(i)
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

    fn players(&self) -> &[PlayerView] {
        &self.views
    }

    fn player(&self, id: i32) -> Option<PlayerView> {
        if self.view_at.is_empty() {
            return self.views.iter().find(|p| p.id == id).copied();
        }
        self.view_at.get(&id).map(|&i| self.views[i])
    }

    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }

    fn mob_griefing(&self) -> bool {
        self.level.env.mobs.griefing
    }

    fn mob_drops(&self) -> bool {
        self.level.env.mobs.drops
    }

    fn difficulty(&self) -> u8 {
        self.level.env.mobs.difficulty
    }

    fn sky_darken(&self) -> i32 {
        self.level.env.mobs.sky_darken
    }

    fn is_raining_at(&self, pos: BlockPos) -> bool {
        crate::weather::is_raining_at(self.level.cells, self.level.env, kb(pos))
    }

    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        kiln_blocks::Level::can_spread_fire_around(&*self.level, kb(pos))
    }

    fn place_lightning_fire(&mut self, pos: BlockPos) -> bool {
        crate::weather::place_lightning_fire(&mut *self.level, kb(pos))
    }

    fn lightning_strike_block(&mut self, pos: BlockPos) {
        kiln_blocks::weather::lightning_strike(&mut *self.level, kb(pos));
    }

    fn thunder_hit_player(&mut self, id: i32) {
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id) else { return };
        // `Entity.thunderHit`: one more tick of fire, 8 seconds if that made it 0.
        let ticks = p.fire_ticks + 1;
        p.set_fire_ticks(if ticks == 0 { 160 } else { ticks });
        let source = health::Source { cause: health::Cause::Entity(DamageKind::LightningBolt), attacker: None, direct: None, weapon: None };
        let env = self.level.env;
        let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
        p.hurt(5.0, &source, &mut ctx);
    }

    fn monsters_burn(&self) -> bool {
        self.level.env.mobs.monsters_burn
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        kiln_blocks::Level::raw_brightness(&*self.level, kb(pos), sky_darken)
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        kiln_world::light::light_at(&*self.level.cells, kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z)
            .map_or(if pos.y >= self.level.env.min_y + self.level.env.height { 15 } else { 0 }, i32::from)
    }

    fn effective_difficulty(&self, _pos: BlockPos) -> f32 {
        crate::mobs::difficulty_instance(self.level.env.mobs.difficulty, self.level.env.game_time, 0, 1.0).effective_difficulty
    }

    fn hurt_player(&mut self, id: i32, source: kiln_entity::mob::DamageSource, amount: f32) -> bool {
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id) else { return false };
        let attacker = source.attacker.and_then(|a| {
            let e = self.list.binary_search_by_key(&a, |e| e.id).ok().and_then(|i| self.list[i].phys.as_ref())?;
            Some(health::Attacker::mob(a, e.type_name, arr(e.position())))
        });
        let source = health::Source { cause: health::Cause::Entity(source.kind), attacker, direct: source.direct.filter(|d| Some(*d) != source.attacker), weapon: None };
        let env = self.level.env;
        let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
        let hurt = p.hurt(amount, &source, &mut ctx);
        if hurt && let Some(a) = source.attacker.as_ref().map(|a| a.id) {
            p.last_hurt_by_mob = Some((a, env.game_time));
        }
        hurt
    }

    fn add_effect_instance(&mut self, id: i32, effect: kiln_entity::effect::Effect, source: Option<i32>) -> bool {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id) {
            return p.add_effect(effect);
        }
        kiln_entity::mob::effects::add_to_entity(self, id, effect, source)
    }

    fn apply_instantaneous_effect(&mut self, id: i32, effect: &kiln_entity::effect::Effect, source: Option<(i32, Vec3)>, owner: Option<i32>, scale: f64) {
        let owner_is_player = owner.is_some_and(|o| self.players.iter().any(|p| p.entity_id == o));
        if !self.players.iter().any(|p| p.entity_id == id) {
            kiln_entity::mob::effects::apply_instantaneous_to_entity(self, id, effect, source, owner, owner_is_player, scale);
            return;
        }
        // `HealOrHarmMobEffect.applyInstantaneousEffect` on a player: healing, or indirect magic
        // caused by the thrower (magic when nothing carried it).
        let attacker = owner.and_then(|o| {
            if let Some(p) = self.players.iter().find(|p| p.entity_id == o) {
                return Some(p.as_attacker());
            }
            let e = self.list.binary_search_by_key(&o, |e| e.id).ok().and_then(|i| self.list[i].phys.as_ref())?;
            Some(health::Attacker::mob(o, e.type_name, arr(e.position())))
        });
        let env = self.level.env;
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id) else { return };
        match effect.kind() {
            kiln_entity::effect::Kind::HealOrHarm { harm: false } => {
                p.heal((scale * 4i32.wrapping_shl(effect.amplifier as u32) as f64 + 0.5) as i32 as f32);
            }
            kiln_entity::effect::Kind::HealOrHarm { harm: true } => {
                let amount = (scale * 6i32.wrapping_shl(effect.amplifier as u32) as f64 + 0.5) as i32 as f32;
                let source = match source {
                    None => health::Source { cause: health::Cause::Entity(DamageKind::Magic), attacker: None, direct: None, weapon: None },
                    Some((direct, _)) => health::Source { cause: health::Cause::Entity(DamageKind::IndirectMagic), attacker, direct: Some(direct), weapon: None },
                };
                let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
                p.hurt(amount, &source, &mut ctx);
            }
            kiln_entity::effect::Kind::Saturation => p.eat(effect.amplifier + 1, (effect.amplifier + 1) as f32 * 2.0),
            _ => {}
        }
    }

    fn max_entity_cramming(&self) -> i32 {
        self.level.env.mobs.cramming
    }

    fn ignite(&mut self, id: i32, seconds: f32) {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id) {
            let ticks = kiln_javamath::math::floor_f32(seconds * 20.0);
            if p.fire_ticks < ticks {
                p.set_fire_ticks(ticks);
            }
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            e.ignite_for_seconds(seconds);
        }
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick(
    entities: &mut Entities,
    level: &mut RegionLevel,
    ticking: &blocks::Ticking,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    any_player: bool,
) {
    if entities.list.is_empty() && players.iter().all(|p| p.vehicle.is_none()) {
        return;
    }
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: 0,
        seeds: 0,
        rng: LegacyRandom::new(0),
        grid: Grid::default(),
        proxy_at: HashMap::new(),
        view_at: HashMap::new(),
        proxy_grid: HashMap::new(),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    for i in 0..sim.list.len() {
        // Passengers tick right after their vehicle (`ServerLevel.tickPassenger`).
        let vehicle = sim.list[i].phys.as_ref().and_then(|p| p.vehicle);
        if vehicle.is_some_and(|v| sim.index(v).is_some_and(|j| !sim.list[j].removed && sim.list[j].phys.as_ref().is_some_and(|p| p.passengers.contains(&sim.list[i].id)))) {
            continue;
        }
        let me = sim.list[i].id;
        if let Some(phys) = sim.list[i].phys.as_mut()
            && let Some(v) = phys.vehicle.take()
            && let Some(j) = sim.index(v)
            && let Some(vp) = sim.list[j].phys.as_mut()
        {
            kiln_entity::ride::remove_passenger(vp, me);
        }
        tick_entity(&mut sim, i, ticking, any_player, None);
        let passengers = sim.list[i].phys.as_ref().map(|p| p.passengers.clone()).unwrap_or_default();
        for id in passengers {
            if let Some(j) = sim.index(id) {
                tick_entity(&mut sim, j, ticking, any_player, Some(i));
            }
        }
    }
    // `Player.aiStep` → `touch`: mobs in the player's box inflated by (1, 0.5, 1) (slimes and
    // magma cubes hurt the player). Vanilla runs it in the player's tick; here after the
    // entities'.
    let touchers: Vec<(i32, Aabb)> = sim
        .views
        .iter()
        .filter(|v| v.alive && !v.spectator)
        .map(|v| {
            let h = if v.sneaking { 1.5 } else { 1.8 };
            (v.id, Aabb::new(v.pos.x - 0.3, v.pos.y, v.pos.z - 0.3, v.pos.x + 0.3, v.pos.y + h, v.pos.z + 0.3).inflate(1.0, 0.5, 1.0))
        })
        .collect();
    for (pid, area) in touchers {
        for id in sim.entities_in(&area, EntityFilter::Living, pid) {
            let Some(i) = sim.index(id) else { continue };
            let Some(mut phys) = sim.list[i].phys.take() else { continue };
            if matches!(phys.kind, EntityKind::Mob(_)) && !phys.is_removed() {
                kiln_entity::mob::player_touch(&mut phys, &mut sim, pid);
            }
            sim.list[i].phys = Some(phys);
        }
    }
    ride_players(&mut sim);
    let SimLevel { level, list, proxies, events, spawns, players, deaths, .. } = sim;
    // Explosion knockback reaches the pushed player's client (it owns its movement).
    for pr in proxies.iter().filter(|e| e.delta != Vec3::ZERO) {
        if let Some(p) = players.iter_mut().find(|p| p.entity_id == pr.id)
            && !(p.game_mode == 1 && p.flying)
        {
            p.send(entity::set_entity_motion(p.entity_id, arr(pr.delta)));
        }
    }
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
}

/// One entity's tick in the entity phase (`tickNonPassenger`, or `tickPassenger` on the
/// vehicle at index `vehicle`, which has just ticked).
fn tick_entity(sim: &mut SimLevel, i: usize, ticking: &blocks::Ticking, any_player: bool, vehicle: Option<usize>) {
    let e = &mut sim.list[i];
    if e.removed || !ticking.contains(chunk_of(e.pos)) {
        return;
    }
    e.age += 1;
    let Some(mut phys) = e.phys.take() else { return };
    (sim.current, sim.seeds) = (phys.id, 0);
    sim.rng = entity_level_random(sim.level.env.seed, sim.level.env.game_time, phys.id);
    // `Mob.checkDespawn` runs before the tick, against the nearest player (regions are
    // farther apart than the despawn distance, so the region's players decide).
    if matches!(phys.kind, EntityKind::Mob(_)) && !phys.is_removed() {
        let p = phys.position();
        let nearest = sim.views.iter().filter(|v| !v.spectator).map(|v| v.pos.distance_to_sqr(p)).min_by(|a, b| a.total_cmp(b));
        kiln_entity::mob::check_despawn(&mut phys, &*sim, nearest.or(any_player.then_some(f64::MAX)));
    }
    if !phys.is_removed() {
        phys.common_tick();
        match vehicle.and_then(|v| sim.list[v].phys.clone()) {
            Some(v) => kiln_entity::ride::ride_tick(&mut phys, sim, &v),
            None => phys.tick(sim),
        }
    }
    let e = &mut sim.list[i];
    e.phys = Some(phys);
    e.sync();
    let pos = e.pos;
    sim.grid.moved(i, pos);
    let e = &mut sim.list[i];
    let cell = chunk_of(e.pos).cell();
    if sim.level.cells.cell(cell).is_some() {
        e.cell = cell;
    }
}

/// `Player.rideTick` for the region's riding players, after the entities ticked: a sneaking
/// player (or one whose mount is gone or threw it off) gets off at the mount's dismount
/// location; the others sit where the mount carries them.
fn ride_players(sim: &mut SimLevel) {
    let now = sim.level.env.game_time;
    for k in 0..sim.players.len() {
        let Some(v) = sim.players[k].vehicle else { continue };
        let pid = sim.players[k].entity_id;
        let idx = sim.index(v).filter(|&j| !sim.list[j].removed && sim.list[j].phys.as_ref().is_some_and(|p| p.is_alive()));
        let seated = idx.is_some_and(|j| sim.list[j].phys.as_ref().is_some_and(|p| p.passengers.contains(&pid)));
        let p = &*sim.players[k];
        let leave = !seated || p.sneaking || p.dead || p.disconnected;
        if !leave {
            let vp = sim.list[idx.unwrap()].phys.as_ref().expect("vehicle state");
            let at = vp.passengers.iter().position(|&x| x == pid).unwrap_or(0);
            let pos = kiln_entity::ride::rider_position(vp, at, "minecraft:player", 1.0);
            let p = &mut *sim.players[k];
            p.pos = arr(pos);
            p.fall_distance = 0.0;
            p.vel = [0.0; 3];
            continue;
        }
        // `stopRiding` → `dismountVehicle`.
        let mut to = sim.players[k].pos;
        if let Some(j) = idx {
            if let Some(vp) = sim.list[j].phys.as_mut() {
                kiln_entity::ride::remove_passenger(vp, pid);
            }
            let vp = sim.list[j].phys.clone().expect("vehicle state");
            let height = if sim.players[k].sneaking { 1.5 } else { 1.8 };
            to = arr(kiln_entity::ride::dismount_location(&*sim, &vp, 0.6, height));
        }
        let p = &mut *sim.players[k];
        p.vehicle = None;
        p.vehicle_type = None;
        if !p.disconnected {
            let rot = p.rot;
            p.teleport(to, rot, now);
        }
    }
}

/// `ServerGamePacketListenerImpl.handleMoveVehicle`: player `i` moves the mount it steers to
/// where its client put it (the mount's own tick leaves it still).
pub(crate) fn move_vehicle(entities: &mut Entities, players: &mut [&mut Player], i: usize, pos: [f64; 3], rot: [f32; 2], on_ground: bool, now: i64) {
    let p = &*players[i];
    let Some(v) = p.vehicle else { return };
    if pos.iter().any(|c| c.is_nan()) || rot.iter().any(|r| !r.is_finite()) {
        return;
    }
    let Ok(idx) = entities.list.binary_search_by_key(&v, |e| e.id) else { return };
    let view = view(p, now);
    let Some(phys) = entities.list[idx].phys.as_mut() else { return };
    let steers = phys.passengers.first() == Some(&p.entity_id)
        && kiln_entity::mob::data(phys).is_some_and(|m| m.kind.ext().is_some_and(|k| k.steerable_by(m, &view)));
    if !steers {
        return;
    }
    // `moved too quickly`: more than 10 blocks from where the mount was.
    let old = phys.position();
    let to = Vec3::new(pos[0].clamp(-3.0e7, 3.0e7), pos[1].clamp(-2.0e7, 2.0e7), pos[2].clamp(-3.0e7, 3.0e7));
    if to.distance_to_sqr(old) - phys.delta.length_sqr() > 100.0 {
        return;
    }
    phys.set_pos(to);
    phys.y_rot = kiln_entity::mob::mth::wrap_degrees(rot[0]);
    phys.x_rot = kiln_entity::mob::mth::wrap_degrees(rot[1]);
    phys.on_ground = on_ground;
    let yaw = phys.y_rot;
    if let Some(m) = kiln_entity::mob::data_mut(phys) {
        m.y_body_rot = yaw;
        m.y_head_rot = yaw;
    }
    let seat = kiln_entity::ride::rider_position(phys, 0, "minecraft:player", 1.0);
    let vehicle_type = phys.type_name;
    let in_lava = phys.is_in_lava();
    entities.list[idx].sync();
    let p = &mut *players[i];
    let d = [seat.x - p.pos[0], seat.y - p.pos[1], seat.z - p.pos[2]];
    p.riding_stats(d, vehicle_type);
    p.pos = arr(seat);
    // `trackEnteredOrExitedLavaOnVehicle`: `ride_entity_in_lava` from where the mount went in.
    if in_lava {
        match p.entered_lava_on_vehicle {
            None => p.entered_lava_on_vehicle = Some(p.pos),
            Some(start) => p.distance_trigger("minecraft:ride_entity_in_lava", start),
        }
    } else {
        p.entered_lava_on_vehicle = None;
    }
}

/// `handlePlayerCommand(START_RIDING_JUMP)`: the steered mount rears with its jump sound.
pub(crate) fn riding_jump(entities: &mut Entities, players: &mut [&mut Player], i: usize, data: i32, env: &blocks::BlockEnv) {
    let Some(v) = players[i].vehicle else { return };
    if data <= 0 {
        return;
    }
    let Ok(idx) = entities.list.binary_search_by_key(&v, |e| e.id) else { return };
    let pid = players[i].entity_id;
    let Some(phys) = entities.list[idx].phys.as_mut() else { return };
    if phys.passengers.first() != Some(&pid) {
        return;
    }
    let at = arr(phys.position());
    let silent = phys.silent;
    let Some(m) = kiln_entity::mob::data_mut(phys) else { return };
    if let Some(sound) = kiln_entity::mob::kinds::horse::start_jump(m)
        && !silent
    {
        send_sound(players, env, 0, at, sound, world_fx::SoundSource::Neutral, 0.4, 1.0);
    }
}

/// A player's melee hit on a mob (`Player.attack` → `LivingEntity.hurtServer`), carried out
/// against the region's entities: damage, the extra knockback, fire aspect, then what the mob
/// did (death loot, sounds, damage events).
pub(crate) fn hit_mob(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    hit: &crate::combat::MobHit,
) {
    let Ok(i) = entities.list.binary_search_by_key(&hit.target, |e| e.id) else { return };
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ 0x6869_74, hit.target);
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: hit.target,
        seeds: 0x6869_7400,
        rng,
        grid: Grid::default(),
        proxy_at: HashMap::new(),
        view_at: HashMap::new(),
        proxy_grid: HashMap::new(),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let Some(mut phys) = sim.list[i].phys.take() else { return };
    let source = kiln_entity::mob::DamageSource {
        kind: DamageKind::PlayerAttack,
        attacker: Some(hit.attacker),
        direct: Some(hit.attacker),
        pos: Some(vec3(hit.attacker_pos)),
        attacker_is_player: true,
    };
    let health_before = kiln_entity::mob::data(&phys).map(|m| m.health);
    let hurt = kiln_entity::mob::hurt_entity(&mut phys, &mut sim, source, hit.amount);
    // `Player.damageStatsAndHearts`.
    if hurt
        && let (Some(before), Some(after)) = (health_before, kiln_entity::mob::data(&phys).map(|m| m.health))
        && let Some(p) = sim.players.iter_mut().find(|p| p.entity_id == hit.attacker)
    {
        p.award_stat(*crate::player_stats::stat::DAMAGE_DEALT, ((before - after) * 10.0).round() as i32);
    }
    if hurt {
        if hit.knockback > 0.0 {
            let rad = (hit.yaw * 0.017453292) as f64;
            let (s, c) = (kiln_entity::mob::mth::sin(rad) as f64, kiln_entity::mob::mth::cos(rad) as f64);
            kiln_entity::mob::knockback_entity(&mut phys, hit.knockback as f64, s, -c);
        }
        if hit.fire_seconds > 0.0 {
            phys.ignite_for_seconds(hit.fire_seconds);
        }
    }
    let victim = kiln_entity::level::Seen::of(&phys);
    let health_after = kiln_entity::mob::data(&phys).map(|m| m.health);
    let e = &mut sim.list[i];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    if hurt && let Some(p) = players.iter_mut().find(|p| p.entity_id == hit.attacker) {
        p.last_hurt_mob = Some((hit.target, level.env.game_time));
        // `PlayerHurtEntityTrigger` (dealt before armor and effects, taken after).
        let taken = health_before.zip(health_after).map_or(hit.amount, |(b, a)| b - a);
        let subject = crate::advancements::triggers::seen_subject(&victim, crate::DIMENSIONS[level.env.dim].0);
        p.player_hurt_entity(&subject, hit.amount, taken, "minecraft:player_attack", true);
    }
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
}

/// `ServerGamePacketListenerImpl.handleInteract` on a mob, then `Player.interactOn`: player
/// `i` of the region's players right-clicks entity `target` with the item in `hand` (0 main,
/// 1 off). The held item changes as the mob says; sheared wool drops.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interact_mob(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    i: usize,
    target: i32,
    off_hand: bool,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
) {
    use kiln_item::component::EquipmentSlot;
    let Ok(idx) = entities.list.binary_search_by_key(&target, |e| e.id) else { return };
    {
        let p = &*players[i];
        if p.dead || p.game_mode == 3 || entities.list[idx].removed {
            return;
        }
        let Some(phys) = entities.list[idx].phys.as_ref() else { return };
        if kiln_entity::mob::data(phys).is_none() {
            return;
        }
        // `canInteractWithEntity(box, 3.0)`: the box within the interaction range plus 3.
        let bb = phys.bounding_box();
        let eye = p.eye_position();
        let d = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
        let (dx, dy, dz) = (d(eye[0], bb.min_x, bb.max_x), d(eye[1], bb.min_y, bb.max_y), d(eye[2], bb.min_z, bb.max_z));
        let range = p.attribute(crate::combat::ENTITY_INTERACTION_RANGE) + 3.0;
        if dx * dx + dy * dy + dz * dz >= range * range {
            return;
        }
    }
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let stack = players[i].inv.equipped(slot).clone();
    let who = kiln_entity::mob::interact::Interactor { id: players[i].entity_id, creative: players[i].game_mode == 1, sneaking: players[i].sneaking };
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ 0x696e_74, target);
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: target,
        seeds: 0x696e_7400,
        rng,
        grid: Grid::default(),
        proxy_at: HashMap::new(),
        view_at: HashMap::new(),
        proxy_grid: HashMap::new(),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let Some(mut phys) = sim.list[idx].phys.take() else { return };
    let out = kiln_entity::mob::interact::interact(&mut phys, &mut sim, &who, &stack);
    // Sheared wool: each item on its own, thrown up from the sheep with a push from its random.
    if let Some(table) = &out.shear
        && let Some(loot) = sim.level.env.loot.clone()
    {
        let env = sim.level.env;
        let ctx = crate::mobs::DeathContext {
            type_name: phys.type_name,
            origin: arr(phys.position()),
            on_fire: false,
            baby: false,
            killed_by_player: false,
            damage_type: "minecraft:generic",
            weapon: Some(stack.clone()),
        };
        let seed = crate::mobs::loot_seed(env.seed, env.game_time, target, 0x7368_6561);
        let mut k = 0u64;
        for drop in crate::mobs::roll(&loot, table, &ctx, seed) {
            for _ in 0..drop.count() {
                let mut one = drop.clone();
                one.set_count(1);
                let push = kiln_entity::mob::species::shear_drop_motion(&mut phys);
                let h = crate::mobs::loot_seed(env.seed, env.game_time, target, 0x7368_0000 | k) as u64;
                k += 1;
                let p = phys.position();
                let mut s = crate::mobs::drop_item(one, [p.x, p.y + 1.0, p.z], h);
                s.vel = [s.vel[0] + push.x, s.vel[1] + push.y, s.vel[2] + push.z];
                sim.spawns.push(s);
            }
        }
    }
    // `startRiding` (`Entity.canRide`: not sneaking), then `ServerPlayer.startRiding`: the
    // rider takes the mount's facing and goes to its seat.
    if out.ride && sim.players[i].vehicle.is_none() && !sim.players[i].sneaking {
        let pid = sim.players[i].entity_id;
        let first_is_player = phys.passengers.first().is_some_and(|f| sim.views.iter().any(|v| v.id == *f));
        kiln_entity::ride::add_passenger(&mut phys, pid, true, first_is_player);
        let at = phys.passengers.iter().position(|&x| x == pid).unwrap_or(0);
        let seat = kiln_entity::ride::rider_position(&phys, at, "minecraft:player", 1.0);
        let now = sim.level.env.game_time;
        let p = &mut *sim.players[i];
        p.vehicle = Some(target);
        p.vehicle_type = Some(phys.type_name);
        p.teleport(arr(seat), [phys.y_rot, phys.x_rot], now);
        p.started_riding();
    }
    let seen = out.success.then(|| kiln_entity::level::Seen::of(&phys));
    let e = &mut sim.list[idx];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let p = &mut *players[i];
    // `PlayerInteractTrigger`: the item as it was when the interaction used it.
    if let Some(seen) = seen {
        let used = if out.held == kiln_entity::mob::interact::HeldChange::None { kiln_item::ItemStack::empty() } else { stack.clone() };
        let subject = crate::advancements::triggers::seen_subject(&seen, crate::DIMENSIONS[level.env.dim].0);
        p.fire_conds("minecraft:player_interacted_with_entity", None, |c, ok, loot| {
            c.item("item").is_none_or(|ip| kiln_loot::predicate::item_matches(&loot.tags, ip, &used)) && c.cap("entity").is_none_or(|cap| ok(cap, &subject))
        });
    }
    let index = kiln_inventory::inventory::equipment_index(slot, p.inv.selected);
    use kiln_entity::mob::interact::HeldChange;
    match &out.held {
        HeldChange::None => {}
        HeldChange::Consume(n) => {
            if p.game_mode != 1 {
                kiln_inventory::Container::item_mut(&mut p.inv, index).shrink(*n);
            }
        }
        HeldChange::Damage(n) => p.hurt_and_break(slot, *n, None),
        HeldChange::Fill(filled) => {
            // `ItemUtils.createFilledResult`: creative players keep the empty item and get the
            // filled one if they have none; others trade one of the held items for it.
            let mut filled = filled.clone();
            if p.game_mode == 1 {
                let has = (0..kiln_inventory::Container::size(&p.inv))
                    .any(|j| kiln_inventory::stack::matches(kiln_inventory::Container::item(&p.inv, j), &filled));
                if !has {
                    p.add_to_inventory(&mut filled);
                }
            } else {
                let held = kiln_inventory::Container::item_mut(&mut p.inv, index);
                held.shrink(1);
                if held.is_empty() {
                    *held = filled;
                } else if p.add_to_inventory(&mut filled) == 0 {
                    spawns.push(crate::mobs::drop_item(filled, p.pos, p.entity_id as u64));
                }
            }
        }
    }
    if let Some(sound) = out.player_sound
        && let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound)
    {
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Players, p.pos, 1.0, 1.0, level.env.game_time);
        p.send(pkt);
    }
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
}

/// Runs `f` on entity `target` with the region as its level (outside the entity tick: menu
/// actions reaching a villager), then carries out what it did.
#[allow(clippy::too_many_arguments)]
pub(crate) fn with_entity<R>(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    target: i32,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    salt: u64,
    f: impl FnOnce(&mut kiln_entity::Entity, &mut dyn EntityLevel) -> R,
) -> Option<R> {
    let idx = entities.list.binary_search_by_key(&target, |e| e.id).ok()?;
    if entities.list[idx].removed {
        return None;
    }
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ salt as i64, target);
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: target,
        seeds: salt << 8,
        rng,
        grid: Grid::default(),
        proxy_at: HashMap::new(),
        view_at: HashMap::new(),
        proxy_grid: HashMap::new(),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let mut phys = sim.list[idx].phys.take()?;
    let r = f(&mut phys, &mut sim);
    let e = &mut sim.list[idx];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
    Some(r)
}

/// Runs `f` with the region as an entity level (block work that affects entities: bed and
/// respawn anchor explosions), then carries out what it did.
#[allow(clippy::too_many_arguments)]
pub(crate) fn with_level<R>(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    salt: u64,
    f: impl FnOnce(&mut dyn EntityLevel) -> R,
) -> R {
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<kiln_entity::Entity> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| proxy(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ salt as i64, 0);
    let mut sim = SimLevel {
        level,
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: 0,
        seeds: salt << 8,
        rng,
        grid: Grid::default(),
        proxy_at: HashMap::new(),
        view_at: HashMap::new(),
        proxy_grid: HashMap::new(),
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let r = f(&mut sim);
    for e in sim.list.iter_mut() {
        e.sync();
    }
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
    r
}

/// Events with the index each gets for its seeds (sounds, loot): counted per source (the
/// entity, or the position), so they do not depend on what else the region's entities did.
fn keyed(events: Vec<Event>) -> Vec<(usize, Event)> {
    let mut seen: std::collections::HashMap<(u8, u64), usize> = std::collections::HashMap::new();
    let pos_key = |p: Vec3| p.x.to_bits() ^ p.y.to_bits().rotate_left(21) ^ p.z.to_bits().rotate_left(42);
    events
        .into_iter()
        .map(|ev| {
            let key = match &ev {
                Event::DeathLoot { entity, .. } | Event::GiftLoot { entity, .. } | Event::ShearLoot { entity, .. } => (0, *entity as u32 as u64),
                Event::EntityEvent { entity, .. } | Event::MobHurt { entity, .. } => (1, *entity as u32 as u64),
                Event::Sound { pos, .. } => (2, pos_key(*pos)),
                Event::Explosion { pos, .. } => (3, pos_key(*pos)),
                Event::Hurt { target, .. } => (4, *target as u32 as u64),
                _ => (5, 0),
            };
            let n = seen.entry(key).or_insert(0);
            let i = *n;
            *n += 1;
            (i, ev)
        })
        .collect()
}

fn carry_out(
    event: Event,
    n: usize,
    level: &mut RegionLevel,
    list: &mut [Entity],
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
        Event::Hurt { target, amount, kind, attacker } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == target) {
                // kiln-entity's attacker is the entity that dealt the damage (TNT, a falling
                // block); none of them is a player.
                let source = health::Source { cause: health::Cause::Entity(kind), attacker: None, direct: attacker, weapon: None };
                let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns, deaths, level_rng: None };
                p.hurt(amount, &source, &mut ctx);
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
        Event::MobHurt { entity: id, kind, attacker, direct } => {
            // `broadcastDamageEvent`: the hurt tilt and sound for the mob's viewers.
            if let Ok(i) = list.binary_search_by_key(&id, |e| e.id) {
                let type_id = kiln_data::synced_id("minecraft:damage_type", kind.type_name()).unwrap_or(0);
                let pkt = entity::damage_event(id, type_id, attacker, direct, None);
                for p in players.iter_mut().filter(|p| list[i].seen_by.binary_search(&p.conn).is_ok()) {
                    p.send(pkt.clone());
                }
            }
        }
        Event::DeathLoot { entity: id, table, pos, killer, attacker, direct: _, kind, on_fire } => {
            let Some(loot) = env.loot.clone() else { return };
            let Ok(i) = list.binary_search_by_key(&id, |e| e.id) else { return };
            let Some(phys) = list[i].phys.as_ref() else { return };
            let weapon = killer.and_then(|k| players.iter().find(|p| p.entity_id == k)).map(|p| p.inv.selected_item().clone());
            let ctx = crate::mobs::DeathContext {
                type_name: phys.type_name,
                origin: arr(pos),
                on_fire,
                baby: kiln_entity::mob::data(phys).is_some_and(|m| m.baby()),
                killed_by_player: killer.is_some(),
                damage_type: kind.type_name(),
                weapon,
            };
            let _ = attacker;
            let seed = crate::mobs::loot_seed(env.seed, env.game_time, id, 0x6465_6174);
            for (k, stack) in crate::mobs::roll(&loot, &table, &ctx, seed).into_iter().enumerate() {
                let h = crate::mobs::loot_seed(env.seed, env.game_time, id, 0x6465_6174_00 | k as u64) as u64;
                spawns.push(crate::mobs::drop_item(stack, arr(pos), h));
            }
        }
        Event::PotionSplash { target, potion, scale, owner: _ } => {
            // `ThrownSplashPotion.onHitAsPotion` on a player: instant effects scaled by the
            // distance, the others with scaled durations (dropped at 20 ticks or less).
            let Some(p) = players.iter_mut().find(|p| p.entity_id == target && !p.dead) else { return };
            let contents = kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id(potion), ..Default::default() };
            for e in crate::effects::potion_effects(&contents, 1.0) {
                match e.kind() {
                    crate::effects::Kind::HealOrHarm { harm: false } => p.heal((scale * (4i32.wrapping_shl(e.amplifier as u32)) as f64 + 0.5) as i32 as f32),
                    crate::effects::Kind::HealOrHarm { harm: true } => {
                        let amount = (scale * (6i32.wrapping_shl(e.amplifier as u32)) as f64 + 0.5) as i32 as f32;
                        let source = health::Source { cause: health::Cause::Entity(DamageKind::IndirectMagic), attacker: None, direct: None, weapon: None };
                        let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns, deaths, level_rng: None };
                        p.hurt(amount, &source, &mut ctx);
                    }
                    _ => {
                        let mut e = e;
                        if e.duration != crate::effects::INFINITE && e.duration != 0 {
                            e.duration = (scale * e.duration as f64 + 0.5) as i32;
                        }
                        if e.duration == crate::effects::INFINITE || e.duration > 20 {
                            p.add_effect(e);
                        }
                    }
                }
            }
        }
        Event::Killed { entity, entity_type, credit, kind, attacker, direct } => {
            if let Some(p) = credit.and_then(|k| players.iter_mut().find(|p| p.entity_id == k)) {
                p.killed_entity(entity_type);
                let dim = crate::DIMENSIONS[env.dim].0;
                if let Some(e) = list.binary_search_by_key(&entity, |e| e.id).ok().and_then(|i| list[i].phys.as_ref()) {
                    let subject = crate::advancements::triggers::mob_subject(e, dim);
                    p.killed("minecraft:player_killed_entity", &subject, kind.type_name(), direct == attacker);
                }
            }
        }
        Event::GiftLoot { entity: id, table, pos } => loot_drop(env, spawns, id, table, pos, n, 0.0),
        Event::ShearLoot { entity: id, table, pos } => loot_drop(env, spawns, id, &table, pos, n, 1.0),
        // Vibrations, other projectile hits and the block effects of entities inside blocks
        // (pressure plates are pressed through the entity boxes) are not simulated yet.
        Event::GameEvent { .. } | Event::EntityInsideBlock { .. } | Event::ProjectileHit { .. } => {}
        Event::Raid(_) => {}
        Event::Criterion { player, criterion } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == player) {
                p.entity_criterion(crate::DIMENSIONS[env.dim].0, &criterion);
            }
        }
    }
}

/// A gift or shearing loot table dropped at `pos` (`y_off` above it).
fn loot_drop(env: &blocks::BlockEnv, spawns: &mut Vec<Spawn>, id: i32, table: &str, pos: Vec3, n: usize, y_off: f64) {
    let Some(loot) = env.loot.clone() else { return };
    let ctx = crate::mobs::DeathContext {
        type_name: "minecraft:chicken",
        origin: arr(pos),
        on_fire: false,
        baby: false,
        killed_by_player: false,
        damage_type: "minecraft:generic",
        weapon: None,
    };
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, id, 0x6966 ^ n as u64);
    for (k, stack) in crate::mobs::roll(&loot, table, &ctx, seed).into_iter().enumerate() {
        let h = crate::mobs::loot_seed(env.seed, env.game_time, id, (n as u64) << 8 | k as u64) as u64;
        spawns.push(crate::mobs::drop_item(stack, [pos.x, pos.y + y_off, pos.z], h));
    }
}

/// A player as the entities see it (`now`: the game time).
fn view(p: &Player, now: i64) -> PlayerView {
    use kiln_item::component::EquipmentSlot as S;
    let armor = [S::Feet, S::Legs, S::Chest, S::Head].iter().filter(|s| !p.inv.equipped(**s).is_empty()).count();
    PlayerView {
        id: p.entity_id,
        uuid: p.uuid.as_u128(),
        pos: vec3(p.pos),
        eye_height: if p.sneaking { 1.27 } else { 1.62 },
        spectator: p.game_mode == 3,
        creative: p.game_mode == 1,
        sneaking: p.sneaking,
        alive: !p.dead && !p.disconnected,
        invisible: p.has_effect("minecraft:invisibility"),
        armor_cover: armor as f32 / 4.0,
        main_hand: p.inv.selected_item().item(),
        off_hand: p.inv.equipped(S::OffHand).item(),
        piglin_safe_armor: [S::Feet, S::Legs, S::Chest, S::Head]
            .iter()
            .any(|s| kiln_entity::mob::item_tag(p.inv.equipped(*s).item(), "minecraft:piglin_safe_armor")),
        in_water: None,
        head: p.inv.equipped(S::Head).item(),
        yaw: p.rot[0],
        pitch: p.rot[1],
        health: p.health,
        effects: p.effects.keys().filter(|&&id| (0..64).contains(&id)).fold(0u64, |b, &id| b | 1 << id),
        last_hurt_by_mob: p.last_hurt_by_mob.filter(|&(_, t)| now - t <= 100).map(|(id, _)| id),
        last_hurt_by_mob_time: p.last_hurt_by_mob.map_or(0, |(_, t)| t as i32),
        last_hurt_mob: p.last_hurt_mob.map(|(id, _)| id),
        last_hurt_mob_time: p.last_hurt_mob.map_or(0, |(_, t)| t as i32),
        hurt_recently: p.last_hurt_by_mob.is_some_and(|(_, t)| now - t <= 100),
        vehicle: p.vehicle,
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
        let picked = item.stack.item();
        let taken = players[i].add_to_inventory(&mut item.stack);
        if taken == 0 {
            continue;
        }
        // `ItemEntity.playerTouch`.
        players[i].award_stat(crate::player_stats::Stat::item(crate::player_stats::PICKED_UP, picked), taken);
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
        if let Some(EntityKind::Mob(m)) = e.phys.as_mut().map(|p| &mut p.kind) {
            if std::mem::take(&mut m.swing) {
                packets.push(entity::swing_animation(e.id, false, entity::swing::WHACK, entity::swing::DEFAULT_DURATION));
            }
        }
        if let Some(phys) = e.phys.as_ref()
            && let EntityKind::Mob(m) = &phys.kind
        {
            let meta = crate::mobs::metadata(phys, m);
            if meta.entries() != e.meta_sent.as_slice() {
                if !e.meta_sent.is_empty() {
                    packets.push(entity::set_entity_data(e.id, &meta));
                }
                e.meta_sent = meta.entries().to_vec();
            }
            let worn = crate::mobs::shown_equipment(m);
            if worn.len() != e.equipment_sent.len() || worn.iter().zip(&e.equipment_sent).any(|(a, b)| a.0 != b.0 || !kiln_inventory::stack::matches(&a.1, &b.1)) {
                let mut slots: Vec<(u8, kiln_item::ItemStack)> = worn.clone();
                for (i, _) in &e.equipment_sent {
                    if !slots.iter().any(|(j, _)| j == i) {
                        slots.push((*i, kiln_item::ItemStack::empty()));
                    }
                }
                if !slots.is_empty() && !(e.equipment_sent.is_empty() && e.age <= 1) {
                    let refs: Vec<(u8, &kiln_item::ItemStack)> = slots.iter().map(|(i, s)| (*i, s)).collect();
                    packets.push(crate::players::set_equipment(e.id, &refs));
                }
                e.equipment_sent = worn;
            }
        }
        if let Some(phys) = e.phys.as_ref()
            && phys.passengers != e.passengers_sent
        {
            e.passengers_sent = phys.passengers.clone();
            packets.push(entity::set_passengers(e.id, &e.passengers_sent));
        }
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
        DamageKind::MobAttack => ("minecraft:mob_attack", "death.attack.mob"),
        DamageKind::PlayerAttack => ("minecraft:player_attack", "death.attack.player"),
        DamageKind::Drown => ("minecraft:drown", "death.attack.drown"),
        DamageKind::InWall => ("minecraft:in_wall", "death.attack.inWall"),
        DamageKind::OutOfWorld => ("minecraft:out_of_world", "death.attack.outOfWorld"),
        DamageKind::Fall => ("minecraft:fall", "death.attack.fall"),
        DamageKind::Kill => ("minecraft:generic_kill", "death.attack.genericKill"),
        DamageKind::Cramming => ("minecraft:cramming", "death.attack.cramming"),
        DamageKind::PlayerExplosion => ("minecraft:player_explosion", "death.attack.explosion.player"),
        DamageKind::Fireball => ("minecraft:fireball", "death.attack.fireball"),
        DamageKind::Trident => ("minecraft:trident", "death.attack.trident"),
        DamageKind::MobProjectile => ("minecraft:mob_projectile", "death.attack.mob"),
        DamageKind::Magic => ("minecraft:magic", "death.attack.magic"),
        DamageKind::IndirectMagic => ("minecraft:indirect_magic", "death.attack.indirectMagic"),
        DamageKind::LightningBolt => ("minecraft:lightning_bolt", "death.attack.lightningBolt"),
        // -- slice 3: mob effects
        DamageKind::Wither => ("minecraft:wither", "death.attack.wither"),

        // -- slice 3: raids
        DamageKind::Starve => ("minecraft:starve", "death.attack.starve"),

        // -- slice 3: the end

        // -- slice 3: wither and guardians

        // -- slice 3: warden

        // -- slice 3: common mobs A

        // -- slice 3: common mobs B

    }
}
