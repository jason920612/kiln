//! Non-player entities: storage that follows region merges and splits, spawning with network
//! ids that do not depend on how regions split the world, ticking inside their region with
//! kiln-entity's vanilla behaviour, and tracking to the region's players (design §6).

use crate::Player;
use crate::blocks::{self, RegionLevel};
use crate::entity_world::World;

mod islands;
mod spec;
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
use kiln_proto::packets::{hud, world_fx};
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
    /// `SpeleothemBlock.spawnFallingStalactite` for the tip: a falling block that hurts what it
    /// lands on (`setHurtsEntities(per_distance, 40)`).
    FallingStalactite { state: u16, per_distance: f32 },
    /// `TntBlock.prime`: a primed TNT with vanilla's random hop.
    Tnt,
    /// An entity kiln-entity built during a tick (its id is replaced by the assigned one).
    Ready(Box<kiln_entity::Entity>),
    /// An entity loaded from its chunk's saved data; keeps its UUID (unless it had none).
    Loaded(Box<kiln_entity::Entity>),
    /// A stack loaded from its root's saved data (`Passengers`): the root, and the riders
    /// depth first, each with the index of what it rides (0 the root, n the rider n - 1).
    LoadedStack(Box<kiln_entity::Entity>, Vec<kiln_entity::persist::Rider>),
    /// An entity kiln-entity built during a tick with the jockeys that go with it (a spawner's
    /// spawn: the entity, its riders, `loaded`: riders read from saved data; `nearby_chicken`: a
    /// baby zombie asks for a chicken): keeps its UUID unless it had none.
    Stacked(Box<kiln_entity::Entity>, Vec<kiln_entity::mob::Companion>, bool, bool),
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
    pub phys: Option<Box<kiln_entity::Entity>>,
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
    /// The entity its lead is tied to, as viewers last got it (Set Entity Link).
    leash_sent: Option<i32>,
    /// A boss's bar fill as its viewers last got it (`ServerBossEvent`, the wither).
    boss_sent: Option<f32>,
    /// What `finalizeSpawn` made along with it, until the level adds and seats it.
    pub(crate) jockeys: Option<Box<Jockeys>>,
    /// How long its last speculative turn took (nanoseconds; the long ones start first).
    pub(crate) spec_ns: u32,
}

/// The jockeys a mob's `finalizeSpawn` made (see `kiln_entity::mob::Companion`).
pub(crate) struct Jockeys {
    pub companions: Vec<kiln_entity::mob::Companion>,
    /// The companions are riders loaded from saved data: they keep their UUIDs and places.
    pub loaded: bool,
    /// A baby zombie looks for an unridden chicken near it.
    pub nearby_chicken: bool,
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
            Body::Loaded(e) | Body::LoadedStack(e, _) => e.uuid,
            _ => 0,
        };
        (self.pos.map(f64::to_bits), self.kind.id, uuid)
    }

    /// A spawn for an entity loaded from its chunk.
    pub fn loaded(e: kiln_entity::Entity) -> Option<Spawn> {
        let kind = kiln_data::entities::by_name(e.type_name)?;
        Some(Spawn { kind, pos: arr(e.position()), vel: arr(e.delta), body: Body::Loaded(Box::new(e)) })
    }

    /// A spawn for a stack loaded from its root's chunk data (`EntityType.loadEntityRecursive`).
    pub fn loaded_stack(root: kiln_entity::Entity, riders: Vec<kiln_entity::persist::Rider>) -> Option<Spawn> {
        if riders.is_empty() {
            return Self::loaded(root);
        }
        let kind = kiln_data::entities::by_name(root.type_name)?;
        Some(Spawn { kind, pos: arr(root.position()), vel: arr(root.delta), body: Body::LoadedStack(Box::new(root), riders) })
    }

    /// The stack `tag` saves (root and `Passengers`) as a spawn; the error says why the
    /// compound is to be kept as saved. Entities without a UUID get random seeds derived
    /// from `seed`.
    pub fn from_saved(tag: &kiln_proto::nbt::Tag, seed: i64, strict: bool) -> Result<Spawn, kiln_entity::persist::LoadError> {
        let n = std::cell::Cell::new(0i64);
        let seeds = |u: u128| {
            n.set(n.get() + 1);
            if u != 0 {
                seed_for_uuid(u)
            } else if n.get() == 1 {
                seed
            } else {
                seed ^ n.get().wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64)
            }
        };
        let (root, riders) = kiln_entity::persist::load_stack(tag, 0, &seeds, strict)?;
        Self::loaded_stack(root, riders).ok_or(kiln_entity::persist::LoadError::NotSimulated)
    }
}

/// Adds the jockeys of the mob `mount` (just pushed, the last of `list`) after it, with the next
/// ids, and seats them (`startRiding`). A baby zombie that asked for a chicken takes the lowest
/// numbered unridden one within 5x3x5 blocks of its box (`ENTITY_NOT_BEING_RIDDEN`).
pub(crate) fn add_jockeys(list: &mut Vec<Entity>, mount: i32, jockeys: Jockeys, next_id: &mut i32, world_seed: i64, game_time: i64) {
    use kiln_entity::mob::Seat;
    let mut seated: Vec<(i32, Seat)> = Vec::new();
    for c in jockeys.companions {
        let Some(kind) = kiln_data::entities::by_name(c.entity.type_name) else { continue };
        let id = *next_id;
        *next_id += 1;
        let pos = arr(c.entity.position());
        let uuid = fresh_uuid(world_seed, game_time, id);
        let (vel, body) = if jockeys.loaded {
            (arr(c.entity.delta), Body::Loaded(Box::new(c.entity)))
        } else {
            ([0.0; 3], Body::Ready(Box::new(c.entity)))
        };
        list.push(Entity::new(id, uuid, Spawn { kind, pos, vel, body }));
        seated.push((id, c.seat));
    }
    // `startRiding`: (rider, vehicle) in the order the companions were made.
    let mut links: Vec<(i32, i32)> = seated
        .iter()
        .filter_map(|&(id, seat)| match seat {
            Seat::OnMob => Some((id, mount)),
            Seat::UnderMob => Some((mount, id)),
            Seat::OnCompanion(i) => Some((id, seated[i].0)),
            Seat::Loose => None,
        })
        .collect();
    if jockeys.nearby_chicken
        && let Some(chicken) = nearby_unridden_chicken(list, mount)
    {
        if let Some(c) = list.iter_mut().find(|e| e.id == chicken).and_then(|e| e.phys.as_deref_mut()).and_then(kiln_entity::mob::data_mut) {
            c.chicken_jockey = true;
        }
        links.push((mount, chicken));
    }
    for (rider, vehicle) in links {
        let (Ok(ri), Ok(vi)) = (list.binary_search_by_key(&rider, |e| e.id), list.binary_search_by_key(&vehicle, |e| e.id)) else { continue };
        if ri == vi {
            continue;
        }
        let Some(mut rp) = list[ri].phys.take() else { continue };
        if let Some(vp) = list[vi].phys.as_deref_mut()
            && kiln_entity::ride::start_riding(&mut rp, vp, false)
        {
            // (A rider loaded from saved data stays where it was saved unless that is far from
            // its seat: the first tick seats it, and its cell must be its vehicle's.)
            let seat = kiln_entity::ride::riding_position(vp, vp.passengers.len().saturating_sub(1));
            if !jockeys.loaded || (rp.position() - seat).length_sqr() > 4.0 {
                kiln_entity::ride::position_rider(&mut rp, vp);
            }
        }
        list[ri].phys = Some(rp);
        list[ri].sync();
    }
}

fn nearby_unridden_chicken(list: &[Entity], mount: i32) -> Option<i32> {
    let me = list.iter().find(|e| e.id == mount)?.phys.as_deref()?;
    let area = me.bounding_box().inflate(5.0, 3.0, 5.0);
    list.iter()
        .filter(|e| e.id != mount && !e.removed)
        .filter(|e| e.phys.as_deref().is_some_and(|p| p.is_alive() && p.type_name == "minecraft:chicken" && p.passengers.is_empty() && p.vehicle.is_none() && p.bounding_box().intersects(&area)))
        .map(|e| e.id)
        .min()
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
            Body::Loaded(e) | Body::LoadedStack(e, _) | Body::Stacked(e, ..) if e.uuid != 0 => Uuid::from_u128(e.uuid),
            _ => uuid,
        };
        let (u, seed, pos) = (uuid.as_u128(), seed_for(id), vec3(spawn.pos));
        let mut jockeys = None;
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
            Body::FallingStalactite { state, per_distance } => {
                let mut e = kiln_entity::falling_block::fall(id, u, BlockPos::containing(pos.x, pos.y, pos.z), state, seed);
                if let EntityKind::FallingBlock(d) = &mut e.kind {
                    d.hurt_entities = true;
                    d.fall_damage_per_distance = per_distance;
                    d.fall_damage_max = 40;
                }
                e
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
                    let mut group = kiln_entity::mob::GroupData { monsters_disabled: f.monsters_disabled, camel_space: f.camel_space, ..Default::default() };
                    kiln_entity::mob::finalize_spawn(&mut e, &mut r, &f.ctx, &mut group, f.natural);
                    if !group.companions.is_empty() || group.nearby_chicken {
                        jockeys = Some(Box::new(Jockeys { companions: std::mem::take(&mut group.companions), loaded: false, nearby_chicken: group.nearby_chicken }));
                    }
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
            Body::Stacked(e, companions, loaded, nearby_chicken) => {
                let mut e = *e;
                e.id = id;
                e.uuid = u;
                jockeys = Some(Box::new(Jockeys { companions, loaded, nearby_chicken }));
                e
            }
            Body::LoadedStack(e, riders) => {
                let mut e = *e;
                e.id = id;
                e.uuid = u;
                let companions = riders
                    .into_iter()
                    .map(|r| kiln_entity::mob::Companion {
                        entity: r.entity,
                        seat: if r.vehicle == 0 { kiln_entity::mob::Seat::OnMob } else { kiln_entity::mob::Seat::OnCompanion(r.vehicle - 1) },
                    })
                    .collect();
                jockeys = Some(Box::new(Jockeys { companions, loaded: true, nearby_chicken: false }));
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
            phys: Some(Box::new(phys)),
            tracker: MovementTracker::new(id, spawn.kind.update_interval, &state),
            sent_vel: vel,
            seen_by: Vec::new(),
            section: None,
            meta_sent: Vec::new(),
            passengers_sent: Vec::new(),
            equipment_sent: Vec::new(),
            leash_sent: None,
            boss_sent: None,
            jockeys,
            spec_ns: 0,
        }
    }

    /// The entity as saved in its chunk (`Entity.save`); `owners` resolves the network ids
    /// of owners to UUIDs.
    pub fn save(&self, owners: &dyn Fn(i32) -> Option<u128>) -> kiln_proto::nbt::Tag {
        kiln_entity::persist::save(self.phys(), owners)
    }

    fn phys(&self) -> &kiln_entity::Entity {
        self.phys.as_deref().expect("entity state is back after its tick")
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

    /// `teleportSetPosition` of a command or portal teleport: the entity stands at `pos` (facing
    /// `rot` if given), still and on the ground, as if it had always been there.
    pub(crate) fn relocate(&mut self, pos: [f64; 3], rot: Option<[f32; 2]>) {
        let Some(p) = self.phys.as_deref_mut() else { return };
        p.set_pos(Vec3::new(pos[0], pos[1], pos[2]));
        if let Some([yaw, pitch]) = rot {
            p.y_rot = yaw;
            p.x_rot = pitch;
        }
        // `setYHeadRot(yRot)`.
        let yaw = p.y_rot;
        if let Some(m) = kiln_entity::mob::data_mut(p) {
            m.y_head_rot = yaw;
            m.y_head_rot_o = yaw;
        }
        p.set_old_pos_and_rot();
        p.delta = Vec3::ZERO;
        p.on_ground = true;
        self.sync();
        self.cell = chunk_of(self.pos).cell();
    }

    /// The entity no longer has viewers and what they were last sent (it moved to another region,
    /// where it is paired afresh): the players that were watching it, who have to be told it is gone.
    pub(crate) fn forget_viewers(&mut self) -> Vec<ConnId> {
        let viewers = std::mem::take(&mut self.seen_by);
        self.section = None;
        self.tracker = MovementTracker::new(self.id, self.kind.update_interval, &self.move_state());
        self.sent_vel = self.vel;
        self.meta_sent = Vec::new();
        self.passengers_sent = Vec::new();
        self.equipment_sent = Vec::new();
        self.leash_sent = None;
        self.boss_sent = None;
        viewers
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
            EntityKind::Arrow(a) => {
                let flags = (a.crit as i8) | if self.phys().no_physics { 2 } else { 0 };
                if flags != 0 {
                    d.set(data::abstract_arrow::ID_FLAGS, &DataValue::Byte(flags));
                }
                if a.pierce_level > 0 {
                    d.set(data::abstract_arrow::PIERCE_LEVEL, &DataValue::Byte(a.pierce_level as i8));
                }
                if a.in_ground {
                    d.set(data::abstract_arrow::IN_GROUND, &DataValue::Boolean(true));
                }
            }
            EntityKind::Throwable(t) => {
                if let Some(item) = &t.item {
                    let mut bytes = bytes::BytesMut::new();
                    item.write_optional(&mut bytes);
                    d.set(data::throwable_item_projectile::ITEM_STACK, &DataValue::EncodedItemStack(bytes.freeze()));
                }
            }
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
            // `Projectile.getAddEntityPacket`: the owner's id.
            EntityKind::Arrow(a) => a.owner.unwrap_or(0),
            EntityKind::Throwable(t) => t.owner.unwrap_or(0),
            _ => 0,
        };
        // `LeashFenceKnotEntity.getAddEntityPacket`: the spawn position is the block's own.
        let spawn = if self.phys().type_name == kiln_entity::leash::KNOT {
            let p = self.phys().position();
            let at = [p.x.floor(), p.y.floor(), p.z.floor()];
            entity::add_entity(&entity::AddEntity {
                entity_id: self.id,
                uuid: self.uuid,
                kind: self.kind.id,
                pos: at,
                velocity: [0.0; 3],
                pitch: kiln_proto::packets::entity::Angle::from_degrees(0.0),
                yaw: kiln_proto::packets::entity::Angle::from_degrees(0.0),
                head_yaw: kiln_proto::packets::entity::Angle::from_degrees(0.0),
                data: 0,
            })
        } else {
            self.tracker.spawn(self.uuid, self.kind.id, self.vel, spawn_data)
        };
        let mut out = vec![entity::bundle_delimiter(), spawn, entity::set_entity_data(self.id, &self.metadata())];
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
        // `ServerEntity.sendPairingData`: a led entity's lead.
        if let Some(h) = kiln_entity::leash::holder_of(self.phys()) {
            out.push(entity::set_entity_link(self.id, h));
        }
        out.push(entity::bundle_delimiter());
        out
    }

    /// The bounding box, and whether it keeps blocks from being placed into it
    /// (`Entity.blocksBuilding`: primed TNT and falling blocks).
    /// `Monster.isPreventingPlayerRest`: monsters do, zombified piglins only while angry.
    pub fn prevents_rest(&self) -> bool {
        let Some(m) = self.phys.as_deref().and_then(kiln_entity::mob::data) else { return false };
        if m.health <= 0.0 || m.kind.category() != kiln_entity::mob::Category::Monster {
            return false;
        }
        m.kind != kiln_entity::mob::MobKind::ZombifiedPiglin || m.target.is_some()
    }

    pub fn body(&self) -> ([f64; 3], [f64; 3], bool) {
        let p = self.phys();
        let bb = p.bounding_box();
        let blocks_building = matches!(p.kind, EntityKind::Tnt(_) | EntityKind::FallingBlock(_)) || p.type_name == "minecraft:end_crystal";
        ([bb.min_x, bb.min_y, bb.min_z], [bb.max_x, bb.max_y, bb.max_z], blocks_building)
    }
}

