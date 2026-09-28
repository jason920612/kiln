//! Chat formatter: rewrites every chat line as `prefix[name] » message` in the configured
//! colours, and cancels lines containing a blocked word. Change `prefix` or `name_color` and
//! `/kiln plugins reload chat-format` to see a hot reload.

use kiln_plugin_sdk::{ChatEvent, ChatVerdict, InitInfo, Plugin, Span, colored, config, event, export_plugin};
use std::sync::Mutex;

struct Format {
    prefix: String,
    name_color: String,
    blocked: Vec<String>,
}

// Guest memory is a cache: every region instance reads its config again in `init_region`.
static FORMAT: Mutex<Option<Format>> = Mutex::new(None);

struct ChatFormat;

impl Plugin for ChatFormat {
    fn init_region(info: InitInfo) {
        let name_color = config(&info, "name_color").unwrap_or("gold").to_owned();
        let blocked = config(&info, "blocked")
            .map(|s| s.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect())
            .unwrap_or_default();
        let prefix = config(&info, "prefix").unwrap_or("").to_owned();
        *FORMAT.lock().unwrap() = Some(Format { prefix, name_color, blocked });
    }

    fn on_chat(ev: ChatEvent) -> ChatVerdict {
        let guard = FORMAT.lock().unwrap();
        let Some(f) = guard.as_ref() else { return ChatVerdict::Pass };
        let lower = ev.message.to_lowercase();
        if f.blocked.iter().any(|w| lower.contains(w.as_str())) {
            return ChatVerdict::Cancel;
        }
        let mut line: Vec<Span> = Vec::with_capacity(5);
        if !f.prefix.is_empty() {
            line.push(colored(&f.prefix, "light_purple"));
        }
        line.extend([
            colored("[", "dark_gray"),
            colored(&event::player_name(ev.player.handle), &f.name_color),
            colored("] \u{bb} ", "dark_gray"),
            colored(&ev.message, "white"),
        ]);
        ChatVerdict::Rewrite(line)
    }
}

export_plugin!(ChatFormat);
