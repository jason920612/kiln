//! Players, level data and entity chunks in the world save: loaded on startup, join and
//! chunk load, saved on leave, chunk unload, autosave and shutdown.

use crate::{Player, Sim, entities};
use kiln_entity::persist::{self, LoadError};
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_storage::{LevelState, LevelStore, PlayerData, PlayerStore, WorldSpawn};
use kiln_world::{Blocks, ChunkPos};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::warn;
use uuid::Uuid;

const OVERWORLD: &str = "minecraft:overworld";
/// Vanilla's `respawn_radius` default.
const DEFAULT_RESPAWN_RADIUS: i64 = 10;
/// Game mode of new players when the world has no `level.dat` (Kiln's flat test worlds).
const KILN_DEFAULT_GAME_MODE: u8 = 1;
const ADVENTURE: u8 = 2;

pub(crate) struct Storage {
    pub level: LevelStore,
    pub players: PlayerStore,
    pub dir: std::path::PathBuf,
}

impl Storage {
    pub fn open(world_dir: &Path) -> Self {
        Self { level: LevelStore::open(world_dir), players: PlayerStore::new(world_dir), dir: world_dir.to_owned() }
    }
}

/// Saved data ids (`data/minecraft/<id>.dat`).
const SCOREBOARD: &str = "scoreboard";
const BOSS_EVENTS: &str = "custom_boss_events";

/// How a joining player starts: their saved state, or vanilla's defaults for a new player.
pub(crate) struct Joining {
    pub pos: [f64; 3],
    pub rot: [f32; 2],
    pub game_mode: u8,
    pub inv: kiln_inventory::PlayerInventory,
    pub inv_extra: kiln_inventory::persist::PlayerItemsExtra,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub food_timer: i32,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    /// `Fire`, `Air`, `AbsorptionAmount` and `active_effects`.
    pub fire_ticks: i32,
    pub air: i32,
    pub absorption: f32,
    pub effects: std::collections::BTreeMap<i32, crate::effects::Effect>,
    pub respawn: Option<[i32; 3]>,
    pub respawn_dim: crate::DimId,
    /// `respawn.yaw` and `respawn.forced`.
    pub respawn_angle: f32,
    pub respawn_forced: bool,
    /// The saved level (`Dimension`).
    pub dim: crate::DimId,
    /// `PortalCooldown` and `seenCredits`.
    pub portal_cooldown: i32,
    pub seen_credits: bool,
    pub saved: PlayerData,
}

impl Sim {
    /// The default game mode for new players: the world's `GameType`.
    pub(crate) fn default_game_mode(&self) -> u8 {
        self.storage.as_ref().and_then(|s| s.level.game_type()).unwrap_or(KILN_DEFAULT_GAME_MODE)
    }