/// A region's entities in id order, which is spawn order (vanilla's tick order is insertion
/// order); merges interleave by id, splits go by each entity's cell.
#[derive(Default)]
pub(crate) struct Entities {
    pub list: Vec<Entity>,
    /// How speculation fared in this region ([`spec`]).
    pub(crate) spec: spec::Pace,
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

/// The entity at `i` of a region's id-ordered `list`, saved with the passengers it carries
/// (`Entity.saveWithoutId`, `Passengers` and all).
pub(crate) fn save_in(list: &[Entity], i: usize, owners: &dyn Fn(i32) -> Option<u128>) -> kiln_proto::nbt::Tag {
    let lookup = |id: i32| list.binary_search_by_key(&id, |e| e.id).ok().map(|j| &list[j]).filter(|e| !e.removed).and_then(|e| e.phys.as_deref());
    kiln_entity::persist::save_with(list[i].phys(), owners, &lookup)
}

/// The index of the entity the stack of `list[i]` stands on: its vehicle's vehicle's... that
/// is in `list` and carries it (a rider is saved and unloaded with its root).
pub(crate) fn root_in(list: &[Entity], i: usize) -> usize {
    let mut at = i;
    for _ in 0..list.len().min(64) {
        let Some(p) = list[at].phys.as_deref() else { break };
        let Some(v) = p.vehicle else { break };
        let Ok(j) = list.binary_search_by_key(&v, |e| e.id) else { break };
        if list[j].removed || !list[j].phys.as_deref().is_some_and(|vp| vp.passengers.contains(&p.id)) {
            break;
        }
        at = j;
    }
    at
}

fn kb(p: BlockPos) -> kiln_blocks::BlockPos {
    kiln_blocks::BlockPos::new(p.x, p.y, p.z)
}

/// The region as kiln-entity's world: blocks through kiln-blocks (so landing falling blocks
/// and explosions update their neighbours), the region's entities by id, and its players as
/// stand-ins that explosions can hurt and push.
pub(crate) struct SimLevel<'a, 'l, 'p> {
    pub(crate) level: World<'a, 'l>,
    list: &'a mut Vec<Entity>,
    /// The region's players, which mobs hurt directly.
    players: &'a mut [&'p mut Player],
    deaths: &'a mut Vec<health::Death>,
    /// Players as `Other` entities, in connection order.
    proxies: Vec<Proxy>,
    views: Vec<PlayerView>,
    spawns: &'a mut Vec<Spawn>,
    events: Vec<Event>,
    /// Ids for entities spawned during the tick until they get their real one.
    next_placeholder: i32,
    /// The entity being ticked and how many seeds it drew, for partition-independent seeds.
    pub(crate) current: i32,
    pub(crate) seeds: u64,
    /// The ticking entity as the source of its game events (its state is out for its tick).
    current_source: Option<kiln_entity::vibration::EventSource>,
    /// The level random as the ticking entity sees it: seeded per entity and tick, so what
    /// one entity draws (explosions, experience orbs) does not depend on the others in its
    /// region (vanilla shares one random per level).
    rng: LegacyRandom,
    /// Entity sections (16³) → indices in `list`, for area queries.
    grid: Grid,
    /// Player entity id → index in `proxies` ([`SimLevel::index_players`]; mobs look players up
    /// several times a tick, and a crowd has a thousand).
    proxy_at: FastMap<i32, usize>,
    /// Player stand-ins by entity section (like `grid`), so area queries skip far players.
    proxy_grid: FastMap<(i32, i32, i32), Vec<usize>>,
    /// Player views (spectators too) by id, UUID and section, for [`EntityLevel::player`],
    /// [`EntityLevel::player_by_uuid`] and [`EntityLevel::players_in`].
    view_index: kiln_entity::level::PlayerGrid,
    /// The region's players for `Mob.checkDespawn` (`views` may hold only an island's).
    despawn: Option<&'a Nearest>,
    /// How many times an entity changed a player (speculation: what read the players before
    /// may be out of date).
    player_writes: u32,
    /// While an entity's turn runs in place of its speculation: the entities it reached with
    /// `entity_mut`, with their box and whether they were alive before.
    touched: Option<Vec<(i32, Aabb, bool)>>,
    /// The type and position of the entity whose state is out of the list for this turn, for
    /// what it does to players meanwhile (thorns damage names it as the attacker).
    current_info: Option<(&'static str, [f64; 3])>,
}

/// The non-spectator players' positions by 32-block cube, for `Mob.checkDespawn`'s nearest
/// player: a player in the 27 cubes around a mob's is nearer than any outside them, so when
/// there is one only those are compared.
pub(crate) struct Nearest {
    cubes: FastMap<(i32, i32, i32), Vec<Vec3>>,
    /// Each cube's players' bounds (min, max corners) and its key, for the far search.
    bounds: Vec<(Vec3, Vec3, (i32, i32, i32))>,
}

impl Nearest {
    fn cube(p: Vec3) -> (i32, i32, i32) {
        ((p.x / 32.0).floor() as i32, (p.y / 32.0).floor() as i32, (p.z / 32.0).floor() as i32)
    }

    pub(crate) fn build(views: &[PlayerView]) -> Nearest {
        let mut cubes: FastMap<(i32, i32, i32), Vec<Vec3>> = Default::default();
        for p in views.iter().filter(|v| !v.spectator).map(|v| v.pos) {
            cubes.entry(Self::cube(p)).or_default().push(p);
        }
        let mut bounds: Vec<(Vec3, Vec3, (i32, i32, i32))> = cubes
            .iter()
            .map(|(&k, ps)| {
                let lo = ps.iter().fold(ps[0], |a, b| Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)));
                let hi = ps.iter().fold(ps[0], |a, b| Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)));
                (lo, hi, k)
            })
            .collect();
        bounds.sort_unstable_by_key(|b| b.2);
        Nearest { cubes, bounds }
    }

    /// The squared distance from `p` to the nearest player, as a scan of every player gives it.
    pub(crate) fn nearest_sqr(&self, p: Vec3) -> Option<f64> {
        let c = Self::cube(p);
        let mut best: Option<f64> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for q in self.cubes.get(&(c.0 + dx, c.1 + dy, c.2 + dz)).into_iter().flatten() {
                        let d = q.distance_to_sqr(p);
                        if best.is_none_or(|b| d < b) {
                            best = Some(d);
                        }
                    }
                }
            }
        }
        if let Some(d) = best
            && d <= 32.0 * 32.0
        {
            return Some(d);
        }
        // Farther: the other cubes whose players' bounds could hold someone nearer. Each
        // axis's gap to the bounds is at most that axis's difference for any player inside,
        // and the squares add up in the same order, so the bound never exceeds a distance.
        for (lo, hi, k) in &self.bounds {
            if (k.0 - c.0).abs() <= 1 && (k.1 - c.1).abs() <= 1 && (k.2 - c.2).abs() <= 1 {
                continue;
            }
            let gap = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
            let (gx, gy, gz) = (gap(p.x, lo.x, hi.x), gap(p.y, lo.y, hi.y), gap(p.z, lo.z, hi.z));
            if best.is_some_and(|b| gx * gx + gy * gy + gz * gz >= b) {
                continue;
            }
            for q in &self.cubes[k] {
                let d = q.distance_to_sqr(p);
                if best.is_none_or(|b| d < b) {
                    best = Some(d);
                }
            }
        }
        best
    }
}

/// Maps of small integer keys (entity ids, sections): looked up several times per entity and tick.
type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<kiln_entity::memory::FastHasher>>;

/// Entities by section, like vanilla's `EntitySectionStorage`.
#[derive(Default)]
struct Grid {
    cells: FastMap<(i32, i32, i32), Vec<usize>>,
    at: Vec<(i32, i32, i32)>,
    /// The ids of the list in its order (ascending): looked up by binary search, which over the
    /// entities themselves (hundreds of bytes each) misses the cache at every step.
    ids: Vec<i32>,
}

fn section_of(p: [f64; 3]) -> (i32, i32, i32) {
    ((p[0].floor() as i32) >> 4, (p[1].floor() as i32) >> 4, (p[2].floor() as i32) >> 4)
}

impl Grid {
    fn build(list: &[Entity]) -> Grid {
        let mut g = Grid { cells: Default::default(), at: Vec::with_capacity(list.len()), ids: Vec::with_capacity(list.len()) };
        for (i, e) in list.iter().enumerate() {
            let s = section_of(e.pos);
            g.cells.entry(s).or_default().push(i);
            g.at.push(s);
            g.ids.push(e.id);
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
    /// Entity or player `id` as the source of a game event.
    fn game_event_source(&self, id: i32) -> Option<kiln_entity::vibration::EventSource> {
        if id == self.current
            && let Some(s) = self.current_source
        {
            return Some(s);
        }
        let e = self.entity(id)?;
        Some(kiln_entity::vibration::source_of(e, self))
    }

    fn index(&self, id: i32) -> Option<usize> {
        self.grid.ids.binary_search(&id).ok()
    }

    fn index_players(&mut self) {
        self.proxy_at = self.proxies.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        self.view_index = kiln_entity::level::PlayerGrid::build(&self.views);
        self.proxy_grid = proxy_sections(&self.proxies);
    }

    /// Logs `id` for [`SimLevel::touched`] (once, with its box and liveness as they were).
    fn note_touched(&mut self, id: i32) {
        if self.touched.as_ref().is_some_and(|t| t.iter().any(|&(t, _, _)| t == id)) {
            return;
        }
        let Some(e) = self.entity(id) else { return };
        let (b, alive) = (e.bounding_box(), e.is_alive());
        if let Some(t) = self.touched.as_mut() {
            t.push((id, b, alive));
        }
    }

    fn proxy_index(&self, id: i32) -> Option<usize> {
        if self.proxy_at.is_empty() { self.proxies.iter().position(|e| e.id == id) } else { self.proxy_at.get(&id).copied() }
    }

    /// [`EntityLevel::entities_in`], with the players' stand-ins only if `players`.
    fn entities_in_with(&self, area: &Aabb, filter: EntityFilter, exclude: i32, players: bool) -> Vec<i32> {
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
        let mut found: SmallVec<[((i32, i64), i32); 32]> = SmallVec::new();
        let span = (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64 * (hi.2 - lo.2 + 1) as i64;
        if span > self.grid.cells.len() as i64 * 4 {
            found.extend(self.list.iter().filter_map(|e| e.phys.as_deref()).filter(|e| wanted(e)).map(|e| (section_key(e), e.id)));
        } else {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        let Some(v) = self.grid.cells.get(&(x, y, z)) else { continue };
                        for &i in v {
                            if let Some(e) = self.list.get(i).and_then(|e| e.phys.as_deref())
                                && wanted(e)
                            {
                                found.push((section_key(e), e.id));
                            }
                        }
                    }
                }
            }
        }
        if !players {
            // Stand-ins left out.
        } else if self.proxy_grid.is_empty() || span > self.proxy_grid.len() as i64 * 4 {
            found.extend(self.proxies.iter().filter(|e| e.wanted(filter, exclude, area)).map(|e| (e.section_key(), e.id)));
        } else {
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        let Some(v) = self.proxy_grid.get(&(x, y, z)) else { continue };
                        found.extend(v.iter().map(|&i| &self.proxies[i]).filter(|e| e.wanted(filter, exclude, area)).map(|e| (e.section_key(), e.id)));
                    }
                }
            }
        }
        found.sort_unstable();
        // (Read in place: moving the inline array out costs more than the search.)
        found.iter().map(|&(_, id)| id).collect()
    }
}

