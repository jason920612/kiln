//! Mob spawners (`BaseSpawner` of `SpawnerBlockEntity`): dungeon, mineshaft, stronghold and
//! fortress spawners, and the spawners players make with spawn eggs.
//!
//! The block entity's state ([`SpawnerBe`], with its `SpawnData` and `SpawnPotentials` as the
//! saved NBT holds them) belongs to whoever owns the level's block entities (the simulation
//! keeps them next to its other block entities; [`crate::memory::MemoryLevel`] keeps them for
//! tests); [`tick`] runs one `serverTick` against an abstract [`EntityLevel`].
//!
//! Vanilla draws everything from the level's random. Replaying vanilla's stream, the level
//! hands that stream out ([`EntityLevel::shared_ai_random`]); otherwise a stream seeded by the
//! spawner's position and the game time stands in (so the outcome does not depend on how the
//! world is split into regions).
//!
//! The `SpawnData` objects are shared by reference in vanilla: the spawner's next spawn data
//! is one of the entries of its potentials (or, loaded without potentials, the only one), so
//! what a spawn egg writes into the next one shows in the saved potentials. [`SpawnerBe`] keeps
//! the same sharing: entries by index.

use crate::collision::{self, CollisionContext};
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::ext::SpawnView;
use crate::mob::{self, Category, MobKind};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;

/// `SpawnData.CustomSpawnRules`: light limits (both inclusive ranges within 0..=15).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomRules {
    pub block_light: (i32, i32),
    pub sky_light: (i32, i32),
}

impl CustomRules {
    /// `CustomRules.CODEC`: both limits optional and lenient (a bad one is the full range).
    fn parse(t: &Tag) -> Option<CustomRules> {
        let Tag::Compound(_) = t else { return None };
        let limit = |name: &str| -> (i32, i32) {
            let Some(v) = t.get(name) else { return (0, 15) };
            let pair = match v {
                Tag::List(l) if l.len() == 2 => l[0].as_i64().zip(l[1].as_i64()),
                Tag::Compound(_) => v.get("min_inclusive").and_then(Tag::as_i64).zip(v.get("max_inclusive").and_then(Tag::as_i64)),
                _ => None,
            };
            match pair {
                Some((lo, hi)) if lo <= hi && lo >= 0 && hi <= 15 => (lo as i32, hi as i32),
                _ => (0, 15),
            }
        };
        Some(CustomRules { block_light: limit("block_light_limit"), sky_light: limit("sky_light_limit") })
    }

    /// The saved form: a limit is a two-element list, left out when it is the full range.
    fn to_tag(&self) -> Tag {
        let mut f = Vec::new();
        for (name, (lo, hi)) in [("block_light_limit", self.block_light), ("sky_light_limit", self.sky_light)] {
            if (lo, hi) != (0, 15) {
                f.push((name.to_owned(), Tag::List(vec![Tag::Int(lo), Tag::Int(hi)])));
            }
        }
        Tag::Compound(f)
    }

    /// `isValidPosition`: the block light and the sky light less the darkening are in range.
    pub fn valid(&self, block_light: i32, effective_sky: i32) -> bool {
        (self.block_light.0..=self.block_light.1).contains(&block_light) && (self.sky_light.0..=self.sky_light.1).contains(&effective_sky)
    }
}

/// `SpawnData`: what a spawner spawns (the entity's saved form: at least its `id`).
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnData {
    /// `entityToSpawn`.
    pub entity: Vec<(String, Tag)>,
    pub rules: Option<CustomRules>,
    /// `equipment` (an `EquipmentTable`), kept as saved.
    pub equipment: Option<Tag>,
}

impl SpawnData {
    /// `new SpawnData()`: an empty compound.
    pub fn empty() -> SpawnData {
        SpawnData { entity: Vec::new(), rules: None, equipment: None }
    }

    /// `SpawnData.CODEC`, with the constructor's id normalisation (a valid id is stored as
    /// `namespace:path`, an invalid one dropped).
    pub fn parse(t: &Tag) -> Option<SpawnData> {
        let Some(Tag::Compound(entity)) = t.get("entity") else { return None };
        let rules = match t.get("custom_spawn_rules") {
            None => None,
            Some(r) => Some(CustomRules::parse(r)?),
        };
        let mut s = SpawnData { entity: entity.clone(), rules, equipment: t.get("equipment").cloned() };
        s.normalize_id();
        Some(s)
    }

