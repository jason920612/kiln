//! Per-player block counter: counts the blocks each player breaks (player-scoped state, from
//! batched observe events in the player's region) and the server-wide total (a global atomic
//! `add`). `/broken` shows both; joining players hear their count.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{chat, CommandSpec, InitInfo, Observed, Player, Plugin, Span, colored, export_plugin, text};

struct Counter;

impl Plugin for Counter {
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        vec![CommandSpec { name: "broken".into(), permission: 0 }]
    }

    fn on_join(p: Player) {
        let mine = state::get_i64(Scope::Player(p.handle), "broken");
        if mine > 0 {
            chat::send(p.handle, &[text("Welcome back! You have broken "), colored(&mine.to_string(), "aqua"), text(" blocks.")]);
        }
    }

    fn on_observe(events: Vec<Observed>) {
        for ev in events {
            if let Observed::BlockBroken(b) = ev {
                let scope = Scope::Player(b.player.handle);
                let n = state::get_i64(scope, "broken");
                state::put_i64(Scope::Player(b.player.handle), "broken", n + 1);
                state::add("broken-total", 1);
            }
        }
    }

    fn on_command(p: Option<Player>, _name: String, _args: String) -> Vec<Span> {
        let total = state::global_i64("broken-total");
        match p {
            Some(p) => {
                let mine = state::get_i64(Scope::Player(p.handle), "broken");
                vec![
                    text("You broke "),
                    colored(&mine.to_string(), "aqua"),
                    text(" blocks; everyone: "),
                    colored(&total.to_string(), "aqua"),
                ]
            }
            None => vec![text(&format!("Blocks broken: {total}"))],
        }
    }
}

export_plugin!(Counter);
