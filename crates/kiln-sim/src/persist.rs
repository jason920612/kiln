//! Players and level data in the world save: loaded on startup and join, saved on leave,
//! autosave and shutdown.

use crate::{Player, Sim};
use kiln_storage::{LevelState, LevelStore, PlayerData, PlayerStore, WorldSpawn};
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
}

impl Storage {
    pub fn open(world_dir: &Path) -> Self {
        Self { level: LevelStore::open(world_dir), players: PlayerStore::new(world_dir) }
    }
}

/// How a joining player starts: their saved state, or vanilla's defaults for a new player.
pub(crate) struct Joining {
    pub pos: [f64; 3],
    pub rot: [f32; 2],
    pub game_mode: u8,
    pub inv: kiln_inventory::PlayerInventory,
    pub inv_extra: kiln_inventory::persist::PlayerItemsExtra,
    pub respawn: Option<[i32; 3]>,
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
        if saved.dimension.as_deref().is_some_and(|d| d != OVERWORLD) {
            // Vanilla falls back to the spawn dimension, keeping the coordinates.
            warn!("player {uuid} was in {:?}, which Kiln does not run; placing them in the overworld", saved.dimension);
        }
        let pos = match saved.pos {
            Some(p) => p,
            None => self.new_player_position(uuid),
        };
        let (mut inv, inv_extra) = kiln_inventory::persist::load_player_inventory(saved.raw());
        inv.selected = saved.selected_slot as usize;
        Joining {
            pos,
            rot: saved.rot.unwrap_or(self.spawn_rot),
            game_mode: saved.game_mode.unwrap_or_else(|| self.default_game_mode()),
            inv,
            inv_extra,
            respawn: saved.respawn,
            saved,
        }
    }

    /// Where a player without a saved position appears (vanilla `PlayerSpawnFinder`). Vanilla
    /// starts the candidate walk at a random index; Kiln derives it from the UUID.
    pub(crate) fn new_player_position(&mut self, uuid: Uuid) -> [f64; 3] {
        let level = self.storage.as_ref().map(|s| &s.level);
        if level.and_then(LevelStore::game_type) == Some(ADVENTURE) {
            return kiln_world::spawn::free_spawn_at(&mut self.dim, self.spawn);
        }
        let radius = match self.commands.game_rules.get("minecraft:respawn_radius") {
            Some(kiln_command::GameRuleValue::Int(r)) => *r as i64,
            _ => level.and_then(|l| l.game_rule("minecraft:respawn_radius")).unwrap_or(DEFAULT_RESPAWN_RADIUS),
        };
        let (hi, lo) = uuid.as_u64_pair();
        let offset = ((hi ^ lo) % 1024) as u32;
        kiln_world::spawn::find_spawn(&mut self.dim, self.spawn, radius.clamp(0, i32::MAX as i64) as i32, offset)
    }

    pub(crate) fn save_player(&self, p: &Player) {
        let Some(storage) = &self.storage else { return };
        let mut data = p.saved.clone();
        data.pos = Some(p.pos);
        data.rot = Some(p.rot);
        data.on_ground = p.on_ground;
        data.game_mode = Some(p.game_mode);
        data.dimension = Some(OVERWORLD.to_owned());
        data.selected_slot = p.inv.selected as u8;
        data.respawn = p.respawn;
        let mut nbt = data.to_nbt(p.uuid);
        kiln_inventory::persist::save_player_inventory(&p.inv, &p.inv_extra, &mut nbt);
        if let Err(e) = storage.players.save_nbt(p.uuid, &nbt) {
            warn!("failed to save player data for {}: {e}", p.name);
        }
    }

    pub(crate) fn save_level(&mut self) {
        let state = LevelState {
            game_time: self.game_time,
            day_time: self.day_time,
            spawn: WorldSpawn { dimension: OVERWORLD.to_owned(), pos: self.spawn, yaw: self.spawn_rot[0], pitch: self.spawn_rot[1] },
        };
        let Some(storage) = &mut self.storage else { return };
        if let Err(e) = storage.level.save(&state) {
            warn!("failed to save level data: {e}");
        }
    }
}