    fn normalize_id(&mut self) {
        let id = self.entity.iter().position(|(k, _)| k == "id");
        let parsed = id.and_then(|i| match &self.entity[i].1 {
            Tag::String(s) => normalize_identifier(s),
            _ => None,
        });
        match (id, parsed) {
            (Some(i), Some(p)) => self.entity[i].1 = Tag::String(p),
            (Some(i), None) => {
                self.entity.remove(i);
            }
            _ => {}
        }
    }

    /// The saved form (`SpawnData.CODEC`).
    pub fn to_tag(&self) -> Tag {
        let mut f = vec![("entity".to_owned(), Tag::Compound(self.entity.clone()))];
        if let Some(r) = &self.rules {
            f.push(("custom_spawn_rules".into(), r.to_tag()));
        }
        if let Some(e) = &self.equipment {
            f.push(("equipment".into(), e.clone()));
        }
        Tag::Compound(f)
    }

    /// The entity's type id (`EntityType.by`'s `id`).
    pub fn id(&self) -> Option<&str> {
        self.entity.iter().find(|(k, _)| k == "id").and_then(|(_, v)| v.as_str())
    }

    /// `getEntityToSpawn().putString("id", id)` (`BaseSpawner.setEntityId`).
    pub fn set_id(&mut self, id: &str) {
        match self.entity.iter_mut().find(|(k, _)| k == "id") {
            Some((_, v)) => *v = Tag::String(id.to_owned()),
            None => self.entity.push(("id".to_owned(), Tag::String(id.to_owned()))),
        }
    }

    /// The entity tag has nothing but a string `id` (`finalizeSpawn` runs only then).
    fn id_only(&self) -> bool {
        self.entity.len() == 1 && self.id().is_some()
    }
}

/// `Identifier.CODEC` on a string: `namespace:path` with vanilla's character sets, the
/// namespace `minecraft` by default.
fn normalize_identifier(s: &str) -> Option<String> {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ok_ns = !ns.is_empty() && ns.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'));
    let ok_path = !path.is_empty() && path.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/'));
    (ok_ns && ok_path).then(|| format!("{ns}:{path}"))
}

/// `SpawnerBlockEntity`'s `BaseSpawner`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnerBe {
    /// `spawnDelay`.
    pub delay: i32,
    /// Every `SpawnData` object the spawner holds (potentials and the next spawn data may be the
    /// same one).
    entries: Vec<SpawnData>,
    /// `spawnPotentials`: entry and weight.
    potentials: Vec<(usize, i32)>,
    /// `nextSpawnData`.
    next: Option<usize>,
    pub min_delay: i32,
    pub max_delay: i32,
    pub spawn_count: i32,
    pub max_nearby: i32,
    pub required_player_range: i32,
    pub spawn_range: i32,
    /// Saved fields the spawner does not model (`components`, ...).
    pub extra: Vec<(String, Tag)>,
}

impl Default for SpawnerBe {
    /// A new `SpawnerBlockEntity`.
    fn default() -> SpawnerBe {
        SpawnerBe {
            delay: 20,
            entries: Vec::new(),
            potentials: Vec::new(),
            next: None,
            min_delay: 200,
            max_delay: 800,
            spawn_count: 4,
            max_nearby: 6,
            required_player_range: 16,
            spawn_range: 4,
            extra: Vec::new(),
        }
    }
}

/// Any numeric tag as an int (`getIntOr` and `getShortOr` read numbers leniently).
fn number(t: Option<&Tag>) -> Option<i64> {
    match t? {
        Tag::Float(v) => Some(*v as i64),
        Tag::Double(v) => Some(*v as i64),
        other => other.as_i64(),
    }
}

