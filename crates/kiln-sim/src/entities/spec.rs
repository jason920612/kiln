//! Speculation: the serial entity phase with its turns tried side by side first.
//!
//! The entity phase is serial: each entity's tick sees what the ones before it did. Most turns,
//! though, read nothing an earlier turn of the same tick changed: a slime in one corner of a
//! crowd does not look at a sheep in another. So every turn first runs on the tick pool against
//! the state at the start of the phase, on a copy of its entity, through [`SpecLevel`], which
//! keeps what the turn left behind (its entity, the other entities it changed, spawns, events,
//! packets) and logs what it read: the entities it looked at, the areas it searched, whether it
//! read the players' own state. Anything else it would change (blocks, players, the region's
//! machinery, fresh entity ids) makes the speculation unusable.
//!
//! Then the turns run in list order. A turn whose speculation read nothing changed so far this
//! phase (no entity it looked at changed, no changed entity's box before or after meets an area
//! it searched, no player changed if it read players, no block changed) is done: what it left
//! behind is put in place, exactly what running it now would leave. Any other turn runs now, in
//! place, as the serial phase runs it, and what it changed is noted. The outcome is the serial
//! phase's whatever the workers, and it does not depend on how many turns were kept.

use super::*;
use std::cell::RefCell;

/// Regions with fewer entities run their turns in order straight away.
const MIN_ENTITIES: usize = 8;

/// A region's speculation record: crowds whose turns mostly read each other (a dense herd
/// pushing itself about) gain nothing, so after a tick where more than [`RERUN_SHARE`] of the
/// turns ran in place, the region runs its turns in order for [`PAUSE`] ticks.
#[derive(Default)]
pub(crate) struct Pace {
    paused: u32,
    /// Whether the region's turns are worth trying side by side this tick: only a region that
    /// holds up the others (its tick takes more than its share of the workers' time) gains, the
    /// copies and logs only cost CPU otherwise ([`crate::Sim::run_regions`] decides).
    pub(crate) wanted: bool,
}

/// Ran-in-place share (numerator, denominator) above which speculation pauses.
const RERUN_SHARE: (u32, u32) = (2, 5);
const PAUSE: u32 = 40;

/// Another entity (or a player's stand-in) a speculative turn changed, with its box and whether
/// it was alive at the start.
struct Overlay {
    id: i32,
    ent: Box<kiln_entity::Entity>,
    stand_in: bool,
    before: Aabb,
    alive: bool,
}

/// What a speculative turn read.
#[derive(Default)]
struct Log {
    /// Entities and stand-ins looked at.
    ids: Vec<i32>,
    /// Areas searched for entities.
    areas: Vec<Aabb>,
    /// The players' own state (not their views) was read.
    players: bool,
    /// Something it cannot leave for later was asked for.
    abort: bool,
}

/// A speculative turn's result.
pub(super) struct Spec {
    phys: Box<kiln_entity::Entity>,
    overlays: Vec<Overlay>,
    spawns: Vec<Spawn>,
    events: Vec<Event>,
    packets: Vec<NearPacket>,
    log: Log,
    seeds: u64,
    rng: LegacyRandom,
}

/// The level a speculative turn sees: the phase's start through `base` (read only), its own
/// changes on top.
struct SpecLevel<'s, 'a, 'l, 'p> {
    base: &'s SimLevel<'a, 'l, 'p>,
    /// The ticking entity (its state is out for its tick, as in the serial phase).
    me: i32,
    rng: LegacyRandom,
    seeds: u64,
    overlays: Vec<Overlay>,
    spawns: Vec<Spawn>,
    events: Vec<Event>,
    packets: Vec<NearPacket>,
    log: RefCell<Log>,
    placeholder: i32,
}

impl SpecLevel<'_, '_, '_, '_> {
    fn abort(&self) {
        self.log.borrow_mut().abort = true;
    }

    fn is_player(&self, id: i32) -> bool {
        self.base.players.iter().any(|p| p.entity_id == id)
    }

    fn overlay(&self, id: i32) -> Option<&kiln_entity::Entity> {
        self.overlays.iter().find(|o| o.id == id).map(|o| &*o.ent)
    }

