//! Raids (`Raids`, `Raid`) and pillager patrols (`PatrolSpawner`), per level, in the serial
//! global phase (vanilla ticks raids before the entities; the patrol spawner is one of the
//! overworld's custom spawners).
//!
//! A raid keeps its own bookkeeping (status, waves, omen level, total health, heroes, the boss
//! bar); its raiders are the level's mobs whose raider state names it (`Raider.raid`), found by
//! scanning the level's entities while it runs (raids are few). Raiders tell their raid what
//! happened through `RaidEvent`s the regions collect; the raids' state reaches the regions as
//! `RaidView`s in the block environment.
//!
//! Approximations: a raid's random is seeded from the world seed and its id (vanilla's is
//! unseeded); spawned wave riders stand next to their ravager and mount it once both exist;
//! raiders' spawn equipment enchantments are rolled but not applied.

use crate::{DIMENSIONS, Dim, DimId, Sim, entities, player_stats, poi};
use kiln_entity::level::{RaidEvent, RaidView};
use kiln_entity::mob::kinds::raider;
use kiln_entity::mob::{self, MobKind};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::hud::{self, BossBarColor, BossBarOverlay, BossEvent};
use kiln_world::poi::Occupancy;
use kiln_world::{Blocks, ChunkPos};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// `Raid.RaidStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Ongoing,
    Victory,
    Loss,
    Stopped,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Ongoing => "ongoing",
            Status::Victory => "victory",
            Status::Loss => "loss",
            Status::Stopped => "stopped",
        }
    }

    fn by_name(s: &str) -> Status {
        match s {
            "victory" => Status::Victory,
            "loss" => Status::Loss,
            "stopped" => Status::Stopped,
            _ => Status::Ongoing,
        }
    }
}

/// `Raid.RaiderType`: the type and its spawns per wave before bonuses.
const RAIDER_TYPES: [(MobKind, [i32; 8]); 5] = [
    (MobKind::Vindicator, [0, 0, 2, 0, 1, 4, 2, 5]),
    (MobKind::Evoker, [0, 0, 0, 0, 0, 1, 1, 2]),
    (MobKind::Pillager, [0, 4, 3, 3, 4, 4, 4, 2]),
    (MobKind::Witch, [0, 0, 0, 0, 3, 0, 0, 1]),
    (MobKind::Ravager, [0, 0, 0, 1, 0, 1, 0, 2]),
];

/// `Raid.getNumGroups`.
pub(crate) fn num_groups(difficulty: u8) -> i32 {
    match difficulty {
        0 => 0,
        1 => 3,
        2 => 5,
        _ => 7,
    }
}

/// The raid's boss bar (`ServerBossEvent`, red, ten notches).
#[derive(Clone, Debug)]
pub(crate) struct BossBar {
    pub uuid: Uuid,
    pub name: Tag,
    pub progress: f32,
    pub visible: bool,
    /// Players seeing it.
    pub players: Vec<kiln_link::ConnId>,
}

pub(crate) struct Raid {
    pub started: bool,
    pub active: bool,
    pub ticks_active: i64,
    pub omen_level: i32,
    pub groups_spawned: i32,
    pub cooldown_ticks: i32,
    pub post_raid_ticks: i32,
    pub total_health: f32,
    pub num_groups: i32,
    pub status: Status,
    pub center: [i32; 3],
    pub heroes: BTreeSet<u128>,
    /// `groupToLeaderMap`: wave → entity id.
    pub leaders: BTreeMap<i32, i32>,
    pub celebration_ticks: i32,
    pub wave_spawn_pos: Option<[i32; 3]>,
    pub random: LegacyRandom,
    pub bar: BossBar,
}

fn raid_name() -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String("event.minecraft.raid".into()))])
}

fn text(key: &str) -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String(key.into()))])
}

impl Raid {
    /// `new Raid(center, difficulty)`.
    pub(crate) fn new(center: [i32; 3], difficulty: u8, seed: i64) -> Raid {
        let mut random = LegacyRandom::new(seed);
        let uuid = Uuid::from_u64_pair(random.next_long() as u64, random.next_long() as u64);
        Raid {
            started: false,
            active: true,
            ticks_active: 0,
            omen_level: 0,
            groups_spawned: 0,
            cooldown_ticks: 300,
            post_raid_ticks: 0,
            total_health: 0.0,
            num_groups: num_groups(difficulty),
            status: Status::Ongoing,
            center,
            heroes: BTreeSet::new(),
            leaders: BTreeMap::new(),
            celebration_ticks: 0,
            wave_spawn_pos: None,
            random,
            bar: BossBar { uuid, name: raid_name(), progress: 0.0, visible: true, players: Vec::new() },
        }
    }

    pub(crate) fn is_over(&self) -> bool {
        matches!(self.status, Status::Victory | Status::Loss)
    }

    fn has_bonus_wave(&self) -> bool {
        self.omen_level > 1
    }

    fn is_final_wave(&self) -> bool {
        self.groups_spawned == self.num_groups
    }

    fn has_spawned_bonus_wave(&self) -> bool {
        self.groups_spawned > self.num_groups
    }

    /// `hasMoreWaves`.
    fn has_more_waves(&self) -> bool {
        if self.has_bonus_wave() { !self.has_spawned_bonus_wave() } else { !self.is_final_wave() }
    }

    /// `shouldSpawnBonusGroup`.
    fn should_spawn_bonus_group(&self, alive: usize) -> bool {
        self.is_final_wave() && alive == 0 && self.has_bonus_wave()
    }

    /// `shouldSpawnGroup`.
    fn should_spawn_group(&self, alive: usize) -> bool {
        self.cooldown_ticks == 0 && (self.groups_spawned < self.num_groups || self.should_spawn_bonus_group(alive)) && alive == 0
    }

    /// `getEnchantOdds`.
    pub(crate) fn enchant_odds(&self) -> f32 {
        match self.omen_level {
            2 => 0.1,
            3 => 0.25,
            4 => 0.5,
            5 => 0.75,
            _ => 0.0,
        }
    }

    pub(crate) fn view(&self, id: i32) -> RaidView {
        RaidView {
            id,
            center: kiln_entity::math::BlockPos::new(self.center[0], self.center[1], self.center[2]),
            active: self.active,
            over: self.is_over(),
            loss: self.status == Status::Loss,
            started: self.started,
            groups_spawned: self.groups_spawned,
            omen_level: self.omen_level,
            leaders: self.leaders.iter().map(|(w, id)| (*w, *id)).collect(),
        }
    }

    fn stop(&mut self) {
        self.active = false;
        self.status = Status::Stopped;
    }