/// Vanilla iterates entity sections by x, then by the packed (z, y) section key.
fn section_key(e: &kiln_entity::Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl EntityLevel for SimLevel<'_, '_, '_> {
    fn biome(&self, pos: BlockPos) -> Option<i32> {
        Some(crate::spawner::biome_at_in(self.level.cells(), self.level.env(), kb(pos)) as i32)
    }

    fn piglins_zombify(&self) -> bool {
        !self.level.env().rules.fast_lava
    }

    fn snow_golem_melts(&self, pos: Vec3) -> bool {
        if self.level.env().rules.fast_lava {
            return true;
        }
        // The biomes whose `minecraft:gameplay/snow_golem_melts` is on.
        const HOT: [&str; 7] = [
            "minecraft:badlands",
            "minecraft:desert",
            "minecraft:eroded_badlands",
            "minecraft:savanna",
            "minecraft:savanna_plateau",
            "minecraft:windswept_savanna",
            "minecraft:wooded_badlands",
        ];
        let b = crate::spawner::biome_at_in(self.level.cells(), self.level.env(), kb(BlockPos::containing(pos.x, pos.y, pos.z))) as i32;
        HOT.iter().any(|n| kiln_data::synced_id("minecraft:worldgen/biome", n) == Some(b))
    }

    fn trade_offers(&mut self, set: &str, merchant: &kiln_entity::level::TradeMerchant) -> Vec<kiln_item::trading::MerchantOffer> {
        let env = self.level.env();
        crate::trading::roll_offers(env.loot.as_deref(), env.seed, env.game_time, set, merchant)
    }

    fn raid(&self, id: i32) -> Option<&kiln_entity::level::RaidView> {
        self.level.env().raids.iter().find(|r| r.id == id)
    }

    fn raid_at(&self, pos: BlockPos) -> Option<&kiln_entity::level::RaidView> {
        crate::raid::raid_at_view(&self.level.env().raids, pos)
    }

    fn village_centers_near(&self, section: (i32, i32, i32), radius: i32) -> Option<Vec<(i32, i32, i32)>> {
        Some(crate::poi::village_centers(self.level.cells(), [section.0, section.1, section.2], radius))
    }

    fn sections_to_village(&self, pos: BlockPos) -> i32 {
        kiln_entity::prof!("lvl", "sections_to_village");
        crate::poi::sections_to_village(self.level.cells(), [pos.x, pos.y, pos.z])
    }

    fn poi_in_range(&self, types: &[&str], center: BlockPos, radius: i32, occupancy: kiln_entity::level::PoiOccupancy) -> Vec<BlockPos> {
        kiln_entity::prof!("lvl", "poi_in_range");
        let kinds = crate::poi::kinds_of(types);
        crate::poi::in_range(self.level.cells(), &kinds, [center.x, center.y, center.z], radius, occupancy_of(occupancy))
            .into_iter()
            .map(|r| BlockPos::new(r.pos[0], r.pos[1], r.pos[2]))
            .collect()
    }

    fn poi_take(&mut self, types: &[&str], center: BlockPos, radius: i32, accept: &dyn Fn(&str, BlockPos) -> bool) -> Option<BlockPos> {
        let kinds = crate::poi::kinds_of(types);
        let accept = |k: u8, p: [i32; 3]| accept(kiln_world::poi::TYPES[k as usize].name, BlockPos::new(p[0], p[1], p[2]));
        let level = self.level.region()?;
        crate::poi::take(&mut *level.cells, &kinds, [center.x, center.y, center.z], radius, &accept).map(|p| BlockPos::new(p[0], p[1], p[2]))
    }

    fn poi_release(&mut self, pos: BlockPos) {
        if let Some(level) = self.level.region() {
            crate::poi::release(&mut *level.cells, [pos.x, pos.y, pos.z]);
        }
    }

    fn poi_type(&self, pos: BlockPos) -> Option<&'static str> {
        crate::poi::type_at(self.level.cells(), [pos.x, pos.y, pos.z]).map(|k| kiln_world::poi::TYPES[k as usize].name)
    }

    fn motion_blocking_no_leaves_height(&self, x: i32, z: i32) -> i32 {
        use kiln_world::Blocks;
        let Some(chunk) = self.level.cells().chunk(ChunkPos::of_block(x, z)) else { return self.level.env().min_y };
        chunk.column_height((x & 15) as usize, (z & 15) as usize, |s| {
            kiln_data::block_props::motion_blocking(s) && !kiln_data::blocks_types::block_of(s).name.ends_with("_leaves")
        })
    }

    fn block(&self, pos: BlockPos) -> u16 {
        self.level.block(kb(pos))
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.level.is_loaded(kb(pos))
    }

    fn read_blocks(&self, min: BlockPos, max: BlockPos, out: &mut [u16]) -> bool {
        use kiln_world::Blocks;
        let (dx, dz) = ((max.x - min.x + 1) as usize, (max.z - min.z + 1) as usize);
        for cz in (min.z >> 4)..=(max.z >> 4) {
            for cx in (min.x >> 4)..=(max.x >> 4) {
                let Some(chunk) = self.level.cells().chunk(ChunkPos::new(cx, cz)) else { return false };
                let (x0, x1) = (min.x.max(cx * 16), max.x.min(cx * 16 + 15));
                let (z0, z1) = (min.z.max(cz * 16), max.z.min(cz * 16 + 15));
                let origin = (z0 - min.z) as usize * dx + (x0 - min.x) as usize;
                chunk.read_box(
                    ((x0 & 15) as usize, (x1 & 15) as usize),
                    (min.y, max.y),
                    ((z0 & 15) as usize, (z1 & 15) as usize),
                    out,
                    origin,
                    dx,
                    dx * dz,
                );
            }
        }
        true
    }

    fn no_fluid_in(&self, min: BlockPos, max: BlockPos) -> bool {
        use kiln_world::Blocks;
        for cz in (min.z >> 4)..=(max.z >> 4) {
            for cx in (min.x >> 4)..=(max.x >> 4) {
                match self.level.cells().chunk(ChunkPos::new(cx, cz)) {
                    Some(chunk) if !chunk.may_have_fluid(min.y, max.y) => {}
                    _ => return false,
                }
            }
        }
        true
    }

    fn blocks_epoch(&self, min: BlockPos, max: BlockPos) -> Option<kiln_entity::level::BlocksEpoch> {
        use kiln_world::Blocks;
        let (x0, x1, z0, z1) = (min.x >> 4, max.x >> 4, min.z >> 4, max.z >> 4);
        if x1 - x0 > 1 || z1 - z0 > 1 {
            return None;
        }
        let mut key = [0u64; 4];
        for (i, (cz, cx)) in (z0..=z1).flat_map(|cz| (x0..=x1).map(move |cx| (cz, cx))).enumerate() {
            key[i] = self.level.cells().chunk(ChunkPos::new(cx, cz))?.block_epoch();
        }
        Some(kiln_entity::level::BlocksEpoch(key))
    }

    fn any_block_in(&self, min: BlockPos, max: BlockPos, pred: &dyn Fn(u16) -> bool) -> bool {
        use kiln_world::Blocks;
        let void = kiln_data::blocks::default_state::VOID_AIR;
        for cx in (min.x >> 4)..=(max.x >> 4) {
            for cz in (min.z >> 4)..=(max.z >> 4) {
                let (x0, x1) = (min.x.max(cx * 16), max.x.min(cx * 16 + 15));
                let (z0, z1) = (min.z.max(cz * 16), max.z.min(cz * 16 + 15));
                match self.level.cells().chunk(ChunkPos::new(cx, cz)) {
                    Some(chunk) => {
                        let local = |a: i32, b: i32, o: i32| ((a - o) as usize, (b - o) as usize);
                        if chunk.any_block_in(local(x0, x1, cx * 16), (min.y, max.y), local(z0, z1, cz * 16), pred) {
                            return true;
                        }
                    }
                    None => {
                        if pred(void) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn set_block(&mut self, pos: BlockPos, state: u16, flags: u32) -> bool {
        self.level.set_block(kb(pos), state, flags)
    }

    fn destroy_block(&mut self, pos: BlockPos, drop: bool) -> bool {
        self.level.destroy_block(kb(pos), drop)
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.rng
    }

    fn game_time(&self) -> i64 {
        self.level.env().game_time
    }

    fn day_time(&self) -> i64 {
        self.level.env().mobs.day_time
    }

    fn min_y(&self) -> i32 {
        self.level.env().min_y
    }

    fn max_y(&self) -> i32 {
        self.level.env().min_y + self.level.env().height - 1
    }

    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        kiln_entity::prof!("lvl", "entities_in");
        self.entities_in_with(area, filter, exclude, true)
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut kiln_entity::Entity> {
        if self.touched.is_some() {
            self.note_touched(id);
        }
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_deref_mut();
        }
        let i = self.proxy_index(id)?;
        self.proxies.get_mut(i).map(Proxy::get_mut)
    }

    fn entity(&self, id: i32) -> Option<&kiln_entity::Entity> {
        if let Some(i) = self.index(id) {
            return self.list[i].phys.as_deref();
        }
        let i = self.proxy_index(id)?;
        self.proxies.get(i).map(Proxy::get)
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
        let mut h = (self.level.env().game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.level.env().seed as u64;
        for v in [self.current as u64, self.seeds] {
            h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        h as i64
    }

    fn players(&self) -> &[PlayerView] {
        &self.views
    }

    fn players_in(&self, area: &Aabb) -> Vec<PlayerView> {
        kiln_entity::prof!("lvl", "players_in");
        self.view_index.in_area(&self.views, area)
    }

    fn enchant_from_provider(&self, stack: &mut kiln_item::ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn kiln_javamath::random::RandomSource) {
        if let Some(loot) = self.level.env().loot.as_deref() {
            crate::enchant::enchant_from_provider(loot, stack, provider, special_multiplier, random);
        }
    }

    fn player(&self, id: i32) -> Option<PlayerView> {
        self.view_index.by_id(&self.views, id)
    }

    fn player_by_uuid(&self, uuid: u128) -> Option<PlayerView> {
        self.view_index.by_uuid(&self.views, uuid)
    }

    fn emit(&mut self, event: Event) {
        if let Event::GameEvent { event, pos, entity } = event {
            // `ServerLevel.gameEvent`: listeners hear it now.
            if self.level.listening() {
                let source = entity.and_then(|id| self.game_event_source(id));
                if let Some(l) = self.level.region() {
                    crate::sculk::post(l, event, pos, kiln_entity::vibration::Context { source, affected_state: None });
                }
            }
            return;
        }
        self.events.push(event);
    }

    fn block_game_event(&mut self, event: &'static str, pos: Vec3, entity: Option<i32>, state: u16) {
        if self.level.listening() {
            let source = entity.and_then(|id| self.game_event_source(id));
            if let Some(l) = self.level.region() {
                crate::sculk::post(l, event, pos, kiln_entity::vibration::Context { source, affected_state: Some(state) });
            }
        }
    }

    fn sculk_step_on(&mut self, pos: BlockPos, entity: i32, at: Vec3) {
        if let Some(mut source) = self.game_event_source(entity) {
            source.pos = at;
            if let Some(l) = self.level.region() {
                crate::sculk::step_on(l, kb(pos), source);
            }
        }
    }

    fn sculk_catalyst_near(&self, pos: Vec3) -> bool {
        self.level.region_ref().is_some_and(|l| crate::sculk::catalyst::nearest(l, pos).is_some())
    }

    fn feed_sculk_catalyst(&mut self, pos: Vec3, charge: i32) {
        if let Some(l) = self.level.region()
            && let Some(at) = crate::sculk::catalyst::nearest(l, pos)
        {
            crate::sculk::catalyst::feed(l, at, pos, charge);
        }
    }

    fn take_vibrations(&mut self, id: i32) -> Vec<kiln_entity::vibration::Heard> {
        self.level.region().map(|l| l.blocks.sculk.take_heard(id)).unwrap_or_default()
    }

    fn set_listener(&mut self, id: i32, ear: Option<kiln_entity::vibration::Ear>) {
        if let Some(l) = self.level.region() {
            l.blocks.sculk.set_warden(id, ear);
        }
    }

    fn take_allay_vibrations(&mut self, id: i32) -> Vec<kiln_entity::vibration::Heard> {
        self.level.region().map(|l| l.blocks.sculk.take_heard_allay(id)).unwrap_or_default()
    }

    fn set_allay_listener(&mut self, id: i32, ear: Option<kiln_entity::vibration::Ear>) {
        if let Some(l) = self.level.region() {
            l.blocks.sculk.set_allay(id, ear);
        }
    }

    fn vibration_particle(&mut self, from: Vec3, entity: i32, y_offset: f32, ticks: i32) {
        let dest = kiln_proto::packets::world_fx::PositionSource::Entity { id: entity, y_offset };
        if let Some(l) = self.level.region() {
            crate::sculk::send_vibration_particle(l, from, dest, ticks);
        }
    }

    fn darkness_around(&mut self, pos: Vec3, radius: f64) {
        self.player_writes += 1;
        crate::sculk::shrieker::darkness_around(self.players, [pos.x, pos.y, pos.z], radius);
    }

    fn particle(&mut self, particle: &'static str, pos: Vec3) {
        if let Some(p) = particle_packet(particle, pos) {
            self.level.push_packet(p);
        }
    }

    fn trail_particle(&mut self, pos: Vec3, target: Vec3, color: i32, duration: i32) {
        if let Some(p) = trail_packet(pos, target, color, duration) {
            self.level.push_packet(p);
        }
    }

    fn crumble_particles(&mut self, pos: Vec3, state: u16, count: i32, spread: Vec3) {
        if let Some(p) = crumble_packet(pos, state, count, spread) {
            self.level.push_packet(p);
        }
    }

    fn mob_griefing(&self) -> bool {
        self.level.env().mobs.griefing
    }

    fn universal_anger(&self) -> bool {
        self.level.env().mobs.universal_anger
    }

    fn forgive_dead_players(&self) -> bool {
        self.level.env().mobs.forgive_dead_players
    }

    fn ender_pearls_vanish_on_death(&self) -> bool {
        self.level.env().mobs.ender_pearls_vanish
    }

    fn explosion_drop_decay(&self, rule: kiln_entity::explosion::DecayRule) -> bool {
        let decay = self.level.env().mobs.explosion_decay;
        match rule {
            kiln_entity::explosion::DecayRule::Block => decay[0],
            kiln_entity::explosion::DecayRule::Mob => decay[1],
            kiln_entity::explosion::DecayRule::Tnt => decay[2],
        }
    }

    fn dragon_fight(&self) -> Option<kiln_entity::level::DragonFightView> {
        self.level.env().dragon_fight.as_ref().map(|f| f.view)
    }

    fn mob_drops(&self) -> bool {
        self.level.env().mobs.drops
    }

    fn entity_drops(&self) -> bool {
        self.level.env().mobs.entity_drops
    }

    fn tnt_explodes(&self) -> bool {
        self.level.env().rules.tnt_explodes
    }

    fn fill_container_loot(&mut self, items: &mut [kiln_item::ItemStack], table: &str, seed: i64, origin: Vec3, player: Option<i32>) {
        let env = self.level.env();
        let Some(loot) = env.loot.clone() else { return };
        let at = [origin.x.floor() as i32, origin.y.floor() as i32, origin.z.floor() as i32];
        crate::container::fill_from_table(items, &loot, table, seed, arr(origin), at, player.is_some(), env.game_time, env.seed);
        // `unpackChestVehicleLootTable(player)`: `player_generates_container_loot`.
        if let Some(pid) = player
            && let Some(p) = self.players.iter_mut().find(|p| p.entity_id == pid).inspect(|_| self.player_writes += 1)
        {
            let table = kiln_item::ident::Identifier::parse(table).map_or(table.to_owned(), |i| i.to_string());
            p.fire_conds("minecraft:player_generates_container_loot", None, |c, _, _| {
                c.get("loot_tables").and_then(|v| v.as_str()).and_then(kiln_item::ident::Identifier::parse).is_some_and(|i| i.to_string() == table)
            });
        }
    }

    fn block_loot(&mut self, state: u16, origin: Vec3, tool: &kiln_item::ItemStack, entity: i32) -> Vec<kiln_item::ItemStack> {
        block_loot_in(self.level.env(), state, origin, tool, entity)
    }

    fn hopper_take_from_block(&mut self, pos: BlockPos, dest: &mut Vec<kiln_item::ItemStack>) -> Option<bool> {
        crate::container::hopper::take_into_cart(self.level.region()?, kb(pos), dest)
    }

    fn difficulty(&self) -> u8 {
        self.level.env().mobs.difficulty
    }

    fn creaking_active(&self, _pos: BlockPos) -> bool {
        self.level.env().mobs.creaking_active
    }

    fn spawning_monsters(&self) -> bool {
        self.level.env().mobs.spawn_mobs && self.level.env().mobs.spawn_monsters
    }

    /// A creaking whose heart is in a chunk that is not loaded stays as it is (vanilla loads
    /// the chunk to look).
    fn heart_protects(&mut self, home: BlockPos, id: i32, uuid: u128) -> bool {
        let h = kb(home);
        !self.level.is_loaded(h) || self.level.region_ref().is_none_or(|l| crate::heart::protects(l, h, id, uuid))
    }

    fn heart_creaking_hurt(&mut self, home: BlockPos, id: i32, uuid: u128, at: Vec3) {
        crate::heart::with_heart(self, kb(home), |sim, be| kiln_entity::mob::kinds::creaking_heart::creaking_hurt(sim, home, be, id, uuid, at));
    }

    fn entity_by_uuid(&self, uuid: u128) -> Option<&kiln_entity::Entity> {
        self.list.iter().find(|e| e.uuid.as_u128() == uuid && !e.removed).and_then(|e| e.phys.as_deref()).or_else(|| self.proxies.iter().find(|p| p.uuid == uuid).map(Proxy::get))
    }

    fn known_movement(&self, id: i32) -> Vec3 {
        if let Some(i) = self.proxy_index(id)
            && let Some(p) = self.players.iter().find(|p| p.entity_id == self.proxies[i].id)
        {
            return vec3(p.known_movement);
        }
        self.entity(id).map_or(Vec3::ZERO, |e| e.delta)
    }

    fn pos_random(&mut self, pos: BlockPos, salt: i64) -> LegacyRandom {
        crate::container::pos_random_in(self.level.env(), kb(pos), salt as u64)
    }

    fn update_neighbours_for_output_signal(&mut self, pos: BlockPos) {
        let p = kb(pos);
        let s = self.level.block(p);
        self.level.change(move |l| kiln_blocks::update::update_neighbour_for_output_signal(l, p, kiln_blocks::BlockId::of(s)));
    }

    /// The entity keeps the UUID it was made with (a heart holds its creaking by it).
    fn add_entity_with_uuid(&mut self, entity: kiln_entity::Entity) {
        let Some(kind) = kiln_data::entities::by_name(entity.type_name) else { return };
        self.spawns.push(Spawn { kind, pos: arr(entity.position()), vel: arr(entity.delta), body: Body::Loaded(Box::new(entity)) });
    }

    fn sky_darken(&self) -> i32 {
        self.level.env().mobs.sky_darken
    }

    fn spawner_blocks_enabled(&self) -> bool {
        self.level.env().mobs.spawner_blocks
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        kiln_world::light::light_at(self.level.cells(), kiln_world::chunk::LightLayer::Block, pos.x, pos.y, pos.z).map_or(0, i32::from)
    }

    /// The dimension type's `monster_spawn_block_light_limit` and `monster_spawn_light_level`.
    fn monster_light_rules(&self) -> (i32, i32, i32) {
        crate::spawner::monster_light_rules(self.level.env().dim)
    }

    fn moon_brightness(&self) -> f32 {
        crate::spawner::moon_brightness(self.level.env().mobs.day_time)
    }

    fn world_seed(&self) -> i64 {
        self.level.env().seed
    }

    fn pending_spawns(&self, area: &Aabb) -> Vec<(&'static str, Aabb, bool)> {
        self.spawns
            .iter()
            .filter_map(|s| {
                let living = kiln_entity::mob::MobKind::by_name(s.kind.name).is_some();
                let (hw, h) = (s.kind.width as f64 / 2.0, s.kind.height as f64);
                let b = Aabb::new(s.pos[0] - hw, s.pos[1], s.pos[2] - hw, s.pos[0] + hw, s.pos[1] + h, s.pos[2] + hw);
                b.intersects(area).then_some((s.kind.name, b, living))
            })
            .collect()
    }

    fn add_entity_stack(&mut self, root: kiln_entity::Entity, companions: Vec<kiln_entity::mob::Companion>, loaded: bool, nearby_chicken: bool) -> bool {
        let Some(kind) = kiln_data::entities::by_name(root.type_name) else { return false };
        let (pos, vel) = (arr(root.position()), arr(root.delta));
        let body = if companions.is_empty() && !nearby_chicken { Body::Loaded(Box::new(root)) } else { Body::Stacked(Box::new(root), companions, loaded, nearby_chicken) };
        self.spawns.push(Spawn { kind, pos, vel, body });
        true
    }

    fn is_raining_at(&self, pos: BlockPos) -> bool {
        crate::weather::is_raining_at(self.level.cells(), self.level.env(), kb(pos))
    }

    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        self.level.region_ref().is_some_and(|l| kiln_blocks::Level::can_spread_fire_around(l, kb(pos)))
    }

    fn place_lightning_fire(&mut self, pos: BlockPos) -> bool {
        self.level.region().is_some_and(|l| crate::weather::place_lightning_fire(l, kb(pos)))
    }

    fn lightning_strike_block(&mut self, pos: BlockPos) {
        let pos = kb(pos);
        self.level.change(move |l| kiln_blocks::weather::lightning_strike(l, pos));
    }

    fn thunder_hit_player(&mut self, id: i32) {
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) else { return };
        // `Entity.thunderHit`: one more tick of fire, 8 seconds if that made it 0.
        let ticks = p.fire_ticks + 1;
        p.set_fire_ticks(if ticks == 0 { 160 } else { ticks });
        let source = health::Source { cause: health::Cause::Entity(DamageKind::LightningBolt), attacker: None, direct: None, weapon: None, position: None };
        let env = self.level.env();
        let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
        p.hurt(5.0, &source, &mut ctx);
    }

    fn monsters_burn(&self) -> bool {
        self.level.env().mobs.monsters_burn
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        self.level.raw_brightness(kb(pos), sky_darken)
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        kiln_world::light::light_at(self.level.cells(), kiln_world::chunk::LightLayer::Sky, pos.x, pos.y, pos.z)
            .map_or(if pos.y >= self.level.env().min_y + self.level.env().height { 15 } else { 0 }, i32::from)
    }

    fn effective_difficulty(&self, _pos: BlockPos) -> f32 {
        crate::mobs::difficulty_instance(self.level.env().mobs.difficulty, self.level.env().game_time, 0, 1.0).effective_difficulty
    }

    fn hurt_player(&mut self, id: i32, source: kiln_entity::mob::DamageSource, amount: f32) -> bool {
        // A player's projectile credits the player.
        let player_attacker = source.attacker.and_then(|a| self.players.iter().find(|p| p.entity_id == a).map(|p| p.as_attacker()));
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) else { return false };
        let current = self.current;
        let current_info = self.current_info;
        let attacker = player_attacker.or_else(|| {
            let a = source.attacker?;
            if a == current && let Some((type_name, pos)) = current_info {
                return Some(health::Attacker::mob(a, type_name, pos));
            }
            let e = self.list.binary_search_by_key(&a, |e| e.id).ok().and_then(|i| self.list[i].phys.as_deref())?;
            Some(health::Attacker::mob(a, e.type_name, arr(e.position())))
        });
        // A projectile's hit judges blocking from where it comes (`getSourcePosition`).
        let position = source.direct.filter(|_| source.kind.is_tag("minecraft:is_projectile")).and(source.pos).map(arr);
        let source = health::Source { cause: health::Cause::Entity(source.kind), attacker, direct: source.direct.filter(|d| Some(*d) != source.attacker), weapon: None, position };
        let env = self.level.env();
        let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
        let hurt = p.hurt(amount, &source, &mut ctx);
        if hurt && let Some(a) = source.attacker.as_ref().map(|a| a.id) {
            p.last_hurt_by_mob = Some((a, env.game_time));
        }
        hurt
    }

    fn push(&mut self, id: i32, v: Vec3) {
        // A player's client owns its motion: it gets the push.
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) {
            p.vel = [p.vel[0] + v.x, p.vel[1] + v.y, p.vel[2] + v.z];
            p.sync_velocity = true;
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            e.delta = e.delta + v;
            e.needs_sync = true;
        }
    }

    fn motion(&self, id: i32) -> Vec3 {
        if let Some(p) = self.players.iter().find(|p| p.entity_id == id) {
            return vec3(p.vel);
        }
        self.entity(id).map_or(Vec3::ZERO, |e| e.delta)
    }

    fn knockback_target(&mut self, id: i32, strength: f64, dx: f64, dz: f64, old_motion: Vec3) {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) {
            p.knockback(strength, dx, dz);
            // (A player is told at once; the server keeps the motion it had.)
            if p.sync_velocity {
                p.send(kiln_proto::packets::entity::set_entity_motion(p.entity_id, p.vel));
                p.sync_velocity = false;
                p.vel = arr(old_motion);
            }
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            kiln_entity::mob::knockback_entity(e, strength, dx, dz);
        }
    }

    fn stop_riding(&mut self, id: i32) {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) {
            if let Some(v) = p.vehicle.take() {
                p.vehicle_type = None;
                if let Some(ve) = self.entity_mut(v) {
                    kiln_entity::ride::remove_passenger(ve, id);
                }
            }
            return;
        }
        kiln_entity::level::stop_riding_entity(self, id);
    }

    fn is_thundering(&self) -> bool {
        self.level.env().weather.weather.thundering
    }

    fn add_effect(&mut self, id: i32, effect: &'static str, duration: i32, amplifier: i32, _source: Option<i32>) -> bool {
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) else { return false };
        let Some(e) = crate::effects::effect_id(effect) else { return false };
        p.add_effect(crate::effects::Effect::simple(e, duration, amplifier))
    }

    fn add_effect_instance(&mut self, id: i32, effect: kiln_entity::effect::Effect, source: Option<i32>) -> bool {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) {
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
            let e = self.list.binary_search_by_key(&o, |e| e.id).ok().and_then(|i| self.list[i].phys.as_deref())?;
            Some(health::Attacker::mob(o, e.type_name, arr(e.position())))
        });
        let env = self.level.env();
        let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) else { return };
        match effect.kind() {
            kiln_entity::effect::Kind::HealOrHarm { harm: false } => {
                p.heal((scale * 4i32.wrapping_shl(effect.amplifier as u32) as f64 + 0.5) as i32 as f32);
            }
            kiln_entity::effect::Kind::HealOrHarm { harm: true } => {
                let amount = (scale * 6i32.wrapping_shl(effect.amplifier as u32) as f64 + 0.5) as i32 as f32;
                let source = match source {
                    None => health::Source { cause: health::Cause::Entity(DamageKind::Magic), attacker: None, direct: None, weapon: None, position: None },
                    Some((direct, _)) => health::Source { cause: health::Cause::Entity(DamageKind::IndirectMagic), attacker, direct: Some(direct), weapon: None, position: None },
                };
                let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns: self.spawns, deaths: self.deaths, level_rng: None };
                p.hurt(amount, &source, &mut ctx);
            }
            kiln_entity::effect::Kind::Saturation => p.eat(effect.amplifier + 1, (effect.amplifier + 1) as f32 * 2.0),
            _ => {}
        }
    }

    fn max_entity_cramming(&self) -> i32 {
        self.level.env().mobs.cramming
    }

    fn player_effect(&self, id: i32, effect: &str) -> Option<(i32, i32)> {
        let p = self.players.iter().find(|p| p.entity_id == id)?;
        let e = p.effects.get(&crate::effects::effect_id(effect)?)?;
        Some((e.amplifier, e.duration))
    }

    fn ignite(&mut self, id: i32, seconds: f32) {
        if let Some(p) = self.players.iter_mut().find(|p| p.entity_id == id).inspect(|_| self.player_writes += 1) {
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

/// [`EntityLevel::block_loot`].
fn block_loot_in(env: &blocks::BlockEnv, state: u16, origin: Vec3, tool: &kiln_item::ItemStack, entity: i32) -> Vec<kiln_item::ItemStack> {
    let Some(loot) = env.loot.clone() else {
        return kiln_item::ItemStack::of(kiln_entity::blocks::block_name(state), 1).into_iter().collect();
    };
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, entity, 0x626c_6f63);
    crate::blocks::block_items(&loot, arr(origin), state, Some(tool.clone()), None, seed)
}

/// A block packet for the players near a point.
type NearPacket = ([f64; 3], f64, Bytes);

/// [`EntityLevel::particle`]'s packet.
fn particle_packet(particle: &'static str, pos: Vec3) -> Option<NearPacket> {
    let kind = kiln_data::builtin_id("minecraft:particle_type", particle)?;
    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
        particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::None },
        override_limiter: false,
        always_show: false,
        pos: [pos.x, pos.y, pos.z],
        offset: [0.0; 3],
        max_speed: [0.0; 3],
        count: 1,
        randomization: world_fx::ParticleRandomization::Default,
    });
    Some(([pos.x, pos.y, pos.z], 32.0, pkt))
}

