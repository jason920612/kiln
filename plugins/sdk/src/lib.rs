//! Guest SDK for Kiln plugins (WIT package `kiln:api`, design §11).
//!
//! A plugin implements [`Plugin`] (every hook has a default) and exports it with
//! [`export_plugin!`]; the crate is a `cdylib` built for `wasm32-wasip2`, which gives a
//! component directly. Hooks are plain functions: a region instance may be replaced at any
//! barrier (split, merge, reload, trap), so anything that must survive goes through [`state`]
//! rather than statics.

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "plugin",
        path: "../../wit",
        pub_export_macro: true,
        export_macro_name: "export_raw",
        default_bindings_module: "kiln_plugin_sdk::bindings",
    });
}

pub use bindings::kiln::api::types::{
    AtomicOp, BlockEvent, BlockPos, ChatEvent, ChatVerdict, CommandEvent, CommandSpec, CompareAndSet, ConfigEntry,
    GlobalValue, InitInfo, Observed, ObservedBlock, PlaceEvent, Player, Span, Verdict,
};

/// Owned namespaces: player, cell and the plugin's global namespace.
pub mod state {
    pub use crate::bindings::kiln::api::state::{Scope, get, global_get, put, submit};
    use crate::{AtomicOp, GlobalValue};

    /// A little-endian `i64` value (missing or malformed reads as 0).
    pub fn get_i64(s: Scope, key: &str) -> i64 {
        get(s, key).and_then(|v| v.try_into().ok()).map_or(0, i64::from_le_bytes)
    }

    pub fn put_i64(s: Scope, key: &str, v: i64) {
        put(s, key, Some(&v.to_le_bytes()));
    }

    /// A global integer (the live value in the global instance, else a snapshot).
    pub fn global_i64(key: &str) -> i64 {
        match global_get(key) {
            Some(GlobalValue::Int(v)) => v,
            _ => 0,
        }
    }

    /// Queues `key += delta` on the global namespace.
    pub fn add(key: &str, delta: i64) -> u64 {
        submit(&AtomicOp::Add((key.to_owned(), delta)))
    }
}

/// `player.message`.
pub mod chat {
    pub use crate::bindings::kiln::api::chat::{broadcast, send};
}

/// Console logging.
pub mod log {
    pub use crate::bindings::kiln::api::log::{info, warn};
}

/// Text helpers.
pub fn text(s: &str) -> Span {
    Span { text: s.to_owned(), color: None, bold: false, italic: false }
}

pub fn colored(s: &str, color: &str) -> Span {
    Span { text: s.to_owned(), color: Some(color.to_owned()), bold: false, italic: false }
}

/// A `[config]` value of the manifest.
pub fn config<'a>(info: &'a InitInfo, key: &str) -> Option<&'a str> {
    info.config.iter().find(|e| e.key == key).map(|e| e.value.as_str())
}

/// A plugin's hooks. Defaults allow everything and ignore notifications.
pub trait Plugin {
    /// Global instance start: returns the commands to register.
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        Vec::new()
    }
    /// Region instance start (after every instantiation).
    fn init_region(_info: InitInfo) {}
    fn on_join(_p: Player) {}
    fn on_leave(_p: Player) {}
    fn on_command(_p: Option<Player>, _name: String, _args: String) -> Vec<Span> {
        Vec::new()
    }
    fn on_block_break(_ev: BlockEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_block_place(_ev: PlaceEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_chat(_ev: ChatEvent) -> ChatVerdict {
        ChatVerdict::Pass
    }
    fn on_region_command(_ev: CommandEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_observe(_events: Vec<Observed>) {}
}

/// Exports a [`Plugin`] implementation as the component's `global-hooks` and `region-hooks`.
#[macro_export]
macro_rules! export_plugin {
    ($t:ty) => {
        struct __KilnExports;
        impl $crate::bindings::exports::kiln::api::global_hooks::Guest for __KilnExports {
            fn init(info: $crate::InitInfo) -> ::std::vec::Vec<$crate::CommandSpec> {
                <$t as $crate::Plugin>::init_global(info)
            }
            fn on_join(p: $crate::Player) {
                <$t as $crate::Plugin>::on_join(p)
            }
            fn on_leave(p: $crate::Player) {
                <$t as $crate::Plugin>::on_leave(p)
            }
            fn on_command(
                p: ::std::option::Option<$crate::Player>,
                name: ::std::string::String,
                args: ::std::string::String,
            ) -> ::std::vec::Vec<$crate::Span> {
                <$t as $crate::Plugin>::on_command(p, name, args)
            }
        }
        impl $crate::bindings::exports::kiln::api::region_hooks::Guest for __KilnExports {
            fn init(info: $crate::InitInfo) {
                <$t as $crate::Plugin>::init_region(info)
            }
            fn on_block_break(ev: $crate::BlockEvent) -> $crate::Verdict {
                <$t as $crate::Plugin>::on_block_break(ev)
            }
            fn on_block_place(ev: $crate::PlaceEvent) -> $crate::Verdict {
                <$t as $crate::Plugin>::on_block_place(ev)
            }
            fn on_chat(ev: $crate::ChatEvent) -> $crate::ChatVerdict {
                <$t as $crate::Plugin>::on_chat(ev)
            }
            fn on_command(ev: $crate::CommandEvent) -> $crate::Verdict {
                <$t as $crate::Plugin>::on_region_command(ev)
            }
            fn on_observe(events: ::std::vec::Vec<$crate::Observed>) {
                <$t as $crate::Plugin>::on_observe(events)
            }
        }
        $crate::bindings::export_raw!(__KilnExports with_types_in $crate::bindings);
    };
}
