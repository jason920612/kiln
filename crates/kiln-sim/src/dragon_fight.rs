//! The End's dragon fight (`EnderDragonFight`, saved as `ender_dragon_fight` in the End's
//! data): the first player near the arena finds the fight's state (`scanState`: an active exit
//! portal means the dragon died before), the dragon is made at (0, 128, 0) and followed with a
//! boss bar, the pillars' crystals are counted every 100 ticks, the dragon's death opens the exit
//! portal, places the egg the first time and a new end gateway, and four crystals on the exit
//! portal's rim start the respawn ritual (the pillars rebuilt one by one, then a new dragon).
//!
//! The dragon and the crystals live in the regions: what they tell the fight
//! ([`DragonFightEvent`]) and crystal placements come back through [`FightEnv::inbox`] and are
//! handled serially, in an order that does not depend on the regions.

use crate::blocks::{self, BlockOut, RegionLevel};
use crate::entities::{self, Spawn};
use crate::{DIMENSIONS, END_ID, Player, Sim, health};
use kiln_blocks::BlockPos;
use kiln_data::blocks::default_state as block;
use kiln_entity::level::{DragonFightEvent, DragonFightView};
use kiln_entity::math::Vec3;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::hud;
use kiln_world::{Blocks, ChunkPos};
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

/// Saved data id (`EnderDragonFight.TYPE`).
const FIGHT_DATA: &str = "ender_dragon_fight";
/// `DRAGON_SPAWN_Y`.
const DRAGON_SPAWN_Y: i32 = 128;
/// The arena kept loaded while the boss bar has players (`ARENA_SIZE_CHUNKS`).
const ARENA_CHUNKS: i32 = 8;

/// `DragonRespawnStage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RespawnStage {
    Start,
    PreparingToSummonPillars,
    SummoningPillars,
    SummoningDragon,
    End,
}

impl RespawnStage {
    const ALL: [RespawnStage; 5] =
        [RespawnStage::Start, RespawnStage::PreparingToSummonPillars, RespawnStage::SummoningPillars, RespawnStage::SummoningDragon, RespawnStage::End];