    /// `Raid.MAP_CODEC` with the id (`RaidWithId`).
    fn to_nbt(&self, id: i32) -> Tag {
        let heroes = self.heroes.iter().map(|u| kiln_entity::persist::uuid_to_tag(*u)).collect();
        Tag::Compound(vec![
            ("id".into(), Tag::Int(id)),
            ("started".into(), Tag::Byte(self.started as i8)),
            ("active".into(), Tag::Byte(self.active as i8)),
            ("ticks_active".into(), Tag::Long(self.ticks_active)),
            ("raid_omen_level".into(), Tag::Int(self.omen_level)),
            ("groups_spawned".into(), Tag::Int(self.groups_spawned)),
            ("cooldown_ticks".into(), Tag::Int(self.cooldown_ticks)),
            ("post_raid_ticks".into(), Tag::Int(self.post_raid_ticks)),
            ("total_health".into(), Tag::Float(self.total_health)),
            ("group_count".into(), Tag::Int(self.num_groups)),
            ("status".into(), Tag::String(self.status.name().into())),
            ("center".into(), Tag::IntArray(self.center.to_vec())),
            ("heroes_of_the_village".into(), Tag::List(heroes)),
        ])
    }

    fn from_nbt(t: &Tag, seed: i64) -> Option<(i32, Raid)> {
        let int = |k: &str| t.get(k).and_then(Tag::as_i64);
        let id = int("id")? as i32;
        let center = match t.get("center") {
            Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
            _ => return None,
        };
        let mut r = Raid::new(center, 2, seed ^ id as i64);
        r.started = int("started").unwrap_or(0) != 0;
        r.active = int("active").unwrap_or(0) != 0;
        r.ticks_active = int("ticks_active").unwrap_or(0);
        r.omen_level = int("raid_omen_level").unwrap_or(0) as i32;
        r.groups_spawned = int("groups_spawned").unwrap_or(0) as i32;
        r.cooldown_ticks = int("cooldown_ticks").unwrap_or(0) as i32;
        r.post_raid_ticks = int("post_raid_ticks").unwrap_or(0) as i32;
        r.total_health = t.get("total_health").and_then(Tag::as_f64).unwrap_or(0.0) as f32;
        r.num_groups = int("group_count").unwrap_or(0) as i32;
        r.status = Status::by_name(t.get("status").and_then(Tag::as_str).unwrap_or("ongoing"));
        if let Some(list) = t.get("heroes_of_the_village").and_then(Tag::as_list) {
            r.heroes = list.iter().filter_map(|h| kiln_entity::persist::uuid_from_tag(h.unwrap_list_element())).collect();
        }
        Some((id, r))
    }
}

/// `Raids`: a level's raids by id.
#[derive(Default)]
pub(crate) struct Raids {
    pub raids: BTreeMap<i32, Raid>,
    pub next_id: i32,
    pub tick: i32,
    /// The raids as the regions see them this tick.
    pub views: std::sync::Arc<Vec<RaidView>>,
    /// Raider news from the regions, for the next raid tick.
    pub events: Vec<RaidEvent>,
    /// `PatrolSpawner.nextTick`.
    pub patrol_next_tick: i32,
}

impl Raids {
    pub(crate) fn new() -> Raids {
        Raids { next_id: 1, ..Default::default() }
    }

    /// `raids.dat`'s `data`.
    pub(crate) fn to_nbt(&self) -> Tag {
        let list = self.raids.iter().map(|(id, r)| r.to_nbt(*id)).collect();
        Tag::Compound(vec![("raids".into(), Tag::List(list)), ("next_id".into(), Tag::Int(self.next_id)), ("tick".into(), Tag::Int(self.tick))])
    }

    pub(crate) fn from_nbt(t: &Tag, seed: i64) -> Raids {
        let mut out = Raids::new();
        out.next_id = t.get("next_id").and_then(Tag::as_i64).unwrap_or(1) as i32;
        out.tick = t.get("tick").and_then(Tag::as_i64).unwrap_or(0) as i32;
        if let Some(list) = t.get("raids").and_then(Tag::as_list) {
            for r in list {
                if let Some((id, raid)) = Raid::from_nbt(r.unwrap_list_element(), seed) {
                    out.raids.insert(id, raid);
                }
            }
        }
        out.refresh_views();
        out
    }

    /// `getNearbyRaid(pos, 9216)`: the nearest active raid whose center is closer than 96.
    pub(crate) fn raid_at(&self, pos: [i32; 3]) -> Option<i32> {
        let mut best: Option<(f64, i32)> = None;
        for (id, r) in &self.raids {
            let d = dist_sqr(r.center, pos);
            if r.active && d < best.map_or(9216.0, |b| b.0) {
                best = Some((d, *id));
            }
        }
        best.map(|b| b.1)
    }

    pub(crate) fn refresh_views(&mut self) {
        self.views = std::sync::Arc::new(self.raids.iter().map(|(id, r)| r.view(*id)).collect());
    }
}

