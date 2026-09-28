//! A small in-memory [`EntityLevel`]: the reference implementation of the world-access trait
//! (used by the parity tests and benchmarks), including vanilla's entity iteration order.

use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event, PlayerView};
use crate::math::{Aabb, BlockPos};
use kiln_javamath::random::LegacyRandom;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A multiplicative hasher for small integer keys (block positions, entity ids).
#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_i32(&mut self, v: i32) {
        self.write_u64(v as u32 as u64);
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

struct Slot {
    entity: Option<Entity>,
    /// Entity section key (x, then z/y as vanilla's `SectionPos.asLong` orders them) and the
    /// sequence number of the entity's insertion into that section.
    section: (i32, i64),
    seq: u64,
}

pub struct MemoryLevel {
    pub blocks: FastMap<BlockPos, u16>,
    /// A block filling the whole bottom layer (`min_y`), like a superflat bedrock floor.
    pub bottom_layer: Option<u16>,
    pub min_y: i32,
    pub events: Vec<Event>,
    pub players: Vec<PlayerView>,
    /// World age (`game_time`), sky darkening and difficulty for mobs.
    pub game_time: i64,
    pub sky_darken: i32,
    pub difficulty: u8,
    /// `getSeaLevel` (63; -63 for a superflat world).
    pub sea_level: i32,
    /// Hits on players: (player id, amount that landed), with each player's hurt cooldown and
    /// last hit (`LivingEntity.damageCooldownTime`, `lastHurt`).
    pub player_hits: Vec<(i32, f32)>,
    pub player_cooldown: FastMap<i32, (i32, f32)>,
    slots: Vec<Slot>,
    index: FastMap<i32, usize>,
    random: LegacyRandom,
    next_id: i32,
    next_seq: u64,
    spawned: Vec<Entity>,
    /// New entities join the level at once (vanilla's `addFreshEntity`: other entities see
    /// them the same tick; they tick from the next), instead of at [`MemoryLevel::flush_spawned`].
    pub immediate_adds: bool,
}