/// [`EntityLevel::trail_particle`]'s packet.
fn trail_packet(pos: Vec3, target: Vec3, color: i32, duration: i32) -> Option<NearPacket> {
    let kind = kiln_data::builtin_id("minecraft:particle_type", "minecraft:trail")?;
    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
        particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::Trail { target: [target.x, target.y, target.z], color, duration } },
        override_limiter: true,
        always_show: true,
        pos: [pos.x, pos.y, pos.z],
        offset: [0.0; 3],
        max_speed: [0.0; 3],
        count: 1,
        randomization: world_fx::ParticleRandomization::Default,
    });
    // `overrideLimiter`: players within 512 blocks.
    Some(([pos.x, pos.y, pos.z], 512.0, pkt))
}

/// [`EntityLevel::crumble_particles`]'s packet.
fn crumble_packet(pos: Vec3, state: u16, count: i32, spread: Vec3) -> Option<NearPacket> {
    let kind = kiln_data::builtin_id("minecraft:particle_type", "minecraft:block_crumble")?;
    let pkt = world_fx::level_particles(&world_fx::LevelParticles {
        particle: world_fx::Particle { kind, options: world_fx::ParticleOptions::Block(state as i32) },
        override_limiter: false,
        always_show: false,
        pos: [pos.x, pos.y, pos.z],
        offset: [spread.x as f32, spread.y as f32, spread.z as f32],
        max_speed: [0.0; 3],
        count,
        randomization: world_fx::ParticleRandomization::Default,
    });
    Some(([pos.x, pos.y, pos.z], 32.0, pkt))
}