/// `Vec3i.distSqr`.
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> f64 {
    let d = |i: usize| (a[i] - b[i]) as f64;
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

/// `ServerLevel.getRaidAt` over a list of views (as the regions see them).
pub(crate) fn raid_at_view(views: &[RaidView], pos: kiln_entity::math::BlockPos) -> Option<&RaidView> {
    let p = [pos.x, pos.y, pos.z];
    let mut best: Option<(f64, &RaidView)> = None;
    for v in views {
        let d = dist_sqr([v.center.x, v.center.y, v.center.z], p);
        if v.active && d < best.map_or(9216.0, |b| b.0) {
            best = Some((d, v));
        }
    }
    best.map(|b| b.1)
}

/// A raider of the level as a raid sees it.
#[derive(Clone, Debug)]
struct RaiderInfo {
    id: i32,
    wave: i32,
    health: f32,
    pos: [f64; 3],
    tick_count: i32,
    no_action_time: i32,
    leader: bool,
    ticks_outside: i32,
}

/// The live raiders of raid `raid` in level `d`.
fn raiders_of(d: &Dim, raid: i32) -> Vec<RaiderInfo> {
    let mut out = Vec::new();
    for r in d.regions.iter() {
        for e in &r.part().0.list {
            if e.removed {
                continue;
            }
            let Some(p) = e.phys.as_ref() else { continue };
            let Some(m) = mob::data(p) else { continue };
            let Some(st) = raider::raider(m) else { continue };
            if st.raid != Some(raid) || m.is_dead_or_dying() || p.is_removed() {
                continue;
            }
            out.push(RaiderInfo {
                id: e.id,
                wave: st.wave,
                health: m.health,
                pos: e.pos,
                tick_count: p.tick_count,
                no_action_time: m.no_action_time,
                leader: st.patrol_leader,
                ticks_outside: st.ticks_outside_raid,
            });
        }
    }
    out.sort_by_key(|r| r.id);
    out
}

/// `spawnGroup`'s count of each raider type (`RAIDER_TYPES` order) for wave `wave`: the
/// defaults (the last wave's for a bonus group) and `getPotentialBonusSpawns`, drawing from
/// the raid's random as vanilla does.
pub(crate) fn wave_counts(random: &mut LegacyRandom, difficulty: u8, num_groups: i32, wave: i32, bonus_group: bool) -> [i32; 5] {
    let mut out = [0; 5];
    let (easy, normal) = (difficulty == 1, difficulty == 2);
    for (i, (kind, per_wave)) in RAIDER_TYPES.iter().enumerate() {
        let default = per_wave[(if bonus_group { num_groups } else { wave }).clamp(0, 7) as usize];
        let n = match kind {
            MobKind::Vindicator | MobKind::Pillager => {
                if easy {
                    random.next_int_bounded(2)
                } else if normal {
                    1
                } else {
                    2
                }
            }
            MobKind::Witch => (!(easy || wave <= 2 || wave == 4)) as i32,
            MobKind::Ravager => (!easy && bonus_group) as i32,
            _ => 0,
        };
        out[i] = default + if n > 0 { random.next_int_bounded(n + 1) } else { 0 };
    }
    out
}

/// A raider's own seed (vanilla seeds each new entity from a global uniquifier), from the world
/// seed, the raid, the wave and its place in it.
fn raider_seed(world_seed: i64, raid: i32, wave: i32, index: i64) -> i64 {
    let mut h = (world_seed as u64) ^ 0x7261_6964_6572;
    for v in [raid as u64, wave as u64, index as u64] {
        h = (h ^ v).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    h as i64
}

/// Each raid of a level for tests: (id, status, waves spawned, omen level, center, raiders
/// alive, boss bar progress).
pub(crate) fn summary(d: &Dim) -> Vec<(i32, &'static str, i32, i32, [i32; 3], usize, f32)> {
    d.raids.raids.iter().map(|(id, r)| (*id, r.status.name(), r.groups_spawned, r.omen_level, r.center, raiders_of(d, *id).len(), r.bar.progress)).collect()
}

/// Runs `f` on the mob `id` of level `d`.
fn with_mob(d: &mut Dim, id: i32, f: impl FnOnce(&mut kiln_entity::Entity, &mut mob::MobData)) {
    for r in d.regions.iter_mut() {
        let list = &mut r.part_mut().0.list;
        if let Ok(i) = list.binary_search_by_key(&id, |e| e.id)
            && let Some(p) = list[i].phys.as_mut()
        {
            let mut kind = std::mem::replace(&mut p.kind, kiln_entity::EntityKind::MobTicking { gravity: 0.08 });
            if let kiln_entity::EntityKind::Mob(m) = &mut kind {
                f(p, m);
            }
            p.kind = kind;
            return;
        }
    }
}

/// Whether the chunk holding `pos` is loaded in a region of `d`.
fn has_chunk_at(d: &Dim, x: i32, z: i32) -> bool {
    d.regions.chunk(ChunkPos::of_block(x, z)).is_some()
}

impl Sim {
    /// The raids of every level, then the overworld's patrol spawner.
    pub(crate) fn tick_raids(&mut self) {
        let raids_rule = self.rule_bool("minecraft:raids");
        for dim in 0..self.dims.len() {
            // Raider news from the regions.
            let mut events = std::mem::take(&mut self.dims[dim].raids.events);
            for r in self.dims[dim].regions.iter_mut() {
                events.append(&mut r.part_mut().1.raid_events);
            }
            for ev in events {
                self.raid_event(dim, ev);
            }
            self.raid_omen_triggers(dim);
            let d = &mut self.dims[dim];
            d.raids.tick += 1;
            let ids: Vec<i32> = d.raids.raids.keys().copied().collect();
            for id in ids {
                let Some(raid) = self.dims[dim].raids.raids.get_mut(&id) else { continue };
                if !raids_rule {
                    raid.stop();
                }
                if raid.status == Status::Stopped {
                    let raid = self.dims[dim].raids.raids.remove(&id).expect("raid");
                    self.hide_bar(&raid.bar);
                    continue;
                }
                self.tick_raid(dim, id);
            }
            self.dims[dim].raids.refresh_views();
        }
        self.tick_patrols();
    }

    fn raid_event(&mut self, dim: DimId, ev: RaidEvent) {
        let d = &mut self.dims[dim];
        match ev {
            RaidEvent::Joined { raid, entity, wave } => {
                let mut health = 0.0;
                with_mob(d, entity, |_, m| health = m.health);
                if let Some(r) = d.raids.raids.get_mut(&raid) {
                    // `addWaveMob`: the total grows by the joiner's health.
                    r.total_health += health;
                    let _ = wave;
                }
            }
            RaidEvent::Died { raid, entity, wave, leader, hero } => {
                let hero_uuid = hero.and_then(|h| self.players.values().find(|p| p.entity_id == h)).map(|p| p.uuid.as_u128());
                let d = &mut self.dims[dim];
                if let Some(r) = d.raids.raids.get_mut(&raid) {
                    if leader && r.leaders.get(&wave) == Some(&entity) {
                        r.leaders.remove(&wave);
                    }
                    if let Some(u) = hero_uuid {
                        r.heroes.insert(u);
                    }
                }
            }
            RaidEvent::Leader { raid, wave, entity } => {
                if let Some(r) = d.raids.raids.get_mut(&raid) {
                    r.leaders.insert(wave, entity);
                }
            }
        }
    }

    /// `Raid.tick`.
    fn tick_raid(&mut self, dim: DimId, id: i32) {
        let difficulty = self.commands.difficulty as u8;
        let center = self.dims[dim].raids.raids[&id].center;
        let status = self.dims[dim].raids.raids[&id].status;
        if status == Status::Ongoing {
            let active = has_chunk_at(&self.dims[dim], center[0], center[2]);
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            let was = raid.active;
            raid.active = active;
            if difficulty == 0 {
                raid.stop();
                return;
            }
            if was != active {
                raid.bar.visible = active;
                if !active {
                    let bar = raid.bar.clone();
                    self.hide_bar(&bar);
                    self.dims[dim].raids.raids.get_mut(&id).expect("raid").bar.players.clear();
                }
            }
            if !active {
                return;
            }
            if !self.is_village(dim, center) {
                self.move_raid_center_to_village(dim, id);
            }
            let center = self.dims[dim].raids.raids[&id].center;
            if !self.is_village(dim, center) {
                let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
                if raid.groups_spawned > 0 {
                    raid.status = Status::Loss;
                } else {
                    raid.stop();
                }
            }
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            raid.ticks_active += 1;
            if raid.ticks_active >= 48000 {
                raid.stop();
                return;
            }
            let alive = raiders_of(&self.dims[dim], id).len();
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            if alive == 0 && raid.has_more_waves() {
                if raid.cooldown_ticks <= 0 {
                    if raid.cooldown_ticks == 0 && raid.groups_spawned > 0 {
                        raid.cooldown_ticks = 300;
                        raid.bar.name = raid_name();
                        let bar = raid.bar.clone();
                        self.bar_name(&bar);
                        return;
                    }
                } else {
                    let cached = raid.wave_spawn_pos.is_some();
                    let mut try_find = !cached && raid.cooldown_ticks % 5 == 0;
                    if let Some(p) = raid.wave_spawn_pos
                        && !has_chunk_at(&self.dims[dim], p[0], p[2])
                    {
                        try_find = true;
                    }
                    if try_find {
                        let p = self.find_random_spawn_pos(dim, id, 8);
                        self.dims[dim].raids.raids.get_mut(&id).expect("raid").wave_spawn_pos = p;
                    }
                    let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
                    if raid.cooldown_ticks == 300 || raid.cooldown_ticks % 20 == 0 {
                        self.update_raid_players(dim, id);
                    }
                    let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
                    raid.cooldown_ticks -= 1;
                    let progress = ((300 - raid.cooldown_ticks) as f32 / 300.0).clamp(0.0, 1.0);
                    self.set_bar_progress(dim, id, progress);
                }
            }
            let raid = &self.dims[dim].raids.raids[&id];
            if raid.ticks_active % 20 == 0 {
                self.update_raid_players(dim, id);
                self.update_raiders(dim, id);
                let name = if alive > 0 && alive <= 2 {
                    Tag::Compound(vec![
                        ("translate".into(), Tag::String("event.minecraft.raid".into())),
                        (
                            "extra".into(),
                            Tag::List(vec![
                                Tag::String(" - ".into()),
                                Tag::Compound(vec![
                                    ("translate".into(), Tag::String("event.minecraft.raid.raiders_remaining".into())),
                                    ("with".into(), Tag::List(vec![Tag::Int(alive as i32)])),
                                ]),
                            ]),
                        ),
                    ])
                } else {
                    raid_name()
                };
                self.set_bar_name(dim, id, name);
            }
            // `shouldSpawnGroup`: waves spawn while none of the raid is alive.
            let mut sound = false;
            let mut attempts = 0;
            let mut pending = 0;
            loop {
                let raid = &self.dims[dim].raids.raids[&id];
                if !raid.should_spawn_group(alive + pending) {
                    break;
                }
                let pos = match raid.wave_spawn_pos {
                    Some(p) => Some(p),
                    None => self.find_random_spawn_pos(dim, id, 20),
                };
                match pos {
                    Some(p) => {
                        self.dims[dim].raids.raids.get_mut(&id).expect("raid").started = true;
                        pending += self.spawn_group(dim, id, p, alive + pending);
                        if !sound {
                            self.play_raid_horn(dim, id, p);
                            sound = true;
                        }
                    }
                    None => attempts += 1,
                }
                if attempts > 5 {
                    self.dims[dim].raids.raids.get_mut(&id).expect("raid").stop();
                    break;
                }
            }
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            if raid.started && !raid.has_more_waves() && alive == 0 && pending == 0 {
                if raid.post_raid_ticks < 40 {
                    raid.post_raid_ticks += 1;
                } else {
                    raid.status = Status::Victory;
                    self.reward_heroes(dim, id);
                }
            }
        } else if self.dims[dim].raids.raids[&id].is_over() {
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            raid.celebration_ticks += 1;
            if raid.celebration_ticks >= 600 {
                raid.stop();
                return;
            }
            if raid.celebration_ticks % 20 == 0 {
                let victory = raid.status == Status::Victory;
                self.update_raid_players(dim, id);
                self.dims[dim].raids.raids.get_mut(&id).expect("raid").bar.visible = true;
                if victory {
                    self.set_bar_progress(dim, id, 0.0);
                    self.set_bar_name(dim, id, text("event.minecraft.raid.victory.full"));
                } else {
                    self.set_bar_name(dim, id, text("event.minecraft.raid.defeat.full"));
                }
            }
        }
    }

    /// `ServerLevel.isVillage` in a serial phase.
    pub(crate) fn is_village(&self, dim: DimId, pos: [i32; 3]) -> bool {
        poi::sections_to_village(&self.dims[dim].regions, pos) <= 1
    }

    /// `moveRaidCenterToNearbyVillageSection`: the center of the nearest village section
    /// within two sections.
    fn move_raid_center_to_village(&mut self, dim: DimId, id: i32) {
        let c = self.dims[dim].raids.raids[&id].center;
        let (sx, sy, sz) = (c[0] >> 4, c[1] >> 4, c[2] >> 4);
        let mut best: Option<(f64, [i32; 3])> = None;
        // `SectionPos.cube`: x outermost, then y, then z.
        for x in sx - 2..=sx + 2 {
            for y in sy - 2..=sy + 2 {
                for z in sz - 2..=sz + 2 {
                    let center = [(x << 4) + 8, (y << 4) + 8, (z << 4) + 8];
                    if !self.is_village(dim, center) {
                        continue;
                    }
                    let d = dist_sqr(center, c);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, center));
                    }
                }
            }
        }
        if let Some((_, p)) = best {
            self.dims[dim].raids.raids.get_mut(&id).expect("raid").center = p;
        }
    }

    /// `updateRaiders` (every second): raiders gone, far away or idle outside the village too
    /// long leave the raid.
    fn update_raiders(&mut self, dim: DimId, id: i32) {
        let center = self.dims[dim].raids.raids[&id].center;
        let mut remove = Vec::new();
        for r in raiders_of(&self.dims[dim], id) {
            let bp = [r.pos[0].floor() as i32, r.pos[1].floor() as i32, r.pos[2].floor() as i32];
            if dist_sqr(center, bp) >= 12544.0 {
                remove.push(r.clone());
                continue;
            }
            if r.tick_count > 600 {
                let mut outside = r.ticks_outside;
                if !self.is_village(dim, bp) && r.no_action_time > 2400 {
                    outside += 1;
                    with_mob(&mut self.dims[dim], r.id, |_, m| {
                        if let Some(st) = raider::raider_mut(m) {
                            st.ticks_outside_raid += 1;
                        }
                    });
                }
                if outside >= 30 {
                    remove.push(r.clone());
                }
            }
        }
        for r in remove {
            // `removeFromRaid(raider, true)`.
            with_mob(&mut self.dims[dim], r.id, |_, m| {
                if let Some(st) = raider::raider_mut(m) {
                    st.raid = None;
                }
            });
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            raid.total_health -= r.health;
            if r.leader && raid.leaders.get(&r.wave) == Some(&r.id) {
                raid.leaders.remove(&r.wave);
            }
        }
    }

    /// `findRandomSpawnPos`: on a circle around the center shrinking as the countdown runs out,
    /// outside the village until the last seven seconds, on ground a ravager can stand on.
    fn find_random_spawn_pos(&mut self, dim: DimId, id: i32, tries: i32) -> Option<[i32; 3]> {
        let (center, cooldown) = {
            let r = &self.dims[dim].raids.raids[&id];
            (r.center, r.cooldown_ticks)
        };
        let seconds = cooldown / 20;
        let how_far = 0.22f32 * seconds as f32 - 0.24;
        let start = self.dims[dim].raids.raids.get_mut(&id).expect("raid").random.next_float() * std::f32::consts::TAU;
        for i in 0..tries {
            let angle = start + std::f32::consts::PI * i as f32 / 8.0;
            let floor = |v: f32| v.floor() as i32;
            let rnd = &mut self.dims[dim].raids.raids.get_mut(&id).expect("raid").random;
            let x = center[0] + floor(mob::mth::cos(angle as f64) * 32.0 * how_far) + rnd.next_int_bounded(3) * floor(how_far);
            let z = center[2] + floor(mob::mth::sin(angle as f64) * 32.0 * how_far) + rnd.next_int_bounded(3) * floor(how_far);
            let d = &self.dims[dim];
            let Some(chunk) = d.regions.chunk(ChunkPos::of_block(x, z)) else { continue };
            let y = chunk.column_height((x & 15) as usize, (z & 15) as usize, |s| !kiln_data::blocks_types::is_air(s));
            if (y - center[1]).abs() > 96 {
                continue;
            }
            let pos = [x, y, z];
            if self.is_village(dim, pos) && seconds > 7 {
                continue;
            }
            let d = &self.dims[dim];
            let loaded = (-10..=10).step_by(10).all(|dx| (-10..=10).step_by(10).all(|dz| has_chunk_at(d, x + dx, z + dz)));
            if !loaded {
                continue;
            }
            let block = |p: [i32; 3]| d.regions.get_block(p[0], p[1], p[2]).unwrap_or(0);
            let below = block([x, y - 1, z]);
            let ok = mob::path::valid_spawn(below, false) && mob::path::valid_empty_spawn(block(pos), false) && mob::path::valid_empty_spawn(block([x, y + 1, z]), false);
            let snow = kiln_data::blocks_types::block_of(below).name == "minecraft:snow" && kiln_data::blocks_types::is_air(block(pos));
            if ok || snow {
                return Some(pos);
            }
        }
        None
    }

    /// `spawnGroup`: the next wave at `pos` (its first possible raider leads it with the
    /// banner); returns how many raiders it spawned.
    fn spawn_group(&mut self, dim: DimId, id: i32, pos: [i32; 3], alive: usize) -> usize {
        let difficulty = self.commands.difficulty as u8;
        let game_time = self.game_time;
        let world_seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        let wave = raid.groups_spawned + 1;
        raid.total_health = 0.0;
        let bonus_group = raid.should_spawn_bonus_group(alive);
        let normal = num_groups(2);
        let hard = num_groups(3);
        let odds = raid.enchant_odds();
        let mut leader_set = false;
        let mut built: Vec<kiln_entity::Entity> = Vec::new();
        let mut mount_tag = 0u32;
        let ctx = crate::mobs::difficulty_instance(difficulty, game_time, 0, 1.0);
        let spawn_at = kiln_entity::math::Vec3::new(pos[0] as f64 + 0.5, pos[1] as f64 + 1.0, pos[2] as f64 + 0.5);
        let counts = {
            let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
            wave_counts(&mut raid.random, difficulty, raid.num_groups, wave, bonus_group)
        };
        let mut index = 0i64;
        for ((kind, _), count) in RAIDER_TYPES.into_iter().zip(counts) {
            let mut ravagers = 0;
            for _ in 0..count {
                index += 1;
                let seed = raider_seed(world_seed, id, wave, index);
                let mut e = self.build_raider(kind, spawn_at, seed, &ctx, id, wave, odds, normal);
                if !leader_set && raider::can_be_leader(kind) {
                    if let Some(m) = mob::data_mut(&mut e) {
                        raider::set_patrol_leader(m, true);
                        m.equipment[mob::HEAD] = raider::ominous_banner();
                        m.drop_chances[mob::HEAD] = 2.0;
                    }
                    leader_set = true;
                    // The leader's id is known once it exists; see `link_new_raiders`.
                    e.extra.push(("KilnRaidLeader".into(), Tag::Int(wave)));
                }
                if kind == MobKind::Ravager {
                    let rider = if wave == normal {
                        Some(MobKind::Pillager)
                    } else if wave >= hard {
                        Some(if ravagers == 0 { MobKind::Evoker } else { MobKind::Vindicator })
                    } else {
                        None
                    };
                    ravagers += 1;
                    if let Some(rk) = rider {
                        mount_tag += 1;
                        index += 1;
                        let seed = raider_seed(world_seed, id, wave, index);
                        let mut r = self.build_raider(rk, spawn_at, seed, &ctx, id, wave, odds, normal);
                        r.extra.push(("KilnRaidMount".into(), Tag::Int(mount_tag as i32)));
                        e.extra.push(("KilnRaidVehicle".into(), Tag::Int(mount_tag as i32)));
                        built.push(e);
                        built.push(r);
                        continue;
                    }
                }
                built.push(e);
            }
        }
        let n = built.len();
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        raid.total_health = built.iter().filter_map(mob::data).map(|m| m.health).sum();
        raid.wave_spawn_pos = None;
        raid.groups_spawned += 1;
        let first_id = self.next_entity_id;
        for e in built {
            let Some(kind) = kiln_data::entities::by_name(e.type_name) else { continue };
            let p = e.position();
            self.dims[dim].spawns.push(entities::Spawn { kind, pos: [p.x, p.y, p.z], vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) });
        }
        self.materialize_spawns();
        self.link_new_raiders(dim, id, first_id);
        self.update_bar_progress(dim, id);
        n
    }

    /// A new raider of wave `wave` at `at` (`joinRaid` with a position: `finalizeSpawn(EVENT)`,
    /// `applyRaidBuffs`, on the ground).
    #[allow(clippy::too_many_arguments)]
    fn build_raider(&mut self, kind: MobKind, at: kiln_entity::math::Vec3, seed: i64, ctx: &mob::SpawnContext, raid: i32, wave: i32, odds: f32, normal: i32) -> kiln_entity::Entity {
        let mut e = mob::new(kind, -1, 0, seed);
        e.set_pos(at);
        e.set_old_pos_and_rot();
        let mut r = LegacyRandom::new(seed ^ 0x7261_6964);
        let mut group = mob::GroupData { event: true, ..Default::default() };
        mob::finalize_spawn(&mut e, &mut r, ctx, &mut group, false);
        let yaw = e.y_rot;
        let mut kind_ = std::mem::replace(&mut e.kind, kiln_entity::EntityKind::MobTicking { gravity: 0.08 });
        if let kiln_entity::EntityKind::Mob(m) = &mut kind_ {
            if let Some(st) = raider::raider_mut(m) {
                st.raid = Some(raid);
                st.wave = wave;
                st.can_join_raid = true;
                st.ticks_outside_raid = 0;
            }
            match kind {
                MobKind::Pillager => mob::kinds::pillager::apply_raid_buffs(&mut e, m, wave, odds, normal, num_groups(1)),
                MobKind::Vindicator => mob::kinds::vindicator::apply_raid_buffs(&mut e, m, odds),
                _ => {}
            }
            m.y_head_rot = yaw;
            m.y_body_rot = yaw;
        }
        e.kind = kind_;
        e.on_ground = true;
        e
    }

    /// Wave leaders and ravager riders of raiders just added (ids `first_id` on).
    fn link_new_raiders(&mut self, dim: DimId, id: i32, first_id: i32) {
        let d = &mut self.dims[dim];
        let mut riders: Vec<(i32, i32)> = Vec::new();
        let mut vehicles: Vec<(i32, i32)> = Vec::new();
        let mut leaders: Vec<(i32, i32)> = Vec::new();
        for r in d.regions.iter_mut() {
            for e in r.part_mut().0.list.iter_mut().filter(|e| e.id >= first_id) {
                let Some(p) = e.phys.as_mut() else { continue };
                let mut take = |key: &str| -> Option<i32> {
                    let i = p.extra.iter().position(|(k, _)| k == key)?;
                    p.extra.remove(i).1.as_i64().map(|v| v as i32)
                };
                if let Some(t) = take("KilnRaidMount") {
                    riders.push((t, e.id));
                }
                if let Some(t) = take("KilnRaidVehicle") {
                    vehicles.push((t, e.id));
                }
                if let Some(w) = take("KilnRaidLeader") {
                    leaders.push((w, e.id));
                }
            }
        }
        if let Some(raid) = d.raids.raids.get_mut(&id) {
            for (w, e) in leaders {
                raid.leaders.insert(w, e);
            }
        }
        for (tag, rider) in riders {
            let Some(&(_, vehicle)) = vehicles.iter().find(|(t, _)| *t == tag) else { continue };
            for r in d.regions.iter_mut() {
                let list = &mut r.part_mut().0.list;
                let Ok(i) = list.binary_search_by_key(&rider, |e| e.id) else { continue };
                if let Some(mut p) = list[i].phys.take() {
                    // The vehicle is in the same region (same spawn position).
                    if let Ok(j) = list.binary_search_by_key(&vehicle, |e| e.id)
                        && let Some(v) = list[j].phys.as_mut()
                    {
                        kiln_entity::ride::start_riding(&mut p, v, false);
                    }
                    list[i].phys = Some(p);
                }
                break;
            }
        }
    }

    /// `playSound`: the raid horn toward the spawn point for players within 64 blocks or in
    /// the raid.
    fn play_raid_horn(&mut self, dim: DimId, id: i32, origin: [i32; 3]) {
        let Some(sound) = kiln_data::builtin_id("minecraft:sound_event", "minecraft:event.raid.horn") else { return };
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        let seed = raid.random.next_long();
        let in_raid = raid.bar.players.clone();
        let (ox, oz) = (origin[0] as f64 + 0.5, origin[2] as f64 + 0.5);
        for p in self.players.values_mut().filter(|p| p.dim == dim) {
            let (dx, dz) = (ox - p.pos[0], oz - p.pos[2]);
            let dist = (dx * dx + dz * dz).sqrt();
            if dist <= 64.0 || in_raid.contains(&p.conn) {
                let (x, z) = (p.pos[0] + 13.0 / dist * dx, p.pos[2] + 13.0 / dist * dz);
                let pkt = kiln_proto::packets::world_fx::sound(
                    &kiln_proto::packets::world_fx::Sound::Registered(sound),
                    kiln_proto::packets::world_fx::SoundSource::Neutral,
                    [x, p.pos[1], z],
                    64.0,
                    1.0,
                    seed,
                );
                p.send(pkt);
            }
        }
    }

    /// Victory: heroes in the level get Hero of the Village, the stat and the trigger.
    fn reward_heroes(&mut self, dim: DimId, id: i32) {
        let raid = &self.dims[dim].raids.raids[&id];
        let heroes = raid.heroes.clone();
        let amp = raid.omen_level - 1;
        let Some(effect) = crate::effects::effect_id("minecraft:hero_of_the_village") else { return };
        for p in self.players.values_mut() {
            if p.dim != dim || p.game_mode == 3 || p.dead || !heroes.contains(&p.uuid.as_u128()) {
                continue;
            }
            p.add_effect(crate::effects::Effect::new(effect, 48000, amp, false, false, true));
            p.award_stat(player_stats::custom("minecraft:raid_win"), 1);
            p.fire("minecraft:hero_of_the_village", None, |c, _, _| matches!(c.trigger, crate::advancements::criteria::Trigger::Player));
        }
    }

    /// `updatePlayers`: the boss bar for the live players of the level whose nearest raid is
    /// this one.
    fn update_raid_players(&mut self, dim: DimId, id: i32) {
        let views = {
            let d = &self.dims[dim];
            d.raids.raids.iter().map(|(i, r)| r.view(*i)).collect::<Vec<_>>()
        };
        let want: Vec<kiln_link::ConnId> = {
            let mut v: Vec<_> = self
                .players
                .values()
                .filter(|p| p.dim == dim && !p.dead && !p.disconnected)
                .filter(|p| {
                    let bp = kiln_entity::math::BlockPos::containing(p.pos[0], p.pos[1], p.pos[2]);
                    raid_at_view(&views, bp).is_some_and(|r| r.id == id)
                })
                .map(|p| p.conn)
                .collect();
            v.sort_unstable();
            v
        };
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        let old = std::mem::take(&mut raid.bar.players);
        let bar = raid.bar.clone();
        for conn in &want {
            if !old.contains(conn)
                && bar.visible
                && let Some(p) = self.players.get_mut(conn)
            {
                p.send(add_packet(&bar));
            }
        }
        for conn in &old {
            if !want.contains(conn)
                && let Some(p) = self.players.get_mut(conn)
            {
                p.send(hud::boss_event(bar.uuid, &BossEvent::Remove));
            }
        }
        self.dims[dim].raids.raids.get_mut(&id).expect("raid").bar.players = want;
    }

    fn set_bar_progress(&mut self, dim: DimId, id: i32, progress: f32) {
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        if raid.bar.progress == progress {
            return;
        }
        raid.bar.progress = progress;
        let bar = raid.bar.clone();
        if bar.visible {
            for conn in &bar.players {
                if let Some(p) = self.players.get_mut(conn) {
                    p.send(hud::boss_event(bar.uuid, &BossEvent::Progress(progress)));
                }
            }
        }
    }

    fn set_bar_name(&mut self, dim: DimId, id: i32, name: Tag) {
        let raid = self.dims[dim].raids.raids.get_mut(&id).expect("raid");
        if raid.bar.name == name {
            return;
        }
        raid.bar.name = name;
        let bar = raid.bar.clone();
        self.bar_name(&bar);
    }

    fn bar_name(&mut self, bar: &BossBar) {
        if bar.visible {
            for conn in &bar.players {
                if let Some(p) = self.players.get_mut(conn) {
                    p.send(hud::boss_event(bar.uuid, &BossEvent::Name(&bar.name)));
                }
            }
        }
    }

    fn hide_bar(&mut self, bar: &BossBar) {
        for conn in &bar.players {
            if let Some(p) = self.players.get_mut(conn) {
                p.send(hud::boss_event(bar.uuid, &BossEvent::Remove));
            }
        }
    }

    /// `updateBossbar`: the living raiders' health over the wave's total.
    fn update_bar_progress(&mut self, dim: DimId, id: i32) {
        let health: f32 = raiders_of(&self.dims[dim], id).iter().map(|r| r.health).sum();
        let total = self.dims[dim].raids.raids[&id].total_health;
        self.set_bar_progress(dim, id, (health / total).clamp(0.0, 1.0));
    }

    /// Players whose raid omen ran out: `Raids.createOrExtendRaid`.
    fn raid_omen_triggers(&mut self, dim: DimId) {
        let mut conns: Vec<kiln_link::ConnId> = self.players.values().filter(|p| p.dim == dim && p.raid_omen_trigger.is_some()).map(|p| p.conn).collect();
        conns.sort_unstable();
        for conn in conns {
            let Some(p) = self.players.get_mut(&conn) else { continue };
            let Some((pos, amplifier)) = p.raid_omen_trigger.take() else { continue };
            if p.game_mode == 3 {
                continue;
            }
            self.create_or_extend_raid(dim, conn, pos, amplifier);
        }
    }

    /// `Raids.createOrExtendRaid`: a raid centered on the occupied village points of interest
    /// within 64 (or the omen's position), created or joined, absorbs the player's omen.
    pub(crate) fn create_or_extend_raid(&mut self, dim: DimId, conn: kiln_link::ConnId, pos: [i32; 3], amplifier: i32) -> Option<i32> {
        if !self.rule_bool("minecraft:raids") || DIMENSIONS[dim].0 == "minecraft:the_nether" {
            return None;
        }
        let village = poi::kinds_of(&["#minecraft:village"]);
        let pois = poi::in_range(&self.dims[dim].regions, &village, pos, 64, Occupancy::IsOccupied);
        let center = if pois.is_empty() {
            pos
        } else {
            let n = pois.len() as f64;
            let (x, y, z) = pois.iter().fold((0.0, 0.0, 0.0), |a, r| (a.0 + r.pos[0] as f64, a.1 + r.pos[1] as f64, a.2 + r.pos[2] as f64));
            [(x / n).floor() as i32, (y / n).floor() as i32, (z / n).floor() as i32]
        };
        let difficulty = self.commands.difficulty as u8;
        let world_seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let raids = &mut self.dims[dim].raids;
        let id = match raids.raid_at(center) {
            Some(id) => id,
            None => {
                raids.next_id += 1;
                let id = raids.next_id;
                let seed = world_seed ^ (id as i64).wrapping_mul(0x5DEE_CE66D) ^ ((center[0] as i64) << 20) ^ center[2] as i64;
                raids.raids.insert(id, Raid::new(center, difficulty, seed));
                id
            }
        };
        let raid = raids.raids.get_mut(&id).expect("raid");
        if !raid.started || raid.omen_level < 5 {
            // `absorbRaidOmen`.
            raid.omen_level = (raid.omen_level + amplifier + 1).clamp(0, 5);
            if raid.groups_spawned == 0
                && let Some(p) = self.players.get_mut(&conn)
            {
                p.award_stat(player_stats::custom("minecraft:raid_trigger"), 1);
                p.fire("minecraft:voluntary_exile", None, |c, _, _| matches!(c.trigger, crate::advancements::criteria::Trigger::Player));
            }
        }
        self.dims[dim].raids.refresh_views();
        Some(id)
    }

    /// `PatrolSpawner.tick` for the overworld: every 10 to 11 minutes, by day, one time in five,
    /// a patrol of pillagers 24 to 47 blocks from a random player away from villages.
    fn tick_patrols(&mut self) {
        const OVERWORLD: DimId = crate::OVERWORLD_ID;
        let spawn_enemies = self.commands.difficulty as u8 != 0 && self.rule_bool("minecraft:spawn_monsters");
        if !spawn_enemies || !self.rule_bool("minecraft:spawn_patrols") {
            return;
        }
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let mut rng = LegacyRandom::new(seed ^ self.game_time.wrapping_mul(0x7061_7472_6f6c));
        let d = &mut self.dims[OVERWORLD];
        d.raids.patrol_next_tick -= 1;
        if d.raids.patrol_next_tick > 0 {
            return;
        }
        d.raids.patrol_next_tick += 12000 + rng.next_int_bounded(1200);
        let sky_darken = crate::weather::sky_darken(OVERWORLD, self.day_time, &self.level_weather[OVERWORLD]);
        if sky_darken >= 4 {
            return;
        }
        if rng.next_int_bounded(5) != 0 {
            return;
        }
        let mut players: Vec<&crate::Player> = self.players.values().filter(|p| p.dim == OVERWORLD).collect();
        players.sort_unstable_by_key(|p| p.conn);
        if players.is_empty() {
            return;
        }
        let p = players[rng.next_int_bounded(players.len() as i32) as usize];
        if p.game_mode == 3 {
            return;
        }
        let ppos = [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32];
        if poi::sections_to_village(&self.dims[OVERWORLD].regions, ppos) <= 2 {
            return;
        }
        let dx = (24 + rng.next_int_bounded(24)) * if rng.next_bool() { -1 } else { 1 };
        let dz = (24 + rng.next_int_bounded(24)) * if rng.next_bool() { -1 } else { 1 };
        let mut pos = [ppos[0] + dx, ppos[1], ppos[2] + dz];
        let d = &self.dims[OVERWORLD];
        let loaded = (-10..=10).step_by(10).all(|a| (-10..=10).step_by(10).all(|b| has_chunk_at(d, pos[0] + a, pos[2] + b)));
        if !loaded {
            return;
        }
        // `can_pillager_patrol_spawn`: not in mushroom fields, not in the first five days.
        if self.day_time < 120000 || self.biome_name(OVERWORLD, pos) == Some("minecraft:mushroom_fields") {
            return;
        }
        let ctx = crate::mobs::difficulty_instance(self.commands.difficulty as u8, self.game_time, 0, 1.0);
        let size = ctx.effective_difficulty.ceil() as i32 + 1;
        for i in 0..size {
            let d = &self.dims[OVERWORLD];
            let Some(chunk) = d.regions.chunk(ChunkPos::of_block(pos[0], pos[2])) else { break };
            pos[1] = chunk.column_height((pos[0] & 15) as usize, (pos[2] & 15) as usize, |s| {
                kiln_data::block_props::motion_blocking(s) && !kiln_data::blocks_types::block_of(s).name.ends_with("_leaves")
            });
            let ok = self.spawn_patrol_member(pos, &mut rng, i == 0, &ctx);
            if i == 0 && !ok {
                break;
            }
            pos[0] += rng.next_int_bounded(5) - rng.next_int_bounded(5);
            pos[2] += rng.next_int_bounded(5) - rng.next_int_bounded(5);
        }
        self.materialize_spawns();
    }

    /// `spawnPatrolMember`.
    fn spawn_patrol_member(&mut self, pos: [i32; 3], rng: &mut LegacyRandom, leader: bool, ctx: &mob::SpawnContext) -> bool {
        let d = &self.dims[crate::OVERWORLD_ID];
        let block = |p: [i32; 3]| d.regions.get_block(p[0], p[1], p[2]).unwrap_or(0);
        if !mob::path::valid_empty_spawn(block(pos), false) {
            return false;
        }
        // `checkPatrollingMonsterSpawnRules`: dark enough for blocks, on a valid floor.
        let block_light = kiln_world::light::light_at(&d.regions, kiln_world::chunk::LightLayer::Block, pos[0], pos[1], pos[2]).unwrap_or(0);
        if block_light > 8 || !mob::path::valid_spawn(block([pos[0], pos[1] - 1, pos[2]]), false) {
            return false;
        }
        let seed = rng.next_long();
        let mut e = mob::new(MobKind::Pillager, -1, 0, seed);
        if leader {
            // `setPatrolLeader`, then `findPatrolTarget` (still at the origin: the position comes
            // after, as vanilla does it).
            let mut kind = std::mem::replace(&mut e.kind, kiln_entity::EntityKind::MobTicking { gravity: 0.08 });
            if let kiln_entity::EntityKind::Mob(m) = &mut kind {
                raider::set_patrol_leader(m, true);
                raider::find_patrol_target(&mut e, m);
            }
            e.kind = kind;
        }
        e.set_pos(kiln_entity::math::Vec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64));
        e.set_old_pos_and_rot();
        let mut group = mob::GroupData { patrol: true, ..Default::default() };
        mob::finalize_spawn(&mut e, rng, ctx, &mut group, false);
        let kind = kiln_data::entities::by_name("minecraft:pillager").expect("pillager");
        let p = e.position();
        self.dims[crate::OVERWORLD_ID].spawns.push(entities::Spawn { kind, pos: [p.x, p.y, p.z], vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) });
        true
    }

    /// The biome's name at `pos` of level `dim`, if its chunk is loaded.
    fn biome_name(&self, dim: DimId, pos: [i32; 3]) -> Option<&'static str> {
        // The stored (quart) biome, without the voronoi zoom.
        let chunk = self.dims[dim].regions.chunk(ChunkPos::of_block(pos[0], pos[2]))?;
        let min_y = self.dims[dim].provider.dimension.min_y;
        let rel = ((pos[1] >> 2) - (min_y >> 2)).max(0);
        let section = chunk.sections.get((rel >> 2) as usize)?;
        let id = match &section.biomes {
            kiln_world::section::Biomes::Single(b) => *b,
            kiln_world::section::Biomes::Cells(c) => c[(((rel & 3) as usize) << 4) | ((((pos[2] >> 2) & 3) as usize) << 2) | ((pos[0] >> 2) & 3) as usize],
        };
        kiln_entity::mob::kinds::villager::biome_name(id as i32)
    }
}

