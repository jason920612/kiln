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
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
    fn write_u128(&mut self, v: u128) {
        self.write_u64(v as u64);
        self.write_u64((v >> 64) as u64);
    }
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

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
    /// Mob AI draws from the level's random, as vanilla's (one stream for all mobs).
    pub share_ai_random: bool,
    /// The overworld clock (the villagers' schedule reads it).
    pub day_time: i64,
    /// Tickets taken at points of interest (`memory_poi`).
    pub(crate) poi_taken: FastMap<BlockPos, i32>,
    /// wp28: the wardens' listeners (`set_listener`) and the vibrations they heard since their
    /// last tick (`game_event` posts to them like `GameEventDispatcher`).
    pub ears: Vec<(i32, crate::vibration::Ear)>,
    pub heard: Vec<(i32, crate::vibration::Heard)>,
    /// The creaking hearts (their block entities) by position.
    pub hearts: FastMap<BlockPos, crate::mob::kinds::creaking_heart::HeartBe>,
    /// The `minecraft:gameplay/creaking_active` attribute.
    pub creaking_active: bool,
    /// The mob spawners (their block entities) by position.
    pub spawners: FastMap<BlockPos, crate::spawner::SpawnerBe>,
    /// Light levels fed from a recording: (sky, block) by position, where known.
    pub lights: FastMap<BlockPos, (i32, i32)>,
    /// `spawner_blocks_work`.
    pub spawner_blocks_work: bool,
}

