//! What `server.properties` decides about the running game ([`crate::ServerSettings`]):
//! applied when the simulation starts, and `pause-when-empty-seconds` every tick.

use crate::Sim;
use kiln_link::ToSim;
use tracing::info;

impl Sim {
    /// `DedicatedServer.initServer` and `MinecraftServer.loadLevel`: the properties' game mode becomes the
    /// world's default one, a new world is hardcore if `hardcore` says so, and the idle timeout starts.
    pub(crate) fn apply_server_settings(&mut self) {
        let s = self.config.server.clone();
        let new_world = self.storage.as_ref().is_some_and(|st| !st.level.exists());
        if let Some(mode) = s.default_game_mode {
            info!("Default game type: {}", ["SURVIVAL", "CREATIVE", "ADVENTURE", "SPECTATOR"][usize::from(mode.min(3))]);
            self.commands.default_game_mode = Some(mode);
            if let Some(storage) = self.storage.as_mut() {
                storage.level.set_game_type(mode);
            }
        }
        if s.hardcore
            && new_world
            && let Some(storage) = self.storage.as_mut()
        {
            storage.level.set_hardcore(true);
            // A hardcore world is hard (`MinecraftServer.setDifficulty`).
            self.commands.difficulty = kiln_command::Difficulty::Hard;
        }
        self.commands.idle_timeout = s.player_idle_timeout.max(0);
    }

    /// `MinecraftServer.tickServer`'s pause: after `pause-when-empty-seconds` without players the world
    /// stops ticking (time, weather, mobs, redstone, crops: nothing); only the connections and the console
    /// keep running. Returns `true` when this tick was spent paused, with the messages it carried handled.
    pub(crate) fn paused_tick(&mut self, inbox: &mut Vec<ToSim>) -> bool {
        let limit = i64::from(self.config.server.pause_when_empty_seconds.max(0)) * 20;
        if limit == 0 {
            self.empty_ticks = 0;
            return false;
        }
        // A player logging in counts as somebody there (vanilla ticks its connections, where the login finishes, first).
        let joining = inbox.iter().any(|m| matches!(m, ToSim::Join(_)));
        if self.players.is_empty() && !joining && !self.world.tick_rate.is_sprinting() {
            self.empty_ticks += 1;
        } else {
            self.empty_ticks = 0;
        }
        if self.empty_ticks < limit {
            return false;
        }
        if self.empty_ticks == limit {
            info!("Server empty for {} seconds, pausing", self.config.server.pause_when_empty_seconds);
            // (`autoSave`.)
            self.start_save();
        }
        // `tickConnection`: the console, remote commands and answers to lookups still run.
        for msg in std::mem::take(inbox) {
            match msg {
                ToSim::Console(command) => self.run_console_command(command.strip_prefix('/').unwrap_or(&command)),
                ToSim::Admin(work) => work(self),
                ToSim::ProfileLookup { request, result } => self.profile_lookup_finished(request, result),
                ToSim::Shutdown { done } => {
                    self.shut_down();
                    let _ = done.send(());
                    self.stopped = true;
                }
                ToSim::Join(_) | ToSim::Leave(_) | ToSim::Packet(..) => {}
            }
        }
        self.continue_save();
        true
    }

    /// Whether the world is paused for want of players.
    pub fn is_paused(&self) -> bool {
        let limit = i64::from(self.config.server.pause_when_empty_seconds.max(0)) * 20;
        limit > 0 && self.empty_ticks >= limit
    }
}
