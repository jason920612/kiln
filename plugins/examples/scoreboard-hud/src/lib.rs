//! A scoreboard HUD: every player gets a private sidebar (deaths, kills, players online,
//! health), a health boss bar and a welcome title, refreshed once a second by a task that
//! follows the player. None of it touches the server's shared scoreboard.
//!
//! - Deaths are counted in the player's own namespace, by the region that holds them, from the
//!   `player-died` observe event.
//! - Kills are a global counter per killer: the killer may stand in another region, so the
//!   count is a typed atomic `add` on the plugin's global namespace, and the sidebar reads the
//!   global snapshot (at most a tick old).
//! - Tasks that a hot reload cancels come back in `on-cancelled` and are scheduled again with
//!   the delay they had left.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    BossColor, BossStyle, CancelReason, CancelledTask, InitInfo, Observed, Player, Plugin, TaskEvent, TaskTarget, Text, colored, event, export_plugin, hud,
    scheduler, text, uuid_string, uuid_u128,
};

/// Task id: refresh a player's HUD.
const REFRESH: u64 = 1;

struct Hud;

fn kills_key(uuid: u128) -> String {
    format!("kills:{}", uuid_string(&kiln_plugin_sdk::uuid_from_u128(uuid)))
}

impl Plugin for Hud {
    fn init_global(_info: InitInfo) -> Vec<kiln_plugin_sdk::CommandSpec> {
        Vec::new()
    }

    fn on_join(p: Player) {
        scheduler::for_player(p.uuid, 1, REFRESH);
    }

    fn on_cancelled(tasks: Vec<CancelledTask>) {
        for t in tasks {
            if let (CancelReason::Reload, TaskTarget::Player(uuid)) = (t.reason, t.target) {
                scheduler::for_player(uuid, t.remaining_ticks.max(1), t.id);
            }
        }
    }

    fn on_observe(events: Vec<Observed>) {
        for ev in events {
            match ev {
                Observed::PlayerDied(d) => {
                    state::bump(Scope::Player(d.player.handle), "deaths", 1);
                    if let Some(killer) = d.killer {
                        state::add(&kills_key(uuid_u128(&killer)), 1);
                    }
                }
                Observed::PlayerSpawned(s) => {
                    hud::title(s.player.uuid, Text::new().color("gold", "Welcome"), Text::new().color("gray", "have fun"));
                }
                _ => {}
            }
        }
    }

    fn on_task(t: TaskEvent) {
        let (REFRESH, Some(p)) = (t.id, t.player) else { return };
        let info = event::info(p.handle);
        let deaths = state::get_i64(Scope::Player(p.handle), "deaths");
        let kills = state::global_i64(&kills_key(uuid_u128(&p.uuid)));
        let online = event::online().len();
        let line = |label: &str, value: String| vec![colored(label, "gray"), colored(&value, "white")];
        hud::sidebar(
            p.uuid,
            Text::new().color("gold", "Kiln"),
            vec![
                line("Deaths: ", deaths.to_string()),
                line("Kills: ", kills.to_string()),
                line("Online: ", online.to_string()),
                vec![text("")],
                line("Health: ", format!("{:.0}", info.health)),
            ],
        );
        hud::bossbar(
            p.uuid,
            "health",
            Text::new().color("red", format!("Health {:.0}/20", info.health)),
            (info.health / 20.0).clamp(0.0, 1.0),
            BossColor::Red,
            BossStyle::Notched10,
        );
        scheduler::for_player(p.uuid, 20, REFRESH);
    }
}

export_plugin!(Hud);