    /// An area search over the phase's start is what the serial phase would find while the
    /// entities this turn changed keep their boxes and lives.
    fn check_overlays(&self) {
        if self.overlays.iter().any(|o| o.ent.bounding_box() != o.before || o.ent.is_alive() != o.alive) {
            self.abort();
        }
    }
}

impl EntityLevel for SpecLevel<'_, '_, '_, '_> {
    fn biome(&self, pos: BlockPos) -> Option<i32> {
        self.base.biome(pos)
    }

    fn piglins_zombify(&self) -> bool {
        self.base.piglins_zombify()
    }

    fn snow_golem_melts(&self, pos: Vec3) -> bool {
        self.base.snow_golem_melts(pos)
    }

    fn trade_offers(&mut self, set: &str, merchant: &kiln_entity::level::TradeMerchant) -> Vec<kiln_item::trading::MerchantOffer> {
        let env = self.base.level.env();
        crate::trading::roll_offers(env.loot.as_deref(), env.seed, env.game_time, set, merchant)
    }

    fn raid(&self, id: i32) -> Option<&kiln_entity::level::RaidView> {
        self.base.raid(id)
    }

    fn raid_at(&self, pos: BlockPos) -> Option<&kiln_entity::level::RaidView> {
        self.base.raid_at(pos)
    }

    fn village_centers_near(&self, section: (i32, i32, i32), radius: i32) -> Option<Vec<(i32, i32, i32)>> {
        self.base.village_centers_near(section, radius)
    }

    fn sections_to_village(&self, pos: BlockPos) -> i32 {
        self.base.sections_to_village(pos)
    }

    fn poi_in_range(&self, types: &[&str], center: BlockPos, radius: i32, occupancy: kiln_entity::level::PoiOccupancy) -> Vec<BlockPos> {
        self.base.poi_in_range(types, center, radius, occupancy)
    }

    fn poi_take(&mut self, _types: &[&str], _center: BlockPos, _radius: i32, _accept: &dyn Fn(&str, BlockPos) -> bool) -> Option<BlockPos> {
        self.abort();
        None
    }

    fn poi_release(&mut self, _pos: BlockPos) {
        self.abort();
    }

