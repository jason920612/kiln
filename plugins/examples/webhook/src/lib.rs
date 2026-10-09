//! Talks to the outside world through its tasks component (the `async-tasks` world, WASI 0.3
//! component-model async): `/web fetch <url>`, `/web post <url> <text>`, `/web sleep <ticks>`,
//! `/web remember <key>=<value>`, `/web recall <key>`, `/web count`.
//!
//! The game side never waits: the command hands the tasks component a job (`jobs.submit`) and
//! returns; the answer comes back a tick or more later in `on-results`, in the region holding
//! the player, which tells them. A hot reload interrupts jobs in flight; the new generation
//! hears of each in `on-cancelled` and submits it again (the jobs are kept in the plugin's global
//! namespace for that).

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{CancelReason, CancelledTask, CommandSpec, GlobalValue, InitInfo, OpResult, Player, Plugin, Span, Text, chat, export_plugin, jobs, log};

struct Web;

fn job_key(id: u64) -> String {
    format!("job:{id}")
}

impl Plugin for Web {
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        vec![CommandSpec { name: "web".into(), permission: 0 }]
    }

    fn on_command(p: Option<Player>, _name: String, args: String) -> Vec<Span> {
        let (kind, payload) = args.trim().split_once(' ').unwrap_or((args.trim(), ""));
        let kind = match kind {
            "fetch" | "post" | "sleep" | "remember" | "recall" | "count" => kind,
            _ => return Text::new().color("gray", "/web fetch|post|sleep|remember|recall|count ...").0,
        };
        let payload = if kind == "post" { payload.replacen(' ', "\n", 1) } else { payload.to_owned() };
        // Numbered by the plugin, kept until the answer is in (a reload needs them).
        let id = state::global_i64("next-job") as u64 + 1;
        state::add("next-job", 1);
        let record = format!("{kind}\n{payload}");
        state::compare_and_set(&job_key(id), None, GlobalValue::Bytes(record.into_bytes()));
        jobs::submit(id, kind, payload.as_bytes());
        if let Some(p) = p {
            state::put_int(Scope::Player(p.handle), "last", id as i64);
        }
        Text::new().color("yellow", format!("Job {id} ({kind}) started.")).0
    }

    /// The answers: the result bytes, or the failure text.
    fn on_results(player: Option<Player>, results: Vec<OpResult>) {
        let Some(p) = player else { return };
        for r in results {
            let Some(GlobalValue::Bytes(bytes)) = r.value else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let shown: String = text.chars().take(120).collect();
            let color = if r.applied { "green" } else { "red" };
            chat::send(&p, Text::new().color(color, if r.applied { format!("[web] {shown}") } else { format!("[web] failed: {shown}") }));
        }
    }

    /// A reload interrupted these jobs: submit them again.
    fn on_cancelled(tasks: Vec<CancelledTask>) {
        for t in tasks {
            if t.reason != CancelReason::Reload {
                continue;
            }
            state::add("interrupted", 1);
            let Some(GlobalValue::Bytes(record)) = state::global_get(&job_key(t.id)) else { continue };
            let record = String::from_utf8_lossy(&record).into_owned();
            let (kind, payload) = record.split_once('\n').unwrap_or((&record, ""));
            log::info(&format!("job {} submitted again after a reload", t.id));
            jobs::submit(t.id, kind, payload.as_bytes());
        }
    }
}

export_plugin!(Web);