fn occupancy_of(o: kiln_entity::level::PoiOccupancy) -> kiln_world::poi::Occupancy {
    match o {
        kiln_entity::level::PoiOccupancy::HasSpace => kiln_world::poi::Occupancy::HasSpace,
        kiln_entity::level::PoiOccupancy::IsOccupied => kiln_world::poi::Occupancy::IsOccupied,
        kiln_entity::level::PoiOccupancy::Any => kiln_world::poi::Occupancy::Any,
    }
}

/// The stand-ins by entity section.
fn proxy_sections(proxies: &[Proxy]) -> FastMap<(i32, i32, i32), Vec<usize>> {
    let mut m: FastMap<(i32, i32, i32), Vec<usize>> = FastMap::default();
    for (i, e) in proxies.iter().enumerate() {
        let p = e.position();
        m.entry(section_of([p.x, p.y, p.z])).or_default().push(i);
    }
    m
}

/// The entity phase's indexes, built side by side (tens of microseconds each in a crowd).
const INDEX_WINDOW: kiln_sched::Window = kiln_sched::Window::new().chunk(1).strategy(kiln_sched::Strategy::Parallel);

/// The players' stand-ins and views, a microsecond or so each.
const PLAYER_VIEWS: kiln_sched::Window = kiln_sched::Window::new().item_ns(110);

/// What a player's stand-in is made from: the player as the entities see it
/// (`minecraft:player`, standing or sneaking).
#[derive(Clone, Copy)]
struct ProxySeed {
    id: i32,
    uuid: u128,
    pos: [f64; 3],
    sneaking: bool,
    invulnerable: bool,
}

impl ProxySeed {
    fn of(p: &Player) -> ProxySeed {
        ProxySeed { id: p.entity_id, uuid: p.uuid.as_u128(), pos: p.pos, sneaking: p.sneaking, invulnerable: matches!(p.game_mode, 1 | 3) }
    }

    fn make(&self) -> kiln_entity::Entity {
        let mut e = kiln_entity::Entity::new("minecraft:player", self.id, self.uuid, EntityKind::Other { type_name: "minecraft:player" }, 0);
        if self.sneaking {
            e.height = 1.5;
            e.eye_height = 1.27;
        }
        e.set_pos(vec3(self.pos));
        e.invulnerable = self.invulnerable;
        e
    }
}

/// A player's stand-in, made the first time an entity reaches for it (a crowd's players are
/// mostly only searched, which the seed answers as the made one would).
pub(crate) struct Proxy {
    pub(crate) id: i32,
    pub(crate) uuid: u128,
    seed: ProxySeed,
    made: std::sync::OnceLock<Box<kiln_entity::Entity>>,
}

impl Proxy {
    fn of(p: &Player) -> Proxy {
        let seed = ProxySeed::of(p);
        Proxy { id: seed.id, uuid: seed.uuid, seed, made: std::sync::OnceLock::new() }
    }

    fn get(&self) -> &kiln_entity::Entity {
        self.made.get_or_init(|| Box::new(self.seed.make()))
    }

    fn get_mut(&mut self) -> &mut kiln_entity::Entity {
        if self.made.get().is_none() {
            let _ = self.made.set(Box::new(self.seed.make()));
        }
        self.made.get_mut().expect("just made")
    }

    fn made(&self) -> Option<&kiln_entity::Entity> {
        self.made.get().map(|b| &**b)
    }

    fn position(&self) -> Vec3 {
        self.made().map_or_else(|| vec3(self.seed.pos), |e| e.position())
    }

    /// The box a fresh stand-in has (`EntityDimensions.makeBoundingBox`), or the made one's.
    fn bounding_box(&self) -> Aabb {
        if let Some(e) = self.made() {
            return e.bounding_box();
        }
        static SIZE: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
        let (width, height) = *SIZE.get_or_init(|| {
            let t = kiln_data::entities::by_name("minecraft:player").expect("the player type");
            (t.width, t.height)
        });
        let w = width / 2.0;
        let h: f32 = if self.seed.sneaking { 1.5 } else { height };
        let p = self.seed.pos;
        Aabb::new(p[0] - w as f64, p[1], p[2] - w as f64, p[0] + w as f64, p[1] + h as f64, p[2] + w as f64)
    }

    /// [`section_key`] of the stand-in.
    fn section_key(&self) -> (i32, i64) {
        match self.made() {
            Some(e) => section_key(e),
            None => {
                let (sx, sy, sz) = ((self.seed.pos[0].floor() as i32) >> 4, (self.seed.pos[1].floor() as i32) >> 4, (self.seed.pos[2].floor() as i32) >> 4);
                (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
            }
        }
    }

    /// Whether an area search with `filter`, leaving out `exclude`, finds it in `area`.
    fn wanted(&self, filter: EntityFilter, exclude: i32, area: &Aabb) -> bool {
        // A stand-in is an `Other` entity: living, neither an item nor an orb.
        let kind = matches!(filter, EntityFilter::Any | EntityFilter::Living);
        kind && self.id != exclude && self.made().is_none_or(|e| e.is_alive()) && self.bounding_box().intersects(area)
    }
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
    ctx: &kiln_sched::Ctx<'_>,
) {
    if entities.list.is_empty() && players.iter().all(|p| p.vehicle.is_none()) && level.blocks.hearts.is_empty() && (level.blocks.spawners.is_empty() || players.is_empty()) {
        return;
    }
    // Mobs finalized during the tick (reinforcements, summoned vexes) enchant from the datapack.
    let _enchanting = crate::enchant::install_enchanter(level.env.loot.as_ref());
    let dt = std::time::Instant::now();
    // The players' stand-ins and views, made side by side (a crowd has a thousand).
    let now = level.env.game_time;
    let made: Vec<(Option<Proxy>, Option<PlayerView>)> = ctx.map_mut_with(PLAYER_VIEWS, players, |_, p| {
        let alive = !p.disconnected && !p.dead;
        ((alive && p.game_mode != 3).then(|| Proxy::of(p)), alive.then(|| view(p, now)))
    });
    let mut proxies: Vec<Proxy> = Vec::with_capacity(made.len());
    let mut views: Vec<PlayerView> = Vec::with_capacity(made.len());
    for (e, v) in made {
        proxies.extend(e);
        views.extend(v);
    }
    // wp32 parrots: what parrots need to know of their owners' footing.
    if entities.list.iter().any(|e| e.kind.name == "minecraft:parrot") {
        let block = |pos: BlockPos| level.block(kb(pos));
        crate::shoulder::mark_views(players, &block, &mut views);
    }
    let dt = crate::diag::lap("e.views", dt);
    // The indexes over the players and the entities, side by side.
    enum Built {
        Nearest(Nearest),
        Grid(Grid),
        ProxyAt(FastMap<i32, usize>),
        Views(kiln_entity::level::PlayerGrid),
        ProxyGrid(FastMap<(i32, i32, i32), Vec<usize>>),
    }
    let built = {
        let (views, proxies, list) = (&views, &proxies, &entities.list);
        let jobs: [u8; 5] = [0, 1, 2, 3, 4];
        ctx.map_indexed_with(INDEX_WINDOW, &jobs, |_, &k| match k {
            0 => Built::Nearest(Nearest::build(views)),
            1 => Built::Grid(Grid::build(list)),
            2 => Built::ProxyAt(proxies.iter().enumerate().map(|(i, e)| (e.id, i)).collect()),
            3 => Built::Views(kiln_entity::level::PlayerGrid::build(views)),
            _ => Built::ProxyGrid(proxy_sections(proxies)),
        })
    };
    let (mut nearest, mut grid, mut proxy_at, mut view_index, mut proxy_grid) = (None, Grid::default(), FastMap::default(), Default::default(), FastMap::default());
    for b in built {
        match b {
            Built::Nearest(n) => nearest = Some(n),
            Built::Grid(g) => grid = g,
            Built::ProxyAt(m) => proxy_at = m,
            Built::Views(v) => view_index = v,
            Built::ProxyGrid(m) => proxy_grid = m,
        }
    }
    let nearest = nearest.expect("built");
    let Entities { list, spec: pace } = entities;
    let mut sim = SimLevel {
        level: World::Region(level),
        list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: 0,
        seeds: 0,
        current_source: None,
        rng: LegacyRandom::new(0),
        grid,
        proxy_at,
        proxy_grid,
        view_index,
        despawn: Some(&nearest),
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    let dt = crate::diag::lap("e.index", dt);
    // Creakings that lost their heart in the block phase go before the entities tick.
    crate::heart::process_released(&mut sim);
    process_pending_kills(&mut sim);
    if !islands::tick_islands(&mut sim, ticking, any_player, ctx) && !spec::tick_speculative(&mut sim, pace, ticking, any_player, ctx) {
        tick_list(&mut sim, ticking, any_player, &mut |_, _| true);
    }
    let dt = crate::diag::lap("e.list", dt);
    // `ServerEntity.sendChanges` → `updateDataBeforeSync`: the invisible flag follows the
    // effects once all the entities have ticked.
    for e in sim.list.iter_mut() {
        if let Some(p) = e.phys.as_deref_mut() {
            kiln_entity::mob::update_data_before_sync(p);
        }
    }
    // `Level.tickBlockEntities`: the creaking hearts, after the entities.
    crate::heart::tick_all(&mut sim, ticking);
    // ... and the mob spawners.
    crate::mob_spawner::tick_all(&mut sim, ticking);
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
    // Only mobs whose type touches back are looked for (by entity section, as the area search
    // finds them, in its order).
    let mut by_section: FastMap<(i32, i32, i32), Vec<usize>> = Default::default();
    for (i, e) in sim.list.iter().enumerate() {
        if !e.removed && e.phys.as_deref().is_some_and(kiln_entity::mob::touches_players) {
            by_section.entry(section_of(e.pos)).or_default().push(i);
        }
    }
    for (pid, area) in touchers.into_iter().filter(|_| !by_section.is_empty()) {
        let lo = section_of([area.min_x - 2.0, area.min_y - 2.0, area.min_z - 2.0]);
        let hi = section_of([area.max_x + 2.0, area.max_y + 2.0, area.max_z + 2.0]);
        let mut found: SmallVec<[((i32, i64), i32, usize); 8]> = SmallVec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    for &i in by_section.get(&(x, y, z)).into_iter().flatten() {
                        if let Some(e) = sim.list[i].phys.as_deref()
                            && e.id != pid
                            && e.is_alive()
                            && e.bounding_box().intersects(&area)
                        {
                            found.push((section_key(e), e.id, i));
                        }
                    }
                }
            }
        }
        found.sort_unstable();
        for (_, _, i) in found {
            let Some(mut phys) = sim.list[i].phys.take() else { continue };
            if matches!(phys.kind, EntityKind::Mob(_)) && !phys.is_removed() {
                kiln_entity::mob::player_touch(&mut phys, &mut sim, pid);
            }
            sim.list[i].phys = Some(phys);
        }
    }
    let dt = crate::diag::lap("e.post_touch", dt);
    ride_players(&mut sim);
    let SimLevel { level, list, proxies, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
    // Explosion knockback reaches the pushed player's client (it owns its movement).
    for pr in proxies.iter().filter_map(Proxy::made).filter(|e| e.delta != Vec3::ZERO) {
        if let Some(p) = players.iter_mut().find(|p| p.entity_id == pr.id)
            && !(p.game_mode == 1 && p.flying)
        {
            p.send(entity::set_entity_motion(p.entity_id, arr(pr.delta)));
        }
    }
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
    crate::diag::lap("e.carry", dt);
}

/// The entities' turns in list order (`tick` for a region, an island or a tile): passengers
/// right after their vehicle. `turn` sees each turn (by list index) before it runs, and says
/// whether it runs.
pub(crate) fn tick_list(sim: &mut SimLevel, ticking: &blocks::Ticking, any_player: bool, turn: &mut dyn FnMut(&SimLevel, usize) -> bool) {
    for i in 0..sim.list.len() {
        if carried(sim, i) || !turn(sim, i) {
            continue;
        }
        tick_turn(sim, i, ticking, any_player);
    }
}

/// Passengers tick right after their vehicle (`ServerLevel.tickPassenger`), not in their own
/// turn: whether entity `i` rides a vehicle that carries it.
fn carried(sim: &SimLevel, i: usize) -> bool {
    let vehicle = sim.list[i].phys.as_deref().and_then(|p| p.vehicle);
    vehicle.is_some_and(|v| sim.index(v).is_some_and(|j| !sim.list[j].removed && sim.list[j].phys.as_deref().is_some_and(|p| p.passengers.contains(&sim.list[i].id))))
}

/// Entity `i`'s turn in the list (it is not [`carried`]): its tick, then its passengers'.
fn tick_turn(sim: &mut SimLevel, i: usize, ticking: &blocks::Ticking, any_player: bool) {
    let me = sim.list[i].id;
    if let Some(phys) = sim.list[i].phys.as_deref_mut()
        && let Some(v) = phys.vehicle.take()
        && let Some(j) = sim.index(v)
        && let Some(vp) = sim.list[j].phys.as_deref_mut()
    {
        kiln_entity::ride::remove_passenger(vp, me);
    }
    tick_entity(sim, i, ticking, any_player, None);
    let passengers = sim.list[i].phys.as_deref().map(|p| p.passengers.clone()).unwrap_or_default();
    for id in passengers {
        if let Some(j) = sim.index(id) {
            tick_entity(sim, j, ticking, any_player, Some(i));
        } else if id < 0 {
            // A rider the vehicle's own tick just made (the skeleton of a trap horse, still
            // waiting for its id) ticks in this very tick, as `tickPassenger` does for what
            // joined the level meanwhile.
            tick_new_passenger(sim, id, i);
        }
    }
}