impl Sim {
    /// Each level's `raids.dat` (`dimensions/<ns>/<level>/data/minecraft/raids.dat`).
    pub(crate) fn load_raids(&mut self) {
        let Some(dir) = self.storage.as_ref().map(|s| s.dir.clone()) else { return };
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        for (dim, d) in self.dims.iter_mut().enumerate() {
            if let Some(data) = kiln_storage::saved_data::read(&dir.join(crate::dimension_dir(DIMENSIONS[dim].0)), "raids") {
                d.raids = Raids::from_nbt(&data, seed);
            }
        }
    }

    pub(crate) fn save_raids(&mut self) {
        let Some(dir) = self.storage.as_ref().map(|s| s.dir.clone()) else { return };
        for (dim, d) in self.dims.iter().enumerate() {
            if d.raids.raids.is_empty() && d.raids.next_id <= 1 {
                continue;
            }
            if let Err(e) = kiln_storage::saved_data::write(&dir.join(crate::dimension_dir(DIMENSIONS[dim].0)), "raids", d.raids.to_nbt()) {
                tracing::warn!("failed to save the raids of {}: {e}", DIMENSIONS[dim].0);
            }
        }
    }
}

/// The boss bar's add packet.
fn add_packet(bar: &BossBar) -> bytes::Bytes {
    hud::boss_event(bar.uuid, &BossEvent::Add { name: &bar.name, progress: bar.progress, color: BossBarColor::Red, overlay: BossBarOverlay::Notched10, flags: 0 })
}