    /// Vanilla `PrepareSpawnTask`: saved position and rotation if present, else the spawn
    /// finder around the world spawn and the world spawn's angles.
    pub(crate) fn joining(&mut self, uuid: Uuid) -> Joining {
        let saved = self.storage.as_ref().and_then(|s| s.players.load(uuid)).unwrap_or_default();
        let dim = match saved.dimension.as_deref() {
            None => crate::OVERWORLD_ID,
            Some(d) => crate::dim_id(d).unwrap_or_else(|| {
                // Vanilla falls back to the spawn dimension, keeping the coordinates.
                warn!("player {uuid} was in {d}, which Kiln does not run; placing them in the overworld");
                crate::OVERWORLD_ID
            }),
        };
        let (pos, dim) = match saved.pos {
            Some(p) => (p, dim),
            None => (self.new_player_position(uuid), crate::OVERWORLD_ID),
        };
        let respawn_dim = saved
            .raw()
            .get("respawn")
            .and_then(|r| r.get("dimension"))
            .and_then(Tag::as_str)
            .and_then(crate::dim_id)
            .unwrap_or(crate::OVERWORLD_ID);
        let (mut inv, inv_extra) = kiln_inventory::persist::load_player_inventory(saved.raw());
        inv.selected = saved.selected_slot as usize;
        Joining {
            pos,
            rot: saved.rot.unwrap_or(self.spawn_rot),
            game_mode: saved.game_mode.unwrap_or_else(|| self.default_game_mode()),
            inv,
            inv_extra,
            health: saved.raw().get("Health").and_then(Tag::as_f64).map_or(crate::health::MAX_HEALTH, |h| h as f32),
            food: saved.raw().get("foodLevel").and_then(Tag::as_i64).map_or(20, |f| f as i32),
            saturation: saved.raw().get("foodSaturationLevel").and_then(Tag::as_f64).map_or(5.0, |s| s as f32),
            exhaustion: saved.raw().get("foodExhaustionLevel").and_then(Tag::as_f64).map_or(0.0, |e| e as f32),
            food_timer: saved.raw().get("foodTickTimer").and_then(Tag::as_i64).map_or(0, |t| t as i32),
            xp_level: saved.raw().get("XpLevel").and_then(Tag::as_i64).map_or(0, |v| v as i32),
            xp_progress: saved.raw().get("XpP").and_then(Tag::as_f64).map_or(0.0, |v| v as f32),
            xp_total: saved.raw().get("XpTotal").and_then(Tag::as_i64).map_or(0, |v| v as i32),
            fire_ticks: saved.raw().get("Fire").and_then(Tag::as_i64).map_or(-crate::hazards::FIRE_IMMUNE_TICKS, |f| f as i32),
            air: saved.raw().get("Air").and_then(Tag::as_i64).map_or(crate::hazards::MAX_AIR, |a| a as i32),
            absorption: saved.raw().get("AbsorptionAmount").and_then(Tag::as_f64).map_or(0.0, |a| a as f32),
            effects: saved.raw().get("active_effects").map(crate::effects::load_effects).unwrap_or_default(),
            respawn: saved.respawn,
            respawn_dim,
            respawn_angle: match saved.raw().get("respawn").and_then(|r| r.get("yaw")) {
                Some(Tag::Float(y)) => *y,
                _ => 0.0,
            },
            respawn_forced: saved.raw().get("respawn").and_then(|r| r.get("forced")).and_then(Tag::as_i64).is_some_and(|f| f != 0),
            dim,
            portal_cooldown: saved.raw().get("PortalCooldown").and_then(Tag::as_i64).map_or(0, |c| c as i32),
            seen_credits: saved.raw().get("seenCredits").and_then(Tag::as_i64) == Some(1),
            saved,
        }
    }

    /// Where a player without a saved position appears (vanilla `PlayerSpawnFinder`). Vanilla
    /// starts the candidate walk at a random index; Kiln derives it from the UUID.
    pub(crate) fn new_player_position(&mut self, uuid: Uuid) -> [f64; 3] {
        let level = self.storage.as_ref().map(|s| &s.level);
        if level.and_then(LevelStore::game_type) == Some(ADVENTURE) {
            return kiln_world::spawn::free_spawn_at(&mut self.dims[crate::OVERWORLD_ID], self.spawn);
        }
        let radius = match self.commands.game_rules.get("minecraft:respawn_radius") {
            Some(kiln_command::GameRuleValue::Int(r)) => *r as i64,
            _ => level.and_then(|l| l.game_rule("minecraft:respawn_radius")).unwrap_or(DEFAULT_RESPAWN_RADIUS),
        };
        let (hi, lo) = uuid.as_u64_pair();
        let offset = ((hi ^ lo) % 1024) as u32;
        kiln_world::spawn::find_spawn(&mut self.dims[crate::OVERWORLD_ID], self.spawn, radius.clamp(0, i32::MAX as i64) as i32, offset)
    }

    pub(crate) fn save_player(&self, p: &Player) {
        let Some(storage) = &self.storage else { return };
        let nbt = self.player_nbt(p);
        if let Err(e) = storage.players.save_nbt(p.uuid, &nbt) {
            warn!("failed to save player data for {}: {e}", p.name);
        }
        self.save_stats(p);
        self.save_player_advancements(p);
    }

