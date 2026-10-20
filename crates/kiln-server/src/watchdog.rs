//! The watchdog of `max-tick-time` (`ServerWatchdog`): if the next tick is overdue by more than that,
//! the server is considered crashed. It logs what vanilla logs, saves a crash report to
//! `crash-reports/crash-<date>-server.txt` and stops the process with status 1.

use crate::javaprops::utc_parts;
use kiln_link::ServerInfo;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};
use tracing::error;

pub fn unix_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// `ServerWatchdog`'s first message: how long the tick took against how long it may take, in seconds.
pub fn tick_message(late_ms: u64, tick_ms: u64) -> String {
    format!("A single server tick took {:.2} seconds (should be max {:.2})", late_ms as f32 / 1000.0, tick_ms as f32 / 1000.0)
}

/// The crash report (`CrashReport.getFriendlyReport`) of a stuck server.
pub fn crash_report(now: SystemTime, late_ms: u64, info: &ServerInfo) -> String {
    let (y, mo, d, h, mi, s) = utc_parts(now);
    let players = info.players().into_iter().map(|p| p.name).collect::<Vec<_>>();
    let mut out = String::new();
    out.push_str("---- Minecraft Crash Report ----\n// But it works on my machine.\n\n");
    out.push_str(&format!("Time: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}\n"));
    out.push_str("Description: Watching Server\n\n");
    out.push_str("java.lang.Error: Watchdog (Watching Server)\n");
    out.push_str("\tat kiln.simulation.tick(Unknown Source)\n\n");
    out.push_str("A detailed walkthrough of the error, its code path and all known details is as follows:\n");
    out.push_str(&"-".repeat(87));
    out.push_str("\n\n-- Head --\nThread: Server Watchdog\nStacktrace:\n\tat kiln.watchdog.run(Unknown Source)\n\n");
    out.push_str("-- Performance stats --\nDetails:\n");
    out.push_str(&format!("\tOverdue by: {late_ms} ms\n"));
    out.push_str("\n-- System Details --\nDetails:\n");
    out.push_str(&format!("\tMinecraft Version: {}\n\tMinecraft Version ID: {}\n", info.version_name, info.version_name));
    out.push_str(&format!("\tOperating System: {} ({})\n", std::env::consts::OS, std::env::consts::ARCH));
    out.push_str(&format!("\tCPUs: {}\n", std::thread::available_parallelism().map_or(1, |n| n.get())));
    out.push_str("\tServer Running: true\n");
    out.push_str(&format!("\tPlayer Count: {} / {}; {:?}\n", players.len(), info.max_players(), players));
    out.push_str("\tType: Dedicated Server (Kiln)");
    out
}

/// Starts the watchdog thread. `max_tick_ms` is the property (0 or less: no watchdog).
pub fn start(info: Arc<ServerInfo>, max_tick_ms: i64, tick_ms: u64) -> Option<std::thread::JoinHandle<()>> {
    if max_tick_ms <= 0 {
        return None;
    }
    let max = max_tick_ms as u64;
    std::thread::Builder::new()
        .name("Server Watchdog".into())
        .spawn(move || loop {
            let due = info.next_tick_due_ms.load(Ordering::Relaxed);
            let now = unix_ms();
            // (Nothing is due while the world loads: the simulation says when it is ticking.)
            if due != 0 && now > due + max {
                let late = now - due;
                error!("{}", tick_message(late, tick_ms));
                error!("Considering it to be crashed, server will forcibly shutdown.");
                let report = crash_report(SystemTime::now(), late, &info);
                let (y, mo, d, h, mi, s) = utc_parts(SystemTime::now());
                let path = Path::new("crash-reports").join(format!("crash-{y:04}-{mo:02}-{d:02}_{h:02}.{mi:02}.{s:02}-server.txt"));
                let saved = std::fs::create_dir_all("crash-reports").and_then(|()| std::fs::write(&path, &report));
                match saved {
                    Ok(()) => error!("This crash report has been saved to: {}", std::fs::canonicalize(&path).unwrap_or(path).display()),
                    Err(_) => error!("We were unable to save this crash report to disk."),
                }
                std::process::exit(1);
            }
            let wait = if due == 0 { max } else { (due + max).saturating_sub(now).max(1) };
            std::thread::sleep(Duration::from_millis(wait));
        })
        .ok()
}

#[cfg(test)]
mod tests;
