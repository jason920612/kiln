//! Chat formatter: rewrites every chat line as `[name] » message` in the configured colours,
//! and cancels lines containing a blocked word.

use kiln_plugin_sdk::{ChatEvent, ChatVerdict, InitInfo, Plugin, Span, colored, config, export_plugin};
use std::sync::Mutex;

struct Format {
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
        *FORMAT.lock().unwrap() = Some(Format { name_color, blocked });
    }

    fn on_chat(ev: ChatEvent) -> ChatVerdict {
        let guard = FORMAT.lock().unwrap();
        let Some(f) = guard.as_ref() else { return ChatVerdict::Pass };
        let lower = ev.message.to_lowercase();
        if f.blocked.iter().any(|w| lower.contains(w.as_str())) {
            return ChatVerdict::Cancel;
        }
        let line: Vec<Span> = vec![
            colored("[", "dark_gray"),
            colored(&ev.player.name, &f.name_color),
            colored("] \u{bb} ", "dark_gray"),
            colored(&ev.message, "white"),
        ];
        ChatVerdict::Rewrite(line)
    }
}

export_plugin!(ChatFormat);