/// Vanilla iterates entity sections by x, then by the packed (z, y) section key.
fn section_of(e: &Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl MemoryLevel {
    pub fn new(min_y: i32, random_seed: i64) -> Self {
        MemoryLevel {
            blocks: FastMap::default(),
            bottom_layer: None,
            min_y,
            events: Vec::new(),
            players: Vec::new(),
            game_time: 0,
            sky_darken: 0,
            difficulty: 2,
            sea_level: 63,
            player_hits: Vec::new(),
            player_cooldown: FastMap::default(),
            slots: Vec::new(),
            index: FastMap::default(),
            random: LegacyRandom::new(random_seed),
            next_id: 1_000_000,
            next_seq: 0,
            spawned: Vec::new(),
            immediate_adds: false,
        }
    }

    /// Adds an entity now (it is ticked from the next `tick` on).
    pub fn insert(&mut self, e: Entity) {
        let section = section_of(&e);
        self.index.insert(e.id, self.slots.len());
        self.slots.push(Slot { entity: Some(e), section, seq: self.next_seq });
        self.next_seq += 1;
    }

    /// All entities in insertion order, removed ones included.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.slots.iter().filter_map(|s| s.entity.as_ref())
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// One level tick of the entities, as `ServerLevel` does it: in order, skipping removed
    /// ones, `commonTick` then `tick`; entities added meanwhile join after the loop.
    pub fn tick(&mut self) {
        for i in 0..self.slots.len() {
            self.tick_one(i, |e, level| {
                e.common_tick();
                e.tick(level);
            });
        }
        self.flush_spawned();
    }

    /// Runs `f` on entity `i` with the level (the entity is detached meanwhile).
    pub fn tick_one(&mut self, i: usize, f: impl FnOnce(&mut Entity, &mut MemoryLevel)) {
        let Some(mut e) = self.slots[i].entity.take() else {
            return;
        };
        if !e.is_removed() {
            f(&mut e, self);
        }
        self.slots[i].entity = Some(e);
        let s = &mut self.slots[i];
        if let Some(e) = &s.entity {
            let now = section_of(e);
            if now != s.section {
                s.section = now;
                s.seq = self.next_seq;
                self.next_seq += 1;
            }
        }
    }

    /// Players' own tick: their hurt cooldowns run down.
    pub fn tick_players(&mut self) {
        for v in self.player_cooldown.values_mut() {
            if v.0 > 0 {
                v.0 -= 1;
            }
        }
    }

    /// Moves entities added by behaviours into the level.
    /// The next entity id handed out will be `id` (to follow another world's numbering).
    pub fn set_next_entity_id(&mut self, id: i32) {
        self.next_id = id - 1;
    }

    pub fn flush_spawned(&mut self) {
        for e in std::mem::take(&mut self.spawned) {
            self.insert(e);
        }
    }

    pub fn entity_at(&self, i: usize) -> Option<&Entity> {
        self.slots.get(i).and_then(|s| s.entity.as_ref())
    }
}

impl EntityLevel for MemoryLevel {
    fn block(&self, pos: BlockPos) -> u16 {
        let floor = match self.bottom_layer {
            Some(b) if pos.y == self.min_y => b,
            _ => 0,
        };
        self.blocks.get(&pos).copied().unwrap_or(floor)
    }

    fn set_block(&mut self, pos: BlockPos, state: u16, _flags: u32) -> bool {
        self.blocks.insert(pos, state).unwrap_or(0) != state
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn sky_darken(&self) -> i32 {
        self.sky_darken
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        (self.sky_light(pos) - sky_darken).max(0)
    }

    /// Open sky above the harness floor, darkness below it; water above dims it by one per
    /// block (its light opacity), as straight down a pool.
    fn sky_light(&self, pos: BlockPos) -> i32 {
        let mut sky = 15;
        for (p, s) in self.blocks.iter() {
            if p.x == pos.x && p.z == pos.z && p.y > pos.y && !kiln_data::blocks_types::is_air(*s) {
                if crate::blocks::block_name(*s) != "minecraft:water" {
                    return 0;
                }
                sky -= 1;
            }
        }
        sky.max(0)
    }

    fn difficulty(&self) -> u8 {
        self.difficulty
    }

    /// `Player.hurtServer` with its hurt cooldown (difficulty scaling at normal: none).
    fn hurt_player(&mut self, id: i32, _source: crate::mob::DamageSource, amount: f32) -> bool {
        let Some(p) = self.players.iter().find(|p| p.id == id) else {
            return false;
        };
        if p.creative || p.spectator || !p.alive {
            return false;
        }
        let (cd, last) = self.player_cooldown.get(&id).copied().unwrap_or((0, 0.0));
        let dealt = if cd as f32 > 10.0 {
            if amount <= last {
                return false;
            }
            self.player_cooldown.insert(id, (cd, amount));
            amount - last
        } else {
            self.player_cooldown.insert(id, (20, amount));
            amount
        };
        self.player_hits.push((id, dealt));
        true
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        let mut found: Vec<((i32, i64), u64, i32)> = self
            .slots
            .iter()
            .filter_map(|s| {
                let e = s.entity.as_ref()?;
                let wanted = match filter {
                    EntityFilter::Any => true,
                    EntityFilter::Item => matches!(e.kind, EntityKind::Item(_)),
                    EntityFilter::ExperienceOrb => matches!(e.kind, EntityKind::ExperienceOrb(_)),
                    EntityFilter::Living => matches!(e.kind, EntityKind::Other { .. } | EntityKind::Player(_) | EntityKind::Mob(_)),
                };
                (wanted && e.id != exclude && e.is_alive() && e.bounding_box().intersects(area)).then_some((s.section, s.seq, e.id))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, _, id)| id).collect()
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut Entity> {
        let i = *self.index.get(&id)?;
        self.slots[i].entity.as_mut()
    }

    fn entity(&self, id: i32) -> Option<&Entity> {
        let i = *self.index.get(&id)?;
        self.slots[i].entity.as_ref()
    }

    fn add_entity(&mut self, entity: Entity) {
        if self.immediate_adds {
            self.insert(entity);
        } else {
            self.spawned.push(entity);
        }
    }

    fn next_entity_id(&mut self) -> i32 {
        self.next_id += 1;
        self.next_id
    }

    fn fresh_seed(&mut self) -> i64 {
        (self.next_id as i64).wrapping_mul(0x5DEE_CE66D)
    }

    fn players(&self) -> &[PlayerView] {
        &self.players
    }

    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}
