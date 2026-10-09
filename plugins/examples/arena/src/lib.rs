//! A tiny minigame arena: `/arena join` teleports the player to the arena, takes their items,
//! hands them a sword and switches them to adventure mode; `/arena reset` rebuilds the arena
//! floor.
//!
//! What it shows:
//! - **Plugin-raised events.** Before it lets anybody in (`arena:join`) or rebuilds the floor
//!   (`arena:reset`) it asks the other plugins with `events.raise`. The question goes to the
//!   plugins of the same context (here the global instances for the command, the region
//!   instance for the floor task), is answered at once, and any plugin can veto it
//!   (`gatekeeper` does).
//! - **Block edits through the owning region.** The floor is rebuilt by a task at the arena's
//!   position, which runs in the region that owns that cell and hands `set-blocks` the cell of
//!   its call; positions outside the cell are refused.
//! - Effects on players (teleport, clear, give, game mode) are queued and applied at the next
//!   serial point.

use kiln_plugin_sdk::{
    CommandSpec, InitInfo, Item, Player, Plugin, Span, TaskEvent, Text, blocks, config, events, export_plugin, inventory, level_id, players,
    scheduler,
};
use kiln_plugin_sdk::GameMode;
use std::sync::Mutex;

const RESET: u64 = 1;

struct Arena {
    level: u32,
    x: i32,
    y: i32,
    z: i32,
}

static ARENA: Mutex<Option<Arena>> = Mutex::new(None);

fn load(info: &InitInfo) {
    let int = |k: &str, d: i32| config(info, k).and_then(|v| v.parse().ok()).unwrap_or(d);
    *ARENA.lock().unwrap() = Some(Arena {
        level: level_id(info, config(info, "level").unwrap_or("minecraft:overworld")).unwrap_or(0),
        x: int("x", 1000),
        y: int("y", 80),
        z: int("z", 1000),
    });
}

fn say(color: &str, s: &str) -> Vec<Span> {
    Text::new().color(color, s).0
}

struct ArenaPlugin;

impl Plugin for ArenaPlugin {
    fn init_global(info: InitInfo) -> Vec<CommandSpec> {
        load(&info);
        vec![CommandSpec { name: "arena".into(), permission: 0 }]
    }

    fn init_region(info: InitInfo) {
        load(&info);
    }

    fn on_command(p: Option<Player>, _name: String, args: String) -> Vec<Span> {
        let guard = ARENA.lock().unwrap();
        let Some(a) = guard.as_ref() else { return Vec::new() };
        match (args.trim(), p) {
            ("join", Some(p)) => {
                // Ask the other plugins first.
                if !events::raise("join", b"", Some(&p)) {
                    return say("red", "The arena is closed.");
                }
                inventory::clear(p.uuid);
                inventory::give(p.uuid, Item::new("minecraft:wooden_sword", 1).name(Text::new().color("red", "Arena sword")));
                players::set_game_mode(p.uuid, GameMode::Adventure);
                players::teleport(p.uuid, a.level, (a.x as f64 + 0.5, a.y as f64 + 1.0, a.z as f64 + 0.5), (0.0, 0.0));
                say("green", "Welcome to the arena.")
            }
            ("out", Some(p)) => {
                players::kill(p.uuid);
                say("red", "Eliminated.")
            }
            ("reset", _) => {
                scheduler::at_position(a.level, a.x, a.z, 1, RESET);
                say("yellow", "Rebuilding the arena floor.")
            }
            _ => say("gray", "/arena join | out | reset"),
        }
    }

    fn on_task(t: TaskEvent) {
        let (RESET, Some(cell)) = (t.id, t.cell) else { return };
        let guard = ARENA.lock().unwrap();
        let Some(a) = guard.as_ref() else { return };
        if !events::raise("reset", b"", None) {
            return;
        }
        let mut floor = Vec::new();
        for dx in -3..=3 {
            for dz in -3..=3 {
                floor.push(blocks::change(a.x + dx, a.y, a.z + dz, if (dx + dz) % 2 == 0 { "minecraft:smooth_stone" } else { "minecraft:stone_bricks" }));
            }
        }
        if let Err(e) = blocks::set(cell, a.level, &floor) {
            kiln_plugin_sdk::log::warn(&format!("arena floor refused: {e:?}"));
        }
    }
}

export_plugin!(ArenaPlugin);