impl SpawnerBe {
    /// `BaseSpawner.load` from the block entity's saved NBT.
    pub fn load(nbt: &Tag) -> SpawnerBe {
        let mut be = SpawnerBe::default();
        be.delay = number(nbt.get("Delay")).map_or(20, |v| v as i16 as i32);
        if let Some(d) = nbt.get("SpawnData").and_then(SpawnData::parse) {
            be.entries.push(d);
            be.next = Some(0);
        }
        // `SpawnPotentials`: a list of `{data, weight}`; one bad entry loses the list.
        let listed = nbt.get("SpawnPotentials").and_then(|l| {
            let list = l.as_list()?;
            let mut v = Vec::with_capacity(list.len());
            for item in list {
                let item = item.unwrap_list_element();
                let data = SpawnData::parse(item.get("data")?)?;
                let weight = number(item.get("weight"))?;
                if weight < 0 {
                    return None;
                }
                v.push((data, weight as i32));
            }
            Some(v)
        });
        match listed {
            Some(list) => {
                for (data, weight) in list {
                    be.entries.push(data);
                    be.potentials.push((be.entries.len() - 1, weight));
                }
            }
            None => {
                // `WeightedList.of(nextSpawnData ?? new SpawnData())`: the same object.
                let i = match be.next {
                    Some(i) => i,
                    None => {
                        be.entries.push(SpawnData::empty());
                        be.entries.len() - 1
                    }
                };
                be.potentials.push((i, 1));
            }
        }
        let get = |name: &str, default: i32| number(nbt.get(name)).map_or(default, |v| v as i32);
        be.min_delay = get("MinSpawnDelay", 200);
        be.max_delay = get("MaxSpawnDelay", 800);
        be.spawn_count = get("SpawnCount", 4);
        be.max_nearby = get("MaxNearbyEntities", 6);
        be.required_player_range = get("RequiredPlayerRange", 16);
        be.spawn_range = get("SpawnRange", 4);
        if let Tag::Compound(f) = nbt {
            const OWN: [&str; 11] = [
                "Delay", "MinSpawnDelay", "MaxSpawnDelay", "SpawnCount", "MaxNearbyEntities", "RequiredPlayerRange", "SpawnRange", "SpawnData", "SpawnPotentials", "x", "y",
            ];
            be.extra = f.iter().filter(|(k, _)| !OWN.contains(&k.as_str()) && k != "z" && k != "id").cloned().collect();
        }
        be
    }

    /// `BaseSpawner.save` (the fields of the block entity without its `id` and position).
    pub fn save(&self) -> Vec<(String, Tag)> {
        let mut f = self.extra.clone();
        let short = |v: i32| Tag::Short(v as i16);
        f.push(("Delay".into(), short(self.delay)));
        f.push(("MinSpawnDelay".into(), short(self.min_delay)));
        f.push(("MaxSpawnDelay".into(), short(self.max_delay)));
        f.push(("SpawnCount".into(), short(self.spawn_count)));
        f.push(("MaxNearbyEntities".into(), short(self.max_nearby)));
        f.push(("RequiredPlayerRange".into(), short(self.required_player_range)));
        f.push(("SpawnRange".into(), short(self.spawn_range)));
        if let Some(i) = self.next {
            f.push(("SpawnData".into(), self.entries[i].to_tag()));
        }
        let list = self
            .potentials
            .iter()
            .map(|&(i, w)| Tag::Compound(vec![("data".into(), self.entries[i].to_tag()), ("weight".into(), Tag::Int(w))]))
            .collect();
        f.push(("SpawnPotentials".into(), Tag::List(list)));
        f
    }

    /// The spawn data of the next spawn, if chosen.
    pub fn next_data(&self) -> Option<&SpawnData> {
        self.next.map(|i| &self.entries[i])
    }

    /// `WeightedList.getRandom` over the potentials: one draw when there are any.
    fn pick_potential(&self, r: &mut LegacyRandom) -> Option<usize> {
        let total: i64 = self.potentials.iter().map(|&(_, w)| w as i64).sum();
        if self.potentials.is_empty() || total <= 0 {
            return None;
        }
        let mut i = r.next_int_bounded(total as i32);
        for &(entry, w) in &self.potentials {
            i -= w;
            if i < 0 {
                return Some(entry);
            }
        }
        None
    }

    /// `getOrCreateNextSpawnData`.
    fn next_index(&mut self, r: &mut LegacyRandom) -> usize {
        if let Some(i) = self.next {
            return i;
        }
        let i = match self.pick_potential(r) {
            Some(i) => i,
            None => {
                self.entries.push(SpawnData::empty());
                self.entries.len() - 1
            }
        };
        self.next = Some(i);
        i
    }

    /// `BaseSpawner.setEntityId` (a spawn egg used on the spawner): the next spawn data's
    /// entity becomes `id`. Draws from `r` when no spawn data was chosen yet.
    pub fn set_entity_id(&mut self, id: &str, r: &mut LegacyRandom) {
        let i = self.next_index(r);
        self.entries[i].set_id(id);
    }

