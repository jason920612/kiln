//! Guest SDK for Kiln plugins (WIT package `kiln:api` 1.0, design §11, `docs/plugin-api.md`).
//!
//! A plugin implements [`Plugin`] (every hook has a default) and exports it with
//! [`export_plugin!`]; the crate is a `cdylib` built for `wasm32-wasip2`, which gives a
//! component directly. Hooks are plain functions: a region instance may be replaced at any
//! barrier (split, merge, reload, trap), so anything that must survive goes through [`state`]
//! rather than statics. The global instance may keep state in statics across a hot reload by
//! returning it from [`Plugin::on_disable`] and reading it back in [`Plugin::on_enable`].
//!
//! Everything a plugin does to the game is an effect: [`chat`], [`hud`], [`players`],
//! [`inventory`], [`entities`] and [`blocks`] calls queue something that the server applies at
//! the next serial point; each returns a [`Ticket`] whose outcome arrives in
//! [`Plugin::on_results`] (with the `op-results` subscription). A handler never sees its own
//! effects happen.
//!
//! Determinism: [`env::random`] and [`env::now_millis`] are what the host hands out (seeded
//! and tick-based in strict mode); do not rely on the iteration order of `HashMap`s built in
//! guest memory.
//!
//! ```ignore
//! use kiln_plugin_sdk::*;
//!
//! struct Hello;
//! impl Plugin for Hello {
//!     fn on_block_break(ev: BlockEvent) -> Verdict {
//!         if ev.pos.y > 200 { Verdict::deny("Too high to dig.") } else { Verdict::Allow }
//!     }
//! }
//! export_plugin!(Hello);
//! ```

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
    AtomicOp, BlockChange, BlockEvent, BlockPos, BossColor, BossStyle, CancelReason, CancelledTask, ChatEvent, ChatVerdict, ClickKind,
    CommandEvent, CommandSpec, CompareAndSet, ConfigEntry, ContainerClickEvent, CustomEvent, DamageEvent, Decision, DeathEvent, EditError,
    EntityEvent, GameMode, GlobalValue, InitInfo, ItemUseEvent, ItemView, Observed, ObservedBlock, OnlinePlayer, OpResult, PlaceEvent,
    Player, PlayerInfo, Span, SpawnEvent, SpawnReason, TaskEvent, TaskTarget, TryAdd, Uuid,
};

/// The id an effect or atomic operation answers to.
pub type Ticket = u64;

/// A cancellable event's outcome, with an optional message for the acting player (sent
/// through `event.deny-message`, so the value crossing back to the host stays flat).
#[derive(Clone, Debug)]
pub enum Verdict {
    Allow,
    Deny(Option<Vec<Span>>),
}

impl Verdict {
    /// Denies and tells the acting player why.
    pub fn deny(message: impl IntoSpans) -> Verdict {
        Verdict::Deny(Some(message.into_spans()))
    }

    /// Denies without a message.
    pub fn deny_silently() -> Verdict {
        Verdict::Deny(None)
    }

    pub fn is_deny(&self) -> bool {
        matches!(self, Verdict::Deny(_))
    }

    #[doc(hidden)]
    pub fn into_decision(self) -> Decision {
        match self {
            Verdict::Allow => Decision::Allow,
            Verdict::Deny(msg) => {
                if let Some(m) = msg {
                    event::deny_message(&m);
                }
                Decision::Deny
            }
        }
    }
}