    fn poi_type(&self, pos: BlockPos) -> Option<&'static str> {
        self.base.poi_type(pos)
    }

    fn motion_blocking_no_leaves_height(&self, x: i32, z: i32) -> i32 {
        self.base.motion_blocking_no_leaves_height(x, z)
    }

    fn block(&self, pos: BlockPos) -> u16 {
        self.base.block(pos)
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.base.is_loaded(pos)
    }

    fn read_blocks(&self, min: BlockPos, max: BlockPos, out: &mut [u16]) -> bool {
        self.base.read_blocks(min, max, out)
    }

    fn no_fluid_in(&self, min: BlockPos, max: BlockPos) -> bool {
        self.base.no_fluid_in(min, max)
    }

    fn blocks_epoch(&self, min: BlockPos, max: BlockPos) -> Option<kiln_entity::level::BlocksEpoch> {
        self.base.blocks_epoch(min, max)
    }

    fn any_block_in(&self, min: BlockPos, max: BlockPos, pred: &dyn Fn(u16) -> bool) -> bool {
        self.base.any_block_in(min, max, pred)
    }

    fn set_block(&mut self, _pos: BlockPos, _state: u16, _flags: u32) -> bool {
        self.abort();
        false
    }

    fn destroy_block(&mut self, _pos: BlockPos, _drop: bool) -> bool {
        self.abort();
        false
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.rng
    }

    fn game_time(&self) -> i64 {
        self.base.game_time()
    }

    fn day_time(&self) -> i64 {
        self.base.day_time()
    }

    fn min_y(&self) -> i32 {
        self.base.min_y()
    }

    fn max_y(&self) -> i32 {
        self.base.max_y()
    }

    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        self.check_overlays();
        self.log.borrow_mut().areas.push(*area);
        let mut found = self.base.entities_in(area, filter, exclude);
        found.retain(|&id| id != self.me);
        found
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut kiln_entity::Entity> {
        if id == self.me {
            return None;
        }
        if let Some(k) = self.overlays.iter().position(|o| o.id == id) {
            return Some(&mut *self.overlays[k].ent);
        }
        let (ent, stand_in) = match self.base.index(id) {
            Some(i) => (self.base.list[i].phys.as_deref()?.clone(), false),
            None => {
                let k = self.base.proxy_index(id)?;
                (self.base.proxies[k].get().clone(), true)
            }
        };
        self.log.get_mut().ids.push(id);
        let (before, alive) = (ent.bounding_box(), ent.is_alive());
        self.overlays.push(Overlay { id, ent: Box::new(ent), stand_in, before, alive });
        self.overlays.last_mut().map(|o| &mut *o.ent)
    }

    fn entity(&self, id: i32) -> Option<&kiln_entity::Entity> {
        if id == self.me {
            return None;
        }
        if let Some(e) = self.overlay(id) {
            return Some(e);
        }
        self.log.borrow_mut().ids.push(id);
        self.base.entity(id)
    }

    fn add_entity(&mut self, entity: kiln_entity::Entity) {
        let Some(kind) = kiln_data::entities::by_name(entity.type_name) else { return };
        self.spawns.push(Spawn { kind, pos: arr(entity.position()), vel: arr(entity.delta), body: Body::Ready(Box::new(entity)) });
    }

    fn next_entity_id(&mut self) -> i32 {
        // Ids depend on how many earlier turns drew one.
        self.abort();
        self.placeholder -= 1;
        self.placeholder
    }

    fn fresh_seed(&mut self) -> i64 {
        self.seeds += 1;
        let env = self.base.level.env();
        let mut h = (env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.seed as u64;
        for v in [self.me as u64, self.seeds] {
            h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        h as i64
    }

    fn players(&self) -> &[PlayerView] {
        self.base.players()
    }

    fn players_in(&self, area: &Aabb) -> Vec<PlayerView> {
        self.base.players_in(area)
    }

    fn enchant_from_provider(&self, stack: &mut kiln_item::ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn kiln_javamath::random::RandomSource) {
        self.base.enchant_from_provider(stack, provider, special_multiplier, random)
    }

    fn player(&self, id: i32) -> Option<PlayerView> {
        self.base.player(id)
    }

    fn player_by_uuid(&self, uuid: u128) -> Option<PlayerView> {
        self.base.player_by_uuid(uuid)
    }

    fn emit(&mut self, event: Event) {
        if let Event::GameEvent { .. } = event {
            // Only listeners hear game events (a region with any ticks its entities in order).
            if self.base.level.listening() {
                self.abort();
            }
            return;
        }
        self.events.push(event);
    }

    fn block_game_event(&mut self, _event: &'static str, _pos: Vec3, _entity: Option<i32>, _state: u16) {
        if self.base.level.listening() {
            self.abort();
        }
    }

    fn sculk_step_on(&mut self, _pos: BlockPos, _entity: i32, _at: Vec3) {
        self.abort();
    }

    fn sculk_catalyst_near(&self, pos: Vec3) -> bool {
        self.base.sculk_catalyst_near(pos)
    }

    fn feed_sculk_catalyst(&mut self, _pos: Vec3, _charge: i32) {
        self.abort();
    }

    fn take_vibrations(&mut self, _id: i32) -> Vec<kiln_entity::vibration::Heard> {
        self.abort();
        Vec::new()
    }

    fn set_listener(&mut self, _id: i32, _ear: Option<kiln_entity::vibration::Ear>) {
        self.abort();
    }

    fn take_allay_vibrations(&mut self, _id: i32) -> Vec<kiln_entity::vibration::Heard> {
        self.abort();
        Vec::new()
    }

    fn set_allay_listener(&mut self, _id: i32, _ear: Option<kiln_entity::vibration::Ear>) {
        self.abort();
    }

    fn vibration_particle(&mut self, _from: Vec3, _entity: i32, _y_offset: f32, _ticks: i32) {
        self.abort();
    }

    fn darkness_around(&mut self, _pos: Vec3, _radius: f64) {
        self.abort();
    }

    fn particle(&mut self, particle: &'static str, pos: Vec3) {
        self.packets.extend(particle_packet(particle, pos));
    }

    fn trail_particle(&mut self, pos: Vec3, target: Vec3, color: i32, duration: i32) {
        self.packets.extend(trail_packet(pos, target, color, duration));
    }

    fn crumble_particles(&mut self, pos: Vec3, state: u16, count: i32, spread: Vec3) {
        self.packets.extend(crumble_packet("minecraft:block_crumble", pos, state, count, spread, 0.0));
    }

    fn block_particles(&mut self, particle: &'static str, pos: Vec3, state: u16, count: i32, spread: Vec3, speed: f32) {
        self.packets.extend(crumble_packet(particle, pos, state, count, spread, speed));
    }

    fn mob_griefing(&self) -> bool {
        self.base.mob_griefing()
    }

    fn universal_anger(&self) -> bool {
        self.base.universal_anger()
    }

    fn forgive_dead_players(&self) -> bool {
        self.base.forgive_dead_players()
    }

    fn ender_pearls_vanish_on_death(&self) -> bool {
        self.base.ender_pearls_vanish_on_death()
    }

    fn explosion_drop_decay(&self, rule: kiln_entity::explosion::DecayRule) -> bool {
        self.base.explosion_drop_decay(rule)
    }

    fn dragon_fight(&self) -> Option<kiln_entity::level::DragonFightView> {
        self.base.dragon_fight()
    }

    fn mob_drops(&self) -> bool {
        self.base.mob_drops()
    }

    fn entity_drops(&self) -> bool {
        self.base.entity_drops()
    }

    fn tnt_explodes(&self) -> bool {
        self.base.tnt_explodes()
    }

    fn fill_container_loot(&mut self, items: &mut [kiln_item::ItemStack], table: &str, seed: i64, origin: Vec3, player: Option<i32>) {
        // A player's criteria fire.
        if player.is_some() {
            self.abort();
            return;
        }
        let env = self.base.level.env();
        let Some(loot) = env.loot.clone() else { return };
        let at = [origin.x.floor() as i32, origin.y.floor() as i32, origin.z.floor() as i32];
        crate::container::fill_from_table(items, &loot, table, seed, arr(origin), at, false, env.game_time, env.seed);
    }

    fn block_loot(&mut self, state: u16, origin: Vec3, tool: &kiln_item::ItemStack, entity: i32) -> Vec<kiln_item::ItemStack> {
        block_loot_in(self.base.level.env(), state, origin, tool, entity)
    }

    fn hopper_take_from_block(&mut self, _pos: BlockPos, _dest: &mut Vec<kiln_item::ItemStack>) -> Option<bool> {
        self.abort();
        None
    }

    fn difficulty(&self) -> u8 {
        self.base.difficulty()
    }

    fn creaking_active(&self, pos: BlockPos) -> bool {
        self.base.creaking_active(pos)
    }

    fn spawning_monsters(&self) -> bool {
        self.base.spawning_monsters()
    }

    fn heart_protects(&mut self, _home: BlockPos, _id: i32, _uuid: u128) -> bool {
        self.abort();
        true
    }

    fn heart_creaking_hurt(&mut self, _home: BlockPos, _id: i32, _uuid: u128, _at: Vec3) {
        self.abort();
    }

    fn entity_by_uuid(&self, uuid: u128) -> Option<&kiln_entity::Entity> {
        // As the serial phase finds it: the first live list entry with the UUID (none for the
        // ticking entity, whose state is out), else a stand-in.
        let listed = self.base.list.iter().find(|e| e.uuid.as_u128() == uuid && !e.removed);
        let found = listed.and_then(|e| {
            if e.id == self.me {
                return None;
            }
            self.log.borrow_mut().ids.push(e.id);
            self.overlay(e.id).or(e.phys.as_deref())
        });
        found.or_else(|| {
            let p = self.base.proxies.iter().find(|p| p.uuid == uuid)?;
            self.log.borrow_mut().ids.push(p.id);
            Some(self.overlay(p.id).unwrap_or_else(|| p.get()))
        })
    }

    fn known_movement(&self, id: i32) -> Vec3 {
        if let Some(i) = self.base.proxy_index(id)
            && let Some(p) = self.base.players.iter().find(|p| p.entity_id == self.base.proxies[i].id)
        {
            self.log.borrow_mut().players = true;
            return vec3(p.known_movement);
        }
        self.entity(id).map_or(Vec3::ZERO, |e| e.delta)
    }

    fn pos_random(&mut self, pos: BlockPos, salt: i64) -> LegacyRandom {
        crate::container::pos_random_in(self.base.level.env(), kb(pos), salt as u64)
    }

    fn update_neighbours_for_output_signal(&mut self, _pos: BlockPos) {
        self.abort();
    }

    fn add_entity_with_uuid(&mut self, entity: kiln_entity::Entity) {
        let Some(kind) = kiln_data::entities::by_name(entity.type_name) else { return };
        self.spawns.push(Spawn { kind, pos: arr(entity.position()), vel: arr(entity.delta), body: Body::Loaded(Box::new(entity)) });
    }

    fn sky_darken(&self) -> i32 {
        self.base.sky_darken()
    }

    fn is_raining_at(&self, pos: BlockPos) -> bool {
        self.base.is_raining_at(pos)
    }

    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        self.base.can_spread_fire_around(pos)
    }

    fn place_lightning_fire(&mut self, _pos: BlockPos) -> bool {
        self.abort();
        false
    }

    fn lightning_strike_block(&mut self, _pos: BlockPos) {
        self.abort();
    }

    fn thunder_hit_player(&mut self, _id: i32) {
        self.abort();
    }

    fn monsters_burn(&self) -> bool {
        self.base.monsters_burn()
    }

    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        self.base.raw_brightness(pos, sky_darken)
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        self.base.sky_light(pos)
    }

    fn effective_difficulty(&self, pos: BlockPos) -> f32 {
        self.base.effective_difficulty(pos)
    }

    fn hurt_player(&mut self, _id: i32, _source: kiln_entity::mob::DamageSource, _amount: f32) -> bool {
        self.abort();
        false
    }

    fn push(&mut self, id: i32, v: Vec3) {
        if self.is_player(id) {
            self.abort();
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            e.delta = e.delta + v;
            e.needs_sync = true;
        }
    }

    fn motion(&self, id: i32) -> Vec3 {
        if let Some(p) = self.base.players.iter().find(|p| p.entity_id == id) {
            self.log.borrow_mut().players = true;
            return vec3(p.vel);
        }
        self.entity(id).map_or(Vec3::ZERO, |e| e.delta)
    }

    fn knockback_target(&mut self, id: i32, strength: f64, dx: f64, dz: f64, _old_motion: Vec3) {
        if self.is_player(id) {
            self.abort();
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            kiln_entity::mob::knockback_entity(e, strength, dx, dz);
        }
    }

    fn stop_riding(&mut self, id: i32) {
        if self.is_player(id) {
            self.abort();
            return;
        }
        kiln_entity::level::stop_riding_entity(self, id);
    }

    fn is_thundering(&self) -> bool {
        self.base.is_thundering()
    }

    fn add_effect(&mut self, id: i32, _effect: &'static str, _duration: i32, _amplifier: i32, _source: Option<i32>) -> bool {
        if self.is_player(id) {
            self.abort();
        }
        false
    }

    fn add_effect_instance(&mut self, id: i32, effect: kiln_entity::effect::Effect, source: Option<i32>) -> bool {
        if self.is_player(id) {
            self.abort();
            return false;
        }
        kiln_entity::mob::effects::add_to_entity(self, id, effect, source)
    }

    fn apply_instantaneous_effect(&mut self, id: i32, effect: &kiln_entity::effect::Effect, source: Option<(i32, Vec3)>, owner: Option<i32>, scale: f64) {
        if self.is_player(id) {
            self.abort();
            return;
        }
        let owner_is_player = owner.is_some_and(|o| self.is_player(o));
        kiln_entity::mob::effects::apply_instantaneous_to_entity(self, id, effect, source, owner, owner_is_player, scale);
    }

    fn max_entity_cramming(&self) -> i32 {
        self.base.max_entity_cramming()
    }

    fn player_effect(&self, id: i32, effect: &str) -> Option<(i32, i32)> {
        self.log.borrow_mut().players = true;
        self.base.player_effect(id, effect)
    }

    fn ignite(&mut self, id: i32, seconds: f32) {
        if self.is_player(id) {
            self.abort();
            return;
        }
        if let Some(e) = self.entity_mut(id) {
            e.ignite_for_seconds(seconds);
        }
    }
}