    /// The player's saved compound (`ServerPlayer.saveWithoutId` as far as Kiln models it,
    /// with everything else as it was loaded).
    pub(crate) fn player_nbt(&self, p: &Player) -> Tag {
        let mut data = p.saved.clone();
        data.pos = Some(p.pos);
        data.rot = Some(p.rot);
        data.on_ground = p.on_ground;
        data.game_mode = Some(p.game_mode);
        data.dimension = Some(crate::DIMENSIONS[p.dim].0.to_owned());
        data.selected_slot = p.inv.selected as u8;
        data.respawn = p.respawn;
        data.respawn_dimension = Some(crate::DIMENSIONS[p.respawn_dim].0.to_owned());
        let mut nbt = data.to_nbt(p.uuid);
        // `ServerPlayer.RespawnConfig`: the facing and whether the point is forced.
        if let (Some(pos), Tag::Compound(fields)) = (p.respawn, &mut nbt) {
            let r = Tag::Compound(vec![
                ("dimension".into(), Tag::String(crate::DIMENSIONS[p.respawn_dim].0.into())),
                ("pos".into(), Tag::IntArray(pos.to_vec())),
                ("yaw".into(), Tag::Float(p.respawn_angle)),
                ("pitch".into(), Tag::Float(0.0)),
                ("forced".into(), Tag::Byte(p.respawn_forced as i8)),
            ]);
            match fields.iter_mut().find(|(k, _)| k == "respawn") {
                Some((_, v)) => *v = r,
                None => fields.push(("respawn".into(), r)),
            }
        }
        kiln_inventory::persist::save_player_inventory(&p.inv, &p.inv_extra, &mut nbt);
        p.containers.save_into(&mut nbt);
        if let Tag::Compound(fields) = &mut nbt {
            for (key, value) in [
                ("Health", Tag::Float(p.health)),
                ("foodLevel", Tag::Int(p.food)),
                ("foodSaturationLevel", Tag::Float(p.saturation)),
                ("foodExhaustionLevel", Tag::Float(p.exhaustion)),
                ("foodTickTimer", Tag::Int(p.food_timer)),
                ("XpLevel", Tag::Int(p.xp_level)),
                ("XpP", Tag::Float(p.xp_progress)),
                ("XpTotal", Tag::Int(p.xp_total)),
                // `Entity.saveWithoutId` and `LivingEntity.addAdditionalSaveData`.
                ("Fire", Tag::Short(p.fire_ticks as i16)),
                ("Air", Tag::Short(p.air as i16)),
                ("AbsorptionAmount", Tag::Float(p.absorption)),
                ("PortalCooldown", Tag::Int(p.portal_cooldown)),
                ("seenCredits", Tag::Byte(p.seen_credits as i8)),
            ] {
                match fields.iter_mut().find(|(k, _)| k == key) {
                    Some((_, v)) => *v = value,
                    None => fields.push((key.to_owned(), value)),
                }
            }
            fields.retain(|(k, _)| k != "active_effects" && k != "recipeBook");
            fields.push(("recipeBook".to_owned(), p.recipe_book.to_nbt()));
            if let Some(list) = p.effects_nbt() {
                fields.push(("active_effects".to_owned(), list));
            }
        }
        nbt
    }

    /// Loads the scoreboard and custom boss bars (`data/minecraft/scoreboard.dat`,
    /// `custom_boss_events.dat`) when the world has them.
    pub(crate) fn load_scoreboard(&mut self) {
        let Some(storage) = &self.storage else { return };
        if let Some(data) = kiln_storage::saved_data::read(&storage.dir, SCOREBOARD) {
            self.commands.scoreboard.load_nbt(&data);
        }
        if let Some(data) = kiln_storage::saved_data::read(&storage.dir, BOSS_EVENTS) {
            self.commands.bossbars.load_nbt(&data);
        }
    }

