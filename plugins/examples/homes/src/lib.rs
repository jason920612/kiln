//! Homes: `/sethome [name]`, `/home [name]`, `/homes`, `/delhome <name>`.
//!
//! - Homes are player-scoped data (one entry per home in the player's own namespace, saved
//!   with the player and carried along when the player moves between regions and levels).
//! - `/home` does not teleport at once: it arms a warm-up (a task that follows the player,
//!   `warmup` ticks) and records where the player stood. The task teleports them if they
//!   have not moved and were not hurt meanwhile; taking damage (the `player-damage` event,
//!   fail-open) disarms it. The teleport itself is an effect, applied at the next serial
//!   point.

use kiln_plugin_sdk::codec::{Reader, Writer};
use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    CommandSpec, DamageEvent, GameMode, InitInfo, Player, Plugin, Span, TaskEvent, Text, Verdict, chat, config, event, export_plugin, players,
    scheduler,
};
use std::sync::Mutex;

const WARMUP_TASK: u64 = 1;
const MAX_HOMES: usize = 5;

static WARMUP: Mutex<u32> = Mutex::new(60);

fn key(name: &str) -> String {
    format!("home:{name}")
}

struct Place {
    level: u32,
    pos: (f64, f64, f64),
    rot: (f32, f32),
}

fn encode(p: &Place) -> Vec<u8> {
    Writer::new().i32(p.level as i32).f64(p.pos.0).f64(p.pos.1).f64(p.pos.2).f64(p.rot.0 as f64).f64(p.rot.1 as f64).finish()
}

fn decode(b: &[u8]) -> Option<Place> {
    let mut r = Reader::new(b);
    Some(Place {
        level: r.i32()? as u32,
        pos: (r.f64()?, r.f64()?, r.f64()?),
        rot: (r.f64()? as f32, r.f64()? as f32),
    })
}

fn say(color: &str, s: impl AsRef<str>) -> Vec<Span> {
    Text::new().color(color, s).0
}

fn home_names(p: &Player) -> Vec<String> {
    state::get_str(Scope::Player(p.handle), "names").map(|s| s.split(',').filter(|n| !n.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

fn set_names(p: &Player, names: &[String]) {
    state::put_str(Scope::Player(p.handle), "names", &names.join(","));
}

struct Homes;

impl Plugin for Homes {
    fn init_global(info: InitInfo) -> Vec<CommandSpec> {
        *WARMUP.lock().unwrap() = config(&info, "warmup").and_then(|v| v.parse().ok()).unwrap_or(60);
        ["sethome", "home", "homes", "delhome"].iter().map(|n| CommandSpec { name: (*n).to_owned(), permission: 0 }).collect()
    }

    fn on_command(p: Option<Player>, name: String, args: String) -> Vec<Span> {
        let Some(p) = p else { return say("red", "Only players have homes.") };
        let arg = args.split_whitespace().next().unwrap_or("home").to_lowercase();
        let mine = Scope::Player(p.handle);
        match name.as_str() {
            "sethome" => {
                let mut names = home_names(&p);
                if !names.contains(&arg) {
                    if names.len() >= MAX_HOMES {
                        return say("red", format!("You can have {MAX_HOMES} homes."));
                    }
                    names.push(arg.clone());
                    set_names(&p, &names);
                }
                let i = event::info(p.handle);
                let place = Place { level: i.level, pos: i.pos, rot: i.rot };
                state::put(mine, &key(&arg), Some(&encode(&place)));
                say("green", format!("Home {arg} set."))
            }
            "delhome" => {
                let mut names = home_names(&p);
                let before = names.len();
                names.retain(|n| *n != arg);
                if names.len() == before {
                    return say("red", format!("No home called {arg}."));
                }
                set_names(&p, &names);
                state::delete(mine, &key(&arg));
                say("green", format!("Home {arg} removed."))
            }
            "homes" => {
                let names = home_names(&p);
                if names.is_empty() { say("gray", "You have no homes. /sethome sets one.") } else { say("aqua", format!("Homes: {}", names.join(", "))) }
            }
            _ => {
                if state::get(mine, &key(&arg)).as_deref().and_then(decode).is_none() {
                    return say("red", format!("No home called {arg}."));
                }
                // Arm the warm-up: where the player stands now, and which home.
                let i = event::info(p.handle);
                let armed = Writer::new().f64(i.pos.0).f64(i.pos.1).f64(i.pos.2).str(&arg).finish();
                state::put(mine, "armed", Some(&armed));
                scheduler::for_player(p.uuid, *WARMUP.lock().unwrap(), WARMUP_TASK);
                say("yellow", format!("Teleporting to {arg} in {} seconds, stand still.", *WARMUP.lock().unwrap() / 20))
            }
        }
    }

    /// Taking damage cancels the warm-up.
    fn on_player_damage(ev: DamageEvent) -> Verdict {
        let mine = Scope::Player(ev.victim.handle);
        if state::get(mine, "armed").is_some() {
            state::delete(mine, "armed");
        }
        Verdict::Allow
    }

    fn on_task(t: TaskEvent) {
        let (WARMUP_TASK, Some(p)) = (t.id, t.player) else { return };
        let mine = Scope::Player(p.handle);
        let Some(armed) = state::get(mine, "armed") else { return };
        state::delete(mine, "armed");
        let mut r = Reader::new(&armed);
        let (Some(x), Some(y), Some(z), Some(name)) = (r.f64(), r.f64(), r.f64(), r.str()) else { return };
        let now = event::info(p.handle);
        let moved = (now.pos.0 - x).powi(2) + (now.pos.1 - y).powi(2) + (now.pos.2 - z).powi(2);
        if moved > 1.0 {
            chat::send(&p, Text::new().color("red", "You moved: teleport cancelled."));
            return;
        }
        if now.game_mode == GameMode::Spectator {
            return;
        }
        if let Some(h) = state::get(mine, &key(&name)).as_deref().and_then(decode) {
            players::teleport(p.uuid, h.level, h.pos, h.rot);
        }
    }
}

export_plugin!(Homes);