    /// `BaseSpawner.setEntityData` (an op using a spawn egg that carries entity data): the next
    /// spawn data's entity becomes the item's tag (`TypedEntityData.loadInto`: the item's
    /// fields replace the entity tag, keeping the id of the egg's type).
    pub fn set_entity_data(&mut self, id: &str, fields: &[(String, Tag)], r: &mut LegacyRandom) {
        let i = self.next_index(r);
        let mut entity: Vec<(String, Tag)> = fields.iter().filter(|(k, _)| k != "id").cloned().collect();
        entity.insert(0, ("id".into(), Tag::String(id.to_owned())));
        self.entries[i].entity = entity;
    }

    /// `delay`: the next delay and the next spawn data.
    fn reset_delay(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, r: &mut LegacyRandom) {
        self.delay = if self.max_delay <= self.min_delay { self.min_delay } else { self.min_delay + r.next_int_bounded(self.max_delay - self.min_delay) };
        if let Some(i) = self.pick_potential(r) {
            self.next = Some(i);
        }
        // `broadcastEvent(level, pos, 1)`.
        level.block_event(pos, 1, 0);
    }
}

/// A draw of the level's random: vanilla's own stream when replaying it, otherwise a stream
/// seeded by the spawner and the game time.
struct Draw {
    r: LegacyRandom,
    shared: bool,
}

fn draw(level: &mut dyn EntityLevel, pos: BlockPos) -> Draw {
    match level.shared_ai_random() {
        Some(r) => Draw { r: r.clone(), shared: true },
        None => Draw { r: level.pos_random(pos, 0x5350_4e52), shared: false },
    }
}

fn finish(level: &mut dyn EntityLevel, d: Draw) {
    if d.shared
        && let Some(r) = level.shared_ai_random()
    {
        *r = d.r;
    }
}

/// `Level.hasNearbyAlivePlayer` (a negative range is any distance).
fn near_player(level: &dyn EntityLevel, at: Vec3, range: f64) -> bool {
    level.players().iter().any(|p| !p.spectator && p.alive && (range < 0.0 || p.pos.distance_to_sqr(at) < range * range))
}

/// Whether a player is within `range` of the spawner at `pos` (`isNearPlayer`).
pub fn player_near(level: &dyn EntityLevel, pos: BlockPos, range: i32) -> bool {
    near_player(level, Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5), range as f64)
}