impl From<Decision> for Verdict {
    fn from(d: Decision) -> Verdict {
        match d {
            Decision::Allow => Verdict::Allow,
            Decision::Deny => Verdict::Deny(None),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------

/// Anything that is chat text: `&str`, `String`, one [`Span`], a list of them or a [`Text`].
pub trait IntoSpans {
    fn into_spans(self) -> Vec<Span>;
}

impl IntoSpans for &str {
    fn into_spans(self) -> Vec<Span> {
        vec![text(self)]
    }
}
impl IntoSpans for String {
    fn into_spans(self) -> Vec<Span> {
        vec![text(&self)]
    }
}
impl IntoSpans for Span {
    fn into_spans(self) -> Vec<Span> {
        vec![self]
    }
}
impl IntoSpans for Vec<Span> {
    fn into_spans(self) -> Vec<Span> {
        self
    }
}
impl IntoSpans for Text {
    fn into_spans(self) -> Vec<Span> {
        self.0
    }
}

/// Chat text built piece by piece:
/// `Text::new().color("gold", "[Shop] ").plain("Welcome, ").bold("friend")`.
#[derive(Clone, Debug, Default)]
pub struct Text(pub Vec<Span>);

impl Text {
    pub fn new() -> Text {
        Text(Vec::new())
    }

    pub fn plain(mut self, s: impl AsRef<str>) -> Text {
        self.0.push(text(s.as_ref()));
        self
    }

    pub fn color(mut self, color: &str, s: impl AsRef<str>) -> Text {
        self.0.push(colored(s.as_ref(), color));
        self
    }

    pub fn bold(mut self, s: impl AsRef<str>) -> Text {
        self.0.push(Span { bold: true, ..text(s.as_ref()) });
        self
    }

    pub fn italic(mut self, s: impl AsRef<str>) -> Text {
        self.0.push(Span { italic: true, ..text(s.as_ref()) });
        self
    }

    pub fn span(mut self, s: Span) -> Text {
        self.0.push(s);
        self
    }
}

pub fn text(s: &str) -> Span {
    Span { text: s.to_owned(), color: None, bold: false, italic: false }
}

pub fn colored(s: &str, color: &str) -> Span {
    Span { text: s.to_owned(), color: Some(color.to_owned()), bold: false, italic: false }
}

/// The plain text of spans.
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

// ---------------------------------------------------------------------------------------
// Players and uuids
// ---------------------------------------------------------------------------------------

/// A uuid as one number (and back).
pub fn uuid_u128(u: &Uuid) -> u128 {
    ((u.hi as u128) << 64) | u.lo as u128
}

pub fn uuid_from_u128(v: u128) -> Uuid {
    Uuid { hi: (v >> 64) as u64, lo: v as u64 }
}

/// A uuid in its usual hyphenated form.
pub fn uuid_string(u: &Uuid) -> String {
    let h = format!("{:032x}", uuid_u128(u));
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// The player's name.
pub fn name_of(p: &Player) -> String {
    event::player_name(p.handle)
}

// ---------------------------------------------------------------------------------------
// Modules over the host interfaces
// ---------------------------------------------------------------------------------------

/// The current event: names, player state, who is online, the denial message.
pub mod event {
    pub use crate::bindings::kiln::api::event::{deny_message, info, online, player_name};
}

/// Owned namespaces: player, cell, entity and the plugin's global namespace.
pub mod state {
    pub use crate::bindings::kiln::api::state::{Scope, get, get_int, global_get, put, put_int, submit};
    use crate::{AtomicOp, CompareAndSet, GlobalValue, Ticket, TryAdd};

    /// A little-endian `i64` value (missing or malformed reads as 0).
    pub fn get_i64(s: Scope, key: &str) -> i64 {
        get_int(s, key).unwrap_or(0)
    }

    pub fn put_i64(s: Scope, key: &str, v: i64) {
        put_int(s, key, v);
    }

    /// Adds to a counter of the namespace and returns the new value.
    pub fn bump(s: Scope, key: &str, delta: i64) -> i64 {
        let v = get_i64(s, key).wrapping_add(delta);
        put_int(s, key, v);
        v
    }

    pub fn get_str(s: Scope, key: &str) -> Option<String> {
        get(s, key).and_then(|b| String::from_utf8(b).ok())
    }

    pub fn put_str(s: Scope, key: &str, v: &str) {
        put(s, key, Some(v.as_bytes()));
    }

    pub fn delete(s: Scope, key: &str) {
        put(s, key, None);
    }

    /// A global integer (the live value in the global instance, else a snapshot).
    pub fn global_i64(key: &str) -> i64 {
        match global_get(key) {
            Some(GlobalValue::Int(v)) => v,
            _ => 0,
        }
    }

    /// Queues `key += delta` on the global namespace.
    pub fn add(key: &str, delta: i64) -> Ticket {
        submit(&AtomicOp::Add((key.to_owned(), delta)))
    }

    /// Queues `key += delta` unless the result would fall below `floor` (a missing key counts
    /// as 0): the result says whether it applied. A withdrawal that cannot overdraw.
    pub fn try_add(key: &str, delta: i64, floor: i64) -> Ticket {
        submit(&AtomicOp::TryAdd(TryAdd { key: key.to_owned(), delta, floor }))
    }

    /// Queues "set `key` to `new` if it is `expected` now".
    pub fn compare_and_set(key: &str, expected: Option<GlobalValue>, new: GlobalValue) -> Ticket {
        submit(&AtomicOp::CompareAndSet(CompareAndSet { key: key.to_owned(), expected, new }))
    }

    pub fn append(key: &str, bytes: &[u8]) -> Ticket {
        submit(&AtomicOp::Append((key.to_owned(), bytes.to_vec())))
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
    use crate::{IntoSpans, Player};
    pub use crate::bindings::kiln::api::chat::{broadcast as broadcast_spans, send as send_spans};

    /// Says something to a player of the call.
    pub fn send(to: &Player, text: impl IntoSpans) {
        send_spans(to.handle, &text.into_spans());
    }

    pub fn broadcast(text: impl IntoSpans) {
        broadcast_spans(&text.into_spans());
    }
}

/// `player.hud`: titles, the action bar, a private sidebar and boss bars. Players are named
/// by uuid.
pub mod hud {
    use crate::bindings::kiln::api::hud as raw;
    use crate::{BossColor, BossStyle, IntoSpans, Ticket, Uuid};

    pub fn title(to: Uuid, title: impl IntoSpans, subtitle: impl IntoSpans) -> Ticket {
        raw::title(to, &title.into_spans(), &subtitle.into_spans(), 10, 60, 20)
    }

    pub fn title_timed(to: Uuid, title: impl IntoSpans, subtitle: impl IntoSpans, fade_in: u32, stay: u32, fade_out: u32) -> Ticket {
        raw::title(to, &title.into_spans(), &subtitle.into_spans(), fade_in, stay, fade_out)
    }

    pub fn action_bar(to: Uuid, text: impl IntoSpans) -> Ticket {
        raw::action_bar(to, &text.into_spans())
    }

    /// Replaces the player's sidebar: a title and up to 15 lines, top to bottom.
    pub fn sidebar(to: Uuid, title: impl IntoSpans, lines: Vec<Vec<crate::Span>>) -> Ticket {
        raw::sidebar(to, &title.into_spans(), &lines)
    }

    pub fn clear_sidebar(to: Uuid) -> Ticket {
        raw::clear_sidebar(to)
    }

    /// Shows or updates the plugin's boss bar `id` for the player; `progress` is 0.0 to 1.0.
    pub fn bossbar(to: Uuid, id: &str, text: impl IntoSpans, progress: f32, color: BossColor, style: BossStyle) -> Ticket {
        raw::bossbar(to, id, &text.into_spans(), progress, color, style)
    }

    pub fn clear_bossbar(to: Uuid, id: &str) -> Ticket {
        raw::clear_bossbar(to, id)
    }
}

/// `player.control`.
pub mod players {
    use crate::bindings::kiln::api::players as raw;
    use crate::{GameMode, IntoSpans, Ticket, Uuid};

    /// To another place of a level (a level id from `registry` or `InitInfo::levels`).
    pub fn teleport(who: Uuid, level: u32, pos: (f64, f64, f64), rot: (f32, f32)) -> Ticket {
        raw::teleport(who, level, pos.0, pos.1, pos.2, rot.0, rot.1)
    }

    pub fn set_game_mode(who: Uuid, mode: GameMode) -> Ticket {
        raw::set_game_mode(who, mode)
    }

    pub fn heal(who: Uuid) -> Ticket {
        raw::heal(who)
    }

    pub fn kick(who: Uuid, reason: impl IntoSpans) -> Ticket {
        raw::kick(who, &reason.into_spans())
    }
}

/// An item to hand out or show in a menu.
#[derive(Clone, Debug)]
pub struct Item(pub bindings::kiln::api::types::ItemStack);

impl Item {
    pub fn new(key: &str, count: u32) -> Item {
        Item(bindings::kiln::api::types::ItemStack {
            item: key.to_owned(),
            count,
            name: None,
            lore: Vec::new(),
            tag: None,
            model: None,
            glint: false,
        })
    }

    pub fn name(mut self, name: impl IntoSpans) -> Item {
        self.0.name = Some(name.into_spans());
        self
    }

    pub fn lore_line(mut self, line: impl IntoSpans) -> Item {
        self.0.lore.push(line.into_spans());
        self
    }

    /// Marks the item as this plugin's: `item-use` events and `ItemView::tag` carry it.
    pub fn tag(mut self, tag: &str) -> Item {
        self.0.tag = Some(tag.to_owned());
        self
    }

    pub fn model(mut self, model: &str) -> Item {
        self.0.model = Some(model.to_owned());
        self
    }

    pub fn glint(mut self) -> Item {
        self.0.glint = true;
        self
    }
}

/// A locked chest-style menu: `Menu::new("main", 3).title("Shop").item(11, Item::new(...))`.
#[derive(Clone, Debug)]
pub struct Menu {
    id: String,
    title: Vec<Span>,
    rows: u8,
    items: Vec<bindings::kiln::api::types::MenuItem>,
}

impl Menu {
    pub fn new(id: &str, rows: u8) -> Menu {
        Menu { id: id.to_owned(), title: Vec::new(), rows, items: Vec::new() }
    }

    pub fn title(mut self, title: impl IntoSpans) -> Menu {
        self.title = title.into_spans();
        self
    }

    pub fn item(mut self, slot: u8, item: Item) -> Menu {
        self.items.push(bindings::kiln::api::types::MenuItem { slot, stack: item.0 });
        self
    }

    fn into_spec(self) -> bindings::kiln::api::types::MenuSpec {
        bindings::kiln::api::types::MenuSpec { id: self.id, title: self.title, rows: self.rows, items: self.items }
    }
}

/// `inventory`: items and menus.
pub mod inventory {
    use crate::bindings::kiln::api::inventory as raw;
    use crate::{Item, Menu, Ticket, Uuid};

    pub fn give(who: Uuid, item: Item) -> Ticket {
        raw::give(who, &item.0)
    }

    pub fn take(who: Uuid, key: &str, count: u32) -> Ticket {
        raw::take(who, key, count)
    }

    pub fn clear(who: Uuid) -> Ticket {
        raw::clear(who)
    }

    /// Opens a locked menu: clicks reach [`Plugin::on_container_click`](crate::Plugin) with
    /// the menu's id and never move items.
    pub fn open_menu(who: Uuid, menu: Menu) -> Ticket {
        raw::open_menu(who, &menu.into_spec())
    }

    pub fn set_slot(who: Uuid, menu: &str, slot: u8, item: Option<Item>) -> Ticket {
        raw::set_slot(who, menu, slot, item.map(|i| i.0).as_ref())
    }

    pub fn close_menu(who: Uuid) -> Ticket {
        raw::close_menu(who)
    }
}

/// `entity.control`: entities the plugin owns.
pub mod entities {
    use crate::bindings::kiln::api::entities as raw;
    use crate::{IntoSpans, Ticket, Uuid};

    /// An entity to spawn.
    #[derive(Clone, Debug)]
    pub struct Spawn(pub crate::bindings::kiln::api::types::SpawnSpec);

    impl Spawn {
        pub fn new(kind: &str, level: u32, pos: (f64, f64, f64)) -> Spawn {
            Spawn(crate::bindings::kiln::api::types::SpawnSpec {
                kind: kind.to_owned(),
                level,
                pos,
                yaw: 0.0,
                name: None,
                no_ai: false,
                invulnerable: false,
                silent: false,
                no_gravity: false,
            })
        }
        pub fn name(mut self, name: impl IntoSpans) -> Spawn {
            self.0.name = Some(name.into_spans());
            self
        }
        pub fn yaw(mut self, yaw: f32) -> Spawn {
            self.0.yaw = yaw;
            self
        }
        /// An NPC: no AI, invulnerable, silent, in place.
        pub fn npc(mut self) -> Spawn {
            self.0.no_ai = true;
            self.0.invulnerable = true;
            self.0.silent = true;
            self.0.no_gravity = true;
            self
        }
    }

    /// Spawns the entity; its uuid is known at once.
    pub fn spawn(s: Spawn) -> Uuid {
        raw::spawn(&s.0)
    }

    pub fn remove(level: u32, id: Uuid) -> Ticket {
        raw::remove(level, id)
    }
}

/// `world.write`: blocks of the cell the call was given.
pub mod blocks {
    use crate::bindings::kiln::api::blocks as raw;
    use crate::{BlockChange, EditError, Ticket};

    pub fn change(x: i32, y: i32, z: i32, state: &str) -> BlockChange {
        BlockChange { x, y, z, state: state.to_owned() }
    }

    /// Queues the changes; every position must lie in `cell` (the cell of the event or of the
    /// position task).
    pub fn set(cell: u64, level: u32, changes: &[BlockChange]) -> Result<Ticket, EditError> {
        raw::set_blocks(cell, level, changes)
    }
}

/// `events.raise`: events between plugins, answered at once by the plugins of the same
/// context (region or global) that subscribed to `custom`.
pub mod events {
    use crate::{Decision, Player};

    /// Raises `<this plugin>:<name>`; false when a subscriber denied.
    pub fn raise(name: &str, payload: &[u8], actor: Option<&Player>) -> bool {
        crate::bindings::kiln::api::events::raise(name, payload, actor.map(|p| p.handle)) == Decision::Allow
    }
}

/// `jobs`: work for the plugin's async component (`tasks.wasm`, built with `kiln-tasks-sdk`;
/// the manifest names it with `tasks = "tasks.wasm"`).
pub mod jobs {
    use crate::Ticket;

    /// Hands the tasks component a job: `id` is the plugin's own number for it (it comes back in
    /// `on_cancelled` if a reload interrupts the job), `kind` and `payload` are the component's to
    /// read. The outcome arrives in `Plugin::on_results` under the ticket returned: `applied` true
    /// with the result bytes as `GlobalValue::Bytes` in `value`, or false with the failure text.
    pub fn submit(id: u64, kind: &str, payload: &[u8]) -> Ticket {
        crate::bindings::kiln::api::jobs::submit(id, kind, payload)
    }
}

/// Console logging.
pub mod log {
    pub use crate::bindings::kiln::api::log::{error, info, warn};
}

/// A small binary encoding for values kept in [`state`]: fixed-width integers and
/// length-prefixed strings, little-endian.
pub mod codec {
    #[derive(Default)]
    pub struct Writer(Vec<u8>);

    impl Writer {
        pub fn new() -> Writer {
            Writer(Vec::new())
        }
        pub fn i32(mut self, v: i32) -> Writer {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        pub fn i64(mut self, v: i64) -> Writer {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        pub fn f64(mut self, v: f64) -> Writer {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        pub fn u8(mut self, v: u8) -> Writer {
            self.0.push(v);
            self
        }
        pub fn str(mut self, s: &str) -> Writer {
            self.0.extend_from_slice(&(s.len() as u32).to_le_bytes());
            self.0.extend_from_slice(s.as_bytes());
            self
        }
        pub fn finish(self) -> Vec<u8> {
            self.0
        }
    }

    pub struct Reader<'a>(&'a [u8]);

    impl<'a> Reader<'a> {
        pub fn new(b: &'a [u8]) -> Reader<'a> {
            Reader(b)
        }
        fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
            let (head, rest) = self.0.split_first_chunk::<N>()?;
            self.0 = rest;
            Some(*head)
        }
        pub fn i32(&mut self) -> Option<i32> {
            self.take::<4>().map(i32::from_le_bytes)
        }
        pub fn i64(&mut self) -> Option<i64> {
            self.take::<8>().map(i64::from_le_bytes)
        }
        pub fn f64(&mut self) -> Option<f64> {
            self.take::<8>().map(f64::from_le_bytes)
        }
        pub fn u8(&mut self) -> Option<u8> {
            self.take::<1>().map(|b| b[0])
        }
        pub fn str(&mut self) -> Option<String> {
            let n = u32::from_le_bytes(self.take::<4>()?) as usize;
            if self.0.len() < n {
                return None;
            }
            let (s, rest) = self.0.split_at(n);
            self.0 = rest;
            String::from_utf8(s.to_vec()).ok()
        }
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }
    }
}

/// A `[config]` value of the manifest.
pub fn config<'a>(info: &'a InitInfo, key: &str) -> Option<&'a str> {
    info.config.iter().find(|e| e.key == key).map(|e| e.value.as_str())
}

/// The level id of a level key (`minecraft:overworld`), from the init info.
pub fn level_id(info: &InitInfo, key: &str) -> Option<u32> {
    info.levels.iter().position(|l| l == key).map(|i| i as u32)
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
    /// Also fires for right-clicks on blocks (chests, doors) and buckets.
    fn on_block_place(_ev: PlaceEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_entity_interact(_ev: EntityEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_entity_attack(_ev: EntityEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_player_damage(_ev: DamageEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_item_use(_ev: ItemUseEvent) -> Verdict {
        Verdict::Allow
    }
    fn on_container_click(_ev: ContainerClickEvent) -> Verdict {
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
    /// Outcomes of this plugin's atomic operations and effects (with the `op-results`
    /// subscription).
    fn on_results(_player: Option<Player>, _results: Vec<OpResult>) {}
    /// Tasks that will not run (global instance).
    fn on_cancelled(_tasks: Vec<CancelledTask>) {}
    /// An event another plugin raised (`custom` subscription), in the global instance or in
    /// a region instance.
    fn on_custom(_ev: CustomEvent) -> Verdict {
        Verdict::Allow
    }
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
            fn on_results(p: ::std::option::Option<$crate::Player>, r: ::std::vec::Vec<$crate::OpResult>) {
                <$t as $crate::Plugin>::on_results(p, r)
            }
            fn on_cancelled(t: ::std::vec::Vec<$crate::CancelledTask>) {
                <$t as $crate::Plugin>::on_cancelled(t)
            }
            fn on_custom(ev: $crate::CustomEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_custom(ev).into_decision()
            }
        }
        impl $crate::bindings::exports::kiln::api::region_hooks::Guest for __KilnExports {
            fn init(info: $crate::InitInfo) {
                <$t as $crate::Plugin>::init_region(info)
            }
            fn on_block_break(ev: $crate::BlockEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_block_break(ev).into_decision()
            }
            fn on_block_place(ev: $crate::PlaceEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_block_place(ev).into_decision()
            }
            fn on_entity_interact(ev: $crate::EntityEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_entity_interact(ev).into_decision()
            }
            fn on_entity_attack(ev: $crate::EntityEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_entity_attack(ev).into_decision()
            }
            fn on_player_damage(ev: $crate::DamageEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_player_damage(ev).into_decision()
            }
            fn on_item_use(ev: $crate::ItemUseEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_item_use(ev).into_decision()
            }
            fn on_container_click(ev: $crate::ContainerClickEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_container_click(ev).into_decision()
            }
            fn on_chat(ev: $crate::ChatEvent) -> $crate::ChatVerdict {
                <$t as $crate::Plugin>::on_chat(ev)
            }
            fn on_command(ev: $crate::CommandEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_region_command(ev).into_decision()
            }
            fn on_observe(events: ::std::vec::Vec<$crate::Observed>) {
                <$t as $crate::Plugin>::on_observe(events)
            }
            fn on_task(t: $crate::TaskEvent) {
                <$t as $crate::Plugin>::on_task(t)
            }
            fn on_results(p: ::std::option::Option<$crate::Player>, r: ::std::vec::Vec<$crate::OpResult>) {
                <$t as $crate::Plugin>::on_results(p, r)
            }
            fn on_custom(ev: $crate::CustomEvent) -> $crate::Decision {
                <$t as $crate::Plugin>::on_custom(ev).into_decision()
            }
        }
        $crate::bindings::export_raw!(__KilnExports with_types_in $crate::bindings);
    };
}