/// Entity `i`'s turn against the phase's start, as [`tick_entity`] runs it (`None`: it does not
/// tick, or it rides or carries something).
fn speculate(sim: &SimLevel, i: usize, ticking: &blocks::Ticking, any_player: bool) -> Option<Spec> {
    let e = &sim.list[i];
    if e.removed || !ticking.contains(chunk_of(e.pos)) {
        return None;
    }
    let base = e.phys.as_deref()?;
    if base.vehicle.is_some() || !base.passengers.is_empty() || !base.pending_hurts.is_empty() || !base.pending_effects.is_empty() {
        return None;
    }
    let env = sim.level.env();
    let _enchanting = crate::enchant::install_enchanter(env.loot.as_ref());
    let mut phys = Box::new(base.clone());
    let mut lvl = SpecLevel {
        base: sim,
        me: phys.id,
        rng: entity_level_random(env.seed, env.game_time, phys.id),
        seeds: 0,
        overlays: Vec::new(),
        spawns: Vec::new(),
        events: Vec::new(),
        packets: Vec::new(),
        log: RefCell::new(Log { ids: vec![phys.id], ..Log::default() }),
        placeholder: -2_000_000_000,
    };
    if matches!(phys.kind, EntityKind::Mob(_)) && !phys.is_removed() {
        let p = phys.position();
        let nearest = match sim.despawn {
            Some(n) => n.nearest_sqr(p),
            None => sim.views.iter().filter(|v| !v.spectator).map(|v| v.pos.distance_to_sqr(p)).min_by(|a, b| a.total_cmp(b)),
        };
        kiln_entity::mob::check_despawn(&mut phys, &lvl, nearest.or(any_player.then_some(f64::MAX)));
    }
    if !phys.is_removed() {
        phys.common_tick();
        phys.tick(&mut lvl);
    }
    let SpecLevel { overlays, spawns, events, packets, log, seeds, rng, .. } = lvl;
    Some(Spec { phys, overlays, spawns, events, packets, log: log.into_inner(), seeds, rng })
}