    fn name(self) -> &'static str {
        match self {
            RespawnStage::Start => "start",
            RespawnStage::PreparingToSummonPillars => "preparing_to_summon_pillars",
            RespawnStage::SummoningPillars => "summoning_pillars",
            RespawnStage::SummoningDragon => "summoning_dragon",
            RespawnStage::End => "end",
        }
    }

    fn by_name(name: &str) -> Option<RespawnStage> {
        RespawnStage::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// A call into the fight from a region.
#[derive(Clone, Debug)]
pub(crate) enum FightMsg {
    Entity(DragonFightEvent),
    /// An end crystal was placed (`EndCrystalItem.useOn` → `tryRespawn`).
    TryRespawn,
}

impl FightMsg {
    /// Order of calls from different regions.
    fn key(&self) -> (u8, i64) {
        match self {
            FightMsg::Entity(DragonFightEvent::Update { dragon, .. }) => (0, *dragon as i64),
            FightMsg::Entity(DragonFightEvent::Killed { dragon, .. }) => (1, *dragon as i64),
            FightMsg::Entity(DragonFightEvent::DeathRoar { pos }) => (2, pos.x as i64 ^ ((pos.z as i64) << 32)),
            FightMsg::Entity(DragonFightEvent::CrystalDestroyed { crystal, .. }) => (3, *crystal as i64),
            FightMsg::TryRespawn => (4, 0),
        }
    }
}

/// What the End's regions see of the fight this tick.
#[derive(Clone)]
pub(crate) struct FightEnv {
    pub view: DragonFightView,
    pub inbox: Arc<Mutex<Vec<FightMsg>>>,
    /// The boss bar has players: the arena (`arena_radius` chunks around `arena_center`) ticks.
    pub active: bool,
    pub arena_center: ChunkPos,
    pub arena_radius: i32,
}

impl FightEnv {
    pub(crate) fn send(&self, msg: FightMsg) {
        self.inbox.lock().unwrap().push(msg);
    }
}

/// The fight's boss bar (`ServerBossEvent`).
struct BossBar {
    uuid: uuid::Uuid,
    players: Vec<ConnId>,
    progress: f32,
    visible: bool,
}

pub(crate) struct DragonFight {
    needs_state_scanning: bool,
    dragon_killed: bool,
    previously_killed: bool,
    respawn_stage: Option<RespawnStage>,
    respawn_time: i32,
    dragon_uuid: Option<u128>,
    exit_portal: Option<BlockPos>,
    gateways: Vec<i32>,
    respawn_crystals: Vec<u128>,
    ticks_since_dragon_seen: i32,
    alive_crystals: i32,
    ticks_since_crystals_scanned: i32,
    ticks_since_player_scan: i32,
    /// Where the dragon was last seen (it may be in an unloaded chunk, not gone).
    last_seen: Option<ChunkPos>,
    boss: BossBar,
    inbox: Arc<Mutex<Vec<FightMsg>>>,
    /// Stand-in for the level random (the dragon's yaw, crystal yaws).
    random: LegacyRandom,
    origin: BlockPos,
}

fn uuid_of(tag: &Tag) -> Option<u128> {
    kiln_entity::persist::uuid_from_tag(tag)
}

impl DragonFight {
    /// `EnderDragonFight.createDefault` / the codec, then `init` (the gateways shuffled with the
    /// world seed when there are none).
    pub(crate) fn load(data: Option<&Tag>, seed: i64) -> DragonFight {
        let get = |k: &str| data.and_then(|d| d.get(k));
        let flag = |k: &str, d: bool| get(k).and_then(Tag::as_i64).map_or(d, |v| v != 0);
        let pos = get("exit_portal_location").and_then(|t| match t {
            Tag::IntArray(v) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        });
        let mut gateways: Vec<i32> = match get("gateways") {
            Some(Tag::List(v)) => v.iter().filter_map(Tag::as_i64).map(|v| v as i32).collect(),
            Some(Tag::IntArray(v)) => v.clone(),
            _ => Vec::new(),
        };
        if gateways.is_empty() {
            gateways = kiln_worldgen::end::end_gateway_order(seed);
        }
        let respawn_crystals = match get("respawn_crystals") {
            Some(Tag::List(v)) => v.iter().filter_map(uuid_of).collect(),
            _ => Vec::new(),
        };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as i64;
        let mut random = LegacyRandom::new(seed ^ 0x6472_6167_6f6e);
        let bar = uuid::Uuid::from_u64_pair(random.next_long() as u64 ^ now as u64, (random.next_long() as u64 & !0xF000) | 0x4000);
        DragonFight {
            needs_state_scanning: flag("needs_state_scanning", true),
            dragon_killed: flag("dragon_killed", false),
            previously_killed: flag("previously_killed", false),
            respawn_stage: get("respawn_stage").and_then(Tag::as_str).and_then(RespawnStage::by_name),
            respawn_time: get("respawn_time").and_then(Tag::as_i64).unwrap_or(0) as i32,
            dragon_uuid: get("dragon_uuid").and_then(uuid_of),
            exit_portal: pos,
            gateways,
            respawn_crystals,
            ticks_since_dragon_seen: 0,
            alive_crystals: 0,
            ticks_since_crystals_scanned: 0,
            ticks_since_player_scan: 21,
            last_seen: None,
            boss: BossBar { uuid: bar, players: Vec::new(), progress: 1.0, visible: true },
            inbox: Arc::new(Mutex::new(Vec::new())),
            random,
            origin: BlockPos::new(0, 0, 0),
        }
    }

    /// The codec's form.
    fn to_nbt(&self) -> Tag {
        let mut c = vec![
            ("needs_state_scanning".to_owned(), Tag::Byte(self.needs_state_scanning as i8)),
            ("dragon_killed".to_owned(), Tag::Byte(self.dragon_killed as i8)),
            ("previously_killed".to_owned(), Tag::Byte(self.previously_killed as i8)),
        ];
        if let Some(s) = self.respawn_stage {
            c.push(("respawn_stage".into(), Tag::String(s.name().into())));
        }
        c.push(("respawn_time".into(), Tag::Int(self.respawn_time)));
        if let Some(u) = self.dragon_uuid {
            c.push(("dragon_uuid".into(), kiln_entity::persist::uuid_to_tag(u)));
        }
        if let Some(p) = self.exit_portal {
            c.push(("exit_portal_location".into(), Tag::IntArray(vec![p.x, p.y, p.z])));
        }
        c.push(("gateways".into(), Tag::List(self.gateways.iter().map(|&g| Tag::Int(g)).collect())));
        if !self.respawn_crystals.is_empty() {
            c.push(("respawn_crystals".into(), Tag::List(self.respawn_crystals.iter().map(|&u| kiln_entity::persist::uuid_to_tag(u)).collect())));
        }
        Tag::Compound(c)
    }

    fn view(&self) -> DragonFightView {
        DragonFightView { dragon: self.dragon_uuid, alive_crystals: self.alive_crystals, previously_killed: self.previously_killed, origin: ebp(self.origin) }
    }

    /// A call into the fight from the serial phases (`/kill` on a crystal).
    pub(crate) fn send(&self, msg: FightMsg) {
        self.inbox.lock().unwrap().push(msg);
    }

    /// Whether the boss bar has players (the arena is kept loaded).
    pub(crate) fn active(&self) -> bool {
        !self.boss.players.is_empty()
    }

    /// The arena's chunks (`ARENA_SIZE_CHUNKS` around the origin's chunk).
    pub(crate) fn arena(&self) -> impl Iterator<Item = ChunkPos> {
        let c = ChunkPos::of_block(self.origin.x, self.origin.z);
        (-ARENA_CHUNKS..=ARENA_CHUNKS).flat_map(move |x| (-ARENA_CHUNKS..=ARENA_CHUNKS).map(move |z| ChunkPos::new(c.x + x, c.z + z)))
    }
}

/// The fight's name on the boss bar.
fn boss_name() -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String("entity.minecraft.ender_dragon".into()))])
}

fn add_packet(bar: &BossBar) -> bytes::Bytes {
    let name = boss_name();
    hud::boss_event(
        bar.uuid,
        &hud::BossEvent::Add {
            name: &name,
            progress: bar.progress,
            color: hud::BossBarColor::Pink,
            overlay: hud::BossBarOverlay::Progress,
            flags: hud::boss_flags::PLAY_BOSS_MUSIC | hud::boss_flags::CREATE_WORLD_FOG,
        },
    )
}