/// A random for a spawn egg used on the spawner at `pos` (standing in for the level random).
pub fn egg_random(seed: i64, game_time: i64, pos: BlockPos) -> LegacyRandom {
    let mut h = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ game_time as u64 ^ 0x4547_47;
    for v in [pos.x as u32 as u64, pos.y as u32 as u64, pos.z as u32 as u64] {
        h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    LegacyRandom::new(h as i64)
}

/// `SpawnerBlockEntity.serverTick` for the spawner at `pos`.
pub fn tick(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut SpawnerBe) {
    let center = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    if !near_player(level, center, be.required_player_range as f64) || !level.spawner_blocks_enabled() {
        return;
    }
    let mut d = draw(level, pos);
    server_tick(level, pos, be, &mut d.r);
    finish(level, d);
}

/// `BaseSpawner.serverTick` after the player and game rule checks.
fn server_tick(level: &mut dyn EntityLevel, pos: BlockPos, be: &mut SpawnerBe, r: &mut LegacyRandom) {
    if be.delay == -1 {
        be.reset_delay(level, pos, r);
    }
    if be.delay > 0 {
        be.delay -= 1;
        return;
    }
    let mut spawned = false;
    let next = be.next_index(r);
    for _ in 0..be.spawn_count {
        let data = be.entries[next].clone();
        // `EntityType.by`: the id must name a known type.
        let Some(type_name) = data.id().map(str::to_owned) else {
            be.reset_delay(level, pos, r);
            return;
        };
        let Some(et) = kiln_data::entities::by_name(&type_name) else {
            be.reset_delay(level, pos, r);
            return;
        };
        // `Pos` of the saved entity, else a random place around the spawner.
        let saved_pos = data.entity.iter().find(|(k, _)| k == "Pos").and_then(|(_, v)| match v.as_list()? {
            [x, y, z] => Some((x.as_f64()?, y.as_f64()?, z.as_f64()?)),
            _ => None,
        });
        let (x, y, z) = match saved_pos {
            Some(p) => p,
            None => {
                let range = be.spawn_range as f64;
                let x = pos.x as f64 + (r.next_double() - r.next_double()) * range + 0.5;
                let y = (pos.y + r.next_int_bounded(3) - 1) as f64;
                let z = pos.z as f64 + (r.next_double() - r.next_double()) * range + 0.5;
                (x, y, z)
            }
        };
        // `getSpawnAABB` and `noCollision`.
        let scale = spawn_dimensions_scale(et.name);
        let half = (scale * et.width / 2.0) as f64;
        let height = (scale * et.height) as f64;
        let spawn_box = Aabb::new(x - half, y, z - half, x + half, y + height, z + half);
        if !collision::no_collision(level, &CollisionContext::EMPTY, i32::MIN, &spawn_box) {
            continue;
        }
        let at = BlockPos::containing(x, y, z);
        let category = MobKind::by_name(et.name).map_or(Category::Misc, MobKind::category);
        if let Some(rules) = &data.rules {
            if !category.friendly() && level.difficulty() == 0 {
                continue;
            }
            let sky = (level.sky_light(at) - level.sky_darken()).max(0);
            if !rules.valid(level.block_light(at), sky) {
                continue;
            }
        } else if !check_spawn_rules(&*level, et.name, at, r) {
            continue;
        }
        // `EntityType.loadEntityRecursive`: the entity and its riders, from the saved form.
        let mut tag = data.entity.clone();
        tag.retain(|(k, _)| k != "Pos");
        tag.push(("Pos".into(), Tag::List(vec![Tag::Double(x), Tag::Double(y), Tag::Double(z)])));
        // (Vanilla numbers the entity as it creates it, even when it is then turned away.)
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let loaded = crate::persist::load_stack(&Tag::Compound(tag), id, &|u| if u != 0 { u as i64 ^ (u >> 64) as i64 } else { seed }, false);
        let Ok((mut e, mut riders)) = loaded else {
            be.reset_delay(level, pos, r);
            return;
        };
        for rd in &mut riders {
            rd.entity.id = level.next_entity_id();
        }
        // The count of its exact type in the box around the spawner (spectators excepted).
        let around = Aabb::new(pos.x as f64, pos.y as f64, pos.z as f64, pos.x as f64 + 1.0, pos.y as f64 + 1.0, pos.z as f64 + 1.0).inflate_all(be.spawn_range as f64);
        let mut count = level
            .entities_in(&around, EntityFilter::Any, i32::MIN)
            .into_iter()
            .filter(|&id| level.entity(id).is_some_and(|o| o.type_name == e.type_name && !level.player(id).is_some_and(|p| p.spectator)))
            .count() as i32;
        count += level.pending_spawns(&around).iter().filter(|(t, _, _)| *t == e.type_name).count() as i32;
        if count >= be.max_nearby {
            be.reset_delay(level, pos, r);
            return;
        }
        // `snapTo(x, y, z, random * 360, 0)`.
        let yaw = r.next_float() * 360.0;
        e.set_pos(Vec3::new(x, y, z));
        e.y_rot = yaw;
        e.x_rot = 0.0;
        e.set_old_pos_and_rot();
        let mut companions = Vec::new();
        let mut nearby_chicken = false;
        if matches!(e.kind, EntityKind::Mob(_)) {
            // `Mob.checkSpawnRules` (without custom rules): a `PathfinderMob` wants a walk target value of
            // at least 0 where it stands (monsters: light 12 at most); `checkSpawnObstruction`: no liquid in
            // its box and nobody in the way.
            if data.rules.is_none() && !walk_target_ok(&*level, &e, at) {
                continue;
            }
            if !spawn_obstruction_ok(level, &e) {
                continue;
            }
            if data.id_only() {
                let ctx = spawn_context(level, &e);
                let mut group = mob::GroupData { monsters_disabled: !level.spawning_monsters(), ..Default::default() };
                group.camel_space = false;
                mob::finalize_spawn(&mut e, r, &ctx, &mut group, false);
                companions = std::mem::take(&mut group.companions);
                for c in &mut companions {
                    c.entity.id = level.next_entity_id();
                }
                nearby_chicken = group.nearby_chicken;
            }
        }
        let riders: Vec<crate::mob::Companion> = riders
            .into_iter()
            .map(|rd| crate::mob::Companion {
                entity: rd.entity,
                seat: if rd.vehicle == 0 { mob::Seat::OnMob } else { mob::Seat::OnCompanion(rd.vehicle - 1) },
            })
            .chain(companions)
            .collect();
        // `Mob.spawnAnim`: the poof, shown to the viewers once it is in the level.
        if let Some(m) = mob::data_mut(&mut e) {
            m.spawn_anim = true;
        }
        // (A cube mob's move control remembers the yaw it was made with, which is random in vanilla: here the one it
        // ends up facing.)
        mob::kinds::slime::pin_move_yaw_of(&mut e);
        if !level.add_entity_stack(e, riders, !data.id_only(), nearby_chicken) {
            be.reset_delay(level, pos, r);
            return;
        }
        // `levelEvent(2004, pos)` (flames), the `entity_place` game event and `spawnAnim`.
        level.emit(Event::LevelEvent { event: 2004, pos, data: 0 });
        level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: Vec3::new(at.x as f64 + 0.5, at.y as f64 + 0.5, at.z as f64 + 0.5), entity: None });
        spawned = true;
    }
    if spawned {
        be.reset_delay(level, pos, r);
    }
}