/// What changed so far this phase, by turns kept or run in place.
struct Dirty {
    ids: std::collections::HashSet<i32, std::hash::BuildHasherDefault<kiln_entity::memory::FastHasher>>,
    /// Boxes (before and after) of entities that moved, grew or died, by 16-block cube.
    boxes: FastMap<(i32, i32, i32), Vec<Aabb>>,
    any_box: bool,
    player_writes: u32,
    edits: u64,
}

fn cube(v: f64) -> i32 {
    (v / 16.0).floor() as i32
}

impl Dirty {
    fn add_box(&mut self, b: Aabb) {
        let b = b.inflate(1e-6, 1e-6, 1e-6);
        self.any_box = true;
        for x in cube(b.min_x)..=cube(b.max_x) {
            for y in cube(b.min_y)..=cube(b.max_y) {
                for z in cube(b.min_z)..=cube(b.max_z) {
                    self.boxes.entry((x, y, z)).or_default().push(b);
                }
            }
        }
    }

    /// An entity changed: its id, and its boxes if it moved, grew or died.
    fn changed(&mut self, id: i32, before: Option<(Aabb, bool)>, after: Option<(Aabb, bool)>) {
        self.ids.insert(id);
        if before != after {
            if let Some((b, _)) = before {
                self.add_box(b);
            }
            if let Some((b, _)) = after {
                self.add_box(b);
            }
        }
    }