fn ebp(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

fn kb(p: kiln_worldgen::pos::BlockPos) -> BlockPos {
    BlockPos::new(p.x, p.y, p.z)
}

fn wg(p: BlockPos) -> kiln_worldgen::pos::BlockPos {
    kiln_worldgen::pos::BlockPos::new(p.x, p.y, p.z)
}

impl Sim {
    /// The End's fight from its saved data (a new one without).
    pub(crate) fn load_dragon_fight(&mut self) {
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let dir = self.storage.as_ref().map(|s| s.dir.join(crate::dimension_dir(DIMENSIONS[END_ID].0)));
        let data = dir.as_ref().and_then(|d| kiln_storage::saved_data::read(d, FIGHT_DATA));
        self.dragon_fight = DragonFight::load(data.as_ref(), seed);
    }

    pub(crate) fn save_dragon_fight(&self) {
        let Some(storage) = &self.storage else { return };
        let dir = storage.dir.join(crate::dimension_dir(DIMENSIONS[END_ID].0));
        if let Err(e) = kiln_storage::saved_data::write(&dir, FIGHT_DATA, self.dragon_fight.to_nbt()) {
            tracing::warn!("saving the dragon fight failed: {e}");
        }
    }

    /// The fight as the End's regions see it this tick.
    pub(crate) fn fight_env(&self, dim: crate::DimId) -> Option<FightEnv> {
        let f = &self.dragon_fight;
        (dim == END_ID).then(|| FightEnv {
            view: f.view(),
            inbox: f.inbox.clone(),
            active: f.active(),
            arena_center: ChunkPos::of_block(f.origin.x, f.origin.z),
            arena_radius: ARENA_CHUNKS,
        })
    }

    /// `EnderDragonFight.tick`, in the End's level tick.
    pub(crate) fn tick_dragon_fight(&mut self) {
        self.dragon_fight_messages();
        let killed = self.dragon_fight.dragon_killed;
        self.boss_visible(!killed);
        self.dragon_fight.ticks_since_player_scan += 1;
        if self.dragon_fight.ticks_since_player_scan >= 20 {
            self.update_boss_players();
            self.dragon_fight.ticks_since_player_scan = 0;
        }
        if !self.dragon_fight.active() {
            return;
        }
        if !self.arena_loaded() {
            // `TicketType.DRAGON` loads the arena; the middle first, at once.
            let o = self.dragon_fight.origin;
            self.load_area(END_ID, o, 16);
            if !self.arena_loaded() {
                return;
            }
        }
        if self.dragon_fight.needs_state_scanning {
            self.scan_state();
            self.dragon_fight.needs_state_scanning = false;
        }
        if let Some(stage) = self.dragon_fight.respawn_stage {
            let crystals = self.respawn_crystal_ids();
            if crystals.is_empty() {
                self.abort_respawn();
                return;
            }
            let time = self.dragon_fight.respawn_time;
            self.dragon_fight.respawn_time += 1;
            self.respawn_stage_tick(stage, &crystals, time);
        }
        if !self.dragon_fight.dragon_killed {
            let f = &mut self.dragon_fight;
            f.ticks_since_dragon_seen += 1;
            if f.dragon_uuid.is_none() || f.ticks_since_dragon_seen >= 1200 {
                self.find_or_create_dragon();
                self.dragon_fight.ticks_since_dragon_seen = 0;
            }
            let f = &mut self.dragon_fight;
            f.ticks_since_crystals_scanned += 1;
            if f.ticks_since_crystals_scanned >= 100 {
                self.update_crystal_count();
                self.dragon_fight.ticks_since_crystals_scanned = 0;
            }
        }
    }

    /// The calls the regions made, in a region-independent order.
    pub(crate) fn dragon_fight_messages(&mut self) {
        let mut msgs = std::mem::take(&mut *self.dragon_fight.inbox.lock().unwrap());
        if msgs.is_empty() {
            return;
        }
        msgs.sort_by_key(FightMsg::key);
        for m in msgs {
            match m {
                FightMsg::Entity(DragonFightEvent::Update { uuid, pos, health, max_health, .. }) => {
                    if Some(uuid) == self.dragon_fight.dragon_uuid {
                        self.boss_progress(health / max_health);
                        self.dragon_fight.ticks_since_dragon_seen = 0;
                        self.dragon_fight.last_seen = Some(entities::chunk_of([pos.x, pos.y, pos.z]));
                    }
                }
                FightMsg::Entity(DragonFightEvent::Killed { uuid, .. }) => self.set_dragon_killed(uuid),
                FightMsg::Entity(DragonFightEvent::DeathRoar { pos }) => {
                    // `globalLevelEvent`: every player of every level.
                    let pkt = kiln_proto::packets::world_fx::level_event(1028, [pos.x, pos.y, pos.z], 0, true);
                    self.broadcast(pkt);
                }
                FightMsg::Entity(DragonFightEvent::CrystalDestroyed { crystal, uuid, pos, attacker, .. }) => {
                    self.on_crystal_destroyed(crystal, uuid, pos, attacker);
                }
                FightMsg::TryRespawn => self.try_respawn(),
            }
        }
    }

    // ------------------------------------------------------------------ the boss bar

    fn boss_visible(&mut self, visible: bool) {
        if self.dragon_fight.boss.visible == visible {
            return;
        }
        self.dragon_fight.boss.visible = visible;
        let bar = &self.dragon_fight.boss;
        let pkt = if visible { add_packet(bar) } else { hud::boss_event(bar.uuid, &hud::BossEvent::Remove) };
        for c in bar.players.clone() {
            if let Some(p) = self.players.get_mut(&c) {
                p.send(pkt.clone());
            }
        }
    }

    fn boss_progress(&mut self, progress: f32) {
        let bar = &mut self.dragon_fight.boss;
        if bar.progress == progress {
            return;
        }
        bar.progress = progress;
        if !bar.visible {
            return;
        }
        let pkt = hud::boss_event(bar.uuid, &hud::BossEvent::Progress(progress));
        for c in bar.players.clone() {
            if let Some(p) = self.players.get_mut(&c) {
                p.send(pkt.clone());
            }
        }
    }

    /// `updatePlayers`: the living players within 192 blocks of (0, 128, 0) see the bar.
    fn update_boss_players(&mut self) {
        let o = self.dragon_fight.origin;
        let center = [o.x as f64, (DRAGON_SPAWN_Y + o.y) as f64, o.z as f64];
        let mut now: Vec<ConnId> = self
            .players
            .values()
            .filter(|p| p.dim == END_ID && !p.dead && !p.disconnected)
            .filter(|p| (0..3).map(|i| (p.pos[i] - center[i]).powi(2)).sum::<f64>() < 192.0 * 192.0)
            .map(|p| p.conn)
            .collect();
        now.sort_unstable();
        let before = std::mem::take(&mut self.dragon_fight.boss.players);
        let bar = &self.dragon_fight.boss;
        for c in now.iter().filter(|c| !before.contains(c)) {
            if bar.visible
                && let Some(p) = self.players.get_mut(c)
            {
                p.send(add_packet(bar));
            }
        }
        for c in before.iter().filter(|c| !now.contains(c)) {
            if bar.visible
                && let Some(p) = self.players.get_mut(c)
            {
                p.send(hud::boss_event(bar.uuid, &hud::BossEvent::Remove));
            }
        }
        self.dragon_fight.boss.players = now;
    }

    /// A player leaving the level or the game leaves the bar (`removePlayer` when it is next
    /// scanned; at once here, its client dropped it with the level).
    pub(crate) fn dragon_fight_left(&mut self, conn: ConnId) {
        self.dragon_fight.boss.players.retain(|&c| c != conn);
    }

    // ------------------------------------------------------------------ state

    /// `isArenaLoaded`: the chunks around the origin are loaded.
    fn arena_loaded(&self) -> bool {
        let c = ChunkPos::of_block(self.dragon_fight.origin.x, self.dragon_fight.origin.z);
        (-1..=1).all(|x| (-1..=1).all(|z| self.dims[END_ID].regions.chunk(ChunkPos::new(c.x + x, c.z + z)).is_some()))
    }

    /// The End's loaded entities.
    fn end_entities(&self) -> impl Iterator<Item = &entities::Entity> {
        self.dims[END_ID].regions.iter().flat_map(|r| r.part().0.list.iter()).filter(|e| !e.removed)
    }

    fn end_entity(&self, id: i32) -> Option<&entities::Entity> {
        self.end_entities().find(|e| e.id == id)
    }

    /// `level.getDragons()`, in id order.
    fn end_dragons(&self) -> Vec<(i32, u128)> {
        let mut v: Vec<(i32, u128)> =
            self.end_entities().filter(|e| e.kind.name == "minecraft:ender_dragon").map(|e| (e.id, e.uuid.as_u128())).collect();
        v.sort_unstable();
        v
    }

    /// `hasActiveExitPortal`: an end portal block on the podium.
    fn has_active_exit_portal(&mut self) -> bool {
        let o = self.dragon_fight.exit_portal.unwrap_or(self.dragon_fight.origin);
        let d = self.dims[END_ID].provider.dimension;
        for x in o.x - 3..=o.x + 3 {
            for z in o.z - 3..=o.z + 3 {
                for y in d.min_y..d.min_y + d.height {
                    if kiln_blocks::state::is(self.block_loading(END_ID, BlockPos::new(x, y, z)), block::END_PORTAL) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `findExitPortal`: the podium's bedrock pillar and rim (at the remembered location, else
    /// down the origin's column); remembers where it is.
    fn find_exit_portal(&mut self) -> Option<BlockPos> {
        let is_podium = |sim: &mut Sim, p: BlockPos| {
            let bedrock = |sim: &mut Sim, q: BlockPos| kiln_blocks::state::is(sim.block_loading(END_ID, q), block::BEDROCK);
            (0..4).all(|dy| bedrock(sim, p.offset(0, dy, 0)))
                && [(3, 0), (-3, 0), (0, 3), (0, -3)].iter().all(|&(dx, dz)| bedrock(sim, p.offset(dx, 0, dz)))
                && bedrock(sim, p.offset(0, -1, 0))
        };
        if let Some(p) = self.dragon_fight.exit_portal
            && is_podium(self, p)
        {
            return Some(p);
        }
        let o = self.dragon_fight.origin;
        let top = self.motion_blocking_height(END_ID, o.x, o.z);
        let min = self.dims[END_ID].provider.dimension.min_y;
        let mut y = top;
        while y >= min {
            let p = BlockPos::new(o.x, y, o.z);
            if is_podium(self, p) {
                if self.dragon_fight.exit_portal.is_none() {
                    self.dragon_fight.exit_portal = Some(p);
                }
                return Some(p);
            }
            y -= 1;
        }
        None
    }

    /// `scanState`: an active exit portal means the dragon died before; a dragon without one is
    /// removed; a world whose dragon never died gets its fight.
    fn scan_state(&mut self) {
        info!("Scanning for legacy world dragon fight...");
        let active = self.has_active_exit_portal();
        if active {
            info!("Found that the dragon has been killed in this world already.");
            self.dragon_fight.previously_killed = true;
        } else {
            info!("Found that the dragon has not yet been killed in this world.");
            self.dragon_fight.previously_killed = false;
            if self.find_exit_portal().is_none() {
                self.spawn_exit_portal(false);
            }
        }
        let dragons = self.end_dragons();
        match dragons.first() {
            None => self.dragon_fight.dragon_killed = true,
            Some(&(id, uuid)) => {
                self.dragon_fight.dragon_uuid = Some(uuid);
                info!("Found that there's a dragon still alive ({id})");
                self.dragon_fight.dragon_killed = false;
                if !active {
                    info!("But we didn't have a portal, let's remove it.");
                    self.discard_end_entity(id);
                    self.dragon_fight.dragon_uuid = None;
                }
            }
        }
        if !self.dragon_fight.previously_killed && self.dragon_fight.dragon_killed {
            self.dragon_fight.dragon_killed = false;
        }
    }

    fn discard_end_entity(&mut self, id: i32) {
        for r in self.dims[END_ID].regions.iter_mut() {
            let (_, part) = r.cells_and_part_mut();
            if let Ok(i) = part.0.list.binary_search_by_key(&id, |e| e.id) {
                let e = &mut part.0.list[i];
                e.removed = true;
                if let Some(p) = e.phys.as_deref_mut() {
                    p.discard();
                }
            }
        }
    }

    /// `findOrCreateDragon`: a loaded dragon becomes the fight's; with none (and none known to
    /// be in an unloaded chunk) a new one appears.
    fn find_or_create_dragon(&mut self) {
        match self.end_dragons().first() {
            Some(&(_, uuid)) => {
                debug!("Haven't seen our dragon, but found another one to use.");
                self.dragon_fight.dragon_uuid = Some(uuid);
            }
            None => {
                let away = self.dragon_fight.dragon_uuid.is_some()
                    && self.dragon_fight.last_seen.is_some_and(|c| self.dims[END_ID].regions.chunk(c).is_none());
                if !away {
                    debug!("Haven't seen the dragon, respawning it");
                    self.create_new_dragon();
                }
            }
        }
    }

    /// `createNewDragon`: in the holding pattern at (0, 128, 0), facing a random way.
    fn create_new_dragon(&mut self) -> Option<kiln_entity::Entity> {
        let o = self.dragon_fight.origin;
        let at = BlockPos::new(o.x, DRAGON_SPAWN_Y + o.y, o.z);
        self.load_area(END_ID, at, 0);
        let world_seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let uuid = entities::fresh_uuid(world_seed ^ 0x6472_6167, self.game_time, self.next_entity_id).as_u128();
        let yaw = self.dragon_fight.random.next_float() * 360.0;
        let pos = Vec3::new(at.x as f64, at.y as f64, at.z as f64);
        let dragon = kiln_entity::mob::kinds::ender_dragon::new_for_fight(0, uuid, entities::seed_for_uuid(uuid), pos, yaw, ebp(o));
        self.dims[END_ID].spawns.push(Spawn::loaded(dragon.clone())?);
        self.materialize_spawns();
        self.dragon_fight.dragon_uuid = Some(uuid);
        self.dragon_fight.last_seen = Some(ChunkPos::of_block(at.x, at.z));
        info!("an ender dragon appeared at {at:?}");
        Some(dragon)
    }

    /// `updateCrystalCount`: end crystals over the pillars' tops.
    fn update_crystal_count(&mut self) {
        let spikes = kiln_worldgen::end::spikes_for_seed(self.config.noise.as_ref().map_or(0, |n| n.seed));
        let n = self
            .end_entities()
            .filter(|e| e.kind.name == "minecraft:end_crystal")
            .map(|e| spikes.iter().filter(|s| on_spike(s, e.pos)).count() as i32)
            .sum();
        self.dragon_fight.alive_crystals = n;
        self.dragon_fight.ticks_since_crystals_scanned = 0;
        debug!("Found {n} end crystals still alive");
    }

    /// `setDragonKilled`: the bar goes, the exit portal opens, a gateway appears, and the egg the
    /// first time.
    fn set_dragon_killed(&mut self, uuid: u128) {
        if Some(uuid) != self.dragon_fight.dragon_uuid {
            return;
        }
        self.boss_progress(0.0);
        self.boss_visible(false);
        self.spawn_exit_portal(true);
        self.spawn_new_gateway();
        if !self.dragon_fight.previously_killed {
            let o = self.dragon_fight.origin;
            let y = self.motion_blocking_height(END_ID, o.x, o.z);
            self.set_level_block(END_ID, BlockPos::new(o.x, y, o.z), block::DRAGON_EGG, kiln_blocks::flags::ALL);
        }
        self.dragon_fight.previously_killed = true;
        self.dragon_fight.dragon_killed = true;
        info!("the ender dragon was killed");
    }

    /// `spawnNewGateway`: the last gateway of the list (the `end_gateway_delayed` feature).
    fn spawn_new_gateway(&mut self) {
        let Some(index) = self.dragon_fight.gateways.pop() else { return };
        let at = kb(kiln_worldgen::end::end_gateway_position(index));
        self.load_area(END_ID, at, 2);
        self.with_level_in(END_ID, [at.x, at.y, at.z], |level| {
            kiln_blocks::Level::effect(level, kiln_blocks::level::Effect::LevelEvent { id: 3000, pos: at, data: 0 });
        });
        self.place_gateway(at, None);
    }

    /// `spawnExitPortal(activated)`: the podium (`END_PODIUM_ACTIVE` / `_INACTIVE`) where it was
    /// first placed, found the first time below the origin's surface.
    pub(crate) fn spawn_exit_portal(&mut self, active: bool) {
        let o = self.dragon_fight.origin;
        self.load_area(END_ID, BlockPos::new(o.x, 0, o.z), 8);
        if self.dragon_fight.exit_portal.is_none() {
            let min_y = self.dims[END_ID].kind.min_y;
            let d = self.dims[END_ID].provider.dimension;
            let mut surface = min_y;
            for y in (d.min_y..d.min_y + d.height).rev() {
                if kiln_data::block_props::motion_blocking_no_leaves(self.block_loading(END_ID, BlockPos::new(o.x, y, o.z))) {
                    surface = y + 1;
                    break;
                }
            }
            let origin = kiln_worldgen::end::exit_portal_origin(surface, min_y, |p| kiln_blocks::state::is(self.block_in_level(END_ID, kb(p)), block::BEDROCK));
            self.dragon_fight.exit_portal = Some(kb(origin));
        }
        let origin = wg(self.dragon_fight.exit_portal.unwrap());
        for b in kiln_worldgen::end::end_podium_blocks(origin, active) {
            let p = kb(b.pos);
            let state = b.state;
            self.with_level_in(END_ID, [p.x, p.y, p.z], |level| {
                use kiln_blocks::Level;
                if b.drop_previous && !kiln_blocks::state::same_block(level.block(p), state) {
                    kiln_blocks::destroy_block(level, p, true, kiln_blocks::flags::LIMIT);
                }
                kiln_blocks::set_block(level, p, state, kiln_blocks::flags::ALL);
            });
        }
        info!("placed the End's exit portal ({}) at {:?}", if active { "active" } else { "inactive" }, self.dragon_fight.exit_portal);
    }

    // ------------------------------------------------------------------ crystals

    /// `onCrystalDestroyed`: a crystal of the respawn ritual ends it; otherwise the count is
    /// renewed and the dragon hears about it.
    fn on_crystal_destroyed(&mut self, crystal: i32, uuid: u128, pos: Vec3, attacker: Option<i32>) {
        if self.dragon_fight.respawn_stage.is_some() && self.dragon_fight.respawn_crystals.contains(&uuid) {
            self.abort_respawn();
            return;
        }
        self.update_crystal_count();
        let Some(dragon) = self.dragon_fight.dragon_uuid.and_then(|u| self.end_entities().find(|e| e.uuid.as_u128() == u)).map(|e| e.id) else {
            return;
        };
        self.with_end_entity(dragon, 0x6372_7973, |e, level| {
            kiln_entity::mob::kinds::ender_dragon::on_crystal_destroyed(e, level, crystal, pos, attacker);
        });
    }

    /// `abortRespawnSequence`.
    fn abort_respawn(&mut self) {
        debug!("Aborting respawn sequence");
        self.dragon_fight.respawn_stage = None;
        self.dragon_fight.respawn_time = 0;
        self.reset_spike_crystals();
        self.spawn_exit_portal(true);
    }

    /// `resetSpikeCrystals`: the pillars' crystals may be hurt again and show no beam.
    fn reset_spike_crystals(&mut self) {
        let spikes = kiln_worldgen::end::spikes_for_seed(self.config.noise.as_ref().map_or(0, |n| n.seed));
        self.update_end_crystals(|e| spikes.iter().any(|s| on_spike(s, [e.x(), e.y(), e.z()])), |e, c| {
            e.invulnerable = false;
            c.beam_target = None;
        });
    }

    /// Changes the End's crystals that `select` picks.
    fn update_end_crystals(
        &mut self,
        select: impl Fn(&kiln_entity::Entity) -> bool,
        mut f: impl FnMut(&mut kiln_entity::Entity, &mut kiln_entity::ext_entity::end_crystal::EndCrystal),
    ) {
        for r in self.dims[END_ID].regions.iter_mut() {
            let (_, part) = r.cells_and_part_mut();
            for e in part.0.list.iter_mut().filter(|e| !e.removed) {
                let Some(phys) = e.phys.as_deref_mut() else { continue };
                if phys.type_name != "minecraft:end_crystal" || !select(phys) {
                    continue;
                }
                let mut kind = std::mem::replace(&mut phys.kind, kiln_entity::EntityKind::Other { type_name: "minecraft:end_crystal" });
                if let kiln_entity::EntityKind::Ext(x) = &mut kind
                    && let Some(c) = x.as_any_mut().downcast_mut::<kiln_entity::ext_entity::end_crystal::EndCrystal>()
                {
                    f(phys, c);
                }
                phys.kind = kind;
            }
        }
    }

    /// The respawn ritual's crystals still there.
    fn respawn_crystal_ids(&self) -> Vec<u128> {
        let want = &self.dragon_fight.respawn_crystals;
        let mut v: Vec<u128> = self
            .end_entities()
            .filter(|e| e.kind.name == "minecraft:end_crystal" && want.contains(&e.uuid.as_u128()))
            .map(|e| e.uuid.as_u128())
            .collect();
        v.sort_unstable_by_key(|u| want.iter().position(|w| w == u));
        v
    }

    fn set_beams(&mut self, crystals: &[u128], target: Option<BlockPos>) {
        let crystals = crystals.to_vec();
        self.update_end_crystals(|e| crystals.contains(&e.uuid), |_, c| c.beam_target = target.map(ebp));
    }

    /// `tryRespawn`: after the dragon died, four crystals on the exit portal's rim (three blocks
    /// out from its center, one above) start the ritual.
    fn try_respawn(&mut self) {
        if !self.dragon_fight.dragon_killed || self.dragon_fight.respawn_stage.is_some() {
            return;
        }
        if self.dragon_fight.exit_portal.is_none() {
            debug!("Tried to respawn, but need to find the portal first.");
            if self.find_exit_portal().is_none() {
                debug!("Couldn't find a portal, so we made one.");
                self.spawn_exit_portal(true);
            }
        }
        let Some(location) = self.dragon_fight.exit_portal else { return };
        let center = location.offset(0, 1, 0);
        let mut crystals = Vec::new();
        // `Plane.HORIZONTAL`: north, east, south, west.
        for (dx, dz) in [(0, -3), (3, 0), (0, 3), (-3, 0)] {
            let b = center.offset(dx, 0, dz);
            let (lo, hi) = ([b.x as f64, b.y as f64, b.z as f64], [b.x as f64 + 1.0, b.y as f64 + 1.0, b.z as f64 + 1.0]);
            let mut found: Vec<(i32, u128)> = self
                .end_entities()
                .filter(|e| e.kind.name == "minecraft:end_crystal")
                .filter(|e| {
                    let (min, max, _) = e.body();
                    (0..3).all(|i| min[i] < hi[i] && max[i] > lo[i])
                })
                .map(|e| (e.id, e.uuid.as_u128()))
                .collect();
            if found.is_empty() {
                return;
            }
            found.sort_unstable();
            crystals.extend(found.into_iter().map(|(_, u)| u));
        }
        debug!("Found all crystals, respawning dragon.");
        self.respawn_dragon(crystals);
    }

    /// `respawnDragon`: the podium's bedrock and portal turn to end stone, an inactive podium is
    /// built and the ritual starts.
    fn respawn_dragon(&mut self, crystals: Vec<u128>) {
        if !self.dragon_fight.dragon_killed || self.dragon_fight.respawn_stage.is_some() {
            return;
        }
        while let Some(p) = self.find_exit_portal() {
            for dx in -3..=3 {
                for dz in -3..=3 {
                    for dy in -1..=3 {
                        let q = p.offset(dx, dy, dz);
                        let s = self.block_loading(END_ID, q);
                        if kiln_blocks::state::is(s, block::BEDROCK) || kiln_blocks::state::is(s, block::END_PORTAL) {
                            self.set_level_block(END_ID, q, block::END_STONE, kiln_blocks::flags::ALL);
                        }
                    }
                }
            }
            if self.find_exit_portal() == Some(p) {
                break;
            }
        }
        self.dragon_fight.respawn_stage = Some(RespawnStage::Start);
        self.dragon_fight.respawn_time = 0;
        self.spawn_exit_portal(false);
        self.dragon_fight.respawn_crystals = crystals;
        info!("the dragon respawn ritual started");
    }

    /// `setRespawnStage`: the end of the ritual makes the new dragon (and its summoners get
    /// `summoned_entity`).
    fn set_respawn_stage(&mut self, stage: RespawnStage) {
        self.dragon_fight.respawn_time = 0;
        if stage != RespawnStage::End {
            self.dragon_fight.respawn_stage = Some(stage);
            return;
        }
        self.dragon_fight.respawn_stage = None;
        self.dragon_fight.dragon_killed = false;
        if let Some(dragon) = self.create_new_dragon() {
            let seen = kiln_entity::level::Seen::of(&dragon);
            let criterion = kiln_entity::level::Criterion::SummonedEntity { entity: seen };
            for c in self.dragon_fight.boss.players.clone() {
                if let Some(p) = self.players.get_mut(&c) {
                    p.entity_criterion(DIMENSIONS[END_ID].0, &criterion);
                }
            }
        }
    }

    /// `DragonRespawnStage.tick`.
    fn respawn_stage_tick(&mut self, stage: RespawnStage, crystals: &[u128], time: i32) {
        let beam = BlockPos::new(0, 128, 0);
        match stage {
            RespawnStage::Start => {
                self.set_beams(crystals, Some(beam));
                self.set_respawn_stage(RespawnStage::PreparingToSummonPillars);
            }
            RespawnStage::PreparingToSummonPillars => {
                if time < 100 {
                    if time == 0 || time == 50 || time == 51 || time == 52 || time >= 95 {
                        self.fight_level_event(3001, beam);
                    }
                } else {
                    self.set_respawn_stage(RespawnStage::SummoningPillars);
                }
            }
            RespawnStage::SummoningPillars => {
                let start = time % 40 == 0;
                let end = time % 40 == 39;
                if start || end {
                    let spikes = kiln_worldgen::end::spikes_for_seed(self.config.noise.as_ref().map_or(0, |n| n.seed));
                    let index = (time / 40) as usize;
                    if let Some(&spike) = spikes.get(index) {
                        if start {
                            self.set_beams(crystals, Some(BlockPos::new(spike.center_x, spike.height + 1, spike.center_z)));
                        } else {
                            self.rebuild_spike(spike);
                        }
                    } else if start {
                        self.set_respawn_stage(RespawnStage::SummoningDragon);
                    }
                }
            }
            RespawnStage::SummoningDragon => {
                if time >= 100 {
                    self.set_respawn_stage(RespawnStage::End);
                    self.reset_spike_crystals();
                    self.set_beams(crystals, None);
                    for &u in crystals {
                        let Some((id, pos)) = self.end_entities().find(|e| e.uuid.as_u128() == u).map(|e| (e.id, e.pos)) else { continue };
                        let at = BlockPos::new(pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32);
                        self.with_end_level(at, 0x7265_7370, |level| {
                            kiln_entity::explosion::explode(level, Some(id), Vec3::new(pos[0], pos[1], pos[2]), 6.0, false, kiln_entity::explosion::Interaction::Keep);
                        });
                        self.discard_end_entity(id);
                    }
                } else if time >= 80 {
                    self.fight_level_event(3001, beam);
                } else if time == 0 {
                    self.set_beams(crystals, Some(beam));
                } else if time < 5 {
                    self.fight_level_event(3001, beam);
                }
            }
            RespawnStage::End => {}
        }
    }

    fn fight_level_event(&mut self, id: i32, pos: BlockPos) {
        self.load_area(END_ID, pos, 0);
        self.with_level_in(END_ID, [pos.x, pos.y, pos.z], |level| {
            kiln_blocks::Level::effect(level, kiln_blocks::level::Effect::LevelEvent { id, pos, data: 0 });
        });
    }

    /// The ritual's end of a pillar's 40 ticks: the space around its top cleared, an explosion,
    /// the pillar rebuilt with an invulnerable crystal beaming at (0, 128, 0).
    fn rebuild_spike(&mut self, s: kiln_worldgen::end::EndSpike) {
        let top = BlockPos::new(s.center_x, s.height, s.center_z);
        self.load_area(END_ID, top, 10);
        for dy in -10..=10 {
            for dz in -10..=10 {
                for dx in -10..=10 {
                    let p = top.offset(dx, dy, dz);
                    if !kiln_blocks::state::is(self.block_in_level(END_ID, p), block::AIR) {
                        self.set_level_block(END_ID, p, block::AIR, kiln_blocks::flags::ALL);
                    }
                }
            }
        }
        let at = Vec3::new(s.center_x as f64 + 0.5, s.height as f64, s.center_z as f64 + 0.5);
        self.with_end_level(top, 0x7069_6c6c, |level| {
            let griefing = kiln_entity::explosion::Interaction::DestroyWithDecay;
            kiln_entity::explosion::explode(level, None, at, 5.0, false, griefing);
        });
        let min_y = self.dims[END_ID].kind.min_y;
        let mut writes = Vec::new();
        kiln_worldgen::end::spike_with(s, min_y, &mut |p, state| writes.push((kb(p), state)));
        for (p, state) in writes {
            if self.block_in_level(END_ID, p) != state {
                self.set_level_block(END_ID, p, state, kiln_blocks::flags::ALL);
            }
        }
        let crystal = BlockPos::new(s.center_x, s.height + 1, s.center_z);
        let yaw = self.dragon_fight.random.next_float() * 360.0;
        let seed = self.dragon_fight.random.next_long();
        let mut e = kiln_entity::ext_entity::end_crystal::new(0, Vec3::new(crystal.x as f64 + 0.5, crystal.y as f64, crystal.z as f64 + 0.5), true, seed);
        e.y_rot = yaw;
        e.invulnerable = true;
        if let Some(c) = kiln_entity::ext_entity::end_crystal::get_mut(&mut e) {
            c.beam_target = Some(kiln_entity::math::BlockPos::new(0, 128, 0));
        }
        if let Some(sp) = Spawn::loaded(e) {
            self.dims[END_ID].spawns.push(sp);
        }
        self.set_level_block(END_ID, crystal.offset(0, -1, 0), block::BEDROCK, kiln_blocks::flags::ALL);
        self.set_level_block(END_ID, crystal, block::FIRE, kiln_blocks::flags::ALL);
        self.materialize_spawns();
    }

    // ------------------------------------------------------------------ serial entity work

    /// Runs `f` with the End's region around `at` as an entity level (explosions), then carries
    /// out what happened.
    fn with_end_level<R>(&mut self, at: BlockPos, salt: u64, f: impl FnOnce(&mut dyn kiln_entity::EntityLevel) -> R) -> Option<R> {
        self.end_region_work(ChunkPos::of_block(at.x, at.z), |ents, level, players, spawns, deaths| {
            entities::with_level(ents, level, players, spawns, deaths, salt, f)
        })
    }

    /// Runs `f` on End entity `id` in its region (the dragon hearing of a crystal).
    fn with_end_entity<R>(&mut self, id: i32, salt: u64, f: impl FnOnce(&mut kiln_entity::Entity, &mut dyn kiln_entity::EntityLevel) -> R) -> Option<R> {
        let pos = self.end_entity(id)?.pos;
        self.end_region_work(entities::chunk_of(pos), |ents, level, players, spawns, deaths| {
            entities::with_entity(ents, level, players, id, spawns, deaths, salt, f)
        })?
    }

    /// The End's region owning `chunk` with its players, serially.
    fn end_region_work<R>(
        &mut self,
        chunk: ChunkPos,
        f: impl FnOnce(&mut entities::Entities, &mut RegionLevel, &mut [&mut Player], &mut Vec<Spawn>, &mut Vec<health::Death>) -> R,
    ) -> Option<R> {
        let env = self.block_env(END_ID);
        let mut deaths = Vec::new();
        let result = {
            let Sim { dims, players, .. } = self;
            let d = &mut dims[END_ID];
            let region = d.regions.at_mut(chunk.cell())?;
            let id = region.id();
            let (cells, part) = region.cells_and_part_mut();
            let (ents, blks) = (&mut part.0, &mut part.1);
            let bodies = blocks::entity_boxes(players.values().filter(|p| p.dim == END_ID && p.region == id), ents);
            let mut out = BlockOut::default();
            let r = {
                let mut here: Vec<&mut Player> = players.values_mut().filter(|p| p.dim == END_ID && p.region == id).collect();
                here.sort_unstable_by_key(|p| p.conn);
                let mut level = RegionLevel { cells: &mut *cells, blocks: blks, env: &env, out: &mut out, bodies: &bodies, actor: None };
                f(ents, &mut level, &mut here, &mut d.spawns, &mut deaths)
            };
            let mut everyone: Vec<&mut Player> = players.values_mut().filter(|p| p.dim == END_ID).collect();
            blocks::finish(cells, out, &mut everyone, &mut d.spawns, &env);
            r
        };
        self.announce_deaths(deaths);
        self.materialize_spawns();
        Some(result)
    }
}

/// `BottleItem.use` near a cloud of the dragon's breath (owned by an ender dragon, within two
/// blocks of the player's box): the cloud shrinks by half a block and the bottle fills with
/// dragon's breath. Returns false when the player does not hold a glass bottle there or no
/// such cloud is near (other uses of the bottle are not Kiln's yet).
pub(crate) fn bottle_breath(ents: &mut entities::Entities, p: &mut Player, off_hand: bool, spawns: &mut Vec<Spawn>, env: &blocks::BlockEnv) -> bool {
    use kiln_entity::ext_entity::area_effect_cloud::AreaEffectCloud;
    use kiln_item::component::EquipmentSlot;
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    if p.dead || p.game_mode == 3 || p.inv.equipped(slot).item_name() != "minecraft:glass_bottle" {
        return false;
    }
    let h = if p.sneaking { 1.5 } else { 1.8 };
    let (lo, hi) = ([p.pos[0] - 2.3, p.pos[1] - 2.0, p.pos[2] - 2.3], [p.pos[0] + 2.3, p.pos[1] + h + 2.0, p.pos[2] + 2.3]);
    let dragons: Vec<i32> = ents.list.iter().filter(|e| !e.removed && e.kind.name == "minecraft:ender_dragon").map(|e| e.id).collect();
    let cloud = ents.list.iter_mut().filter(|e| !e.removed).find(|e| {
        let (min, max, _) = e.body();
        let owner = e.phys.as_deref().and_then(|x| kiln_entity::ext_entity::get::<AreaEffectCloud>(x)).and_then(|c| c.owner);
        owner.is_some_and(|o| dragons.contains(&o)) && (0..3).all(|i| min[i] < hi[i] && max[i] > lo[i])
    });
    let Some(cloud) = cloud else { return false };
    if let Some(c) = cloud.phys.as_deref_mut().and_then(kiln_entity::ext_entity::get_mut::<AreaEffectCloud>) {
        c.radius = (c.radius - 0.5).clamp(0.0, 32.0);
    }
    if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", "minecraft:item.bottle.fill_dragonbreath") {
        let seed = crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, 0x6272_6561);
        let pkt = kiln_proto::packets::world_fx::sound(&kiln_proto::packets::world_fx::Sound::Registered(id), kiln_proto::packets::world_fx::SoundSource::Neutral, p.pos, 1.0, 1.0, seed);
        p.send(pkt);
    }
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, p.inv.equipped(slot).item()), 1);
    // `ItemUtils.createFilledResult`.
    let Some(mut filled) = kiln_item::ItemStack::of("minecraft:dragon_breath", 1) else { return true };
    if p.game_mode == 1 {
        let has = (0..kiln_inventory::Container::size(&p.inv)).any(|j| kiln_inventory::stack::matches(kiln_inventory::Container::item(&p.inv, j), &filled));
        if !has {
            p.add_to_inventory(&mut filled);
        }
    } else {
        let index = kiln_inventory::inventory::equipment_index(slot, p.inv.selected);
        let held = kiln_inventory::Container::item_mut(&mut p.inv, index);
        held.shrink(1);
        if held.is_empty() {
            *held = filled;
        } else if p.add_to_inventory(&mut filled) == 0 {
            spawns.push(crate::mobs::drop_item(filled, p.pos, p.entity_id as u64));
        }
    }
    true
}

/// Whether an end crystal at `pos` stands over pillar `s` (`getTopBoundingBox`: the pillar's
/// square over the whole height, against the crystal's 2x2 box).
fn on_spike(s: &kiln_worldgen::end::EndSpike, p: [f64; 3]) -> bool {
    let (x0, x1) = ((s.center_x - s.radius) as f64, (s.center_x + s.radius) as f64);
    let (z0, z1) = ((s.center_z - s.radius) as f64, (s.center_z + s.radius) as f64);
    p[0] - 1.0 < x1 && p[0] + 1.0 > x0 && p[2] - 1.0 < z1 && p[2] + 1.0 > z0
}