/// `EntityType.spawnDimensionsScale` (slimes and magma cubes spawn in a box four times their
/// size).
pub fn spawn_dimensions_scale(type_name: &str) -> f32 {
    match type_name {
        "minecraft:slime" | "minecraft:magma_cube" => 4.0,
        "minecraft:sulfur_cube" => 2.0,
        _ => 1.0,
    }
}

/// `DifficultyInstance` for a new mob.
fn spawn_context(level: &dyn EntityLevel, e: &Entity) -> mob::SpawnContext {
    let at = e.block_position();
    let effective = level.effective_difficulty(at);
    let special = if effective < 2.0 {
        0.0
    } else if effective > 4.0 {
        1.0
    } else {
        (effective - 2.0) / 2.0
    };
    mob::SpawnContext { biome: level.biome(at), moon_brightness: level.moon_brightness(), special_multiplier: special, effective_difficulty: effective, hard: level.difficulty() == 3, halloween: false }
}

/// `PathfinderMob.checkSpawnRules`: the walk target value at the mob's block is not negative (types that are
/// no `PathfinderMob`, or have no preference, pass).
fn walk_target_ok(level: &dyn EntityLevel, e: &Entity, at: BlockPos) -> bool {
    let Some(m) = mob::data(e) else { return true };
    m.kind.ext().is_some_and(|k| k.spawn_ignores_light()) || mob::walk_target_value(m, level, at) >= 0.0
}

/// `Mob.checkSpawnObstruction`: no liquid in the box, and nothing that blocks building in it.
fn spawn_obstruction_ok(level: &dyn EntityLevel, e: &Entity) -> bool {
    let kind = mob::data(e).map(|m| m.kind);
    let in_liquid_ok = kind.and_then(MobKind::ext).is_some_and(|k| k.spawn_in_liquids());
    let bb = e.bounding_box();
    if !in_liquid_ok {
        let (x0, x1) = (crate::math::floor(bb.min_x), crate::math::ceil(bb.max_x));
        let (y0, y1) = (crate::math::floor(bb.min_y), crate::math::ceil(bb.max_y));
        let (z0, z1) = (crate::math::floor(bb.min_z), crate::math::ceil(bb.max_z));
        if (x0..x1).any(|x| (y0..y1).any(|y| (z0..z1).any(|z| kiln_data::blocks_types::has_fluid(level.block(BlockPos::new(x, y, z)))))) {
            return false;
        }
    }
    if !crate::mob::kinds::creaking_heart::is_unobstructed(level, &bb) {
        return false;
    }
    !level.pending_spawns(&bb).iter().any(|(_, b, living)| *living && b.intersects(&bb))
}

/// What natural spawning's rules let a type see, from an abstract level (spawner reason).
struct LevelView<'a>(&'a dyn EntityLevel);