/// `/kill` of mobs (`LivingEntity.kill`: `hurtServer(genericKill, Float.MAX_VALUE)`): whatever
/// the command queued runs through the mob's own damage code (death sound, loot, experience,
/// equipment drops, death events), in loaded chunks that do not tick too.
fn process_pending_kills(sim: &mut SimLevel) {
    for i in 0..sim.list.len() {
        let queued = sim.list[i].phys.as_deref().is_some_and(|p| !p.pending_hurts.is_empty() && matches!(p.kind, EntityKind::Mob(_)));
        if !queued {
            continue;
        }
        let Some(mut phys) = sim.list[i].phys.take() else { continue };
        (sim.current, sim.seeds) = (phys.id, 0x6b69_6c6c);
        sim.current_source = sim.level.listening().then(|| kiln_entity::vibration::source_of(&phys, &*sim));
        sim.rng = entity_level_random(sim.level.env().seed, sim.level.env().game_time ^ 0x6b69_6c6c, phys.id);
        for (kind, amount, attacker) in std::mem::take(&mut phys.pending_hurts) {
            let mut source = kiln_entity::mob::DamageSource::of(kind);
            source.attacker = attacker;
            kiln_entity::mob::hurt_entity(&mut phys, sim, source, amount);
        }
        let e = &mut sim.list[i];
        e.phys = Some(phys);
        e.sync();
    }
}

