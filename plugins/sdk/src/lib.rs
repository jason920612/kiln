//! Guest SDK for Kiln plugins (WIT package `kiln:api`, design §11).
//!
//! A plugin implements [`Plugin`] (every hook has a default) and exports it with
//! [`export_plugin!`]; the crate is a `cdylib` built for `wasm32-wasip2`, which gives a
//! component directly. Hooks are plain functions: a region instance may be replaced at any
//! barrier (split, merge, reload, trap), so anything that must survive goes through [`state`]
//! rather than statics. The global instance may keep state in statics across a hot reload by
//! returning it from [`Plugin::on_disable`] and reading it back in [`Plugin::on_enable`].
//!
//! Determinism: [`env::random`] and [`env::now_millis`] are what the host hands out (seeded
//! and tick-based in strict mode); do not rely on the iteration order of `HashMap`s built in
//! guest memory.

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
    AtomicOp, BlockEvent, BlockPos, CancelReason, CancelledTask, ChatEvent, ChatVerdict, CommandEvent, CommandSpec,
    CompareAndSet, ConfigEntry, EntityEvent, GlobalValue, InitInfo, Observed, ObservedBlock, OpResult, PlaceEvent, Player,
    Span, TaskEvent, TaskTarget, Uuid, Verdict,
};

/// Owned namespaces: player, cell, entity and the plugin's global namespace.
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

/// The host's clock and random stream (deterministic in strict mode).
pub mod env {
    pub use crate::bindings::kiln::api::env::{now_millis, random, tick};
}

/// Per-run registry ids and their keys.
pub mod registry {
    pub use crate::bindings::kiln::api::registry::{Kind, id, key};
}

/// `scheduler`: delayed tasks.
pub mod scheduler {
    pub use crate::bindings::kiln::api::scheduler::{at_position, cancel, for_player, global};
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

/// The level id of a level key (`minecraft:overworld`), from the init info.
pub fn level_id(info: &InitInfo, key: &str) -> Option<u32> {
    info.levels.iter().position(|l| l == key).map(|i| i as u32)
}

/// A uuid in its usual hyphenated form.
pub fn uuid_string(u: &Uuid) -> String {
    let v = ((u.hi as u128) << 64) | u.lo as u128;
    let h = format!("{v:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// A plugin's hooks. Defaults allow everything and ignore notifications.
pub trait Plugin {
    /// Global instance start: returns the commands to register.
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        Vec::new()
    }
    /// After `init_global`: what the previous generation handed over (hot reload).
    fn on_enable(_blob: Option<Vec<u8>>) {}
    /// Before a hot reload: state for the next generation.
    fn on_disable() -> Option<Vec<u8>> {
        None
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
    fn on_entity_interact(_ev: EntityEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_chat(_ev: ChatEvent) -> ChatVerdict {
        ChatVerdict::Pass
    }
    fn on_region_command(_ev: CommandEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_observe(_events: Vec<Observed>) {}
    /// A task ran (in the global instance or a region instance, by its target).
    fn on_task(_t: TaskEvent) {}
    /// Outcomes of this plugin's atomic operations (with the `op-results` subscription).
    fn on_results(_results: Vec<OpResult>) {}
    /// Tasks that will not run (global instance).
    fn on_cancelled(_tasks: Vec<CancelledTask>) {}
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
            fn on_enable(blob: ::std::option::Option<::std::vec::Vec<u8>>) {
                <$t as $crate::Plugin>::on_enable(blob)
            }
            fn on_disable() -> ::std::option::Option<::std::vec::Vec<u8>> {
                <$t as $crate::Plugin>::on_disable()
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
            fn on_task(t: $crate::TaskEvent) {
                <$t as $crate::Plugin>::on_task(t)
            }
            fn on_results(r: ::std::vec::Vec<$crate::OpResult>) {
                <$t as $crate::Plugin>::on_results(r)
            }
            fn on_cancelled(t: ::std::vec::Vec<$crate::CancelledTask>) {
                <$t as $crate::Plugin>::on_cancelled(t)
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
            fn on_entity_interact(ev: $crate::EntityEvent) -> $crate::Verdict {
                <$t as $crate::Plugin>::on_entity_interact(ev)
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
            fn on_task(t: $crate::TaskEvent) {
                <$t as $crate::Plugin>::on_task(t)
            }
            fn on_results(r: ::std::vec::Vec<$crate::OpResult>) {
                <$t as $crate::Plugin>::on_results(r)
            }
        }
        $crate::bindings::export_raw!(__KilnExports with_types_in $crate::bindings);
    };
}