    fn meets(&self, area: &Aabb) -> bool {
        if !self.any_box {
            return false;
        }
        let (x0, x1, y0, y1, z0, z1) = (cube(area.min_x), cube(area.max_x), cube(area.min_y), cube(area.max_y), cube(area.min_z), cube(area.max_z));
        let span = (x1 - x0 + 1) as i64 * (y1 - y0 + 1) as i64 * (z1 - z0 + 1) as i64;
        if span > 4096 {
            return true;
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    if self.boxes.get(&(x, y, z)).is_some_and(|v| v.iter().any(|b| b.intersects(area))) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Entity `id`'s box and life, if it is in the list or a stand-in.
fn shape(sim: &SimLevel, id: i32) -> Option<(Aabb, bool)> {
    sim.entity(id).map(|e| (e.bounding_box(), e.is_alive()))
}

/// The region's entity turns with speculation ([`self`]); `false` if the region does not qualify
/// (the caller runs the turns in order).
pub(super) fn tick_speculative(sim: &mut SimLevel, pace: &mut Pace, ticking: &blocks::Ticking, any_player: bool, ctx: &kiln_sched::Ctx<'_>) -> bool {
    if !sim.level.env().speculate || !pace.wanted || sim.list.len() < MIN_ENTITIES || ctx.workers() < 2 {
        return false;
    }
    if pace.paused > 0 {
        pace.paused -= 1;
        return false;
    }
    {
        let Some(region) = sim.level.region_ref() else { return false };
        if !region.blocks.hearts.is_empty() || crate::sculk::listening(region) || sim.list.iter().any(|e| islands::SERIAL.contains(&e.kind.name)) {
            return false;
        }
    }
    let t0 = std::time::Instant::now();
    let n = sim.list.len();
    // The turns that took longest last time first, so none starts late and holds up the rest.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sim.list[i].spec_ns));
    let mut specs: Vec<Option<Spec>> = (0..n).map(|_| None).collect();
    {
        let base = &*sim;
        let done = ctx.map_indexed_with(SPEC_WINDOW, &order, |_, &i| {
            let t = std::time::Instant::now();
            let s = speculate(base, i, ticking, any_player);
            (s, t.elapsed().as_nanos().min(u32::MAX as u128) as u32)
        });
        for (&i, (s, ns)) in order.iter().zip(done) {
            specs[i] = s;
            sim.list[i].spec_ns = ns;
        }
    }
    let t1 = crate::diag::lap("spec.window", t0);
    let (mut t_keep, mut t_run) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let edits = |sim: &SimLevel| sim.level.region_ref().map_or(0, |r| r.out.edits);
    let mut dirty = Dirty {
        ids: Default::default(),
        boxes: Default::default(),
        any_box: false,
        player_writes: sim.player_writes,
        edits: edits(sim),
    };
    let (mut kept, mut ran) = (0u32, 0u32);
    // What the kept turns replaced, freed side by side afterwards (a mob's brain is many
    // small allocations).
    let mut garbage: Vec<Box<kiln_entity::Entity>> = Vec::new();
    for i in 0..n {
        if carried(sim, i) {
            continue;
        }
        let me = sim.list[i].id;
        let mut spec = specs[i].take();
        let valid = spec.as_ref().is_some_and(|s| {
            let r = if s.log.abort {
                "spec.why_abort"
            } else if edits(sim) != dirty.edits {
                "spec.why_edits"
            } else if s.log.players && sim.player_writes != dirty.player_writes {
                "spec.why_players"
            } else if !s.log.ids.iter().all(|id| !dirty.ids.contains(id)) {
                "spec.why_ids"
            } else if !s.log.areas.iter().all(|a| !dirty.meets(a)) {
                "spec.why_areas"
            } else {
                ""
            };
            if !r.is_empty() {
                crate::diag::add(r, std::time::Duration::from_nanos(1000));
            }
            r.is_empty()
        });
        if !valid && let Some(s) = spec.take() {
            garbage.push(s.phys);
            garbage.extend(s.overlays.into_iter().map(|o| o.ent));
        }
        let tk = std::time::Instant::now();
        let was = spec.is_some();
        match spec {
            Some(s) => {
                kept += 1;
                let before = shape(sim, me);
                let e = &mut sim.list[i];
                e.age += 1;
                garbage.extend(e.phys.replace(s.phys));
                (sim.current, sim.seeds, sim.rng) = (me, s.seeds, s.rng);
                sim.current_source = None;
                settle(sim, i);
                let after = shape(sim, me);
                dirty.changed(me, before, after);
                for o in s.overlays {
                    let (id, before) = (o.id, Some((o.before, o.alive)));
                    if o.stand_in {
                        if let Some(k) = sim.proxy_index(id) {
                            garbage.extend(std::mem::replace(&mut sim.proxies[k].made, std::sync::OnceLock::from(o.ent)).into_inner());
                        }
                    } else if let Some(j) = sim.index(id) {
                        garbage.extend(sim.list[j].phys.replace(o.ent));
                    }
                    let after = shape(sim, id);
                    dirty.changed(id, before, after);
                }
                sim.spawns.extend(s.spawns);
                sim.events.extend(s.events);
                for p in s.packets {
                    sim.level.push_packet(p);
                }
            }
            None => {
                // An entity that does not tick and rides nothing changes nothing.
                let e = &sim.list[i];
                if (e.removed || !ticking.contains(chunk_of(e.pos))) && e.phys.as_deref().is_none_or(|p| p.vehicle.is_none()) {
                    continue;
                }
                ran += 1;
                // In place, noting what it reaches: itself, its riders and vehicle, what it
                // changed through `entity_mut`.
                let mut near: Vec<i32> = vec![me];
                if let Some(p) = sim.list[i].phys.as_deref() {
                    near.extend(p.vehicle);
                    near.extend(p.passengers.iter().copied());
                }
                let before: Vec<(i32, Option<(Aabb, bool)>)> = near.iter().map(|&id| (id, shape(sim, id))).collect();
                sim.touched = Some(Vec::new());
                tick_turn(sim, i, ticking, any_player);
                let touched = sim.touched.take().unwrap_or_default();
                for (id, b) in before {
                    let after = shape(sim, id);
                    dirty.changed(id, b, after);
                }
                if let Some(p) = sim.list[i].phys.as_deref() {
                    let riders: Vec<i32> = p.passengers.iter().copied().chain(p.vehicle).filter(|id| !near.contains(id)).collect();
                    for id in riders {
                        let after = shape(sim, id);
                        dirty.changed(id, None, after);
                    }
                }
                for (id, b, alive) in touched {
                    let after = shape(sim, id);
                    dirty.changed(id, Some((b, alive)), after);
                }
            }
        }
        if was { t_keep += tk.elapsed() } else { t_run += tk.elapsed() }
    }
    crate::diag::add("spec.t_keep", t_keep);
    crate::diag::add("spec.t_run", t_run);
    crate::diag::lap("spec.commits", t1);
    let mut chunks: Vec<Vec<Box<kiln_entity::Entity>>> = Vec::new();
    while !garbage.is_empty() {
        let rest = garbage.split_off(garbage.len().saturating_sub(16));
        chunks.push(rest);
    }
    ctx.map_mut_with(GARBAGE_WINDOW, &mut chunks, |_, c| drop(std::mem::take(c)));
    if ran * RERUN_SHARE.1 > (kept + ran) * RERUN_SHARE.0 {
        pace.paused = PAUSE;
    }
    crate::diag::add("spec.kept", std::time::Duration::from_nanos(kept as u64 * 1000));
    crate::diag::add("spec.ran", std::time::Duration::from_nanos(ran as u64 * 1000));
    true
}

/// Window hint: freeing sixteen entities.
const GARBAGE_WINDOW: kiln_sched::Window = kiln_sched::Window::new().item_ns(10_000);

/// Window hint: an entity's turn, a few microseconds.
const SPEC_WINDOW: kiln_sched::Window = kiln_sched::Window::new().item_ns(4_000).chunk(2);