    /// Writes the scoreboard and boss bars if they changed (vanilla saves dirty saved data).
    pub(crate) fn save_scoreboard(&mut self) {
        let Some(storage) = &self.storage else { return };
        let dir = storage.dir.clone();
        if self.commands.scoreboard.take_dirty()
            && let Err(e) = kiln_storage::saved_data::write(&dir, SCOREBOARD, self.commands.scoreboard.to_nbt())
        {
            warn!("failed to save the scoreboard: {e}");
        }
        if self.commands.bossbars.take_dirty()
            && let Err(e) = kiln_storage::saved_data::write(&dir, BOSS_EVENTS, self.commands.bossbars.to_nbt())
        {
            warn!("failed to save boss bars: {e}");
        }
    }

    pub(crate) fn save_level(&mut self) {
        let state = LevelState {
            game_time: self.game_time,
            day_time: self.day_time,
            spawn: WorldSpawn { dimension: OVERWORLD.to_owned(), pos: self.spawn, yaw: self.spawn_rot[0], pitch: self.spawn_rot[1] },
            data_packs: Some((self.commands.packs.selected.clone(), self.commands.packs.disabled.clone())),
        };
        let Some(storage) = &mut self.storage else { return };
        if let Err(e) = storage.level.save(&state) {
            warn!("failed to save level data: {e}");
        }
    }

    /// Network ids of possible entity owners (players) to their UUIDs.
    pub(crate) fn owner_uuids(&self) -> HashMap<i32, u128> {
        self.players.values().map(|p| (p.entity_id, p.uuid.as_u128())).collect()
    }

    /// Viewers forget entities that left the simulation with their chunk.
    pub(crate) fn forget_entities(&mut self, gone: Vec<(i32, Vec<ConnId>)>) {
        for (id, viewers) in gone {
            let pkt = kiln_proto::packets::entity::remove_entities(&[id]);
            for v in viewers {
                if let Some(p) = self.players.get_mut(&v) {
                    p.send(pkt.clone());
                }
            }
        }
    }

    /// Saved data of every simulated entity, in id order (for tests and tools).
    pub fn entity_nbt(&self) -> Vec<Tag> {
        let owners = self.owner_uuids();
        let owner = |id: i32| owners.get(&id).copied();
        let mut all: Vec<&entities::Entity> =
            self.dims.iter().flat_map(|d| d.regions.iter()).flat_map(|r| r.part().0.list.iter()).filter(|e| !e.removed).collect();
        all.sort_by_key(|e| e.id);
        all.into_iter().map(|e| e.save(&owner)).collect()
    }

    /// How many entities of loaded chunks are kept as saved without being simulated.
    pub fn kept_entity_count(&self) -> usize {
        self.dims.iter().flat_map(|d| d.raw_entities.values()).map(Vec::len).sum()
    }
}

// ---------------------------------------------------------------------------- entity chunks

impl crate::Dim {
    /// `EntityStorage.loadEntities` for a chunk that just went into its region: simulated
    /// entities become spawns (ids are assigned with the tick's other spawns, in canonical
    /// order); others are kept as saved until the chunk is written again.
    pub(crate) fn load_entities(&mut self, pos: ChunkPos) {
        let Some(store) = &mut self.entity_store else { return };
        let tags = store.load(pos);
        self.add_saved_entities(pos, tags);
    }

    /// Saved entities of a chunk entering the simulation (stored, or placed by generation):
    /// simulated ones become spawns, the rest are kept as saved.
    pub(crate) fn add_saved_entities(&mut self, pos: ChunkPos, tags: Vec<Tag>) {
        if tags.is_empty() {
            return;
        }
        let mut raw = Vec::new();
        for tag in tags {
            let uuid = tag.get("UUID").and_then(persist::uuid_from_tag).unwrap_or(0);
            match persist::load(&tag, 0, entities::seed_for_uuid(uuid)) {
                Ok(e) => match entities::Spawn::loaded(e) {
                    Some(spawn) => self.spawns.push(spawn),
                    None => raw.push(tag),
                },
                Err(LoadError::Discarded) => {}
                Err(LoadError::NotSimulated) => raw.push(tag),
                Err(LoadError::Invalid(why)) => {
                    warn!("entity in chunk {pos:?} kept as saved: {why}");
                    raw.push(tag);
                }
            }
        }
        if !raw.is_empty() {
            self.raw_entities.entry(pos).or_default().extend(raw);
        }
    }

