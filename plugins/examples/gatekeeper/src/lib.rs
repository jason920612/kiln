//! Answers events other plugins raise: denies the names listed in `deny` (a comma-separated
//! config value such as `arena:join`). The same handler serves the global instance (events
//! raised by a command) and the region instance (events raised by a region's task).

use kiln_plugin_sdk::{CustomEvent, InitInfo, Plugin, Verdict, config, export_plugin};
use std::sync::Mutex;

static DENY: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn load(info: &InitInfo) {
    *DENY.lock().unwrap() = config(info, "deny").map(|s| s.split(',').map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
}

struct Gatekeeper;

impl Plugin for Gatekeeper {
    fn init_global(info: InitInfo) -> Vec<kiln_plugin_sdk::CommandSpec> {
        load(&info);
        Vec::new()
    }

    fn init_region(info: InitInfo) {
        load(&info);
    }

    fn on_custom(ev: CustomEvent) -> Verdict {
        if DENY.lock().unwrap().iter().any(|n| *n == ev.name) { Verdict::deny_silently() } else { Verdict::Allow }
    }
}

export_plugin!(Gatekeeper);