impl SpawnView for LevelView<'_> {
    fn block(&self, pos: BlockPos) -> u16 {
        self.0.block(pos)
    }
    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32 {
        self.0.raw_brightness(pos, sky_darken)
    }
    fn sky_darken(&self) -> i32 {
        self.0.sky_darken()
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        self.0.sky_light(pos)
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        self.0.block_light(pos)
    }
    fn biome(&self, pos: BlockPos) -> i32 {
        self.0.biome(pos).unwrap_or(0)
    }
    fn difficulty(&self) -> u8 {
        self.0.difficulty()
    }
    fn world_seed(&self) -> i64 {
        self.0.world_seed()
    }
    fn moon_brightness(&self) -> f32 {
        self.0.moon_brightness()
    }
    fn min_y(&self) -> i32 {
        self.0.min_y()
    }
    fn sea_level(&self) -> i32 {
        self.0.sea_level()
    }
    fn spawner(&self) -> bool {
        true
    }
    fn monster_block_light_limit(&self) -> i32 {
        self.0.monster_light_rules().0
    }
    fn monster_light_test(&self) -> (i32, i32) {
        let (_, lo, hi) = self.0.monster_light_rules();
        (lo, hi)
    }
    fn thundering(&self) -> bool {
        self.0.is_thundering()
    }
}

/// `EntityType.isAllowedInPeaceful` (false for `notInPeaceful()` types).
pub fn allowed_in_peaceful(type_name: &str) -> bool {
    !matches!(
        type_name.strip_prefix("minecraft:").unwrap_or(type_name),
        "blaze"
            | "bogged"
            | "breeze"
            | "cave_spider"
            | "creaking"
            | "creeper"
            | "drowned"
            | "elder_guardian"
            | "enderman"
            | "endermite"
            | "evoker"
            | "ghast"
            | "giant"
            | "guardian"
            | "hoglin"
            | "husk"
            | "illusioner"
            | "magma_cube"
            | "parched"
            | "phantom"
            | "piglin_brute"
            | "pillager"
            | "ravager"
            | "silverfish"
            | "skeleton"
            | "slime"
            | "spider"
            | "stray"
            | "vex"
            | "vindicator"
            | "warden"
            | "witch"
            | "wither"
            | "wither_skeleton"
            | "zoglin"
            | "zombie"
            | "zombie_villager"
            | "zombified_piglin"
    )
}

/// `SpawnPlacements.checkSpawnRules(type, level, EntitySpawnReason.SPAWNER, pos, random)`: not
/// a monster in a peaceful world, then the type's registered rule (types without one pass).
pub fn check_spawn_rules(level: &dyn EntityLevel, type_name: &str, pos: BlockPos, r: &mut LegacyRandom) -> bool {
    if !allowed_in_peaceful(type_name) && level.difficulty() == 0 {
        return false;
    }
    let short = type_name.strip_prefix("minecraft:").unwrap_or(type_name);
    let view = LevelView(level);
    match short {
        // `Mob.checkMobSpawnRules` (a spawner needs no valid block below).
        "ender_dragon" | "iron_golem" | "phantom" | "shulker" | "snow_golem" | "villager" | "wandering_trader" => true,
        // `Monster.checkAnyLightMonsterSpawnRules`.
        "blaze" | "breeze" | "zoglin" => true,
        // `Monster.checkMonsterSpawnRules`: dark enough.
        "bogged" | "cave_spider" | "creeper" | "enderman" | "giant" | "skeleton" | "spider" | "witch" | "wither" | "wither_skeleton" | "creaking" | "zombie" | "zombie_horse" | "zombie_villager"
        | "evoker" | "illusioner" | "ravager" | "vex" | "vindicator" | "warden" => crate::mob::kinds::zombie::dark_enough_view(&view, pos, r),
        // `Monster.checkSurfaceMonstersSpawnRules`: dark enough (the sky is not looked at).
        "camel_husk" | "husk" | "parched" => crate::mob::kinds::zombie::dark_enough_view(&view, pos, r),
        "silverfish" => true,
        _ => match MobKind::by_name(type_name).and_then(MobKind::ext) {
            Some(k) => k.check_spawn_rules(&view, pos, r).unwrap_or(true),
            None => match MobKind::by_name(type_name) {
                // The animals' rule: a block animals spawn on, and light.
                Some(kind) if kind.is_animal() => {
                    crate::blocks::has_tag(level.block(pos.below()), crate::blocks::Tag::AnimalsSpawnableOn) && level.raw_brightness(pos, 0) > 8
                }
                _ => true,
            },
        },
    }
}