    /// Adds saved entities to a chunk that is not in a region (merging with what it has
    /// stored; an entity already stored there under the same UUID is replaced).
    pub(crate) fn stash_entities(&mut self, pos: ChunkPos, tags: Vec<Tag>) {
        let Some(store) = &mut self.entity_store else { return };
        let uuid = |t: &Tag| t.get("UUID").and_then(persist::uuid_from_tag);
        let mut stored = store.load(pos);
        stored.retain(|s| uuid(s).is_none_or(|u| !tags.iter().any(|t| uuid(t) == Some(u))));
        stored.extend(tags);
        store.store(pos, stored);
    }

    /// `EntityStorage.storeEntities`. With `all` (autosave, shutdown) every loaded chunk is
    /// written with its entities; otherwise (after unloads) the chunks in `unloaded` and those
    /// of entities no longer in a loaded chunk. Entities outside loaded chunks leave the
    /// simulation; returns their ids and viewers, who must forget them.
    pub(crate) fn store_entities(&mut self, unloaded: &[ChunkPos], all: bool, owners: &HashMap<i32, u128>) -> Vec<(i32, Vec<ConnId>)> {
        // Without storage, entities outside loaded chunks are lost (their cells may go away).
        let storing = self.entity_store.is_some();
        let owner = |id: i32| owners.get(&id).copied();
        let mut groups: HashMap<ChunkPos, Vec<Tag>> = HashMap::new();
        let mut leaving: HashSet<i32> = HashSet::new();
        for r in self.regions.iter() {
            // Lightning bolts and fishing bobbers are never saved (`EntityType.noSave`).
            for e in r.part().0.list.iter().filter(|e| !e.removed && !matches!(e.kind.name, "minecraft:lightning_bolt" | "minecraft:fishing_bobber")) {
                let c = entities::chunk_of(e.pos);
                let loaded = self.regions.chunk(c).is_some();
                if storing && (all || !loaded) {
                    groups.entry(c).or_default().push(e.save(&owner));
                }
                if !loaded {
                    leaving.insert(e.id);
                }
            }
        }
        let mut gone = Vec::new();
        if !leaving.is_empty() {
            for r in self.regions.iter_mut() {
                r.part_mut().0.list.retain_mut(|e| {
                    if !leaving.contains(&e.id) {
                        return true;
                    }
                    gone.push((e.id, std::mem::take(&mut e.seen_by)));
                    false
                });
            }
        }
        if !storing {
            return gone;
        }
        let mut chunks: Vec<ChunkPos> = groups.keys().copied().chain(unloaded.iter().copied()).collect();
        if all {
            for r in self.regions.iter() {
                for (cell_pos, cell) in r.cells().iter() {
                    chunks.extend(cell.chunks(cell_pos).map(|(p, _)| p));
                }
            }
        }
        chunks.sort_unstable_by_key(|c| (c.x, c.z));
        chunks.dedup();
        for c in chunks {
            let mut list = groups.remove(&c).unwrap_or_default();
            let was_unloaded = unloaded.contains(&c);
            if self.regions.chunk(c).is_some() {
                list.extend(self.raw_entities.get(&c).into_iter().flatten().cloned());
            } else if was_unloaded {
                list.extend(self.raw_entities.remove(&c).unwrap_or_default());
            } else {
                // Entities that moved into a chunk not in a region join what it has stored
                // (a chunk still waiting for its region loads them with it).
                self.stash_entities(c, list);
                continue;
            }
            let store = self.entity_store.as_mut().expect("checked above");
            store.store(c, list);
            if was_unloaded {
                store.unloaded(c);
            }
        }
        gone
    }

    pub(crate) fn flush_entities(&mut self) -> std::io::Result<usize> {
        self.entity_store.as_mut().map_or(Ok(0), |s| s.flush())
    }
}
