//! Talks to the outside world through its tasks component (the `async-tasks` world, WASI 0.3
//! component-model async): `/web fetch <url>`, `/web post <url> <text>`, `/web sleep <ticks>`,
//! `/web remember <key>=<value>`, `/web recall <key>`, `/web count`.
//!
//! The game side never waits: the command hands the tasks component a job (`jobs.submit`) and
//! returns; the answer comes back a tick or more later in `on-results` (every answer starts with
//! the job's number and a newline, which is how this side knows whose job it was). A hot reload
//! interrupts the jobs in flight; the new generation hears of each in `on-cancelled` and submits
//! it again. Jobs are kept in the plugin's global namespace until they are answered.

use kiln_plugin_sdk::state;
use kiln_plugin_sdk::{
    CancelReason, CancelledTask, CommandSpec, GlobalValue, InitInfo, OpResult, Player, Plugin, Span, Text, Uuid, chat, export_plugin, jobs, log, uuid_from_u128,
    uuid_u128,
};

struct Web;

fn job_key(id: u64) -> String {
    format!("job:{id}")
}

/// A job as kept: kind, who asked (a uuid in hex, empty if nobody), payload.
fn record(kind: &str, who: Option<&Uuid>, payload: &str) -> Vec<u8> {
    format!("{kind}\n{}\n{payload}", who.map(|u| format!("{:032x}", uuid_u128(u))).unwrap_or_default()).into_bytes()
}

fn parse(record: &[u8]) -> (String, Option<Uuid>, String) {
    let text = String::from_utf8_lossy(record).into_owned();
    let mut parts = text.splitn(3, '\n');
    let kind = parts.next().unwrap_or("").to_owned();
    let who = parts.next().and_then(|h| u128::from_str_radix(h, 16).ok()).map(uuid_from_u128);
    (kind, who, parts.next().unwrap_or("").to_owned())
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
        state::compare_and_set(&job_key(id), None, GlobalValue::Bytes(record(kind, p.as_ref().map(|p| &p.uuid), &payload)));
        jobs::submit(id, kind, payload.as_bytes());
        Text::new().color("yellow", format!("Job {id} ({kind}) started.")).0
    }

    /// The answers: `<job number>\n<result>`, or the failure text. Answers the host itself
    /// makes (no tasks component running, strict mode) carry no number: they go to the player
    /// whose call asked.
    fn on_results(player: Option<Player>, results: Vec<OpResult>) {
        for r in results {
            let Some(GlobalValue::Bytes(bytes)) = r.value else { continue };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let numbered = text.split_once('\n').and_then(|(id, rest)| Some((id.parse::<u64>().ok()?, rest.to_owned())));
            let (who, shown) = match numbered {
                Some((id, rest)) => {
                    // A number of ours: the job record says who asked. (The plugin's own
                    // operations answer here too, and carry no number.)
                    let Some(GlobalValue::Bytes(rec)) = state::global_get(&job_key(id)) else { continue };
                    (parse(&rec).1, rest)
                }
                None if !r.applied => (player.as_ref().map(|p| p.uuid), text),
                None => continue,
            };
            let Some(who) = who else { continue };
            let shown: String = shown.chars().take(120).collect();
            let color = if r.applied { "green" } else { "red" };
            chat::tell(who, Text::new().color(color, if r.applied { format!("[web] {shown}") } else { format!("[web] failed: {shown}") }));
        }
    }

    /// A reload interrupted these jobs: submit them again.
    fn on_cancelled(tasks: Vec<CancelledTask>) {
        for t in tasks {
            if t.reason != CancelReason::Reload {
                continue;
            }
            state::add("interrupted", 1);
            let Some(GlobalValue::Bytes(rec)) = state::global_get(&job_key(t.id)) else { continue };
            let (kind, _, payload) = parse(&rec);
            log::info(&format!("job {} submitted again after a reload", t.id));
            jobs::submit(t.id, &kind, payload.as_bytes());
        }
    }
}

export_plugin!(Web);
