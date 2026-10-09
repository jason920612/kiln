//! Tasks, hot reload and the host environment.
//!
//! - `/beat <ticks>` schedules a task that follows the player (it pings them and counts the
//!   pings in their player state) and a global task (it counts beats in the global instance's
//!   memory); `/beat` shows the count, `/beat roll` a value of the host's random stream and
//!   clock.
//! - Every ping also adds to the global `pings-total` (an atomic `add`); its result comes back
//!   the next tick (`op-results`) in the instance that holds the pinged player, which
//!   broadcasts the new total.
//! - Joining schedules a welcome task a second later that records a roll and the time in the
//!   player's state.
//! - Hot reload: the beat count crosses generations as the state blob (`on-disable` →
//!   `on-enable`); tasks the reload cancelled are scheduled again with their remaining delay,
//!   so each still runs exactly once. Messages carry the configured `label`, which shows which
//!   generation ran a task.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    CancelReason, CancelledTask, CommandSpec, GlobalValue, InitInfo, OpResult, Player, Plugin, Span, TaskEvent, TaskTarget, chat,
    colored, config, env, event, export_plugin, scheduler, text,
};
use std::sync::Mutex;

const BEAT: u64 = 1;
const PING: u64 = 2;
const WELCOME: u64 = 3;

struct Global {
    beats: u64,
    label: String,
}

// The global instance's memory: survives only through the reload blob.
static STATE: Mutex<Global> = Mutex::new(Global { beats: 0, label: String::new() });

fn label() -> String {
    STATE.lock().unwrap().label.clone()
}

fn set_label(info: &InitInfo) {
    STATE.lock().unwrap().label = config(info, "label").unwrap_or("v1").to_owned();
}

fn tag() -> Span {
    colored(&format!("[{}] ", label()), "light_purple")
}

struct Heartbeat;

impl Plugin for Heartbeat {
    fn init_global(info: InitInfo) -> Vec<CommandSpec> {
        set_label(&info);
        vec![CommandSpec { name: "beat".into(), permission: 0 }]
    }

    fn init_region(info: InitInfo) {
        set_label(&info);
    }

    fn on_enable(blob: Option<Vec<u8>>) {
        let beats = blob.and_then(|b| b.try_into().ok()).map_or(0, u64::from_le_bytes);
        STATE.lock().unwrap().beats = beats;
    }

    fn on_disable() -> Option<Vec<u8>> {
        Some(STATE.lock().unwrap().beats.to_le_bytes().to_vec())
    }

    fn on_join(p: Player) {
        scheduler::for_player(p.uuid, 20, WELCOME);
    }

    fn on_command(p: Option<Player>, _name: String, args: String) -> Vec<Span> {
        let args = args.trim();
        if args == "roll" {
            return vec![tag(), text(&format!("roll {} at {} ms (tick {})", env::random() % 100, env::now_millis(), env::tick()))];
        }
        if let (Ok(delay), Some(p)) = (args.parse::<u32>(), p.as_ref()) {
            scheduler::for_player(p.uuid, delay, PING);
            scheduler::global(delay, BEAT);
            return vec![tag(), text(&format!("scheduled in {delay} ticks"))];
        }
        let beats = STATE.lock().unwrap().beats;
        vec![tag(), text(&format!("{beats} beats"))]
    }

    fn on_task(t: TaskEvent) {
        match (t.id, t.player) {
            (BEAT, _) => STATE.lock().unwrap().beats += 1,
            (PING, Some(p)) => {
                let n = state::get_i64(Scope::Player(p.handle), "pings") + 1;
                state::put_i64(Scope::Player(p.handle), "pings", n);
                chat::send_spans(p.handle, &[tag(), text(&format!("ping {n}"))]);
                state::add("pings-total", 1);
            }
            (WELCOME, Some(p)) => {
                let roll = env::random() % 100;
                state::put_i64(Scope::Player(p.handle), "roll", roll as i64);
                state::put_i64(Scope::Player(p.handle), "seen-at", env::now_millis() as i64);
                let name = event::player_name(p.handle);
                chat::send_spans(p.handle, &[tag(), text(&format!("welcome, {name}! Your roll: {roll}"))]);
            }
            _ => {}
        }
    }

    fn on_results(_player: Option<Player>, results: Vec<OpResult>) {
        for r in results {
            if let (true, Some(GlobalValue::Int(total))) = (r.applied, r.value) {
                chat::broadcast_spans(&[tag(), text(&format!("{total} pings so far"))]);
            }
        }
    }

    fn on_cancelled(tasks: Vec<CancelledTask>) {
        for t in tasks {
            if t.reason != CancelReason::Reload {
                continue;
            }
            let delay = t.remaining_ticks.max(1);
            match t.target {
                TaskTarget::Global => drop(scheduler::global(delay, t.id)),
                TaskTarget::Player(u) => drop(scheduler::for_player(u, delay, t.id)),
                TaskTarget::Position((level, x, z)) => drop(scheduler::at_position(level, x, z, delay, t.id)),
            }
        }
    }
}

export_plugin!(Heartbeat);