/// Vanilla iterates entity sections by x, then by the packed (z, y) section key.
fn section_of(e: &Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl MemoryLevel {
    /// The level random's state (`Random.seed` in Java terms), for traces.
    pub fn random_state(&self) -> i64 {
        self.random.state()
    }

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
            share_ai_random: false,
            day_time: 1000,
            poi_taken: FastMap::default(),
            ears: Vec::new(),
            heard: Vec::new(),
            hearts: FastMap::default(),
            creaking_active: false,
            spawners: FastMap::default(),
            lights: FastMap::default(),
            spawner_blocks_work: true,
        }
    }

    /// Ticks the mob spawners (`Level.tickBlockEntities`), in position order.
    pub fn tick_spawners(&mut self) {
        let mut at: Vec<BlockPos> = self.spawners.keys().copied().collect();
        at.sort_by_key(|p| (p.x, p.y, p.z));
        for p in at {
            if let Some(mut be) = self.spawners.remove(&p) {
                crate::spawner::tick(self, p, &mut be);
                self.spawners.insert(p, be);
            }
        }
    }

    /// The creaking heart at `pos` goes away: a player breaking it (`source`: `playerWillDestroy`)
    /// makes its creaking twitch and die; either way removing the block entity lets the creaking
    /// go (`preRemoveSideEffects`).
    pub fn destroy_heart(&mut self, pos: BlockPos, source: Option<crate::mob::DamageSource>) {
        use crate::mob::kinds::creaking_heart::remove_protector;
        if source.is_some()
            && let Some(mut be) = self.hearts.remove(&pos)
        {
            remove_protector(self, pos, &mut be, source);
            self.hearts.insert(pos, be);
        }
        self.blocks.insert(pos, 0);
        if let Some(mut be) = self.hearts.remove(&pos) {
            remove_protector(self, pos, &mut be, None);
        }
    }

    /// Ticks the creaking hearts (`Level.tickBlockEntities`), in position order.
    pub fn tick_hearts(&mut self) {
        let mut at: Vec<BlockPos> = self.hearts.keys().copied().collect();
        at.sort_by_key(|p| (p.x, p.y, p.z));
        for p in at {
            if let Some(mut be) = self.hearts.remove(&p) {
                crate::mob::kinds::creaking_heart::tick(self, p, &mut be);
                self.hearts.insert(p, be);
            }
        }
    }

    /// `Level.gameEvent(event, pos, ctx)` for the wardens' listeners (`Listener.handleGameEvent`
    /// of `VibrationSystem`: within 16 blocks, `#warden_can_listen`, not occluded by wool).
    pub fn game_event(&mut self, event: &'static str, from: crate::math::Vec3, ctx: crate::vibration::Context) {
        use crate::vibration::{self, Validity};
        let block = |p: BlockPos| self.block(p);
        let mut got = Vec::new();
        for &(id, ear) in &self.ears {
            let (c, e) = (BlockPos::containing(from.x, from.y, from.z), BlockPos::containing(ear.pos.x, ear.pos.y, ear.pos.z));
            let d = [(c.x - e.x) as i64, (c.y - e.y) as i64, (c.z - e.z) as i64];
            if ear.busy || d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 16 * 16 {
                continue;
            }
            if vibration::is_valid_vibration(event, &ctx, "minecraft:warden_can_listen", true) != Validity::Valid {
                continue;
            }
            let untargetable = ctx.source.is_some_and(|s| s.living && (s.untargetable || matches!(s.type_name, "minecraft:warden" | "minecraft:armor_stand")));
            if !ear.can_hear || untargetable || vibration::is_occluded(&block, from, ear.pos) {
                continue;
            }
            got.push((id, vibration::Heard { event, from, to: ear.pos, source: ctx.source, tick: self.game_time }));
        }
        self.heard.extend(got);
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
    /// A wandering trader sells one thing (emeralds for a stick): the tests have no trade data.
    fn trade_offers(&mut self, set: &str, _merchant: &crate::level::TradeMerchant) -> Vec<kiln_item::trading::MerchantOffer> {
        use kiln_item::trading::{ItemCost, MerchantOffer};
        if !set.starts_with("minecraft:wandering_trader/") {
            return Vec::new();
        }
        let (Some(emerald), Some(stick)) = (kiln_data::builtin_id("minecraft:item", "minecraft:emerald"), kiln_item::ItemStack::of("minecraft:stick", 1)) else { return Vec::new() };
        vec![MerchantOffer::new(ItemCost::new(emerald, 1), None, stick, 12, 1, 0.05)]
    }

    fn block(&self, pos: BlockPos) -> u16 {
        let floor = match self.bottom_layer {
            Some(b) if pos.y == self.min_y => b,
            _ => 0,
        };
        self.blocks.get(&pos).copied().unwrap_or(floor)
    }

    /// The states themselves, hashed: tests poke `blocks` directly, so there is no change counter
    /// to trust, and this makes the reuse of block scans (`memo`) depend on the blocks alone.
    fn blocks_epoch(&self, min: BlockPos, max: BlockPos) -> Option<crate::level::BlocksEpoch> {
        let mut h = [0xcbf2_9ce4_8422_2325u64, 0x9e37_79b9_7f4a_7c15, 0x1234_5678_9abc_def1, 0x0fed_cba9_8765_4321];
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    let s = self.block(BlockPos::new(x, y, z)) as u64 + 1;
                    for (i, v) in h.iter_mut().enumerate() {
                        *v = (*v ^ s).wrapping_mul(0x100_0000_01b3 + 2 * i as u64);
                        *v ^= *v >> 29;
                    }
                }
            }
        }
        Some(crate::level::BlocksEpoch(h))
    }

    fn set_block(&mut self, pos: BlockPos, state: u16, _flags: u32) -> bool {
        // `FrogspawnBlock.onPlace` schedules its hatching with a draw from the level's random
        // (the real level runs the block's behaviour; this one only keeps the states).
        if state == kiln_data::blocks::default_state::FROGSPAWN {
            let _ = kiln_javamath::random::RandomSource::next_int_bounded(&mut self.random, 12000 - 3600);
        }
        self.blocks.insert(pos, state).unwrap_or(0) != state
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn shared_ai_random(&mut self) -> Option<&mut LegacyRandom> {
        if self.share_ai_random { Some(&mut self.random) } else { None }
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn day_time(&self) -> i64 {
        self.day_time
    }

    /// A hive or bee nest block (never full: the replay's hives take every bee).
    fn beehive_at(&self, pos: BlockPos) -> Option<crate::level::BeehiveView> {
        use kiln_data::block_logic::{BlockClass, block_class};
        if block_class(self.block(pos)) != BlockClass::BeehiveBlock {
            return None;
        }
        let mut fire_nearby = false;
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    fire_nearby |= block_class(self.block(BlockPos::new(pos.x + x, pos.y + y, pos.z + z))) == BlockClass::FireBlock;
                }
            }
        }
        Some(crate::level::BeehiveView { full: false, fire_nearby })
    }

    fn bees_stay_in_hive(&self) -> bool {
        (12542..23460).contains(&self.day_time.rem_euclid(24000))
    }

    fn poi_in_range(&self, types: &[&str], center: BlockPos, radius: i32, occupancy: crate::level::PoiOccupancy) -> Vec<BlockPos> {
        self.poi_in_range_impl(types, center, radius, occupancy)
    }

    fn poi_take(&mut self, types: &[&str], center: BlockPos, radius: i32, accept: &dyn Fn(&str, BlockPos) -> bool) -> Option<BlockPos> {
        self.poi_take_impl(types, center, radius, accept)
    }

    fn poi_release(&mut self, pos: BlockPos) {
        self.poi_release_impl(pos);
    }

    fn poi_type(&self, pos: BlockPos) -> Option<&'static str> {
        self.poi_type_impl(pos)
    }

    fn sections_to_village(&self, pos: BlockPos) -> i32 {
        self.sections_to_village_impl(pos)
    }

    fn sky_darken(&self) -> i32 {
        self.sky_darken
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        (self.sky_light(pos) - sky_darken).max(self.block_light(pos)).max(0)
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        self.lights.get(&pos).map_or(0, |l| l.1)
    }

    fn spawner_blocks_enabled(&self) -> bool {
        self.spawner_blocks_work
    }

    fn add_entity_stack(&mut self, root: Entity, companions: Vec<crate::mob::Companion>, _loaded: bool, _nearby_chicken: bool) -> bool {
        use crate::mob::Seat;
        let root_id = root.id;
        self.add_entity(root);
        let mut ids = Vec::new();
        for c in companions {
            ids.push(c.entity.id);
            let (id, seat) = (c.entity.id, c.seat);
            self.add_entity(c.entity);
            // Seats only where the entities are in the level at once.
            if self.immediate_adds {
                let (rider, vehicle) = match seat {
                    Seat::OnMob => (id, root_id),
                    Seat::OnCompanion(i) => (id, ids[i]),
                    Seat::UnderMob => (root_id, id),
                    Seat::Loose => continue,
                };
                let marker = Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0);
                if let Some(slot) = self.entity_mut(vehicle) {
                    let mut v = std::mem::replace(slot, marker);
                    if let Some(r) = self.entity_mut(rider) {
                        crate::ride::start_riding(r, &mut v, false);
                    }
                    if let Some(slot) = self.entity_mut(vehicle) {
                        *slot = v;
                    }
                }
            }
        }
        true
    }

    /// Open sky above the harness floor, darkness below it; water above dims it by one per
    /// block (its light opacity), as straight down a pool.
    fn sky_light(&self, pos: BlockPos) -> i32 {
        if let Some(l) = self.lights.get(&pos) {
            return l.0;
        }
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
    fn hurt_player(&mut self, id: i32, source: crate::mob::DamageSource, amount: f32) -> bool {
        let Some(p) = self.players.iter().find(|p| p.id == id) else {
            return false;
        };
        let by_mob = source.attacker.filter(|&a| a != id && self.index.contains_key(&a) && self.player(a).is_none());
        if p.creative || p.spectator || !p.alive {
            return false;
        }
        let (cd, last) = self.player_cooldown.get(&id).copied().unwrap_or((0, 0.0));
        // `Player.hurtServer`: no damage, no hit.
        if amount == 0.0 {
            return false;
        }
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
        // `LivingEntity.actuallyHurt`: the player's `ENTITY_DAMAGE` game event (a warden hears it).
        if !self.ears.is_empty()
            && let Some(p) = self.players.iter().find(|p| p.id == id).copied()
        {
            let ctx = crate::vibration::Context { source: Some(crate::vibration::EventSource::player(p.id, p.uuid, p.pos, p.sneaking, p.spectator, p.creative)), affected_state: None };
            self.game_event("minecraft:entity_damage", p.pos, ctx);
        }
        // `setLastHurtByMob`: stamped with the player's own clock.
        if by_mob.is_some()
            && let Some(p) = self.players.iter_mut().find(|p| p.id == id)
        {
            p.last_hurt_by_mob_time = p.tick_count;
        }
        true
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn add_effect_instance(&mut self, id: i32, effect: crate::effect::Effect, source: Option<i32>) -> bool {
        crate::mob::effects::add_to_entity(self, id, effect, source)
    }

    fn apply_instantaneous_effect(&mut self, id: i32, effect: &crate::effect::Effect, source: Option<(i32, crate::math::Vec3)>, owner: Option<i32>, scale: f64) {
        let owner_is_player = owner.is_some_and(|o| self.player(o).is_some());
        // A player: instant damage hurts (`indirectMagic(source, owner)`, `magic` without a
        // source); healing is not tracked.
        if self.player(id).is_some() {
            if effect.id == crate::effect::ids::instant_damage() {
                let amount = (scale * (6i32.wrapping_shl(effect.amplifier as u32)) as f64 + 0.5) as i32 as f32;
                let kind = if source.is_some() { crate::level::DamageKind::IndirectMagic } else { crate::level::DamageKind::Magic };
                let src = crate::mob::DamageSource { kind, attacker: owner.or(source.map(|s| s.0)), direct: source.map(|s| s.0), pos: source.map(|s| s.1), attacker_is_player: owner_is_player };
                self.hurt_player(id, src, amount);
            }
            return;
        }
        crate::mob::effects::apply_instantaneous_to_entity(self, id, effect, source, owner, owner_is_player, scale);
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

    fn take_vibrations(&mut self, id: i32) -> Vec<crate::vibration::Heard> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.heard).into_iter().partition(|h| h.0 == id);
        self.heard = rest;
        mine.into_iter().map(|h| h.1).collect()
    }

    fn set_listener(&mut self, id: i32, ear: Option<crate::vibration::Ear>) {
        self.ears.retain(|e| e.0 != id);
        if let Some(ear) = ear {
            self.ears.push((id, ear));
        }
    }
    fn heart_protects(&mut self, home: BlockPos, id: i32, uuid: u128) -> bool {
        crate::mob::kinds::creaking_heart::is_heart(self.block(home)) && self.hearts.get(&home).is_some_and(|h| h.protects(id, uuid))
    }

    fn heart_creaking_hurt(&mut self, home: BlockPos, id: i32, uuid: u128, at: crate::math::Vec3) {
        if let Some(mut be) = self.hearts.remove(&home) {
            crate::mob::kinds::creaking_heart::creaking_hurt(self, home, &mut be, id, uuid, at);
            self.hearts.insert(home, be);
        }
    }

    fn entity_by_uuid(&self, uuid: u128) -> Option<&Entity> {
        self.slots.iter().filter_map(|s| s.entity.as_ref()).find(|e| e.uuid == uuid && !e.is_removed())
    }

    fn creaking_active(&self, _pos: BlockPos) -> bool {
        self.creaking_active
    }
}