/// `ServerLevel.tickPassenger` for a rider spawned during its vehicle's tick (`vehicle` is the
/// index of the vehicle, which has just ticked): it is still among the spawns of this tick, under
/// the placeholder id its vehicle lists it by.
fn tick_new_passenger(sim: &mut SimLevel, id: i32, vehicle: usize) {
    let Some(k) = sim.spawns.iter().position(|s| matches!(&s.body, Body::Ready(e) if e.id == id)) else { return };
    let Body::Ready(boxed) = &mut sim.spawns[k].body else { return };
    let marker = kiln_entity::Entity::new("minecraft:marker", 0, 0, kiln_entity::EntityKind::Other { type_name: "minecraft:marker" }, 0);
    let mut phys = std::mem::replace(&mut **boxed, marker);
    if phys.is_removed() || phys.vehicle != sim.list[vehicle].phys.as_deref().map(|p| p.id) {
        if let Body::Ready(b) = &mut sim.spawns[k].body {
            **b = phys;
        }
        return;
    }
    (sim.current, sim.seeds) = (phys.id, 0);
    sim.current_source = sim.level.listening().then(|| kiln_entity::vibration::source_of(&phys, &*sim));
    sim.rng = entity_level_random(sim.level.env().seed, sim.level.env().game_time, phys.id);
    phys.common_tick();
    if let Some(mut v) = sim.list[vehicle].phys.clone() {
        if kiln_entity::ride::ride_tick(&mut phys, sim, &mut v)
            && let Some(real) = sim.list[vehicle].phys.as_deref_mut()
        {
            kiln_entity::ride::copy_steering_back(&v, real);
        }
    }
    // (Spawns only get appended meanwhile: `k` still holds the rider's place.)
    sim.spawns[k].pos = arr(phys.position());
    sim.spawns[k].vel = arr(phys.delta);
    if let Body::Ready(b) = &mut sim.spawns[k].body {
        **b = phys;
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
    sim.current_source = sim.level.listening().then(|| kiln_entity::vibration::source_of(&phys, &*sim));
    sim.rng = entity_level_random(sim.level.env().seed, sim.level.env().game_time, phys.id);
    // `Mob.checkDespawn` runs before the tick, against the nearest player (regions are
    // farther apart than the despawn distance, so the region's players decide).
    if matches!(phys.kind, EntityKind::Mob(_)) && !phys.is_removed() {
        let p = phys.position();
        let nearest = match sim.despawn {
            Some(n) => n.nearest_sqr(p),
            None => sim.views.iter().filter(|v| !v.spectator).map(|v| v.pos.distance_to_sqr(p)).min_by(|a, b| a.total_cmp(b)),
        };
        kiln_entity::mob::check_despawn(&mut phys, &*sim, nearest.or(any_player.then_some(f64::MAX)));
    }
    if !phys.is_removed() {
        phys.common_tick();
        match vehicle.and_then(|v| sim.list[v].phys.clone().map(|p| (v, p))) {
            Some((vi, mut v)) => {
                // (The vehicle is ticked as a copy: what its rider steered it by comes back.)
                if kiln_entity::ride::ride_tick(&mut phys, sim, &mut v)
                    && let Some(real) = sim.list[vi].phys.as_deref_mut()
                {
                    kiln_entity::ride::copy_steering_back(&v, real);
                }
            }
            None => phys.tick(sim),
        }
    }
    sim.list[i].phys = Some(phys);
    settle(sim, i);
}

/// After entity `i`'s tick: its outer state follows the vanilla state, its section and cell
/// follow its position.
fn settle(sim: &mut SimLevel, i: usize) {
    let e = &mut sim.list[i];
    e.sync();
    let pos = e.pos;
    sim.grid.moved(i, pos);
    let e = &mut sim.list[i];
    let cell = chunk_of(e.pos).cell();
    if sim.level.cells().cell(cell).is_some() {
        e.cell = cell;
    }
}

/// `Player.rideTick` for the region's riding players, after the entities ticked: a sneaking
/// player (or one whose mount is gone or threw it off) gets off at the mount's dismount
/// location; the others sit where the mount carries them.
fn ride_players(sim: &mut SimLevel) {
    let now = sim.level.env().game_time;
    for k in 0..sim.players.len() {
        let Some(v) = sim.players[k].vehicle else { continue };
        let pid = sim.players[k].entity_id;
        let idx = sim.index(v).filter(|&j| !sim.list[j].removed && sim.list[j].phys.as_deref().is_some_and(|p| p.is_alive()));
        let seated = idx.is_some_and(|j| sim.list[j].phys.as_deref().is_some_and(|p| p.passengers.contains(&pid)));
        // A teleport of the player's own got it off at once (`Entity.teleport`: `stopRiding`): it
        // stays where it went.
        if std::mem::take(&mut sim.players[k].dismount_on_teleport) {
            if let Some(vp) = idx.and_then(|j| sim.list[j].phys.as_deref_mut()) {
                kiln_entity::ride::remove_passenger(vp, pid);
            }
            let p = &mut *sim.players[k];
            p.vehicle = None;
            p.vehicle_type = None;
            continue;
        }
        let p = &*sim.players[k];
        let leave = !seated || p.sneaking || p.dead || p.disconnected;
        if !leave {

            let vp = sim.list[idx.unwrap()].phys.as_deref().expect("vehicle state");
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
            if let Some(vp) = sim.list[j].phys.as_deref_mut() {
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
    let Some(phys) = entities.list[idx].phys.as_deref_mut() else { return };
    let steers = phys.passengers.first() == Some(&p.entity_id)
        && (kiln_entity::mob::data(phys).is_some_and(|m| m.kind.ext().is_some_and(|k| k.steerable_by(m, &view)))
            || kiln_entity::ext_entity::boat::is_boat(phys.type_name));
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

/// `ServerGamePacketListenerImpl.handlePaddleBoat`: the blades of the boat player `i` steers.
pub(crate) fn paddle_boat(entities: &mut Entities, players: &[&mut Player], i: usize, left: bool, right: bool) {
    let p = &*players[i];
    let Some(v) = p.vehicle else { return };
    let Ok(idx) = entities.list.binary_search_by_key(&v, |e| e.id) else { return };
    let Some(phys) = entities.list[idx].phys.as_deref_mut() else { return };
    if phys.passengers.first() != Some(&p.entity_id) {
        return;
    }
    if let Some(b) = kiln_entity::ext_entity::get_mut::<kiln_entity::ext_entity::boat::Boat>(phys) {
        b.set_paddle_state(left, right);
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
    let Some(phys) = entities.list[idx].phys.as_deref_mut() else { return };
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

/// Runs `f` on entity `target` of the region with the level around it (what `Player.attack` does
/// to a victim that is not a player: hurt it, push it, set it on fire, give it an effect), then
/// carries out what the entity did meanwhile (death loot, sounds, damage events). `salt` seeds
/// the level random the entity sees. `None` when the entity is gone.
#[allow(clippy::too_many_arguments)]
pub(crate) fn with_mob<R>(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    target: i32,
    salt: u64,
    f: impl FnOnce(&mut kiln_entity::Entity, &mut SimLevel<'_, '_, '_>) -> R,
) -> Option<R> {
    let i = entities.list.binary_search_by_key(&target, |e| e.id).ok()?;
    // The datapack's enchantments act on the blow (armor protection, breach).
    let _enchanting = crate::enchant::install_enchanter(level.env.loot.as_ref());
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ salt as i64, target);
    let mut sim = SimLevel {
        level: World::Region(level),
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: target,
        seeds: 0x6869_7400,
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let mut phys = sim.list[i].phys.take()?;
    sim.current_info = Some((phys.type_name, arr(phys.position())));
    let out = f(&mut phys, &mut sim);
    let e = &mut sim.list[i];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
    Some(out)
}

/// `Player.deflectProjectile` on entity `id`: a fireball or wind charge flies on along the look
/// (`rot` = yaw, pitch) of player `by`, who owns it from now on. False when it is not one.
pub(crate) fn aim_deflect(entities: &mut Entities, id: i32, by: (i32, u128), rot: [f32; 2]) -> bool {
    let Ok(i) = entities.list.binary_search_by_key(&id, |e| e.id) else { return false };
    let Some(phys) = entities.list[i].phys.as_deref_mut() else { return false };
    let look = kiln_entity::ext_entity::fireball::view_vector(rot[1], rot[0]);
    let done = phys.aim_deflect(by, look);
    entities.list[i].sync();
    done
}

/// A player's stab on an entity (`Player.stabAttack` with a spear), carried out against the
/// region's entities.
#[derive(Debug, Clone)]
pub(crate) struct MobStab {
    pub target: i32,
    pub attacker: i32,
    pub attacker_pos: [f64; 3],
    pub yaw: f32,
    /// The weapon's damage type (`minecraft:spear`).
    pub kind: DamageKind,
    pub amount: f32,
    /// Whether the stab hurts at all (a charge may only push or dismount).
    pub damage: bool,
    /// The `causeExtraKnockback` strengths in order (0 skips one).
    pub knockbacks: [f32; 2],
    pub dismount: bool,
    /// Fire aspect: seconds the entity burns when hurt.
    pub fire_seconds: f32,
}

/// What a stab did to the entity.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MobStabOutcome {
    pub hurt: bool,
    pub dismounted: bool,
    pub health_before: Option<f32>,
}

/// `Player.stabAttack` on entity `stab.target`: hurt (if it hurts), pushed once or twice, taken
/// off what it rides, set on fire; `None` when the entity is gone.
pub(crate) fn stab_mob(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
    stab: &MobStab,
) -> Option<MobStabOutcome> {
    let i = entities.list.binary_search_by_key(&stab.target, |e| e.id).ok()?;
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ 0x7374_6162, stab.target);
    let mut sim = SimLevel {
        level: World::Region(level),
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: stab.target,
        seeds: 0x7374_6162_00,
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let mut phys = sim.list[i].phys.take()?;
    let source = kiln_entity::mob::DamageSource {
        kind: stab.kind,
        attacker: Some(stab.attacker),
        direct: Some(stab.attacker),
        pos: Some(vec3(stab.attacker_pos)),
        attacker_is_player: true,
    };
    let is_mob = kiln_entity::mob::data(&phys).is_some();
    let health_before = kiln_entity::mob::data(&phys).map(|m| m.health);
    let hurt = stab.damage
        && if is_mob {
            kiln_entity::mob::hurt_entity(&mut phys, &mut sim, source, stab.amount)
        } else {
            phys.hurt(&mut sim, stab.kind, stab.amount, Some(stab.attacker))
        };
    // `Player.causeExtraKnockback`: a living entity is knocked back, the others pushed.
    let rad = (stab.yaw * 0.017453292) as f64;
    let (s, c) = (kiln_entity::mob::mth::sin(rad) as f64, kiln_entity::mob::mth::cos(rad) as f64);
    for strength in stab.knockbacks {
        if strength <= 0.0 {
            continue;
        }
        if is_mob {
            kiln_entity::mob::knockback_entity(&mut phys, strength as f64, s, -c);
        } else {
            let k = strength as f64;
            phys.delta = phys.delta + Vec3::new(-s * k, 0.1, c * k);
            phys.needs_sync = true;
        }
    }
    let mut dismounted = false;
    if stab.dismount
        && let Some(v) = phys.vehicle
        && !kiln_entity::mob::entity_type_tag(phys.type_name, "minecraft:cannot_be_dismounted_by_item_usage")
    {
        dismounted = true;
        phys.vehicle = None;
        if let Ok(j) = sim.list.binary_search_by_key(&v, |e| e.id)
            && let Some(vp) = sim.list[j].phys.as_deref_mut()
        {
            kiln_entity::ride::remove_passenger(vp, stab.target);
        }
    }
    if hurt && stab.fire_seconds > 0.0 && is_mob {
        phys.ignite_for_seconds(stab.fire_seconds);
    }
    let victim = kiln_entity::level::Seen::of(&phys);
    let health_after = kiln_entity::mob::data(&phys).map(|m| m.health);
    let e = &mut sim.list[i];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
    if hurt && let Some(p) = players.iter_mut().find(|p| p.entity_id == stab.attacker) {
        p.last_hurt_mob = Some((stab.target, level.env.game_time));
        // `PlayerHurtEntityTrigger` (dealt before armor and effects, taken after).
        let taken = health_before.zip(health_after).map_or(stab.amount, |(b, a)| b - a);
        let subject = crate::advancements::triggers::seen_subject(&victim, crate::DIMENSIONS[level.env.dim].0);
        p.player_hurt_entity(&subject, stab.amount, taken, stab.kind.type_name(), true);
    }
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
    Some(MobStabOutcome { hurt, dismounted, health_before })
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
) -> bool {
    use kiln_item::component::EquipmentSlot;
    let Ok(idx) = entities.list.binary_search_by_key(&target, |e| e.id) else { return false };
    {
        let p = &*players[i];
        if p.dead || entities.list[idx].removed {
            return false;
        }
        let Some(phys) = entities.list[idx].phys.as_deref() else { return false };
        if kiln_entity::mob::data(phys).is_none() && !matches!(phys.kind, EntityKind::Ext(_)) {
            return false;
        }
        // `canInteractWithEntity(box, 3.0)`: the box within the interaction range plus 3.
        let bb = phys.bounding_box();
        let eye = p.eye_position();
        let d = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
        let (dx, dy, dz) = (d(eye[0], bb.min_x, bb.max_x), d(eye[1], bb.min_y, bb.max_y), d(eye[2], bb.min_z, bb.max_z));
        let range = p.attribute(crate::combat::ENTITY_INTERACTION_RANGE) + 3.0;
        if dx * dx + dy * dy + dz * dz >= range * range {
            return false;
        }
        // `Player.interactOn` for a spectator: a `MenuProvider` opens its menu (a minecart whose
        // loot table is unrolled has none for them), nothing else reacts.
        if p.game_mode == 3 {
            return kiln_entity::ext_entity::container(phys).is_some_and(|c| c.loot_table.is_none());
        }
    }
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let stack = players[i].inv.equipped(slot).clone();
    let who = kiln_entity::mob::interact::Interactor { id: players[i].entity_id, creative: players[i].game_mode == 1, sneaking: players[i].sneaking };
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ 0x696e_74, target);
    let mut sim = SimLevel {
        level: World::Region(level),
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
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let Some(mut phys) = sim.list[idx].phys.take() else { return false };
    let out = kiln_entity::mob::interact::interact(&mut phys, &mut sim, &who, &stack);
    // Sheared wool: each item on its own, thrown up from the sheep with a push from its random.
    if let Some(table) = &out.shear
        && let Some(loot) = sim.level.env().loot.clone()
    {
        let env = sim.level.env();
        let ctx = crate::mobs::DeathContext {
            type_name: phys.type_name,
            origin: arr(phys.position()),
            on_fire: false,
            baby: false,
            killed_by_player: false,
            damage_type: "minecraft:generic",
            weapon: Some(stack.clone()),
            raider: None,
            attacker: None,
        };
        let seed = crate::mobs::loot_seed(env.seed, env.game_time, target, 0x7368_6561);
        let mut k = 0u64;
        for drop in crate::mobs::roll(&loot, table, &ctx, seed) {
            for _ in 0..drop.count() {
                let mut one = drop.clone();
                one.set_count(1);
                // Only wool gets the push of `Sheep.shear`; the snow golem's pumpkin drops from its eyes.
                let sheep = phys.type_name == "minecraft:sheep";
                let push = if sheep { kiln_entity::mob::species::shear_drop_motion(&mut phys) } else { kiln_entity::math::Vec3::ZERO };
                let lift = if sheep { 1.0 } else { phys.eye_y() - phys.y() };
                let h = crate::mobs::loot_seed(env.seed, env.game_time, target, 0x7368_0000 | k) as u64;
                k += 1;
                let p = phys.position();
                let mut s = crate::mobs::drop_item(one, [p.x, p.y + lift, p.z], h);
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
        let now = sim.level.env().game_time;
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
    let level = level.into_region();
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
        HeldChange::Shrink(n) => {
            kiln_inventory::Container::item_mut(&mut p.inv, index).shrink(*n);
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
    // A chest or hopper minecart's click opens its menu (the caller opens it).
    out.open_container
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
    let _enchanting = crate::enchant::install_enchanter(level.env.loot.as_ref());
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ salt as i64, target);
    let mut sim = SimLevel {
        level: World::Region(level),
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
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let mut phys = sim.list[idx].phys.take()?;
    let r = f(&mut phys, &mut sim);
    let e = &mut sim.list[idx];
    e.phys = Some(phys);
    e.sync();
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
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
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ salt as i64, 0);
    let mut sim = SimLevel {
        level: World::Region(level),
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
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    let r = f(&mut sim);
    for e in sim.list.iter_mut() {
        e.sync();
    }
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
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
        Event::BlockExploded { pos, state, decay, radius, .. } => {
            level.effect(if decay { Effect::ExplosionDrop { pos: kb(pos), state, radius } } else { Effect::Drop { pos: kb(pos), state } })
        }
        Event::BlockEvent { pos, a, b } => {
            let at = kb(pos);
            let block = kiln_blocks::BlockId::of(level.block(at));
            level.effect(Effect::BlockEvent { pos: at, block, a, b });
        }
        Event::Hurt { target, amount, kind, attacker } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == target) {
                // kiln-entity's attacker is the entity that dealt the damage (TNT, a falling
                // block); none of them is a player.
                let source = health::Source { cause: health::Cause::Entity(kind), attacker: None, direct: attacker, weapon: None, position: None };
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
            let Some(phys) = list[i].phys.as_deref() else { return };
            let weapon = killer.and_then(|k| players.iter().find(|p| p.entity_id == k)).map(|p| p.inv.selected_item().clone());
            // The killer as `damage_source_properties` sees it (a frog's variant decides the froglight).
            let attacker_view = attacker.and_then(|a| {
                if players.iter().any(|p| p.entity_id == a) {
                    return Some(crate::mobs::AttackerView { type_name: "minecraft:player", components: Vec::new() });
                }
                let i = list.binary_search_by_key(&a, |e| e.id).ok()?;
                let e = list[i].phys.as_deref()?;
                Some(crate::mobs::AttackerView { type_name: e.type_name, components: kiln_entity::mob::data(e).map(kiln_entity::mob::variant_components).unwrap_or_default() })
            });
            let ctx = crate::mobs::DeathContext {
                type_name: phys.type_name,
                origin: arr(pos),
                on_fire,
                baby: kiln_entity::mob::data(phys).is_some_and(|m| m.baby()),
                killed_by_player: killer.is_some(),
                damage_type: kind.type_name(),
                weapon,
                // `Raider.isCaptain` as it died: the leader still wore the banner (its drop
                // chance of 2 marks it; the banner itself dropped with the equipment).
                raider: kiln_entity::mob::data(phys).and_then(|m| {
                    let r = kiln_entity::mob::kinds::raider::raider(m)?;
                    Some((r.raid.is_some(), r.patrol_leader && m.drop_chances[kiln_entity::mob::HEAD] >= 2.0))
                }),
                attacker: attacker_view,
            };
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
                        let source = health::Source { cause: health::Cause::Entity(DamageKind::IndirectMagic), attacker: None, direct: None, weapon: None, position: None };
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
        Event::Killed { entity, entity_type, credit, kind, attacker, direct, equipment } => {
            if let Some(p) = credit.and_then(|k| players.iter_mut().find(|p| p.entity_id == k)) {
                p.killed_entity(entity_type);
                let dim = crate::DIMENSIONS[env.dim].0;
                if let Some(e) = list.binary_search_by_key(&entity, |e| e.id).ok().and_then(|i| list[i].phys.as_deref()) {
                    let mut subject = crate::advancements::triggers::mob_subject(e, dim);
                    // `minecraft:equipment` as the mob wore it when it died (a captain's banner).
                    subject.equipment = equipment.iter().map(|(slot, s)| (*slot, s)).collect();
                    p.killed("minecraft:player_killed_entity", &subject, kind.type_name(), direct == attacker);
                }
            }
        }
        Event::GiftLoot { entity: id, table, pos } => loot_drop(env, spawns, id, table, pos, n, 0.0),
        Event::ShearLoot { entity: id, table, pos } => loot_drop(env, spawns, id, &table, pos, n, 1.0),
        // `ThrownEnderpearl.onHit`: its player goes to where the pearl was at the start of the
        // tick, takes 5 `ender_pearl` damage and hears the teleport.
        Event::ProjectileHit { projectile, projectile_type: "minecraft:ender_pearl", owner: Some(owner), .. } => {
            let Some(to) = list.binary_search_by_key(&projectile, |e| e.id).ok().and_then(|i| list[i].phys.as_deref()).map(|e| arr(e.old_pos)) else { return };
            let Some(p) = players.iter_mut().find(|p| p.entity_id == owner && !p.dead && !p.disconnected) else { return };
            let now = env.game_time;
            let rot = p.rot;
            p.dismount_on_teleport |= p.vehicle.is_some();
            p.teleport(to, rot, now);
            p.fall_distance = 0.0;
            let source = health::Source { cause: health::Cause::Other("minecraft:ender_pearl"), attacker: None, direct: None, weapon: None, position: None };
            let mut ctx = health::DamageCtx { rules: env.damage, game_time: env.game_time, spawns, deaths, level_rng: None };
            p.hurt(5.0, &source, &mut ctx);
            p.sound_for_all("minecraft:entity.player.teleport", world_fx::SoundSource::Players, 1.0, 1.0);
        }
        // Vibrations, other projectile hits and the block effects of entities inside blocks
        // (pressure plates are pressed through the entity boxes) are not simulated yet.
        Event::GameEvent { .. } | Event::EntityInsideBlock { .. } | Event::ProjectileHit { .. } => {}
        Event::Raid(ev) => level.blocks.raid_events.push(ev),
        // `ServerLevel.globalLevelEvent`: with `global_sound_events` every player (here: of the
        // region) hears it, from where it is heard best within 32 blocks of them; without, only
        // the players within 64 blocks of it, as an ordinary level event.
        Event::GlobalLevelEvent { event, pos, data } => {
            if env.mobs.global_sound_events {
                let center = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
                for p in players.iter_mut() {
                    let d = [center[0] - p.pos[0], center[1] - p.pos[1], center[2] - p.pos[2]];
                    let sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    let at = if sq < 1024.0 {
                        center
                    } else {
                        let len = sq.sqrt();
                        [p.pos[0] + d[0] / len * 32.0, p.pos[1] + d[1] / len * 32.0, p.pos[2] + d[2] / len * 32.0]
                    };
                    let at = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
                    p.send(world_fx::level_event(event, at, data, true));
                }
            } else {
                let pkt = world_fx::level_event(event, [pos.x, pos.y, pos.z], data, false);
                for p in players.iter_mut() {
                    let d = [pos.x as f64 - p.pos[0], pos.y as f64 - p.pos[1], pos.z as f64 - p.pos[2]];
                    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 64.0 * 64.0 {
                        p.send(pkt.clone());
                    }
                }
            }
        }
        Event::PlayerGameEvent { player, event, param } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == player) {
                p.send(kiln_proto::packets::game_event(event, param));
            }
        }
        Event::Criterion { player, criterion } => {
            if let Some(p) = players.iter_mut().find(|p| p.entity_id == player) {
                p.entity_criterion(crate::DIMENSIONS[env.dim].0, &criterion);
            }
        }
        Event::DragonFight(ev) => {
            if let Some(f) = &env.dragon_fight {
                f.send(crate::dragon_fight::FightMsg::Entity(ev));
            }
        }
        // wp32 parrots: a parrot flew onto its owner's shoulder (or, when the shoulder turns out
        // taken after all, stays where it is).
        Event::MountShoulder { player, tag, .. } => {
            let block = |pos: BlockPos| level.block(kb(pos));
            let Some(p) = players.iter_mut().find(|p| p.entity_id == player) else { return };
            if let Some(back) = crate::shoulder::mount(p, tag, env.game_time, &block) {
                let uuid = back.get("UUID").and_then(kiln_entity::persist::uuid_from_tag).unwrap_or(0);
                if let Ok(e) = kiln_entity::persist::load(&back, 0, seed_for_uuid(uuid))
                    && let Some(spawn) = Spawn::loaded(e)
                {
                    spawns.push(spawn);
                }
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
        raider: None,
        attacker: None,
    };
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, id, 0x6966 ^ n as u64);
    for (k, stack) in crate::mobs::roll(&loot, table, &ctx, seed).into_iter().enumerate() {
        let h = crate::mobs::loot_seed(env.seed, env.game_time, id, (n as u64) << 8 | k as u64) as u64;
        spawns.push(crate::mobs::drop_item(stack, [pos.x, pos.y + y_off, pos.z], h));
    }
}

/// A player as the entities see it (`now`: the game time).
pub(crate) fn view(p: &Player, now: i64) -> PlayerView {
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
        sprinting: p.sprinting,
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
        tick_count: now as i32,
        last_hurt_by_mob_time: p.last_hurt_by_mob.map_or(0, |(_, t)| t as i32),
        last_hurt_mob: p.last_hurt_mob.map(|(id, _)| id),
        last_hurt_mob_time: p.last_hurt_mob.map_or(0, |(_, t)| t as i32),
        hurt_recently: p.last_hurt_by_mob.is_some_and(|(_, t)| now - t <= 100),
        hero_of_the_village: p.effect_amplifier("minecraft:hero_of_the_village"),
        vehicle: p.vehicle,
        // (filled in by `shoulder::mark_views` where parrots are about)
        parrot_may_land: false,
        parrot_can_sit: false,
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
    arrow_pickups(entities, players);
    for e in &mut entities.list {
        if e.removed {
            continue;
        }
        let (lo, hi, _) = e.body();
        let Some(EntityKind::Item(item)) = e.phys.as_deref_mut().map(|p| &mut p.kind) else { continue };
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
            if let Some(p) = e.phys.as_deref_mut() {
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

/// `AbstractArrow.playerTouch` for arrows and tridents at rest (stuck, or a loyal trident on
/// its way back): `tryPickup` by the pickup mode (a returning trident goes to its thrower);
/// the viewers see it taken.
fn arrow_pickups(entities: &mut Entities, players: &mut [&mut Player]) {
    use kiln_entity::arrow::{PICKUP_ALLOWED, PICKUP_CREATIVE_ONLY};
    use kiln_entity::ext_entity::trident::Trident;
    for e in &mut entities.list {
        if e.removed {
            continue;
        }
        let (lo, hi, _) = e.body();
        let Some(phys) = e.phys.as_deref() else { continue };
        // (at rest, pickup mode, the item, a loyal trident's owner)
        let (resting, mode, item, owner) = match &phys.kind {
            EntityKind::Arrow(a) => (
                (a.in_ground || phys.no_physics) && a.shake_time <= 0,
                a.pickup,
                a.pickup_item.clone().unwrap_or_else(|| kiln_item::ItemStack::of(phys.type_name, 1).unwrap_or_else(kiln_item::ItemStack::empty)),
                None,
            ),
            _ => match kiln_entity::ext_entity::get::<Trident>(phys) {
                Some(t) => (
                    (t.in_ground || phys.no_physics) && t.shake_time <= 0,
                    if t.creative_only { PICKUP_CREATIVE_ONLY } else if t.pickup { PICKUP_ALLOWED } else { 0 },
                    t.item.clone().unwrap_or_else(|| kiln_item::ItemStack::of("minecraft:trident", 1).unwrap_or_else(kiln_item::ItemStack::empty)),
                    t.owner.filter(|_| phys.no_physics),
                ),
                None => continue,
            },
        };
        if !resting {
            continue;
        }
        let touching = |p: &Player| {
            let pmin = [p.pos[0] - 1.3, p.pos[1] - 0.5, p.pos[2] - 1.3];
            let pmax = [p.pos[0] + 1.3, p.pos[1] + 2.3, p.pos[2] + 1.3];
            (0..3).all(|i| pmin[i] < hi[i] && pmax[i] > lo[i])
        };
        let Some(i) = players.iter().position(|p| !p.disconnected && !p.dead && p.game_mode != 3 && touching(p)) else { continue };
        let p = &mut *players[i];
        let mut stack = item;
        let taken = match mode {
            PICKUP_ALLOWED => {
                p.add_to_inventory(&mut stack);
                stack.is_empty()
            }
            PICKUP_CREATIVE_ONLY => p.infinite_materials(),
            _ => false,
        } || owner == Some(p.entity_id) && {
            p.add_to_inventory(&mut stack);
            stack.is_empty()
        };
        if !taken {
            continue;
        }
        let pkt = entity::take_item_entity(e.id, players[i].entity_id, 1);
        players[i].send(pkt.clone());
        for v in &e.seen_by {
            if let Ok(j) = players.binary_search_by_key(v, |q| q.conn)
                && j != i
            {
                players[j].send(pkt.clone());
            }
        }
        e.removed = true;
        if let Some(p) = e.phys.as_deref_mut() {
            p.discard();
        }
    }
}

/// Tracks the region's entities to its players (sorted by connection) with vanilla's
/// triggers: an entity whose section changed is re-evaluated against every player, and every
/// entity against the players in `movers` (whose section changed). Then sends movement and
/// velocity, and removes dead entities from their viewers and the region.
///
/// Two windows: each entity works out its viewers and encodes its packets (touching only
/// itself, against a snapshot of the players), then each run of consecutive players collects
/// what the entities it sees encoded, in list order; every player gets the packets a serial
/// loop over the entities would have sent it, in the same order.
pub(crate) fn track(entities: &mut Entities, players: &mut [&mut Player], movers: &[ConnId], ctx: &kiln_sched::Ctx<'_>) {
    let viewers: Vec<Viewer> = players.iter().map(|p| Viewer { conn: p.conn, pos: p.pos, view: p.view_distance }).collect();
    let movers: Vec<usize> = movers.iter().filter_map(|m| viewers.binary_search_by_key(m, |v| v.conn).ok()).collect();
    let present = Present::of(&viewers);
    let encoded = ctx.map_mut_with(TRACK_WINDOW, &mut entities.list, |_, e| track_entity(e, &viewers, &movers, &present));
    let mut runs: Vec<(usize, &mut [&mut Player])> = Vec::new();
    let mut start = 0;
    for run in players.chunks_mut(TRACK_RUN) {
        let n = run.len();
        runs.push((start, run));
        start += n;
    }
    let encoded = &encoded[..];
    ctx.map_mut_with(kiln_sched::Window::new().item_ns(850), &mut runs, |_, (_, run)| deliver_tracking(run, encoded));
    entities.list.retain(|e| !e.removed);
}

/// Entities per chunk of the encoding window, and players per delivery run.
const TRACK_WINDOW: kiln_sched::Window = kiln_sched::Window::new().item_ns(1_000);
const TRACK_RUN: usize = 32;

/// The region's players by connection, for dropping viewers that left: a bit per connection
/// id when the ids are small (they are handed out in order), else the sorted list.
enum Present {
    Bits(Vec<u64>),
    Sorted(Vec<ConnId>),
}

impl Present {
    fn of(viewers: &[Viewer]) -> Present {
        match viewers.last() {
            Some(v) if v.conn < 1 << 20 => {
                let mut bits = vec![0u64; v.conn as usize / 64 + 1];
                for v in viewers {
                    bits[v.conn as usize / 64] |= 1 << (v.conn % 64);
                }
                Present::Bits(bits)
            }
            _ => Present::Sorted(viewers.iter().map(|v| v.conn).collect()),
        }
    }

    fn contains(&self, conn: ConnId) -> bool {
        match self {
            Present::Bits(b) => b.get(conn as usize / 64).is_some_and(|w| w >> (conn % 64) & 1 != 0),
            Present::Sorted(v) => v.binary_search(&conn).is_ok(),
        }
    }
}

/// What tracking reads of a player.
struct Viewer {
    conn: ConnId,
    pos: [f64; 3],
    view: i32,
}

/// One entity's tracking changes this tick, by viewer: (who, packets) in send order.
#[derive(Default)]
struct Tracked {
    /// Players that start seeing it, and stop (sorted), and what they get.
    added: Vec<ConnId>,
    removed: Vec<ConnId>,
    /// The boss bar's progress for those that keep seeing it (wither), and who they are.
    boss_progress: Option<Bytes>,
    progress_to: Vec<ConnId>,
    /// Boss bar add for `added`, remove for `removed`.
    boss_add: Option<Bytes>,
    boss_remove: Option<Bytes>,
    spawn: Vec<Bytes>,
    despawn: Option<Bytes>,
    /// Its viewers after the changes (sorted), and what they all get.
    viewers: Vec<ConnId>,
    packets: Vec<Bytes>,
}

fn track_entity(e: &mut Entity, viewers: &[Viewer], movers: &[usize], present: &Present) -> Tracked {
    let sees = |p: &Viewer, e: &Entity| {
        let range = (e.kind.tracking_range as f64 * 16.0).min(p.view as f64 * 16.0);
        let (dx, dz) = (p.pos[0] - e.pos[0], p.pos[2] - e.pos[2]);
        let (pc, ec) = (chunk_of(p.pos), chunk_of(e.pos));
        !e.removed && dx * dx + dz * dz <= range * range && (pc.x - ec.x).abs() <= p.view && (pc.z - ec.z).abs() <= p.view
    };
    let mut t = Tracked::default();
    let section = e.pos.map(|c| c.floor() as i32 >> 4);
    let moved = e.section != Some(section);
    e.section = Some(section);
    {
        let (added, removed) = (&mut t.added, &mut t.removed);
        let mut check = |p: &Viewer, seen: bool| match (sees(p, e), seen) {
            (true, false) => added.push(p.conn),
            (false, true) => removed.push(p.conn),
            _ => {}
        };
        if moved || e.removed {
            for p in viewers {
                check(p, e.seen_by.binary_search(&p.conn).is_ok());
            }
        } else {
            for &m in movers {
                let p = &viewers[m];
                check(p, e.seen_by.binary_search(&p.conn).is_ok());
            }
        }
    }
    // `ServerBossEvent`: a boss's bar for the players that see it.
    let boss = e.phys.as_deref().and_then(kiln_entity::mob::data).and_then(kiln_entity::mob::kinds::wither::boss_bar);
    let bar_id = Uuid::from_u128(e.uuid.as_u128() ^ 0x626f_7373_6261_72);
    if let Some(progress) = boss {
        let name = kiln_proto::nbt::Tag::Compound(vec![("translate".into(), kiln_proto::nbt::Tag::String("entity.minecraft.wither".into()))]);
        let op = hud::BossEvent::Add {
            name: &name,
            progress,
            color: hud::BossBarColor::Purple,
            overlay: hud::BossBarOverlay::Progress,
            flags: hud::boss_flags::DARKEN_SCREEN,
        };
        t.boss_add = Some(hud::boss_event(bar_id, &op));
        if e.boss_sent.is_some_and(|p| p != progress) {
            t.boss_progress = Some(hud::boss_event(bar_id, &hud::BossEvent::Progress(progress)));
        }
        e.boss_sent = Some(progress);
    }
    if (boss.is_some() || e.boss_sent.is_some()) && !t.removed.is_empty() {
        t.boss_remove = Some(hud::boss_event(bar_id, &hud::BossEvent::Remove));
    }
    if !t.added.is_empty() {
        t.spawn = e.spawn_packets();
    }
    if !t.removed.is_empty() {
        t.despawn = Some(entity::remove_entities(&[e.id]));
    }
    // (The progress goes to the viewers before this tick's changes that keep seeing it.)
    let before = std::mem::take(&mut e.seen_by);
    let mut seen_by: Vec<ConnId> = before.iter().copied().filter(|v| t.removed.binary_search(v).is_err()).collect();
    if t.boss_progress.is_some() {
        t.progress_to = before.iter().copied().filter(|v| !t.added.contains(v) && !t.removed.contains(v)).collect();
    }
    seen_by.extend(t.added.iter().copied());
    seen_by.sort_unstable();
    // Players that left the region or the game are no longer viewers.
    seen_by.retain(|&v| present.contains(v));
    e.seen_by = seen_by;
    if e.removed {
        return t;
    }
    let mut packets = e.tracker.tick(&e.move_state());
    if let Some(EntityKind::Mob(m)) = e.phys.as_deref_mut().map(|p| &mut p.kind) {
        if std::mem::take(&mut m.swing) {
            packets.push(entity::swing_animation(e.id, false, entity::swing::WHACK, entity::swing::DEFAULT_DURATION));
        }
        // `Mob.spawnAnim`: entity event 20, the poof of a spawner's new mob.
        if std::mem::take(&mut m.spawn_anim) {
            packets.push(entity::entity_event(e.id, 20));
        }
    }
    if let Some(phys) = e.phys.as_deref()
        && let EntityKind::Ext(x) = &phys.kind
    {
        let mut meta = EntityData::new();
        x.entity_data(phys, &mut meta);
        if meta.entries() != e.meta_sent.as_slice() {
            if !e.meta_sent.is_empty() {
                packets.push(entity::set_entity_data(e.id, &meta));
            }
            e.meta_sent = meta.entries().to_vec();
        }
    }
    if let Some(phys) = e.phys.as_deref()
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
    // Arrows: crit and in-ground flags change in flight (extension entities: above).
    if e.phys.as_deref().is_some_and(|p| matches!(p.kind, EntityKind::Arrow(_))) {
        let meta = e.metadata();
        if meta.entries() != e.meta_sent.as_slice() {
            if !e.meta_sent.is_empty() || e.age > 1 {
                packets.push(entity::set_entity_data(e.id, &meta));
            }
            e.meta_sent = meta.entries().to_vec();
        }
    }
    if let Some(phys) = e.phys.as_deref()
        && phys.passengers != e.passengers_sent
    {
        e.passengers_sent = phys.passengers.clone();
        packets.push(entity::set_passengers(e.id, &e.passengers_sent));
    }
    // `Leashable.setLeashedTo` / `dropLeash`: Set Entity Link when the holder changes.
    if let Some(phys) = e.phys.as_deref() {
        let holder = kiln_entity::leash::holder_of(phys);
        if holder != e.leash_sent {
            // (A knot made this tick is still under a stand-in id: the link waits.)
            if !holder.is_some_and(|h| h < 0) {
                e.leash_sent = holder;
                packets.push(entity::set_entity_link(e.id, holder.unwrap_or(0)));
            }
        }
    }
    // `ServerEntity.sendChanges`: velocity on update ticks when it changed, or at once
    // after an impulse (explosion knockback).
    let impulse = e.phys.as_deref_mut().is_some_and(|p| std::mem::take(&mut p.needs_sync));
    if e.kind.track_deltas && (impulse || e.age % e.kind.update_interval.max(1) == 0) {
        let d: f64 = (0..3).map(|i| (e.vel[i] - e.sent_vel[i]).powi(2)).sum();
        let still = e.vel.iter().all(|&v| v == 0.0);
        if impulse || d > 1.0e-7 || (d > 0.0 && still) {
            e.sent_vel = e.vel;
            packets.push(entity::set_entity_motion(e.id, e.vel));
        }
    }
    if !packets.is_empty() {
        t.viewers = e.seen_by.clone();
    }
    t.packets = packets;
    t
}

/// Hands the players of `run` (consecutive, sorted) what every entity encoded for them, in
/// list order.
fn deliver_tracking(run: &mut [&mut Player], encoded: &[Tracked]) {
    let (Some(first), Some(last)) = (run.first().map(|p| p.conn), run.last().map(|p| p.conn)) else { return };
    let send = |run: &mut [&mut Player], to: &[ConnId], packets: &[Bytes]| {
        if packets.is_empty() || to.is_empty() {
            return;
        }
        let from = to.partition_point(|&v| v < first);
        let upto = to.partition_point(|&v| v <= last);
        let mut j = 0;
        for &v in &to[from..upto] {
            while j < run.len() && run[j].conn < v {
                j += 1;
            }
            if j < run.len() && run[j].conn == v {
                run[j].outbox.extend(packets.iter().cloned());
            }
        }
    };
    for t in encoded {
        send(run, &t.added, t.boss_add.as_slice());
        send(run, &t.progress_to, t.boss_progress.as_slice());
        send(run, &t.removed, t.boss_remove.as_slice());
        send(run, &t.added, &t.spawn);
        send(run, &t.removed, t.despawn.as_slice());
        send(run, &t.viewers, &t.packets);
    }
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
        DamageKind::Fireworks => ("minecraft:fireworks", "death.attack.fireworks"),
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
        DamageKind::WitherSkull => ("minecraft:wither_skull", "death.attack.witherSkull"),
        DamageKind::Thorns => ("minecraft:thorns", "death.attack.thorns"),

        // -- slice 3: warden
        DamageKind::SonicBoom => ("minecraft:sonic_boom", "death.attack.sonic_boom"),

        // -- slice 3: common mobs A

        // -- slice 3: common mobs B
        DamageKind::WindCharge => ("minecraft:wind_charge", "death.attack.mob"),

        // -- wp28: axolotl and goat
        DamageKind::DryOut => ("minecraft:dry_out", "death.attack.dryout"),
        DamageKind::NoAggroMobAttack => ("minecraft:mob_attack_no_aggro", "death.attack.mob"),
        DamageKind::Spit => ("minecraft:spit", "death.attack.mob"),
        DamageKind::Named(name) => (name, health::death_message_key(name)),
    }
}

#[allow(dead_code)]
fn _assert_sync() {
    fn f<T: Sync>() {}
    f::<SimLevel<'static, 'static, 'static>>();
}
