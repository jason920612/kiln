//! WASM plugins in the tick (design §11), through `kiln-plugin-host`.
//!
//! - Plugins load from `SimConfig::plugins` (`KILN_PLUGINS_DIR`) at start; their namespaces
//!   live in `<world>/kiln/plugins`. Commands they register join the dispatcher.
//! - Region instance sets follow the regionizer: after every topology change each level's
//!   regions get or lose theirs ([`Sim::sync_plugin_regions`]).
//! - B0 applies the global atomic operations queued in the last tick.
//! - P: a player's break (start and finish of digging) and use-on-block (placement and
//!   interaction) are cancellable events in the region's instances, before the vanilla
//!   handling; a denial puts the client's blocks back. Block changes that went through are
//!   observed and sent in one batch per region at the end of the phase.
//! - PX: chat (cancel or rewrite), commands (cancel), plugin commands (global instance), and
//!   region packets queued behind them (the same break and place checks).
//! - Join and leave run in the global instances; messages plugins sent go out after P and
//!   after G, in a deterministic order.

use crate::region::Env;
use crate::{DIMENSIONS, Player, Sim};
use kiln_command::dispatcher::{argument, literal};
use kiln_command::arguments::ArgumentType;
use kiln_link::{ConnId, PlayIn};
use kiln_plugin_host::{Actor, ChatOutcome, PluginRuntime, RegionPlugins, RuntimeConfig, Span, Verdict};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::{CellSet, RegionId};
use kiln_world::{Blocks, Cell};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Where plugins come from and how long a cancellable call may run.
#[derive(Clone, Debug)]
pub struct PluginSettings {
    /// `<dir>/<plugin>/plugin.toml` + `plugin.wasm` (`KILN_PLUGINS_DIR`).
    pub dir: std::path::PathBuf,
    /// Wall-clock budget of each cancellable call (`KILN_PLUGIN_BUDGET_US`, default 500 µs).
    /// Timeouts depend on the machine's load, so lockstep determinism tests use a budget no
    /// call reaches (strict mode with fuel is future work).
    pub call_budget: std::time::Duration,
}

impl PluginSettings {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        PluginSettings { dir: dir.into(), call_budget: RuntimeConfig::default().call_budget }
    }
}

pub(crate) struct SimPlugins {
    rt: PluginRuntime,
    /// Operators, for the events' `operator` flag (refreshed in B0).
    ops: Arc<HashSet<Uuid>>,
}

/// A region's plugins for one phase of region work.
pub(crate) struct RegionHook<'a> {
    rp: &'a mut RegionPlugins,
    ops: Arc<HashSet<Uuid>>,
    /// Allowed breaks and placements, to observe if the block really changed: (player index
    /// key, position, state before, broken).
    watch: Vec<Watch>,
}

struct Watch {
    uuid: Uuid,
    name: String,
    operator: bool,
    pos: [i32; 3],
    before: u16,
    broken: bool,
}

fn actor<'a>(p: &'a Player, ops: &HashSet<Uuid>) -> Actor<'a> {
    Actor { uuid: p.uuid.as_u128(), name: &p.name, operator: ops.contains(&p.uuid) }
}

/// Plugin chat text as a text component.
fn text_tag(spans: &[Span]) -> Tag {
    let part = |s: &Span| {
        let mut c = vec![("text".to_owned(), Tag::String(s.text.clone()))];
        if let Some(color) = &s.color {
            c.push(("color".into(), Tag::String(color.clone())));
        }
        if s.bold {
            c.push(("bold".into(), Tag::Byte(1)));
        }
        if s.italic {
            c.push(("italic".into(), Tag::Byte(1)));
        }
        Tag::Compound(c)
    };
    match spans {
        [one] => part(one),
        _ => Tag::Compound(vec![
            ("text".into(), Tag::String(String::new())),
            ("extra".into(), Tag::List(spans.iter().map(part).collect())),
        ]),
    }
}

fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

fn block_name(state: u16) -> &'static str {
    kiln_blocks::BlockId::of(state).name()
}

/// P and PX: a region packet that breaks or uses a block goes to the region's plugins first.
/// Returns whether a plugin denied it (the packet is then dropped and the client's view of
/// the blocks restored).
pub(crate) fn deny_packet(
    hook: &mut RegionHook,
    p: &mut Player,
    cells: &CellSet<Cell>,
    env: &Env,
    pkt: &PlayIn,
    spawns: &mut Vec<crate::entities::Spawn>,
) -> bool {
    let level = DIMENSIONS[env.dim].0;
    let block = |pos: [i32; 3]| cells.get_block(pos[0], pos[1], pos[2]).unwrap_or(0);
    match *pkt {
        PlayIn::PlayerAction { action, pos, sequence, .. }
            if action == crate::digging::START_DESTROY_BLOCK || action == crate::digging::STOP_DESTROY_BLOCK =>
        {
            let state = block(pos);
            if kiln_data::blocks_types::is_air(state) {
                return false;
            }
            let a = actor(p, &hook.ops);
            match hook.rp.block_break(&a, level, pos, block_name(state)) {
                Verdict::Allow => {
                    hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos, before: state, broken: true });
                    false
                }
                Verdict::Deny(msg) => {
                    p.digging = None;
                    p.send(packets::block_update(pos, state));
                    p.ack_block_changes = p.ack_block_changes.max(sequence);
                    if let Some(m) = msg {
                        p.send(packets::system_chat(text_tag(&m), false));
                    }
                    true
                }
            }
        }
        PlayIn::UseItemOn { hand, pos, face, sequence, .. } => {
            let Some(dir) = crate::blocks::direction(face) else { return false };
            let step = dir.step();
            let next = [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]];
            use kiln_item::component::EquipmentSlot;
            let held = if hand == 0 { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
            let item = if held.is_empty() {
                "minecraft:air"
            } else {
                kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(held.item() as usize).copied()).unwrap_or("minecraft:air")
            };
            let a = actor(p, &hook.ops);
            match hook.rp.block_place(&a, level, next, pos, item) {
                Verdict::Allow => {
                    for at in [pos, next] {
                        hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos: at, before: block(at), broken: false });
                    }
                    false
                }
                Verdict::Deny(msg) => {
                    p.send(packets::block_update(pos, block(pos)));
                    p.send(packets::block_update(next, block(next)));
                    p.ack_block_changes = p.ack_block_changes.max(sequence);
                    // The client may have used up the item in its prediction.
                    p.with_menu(&env.rules, spawns, |menu, _, env| menu.send_all_data_to_remote(env));
                    if let Some(m) = msg {
                        p.send(packets::system_chat(text_tag(&m), false));
                    }
                    true
                }
            }
        }
        _ => false,
    }
}

/// End of a phase's packets: allowed breaks and placements that changed their block are
/// observed, and the batch goes to the region's observers.
pub(crate) fn after_packets(hook: &mut RegionHook, cells: &CellSet<Cell>, env: &Env) {
    let level = DIMENSIONS[env.dim].0;
    for w in std::mem::take(&mut hook.watch) {
        let now = cells.get_block(w.pos[0], w.pos[1], w.pos[2]).unwrap_or(0);
        if now == w.before || (w.broken && !kiln_data::blocks_types::is_air(now)) {
            continue;
        }
        let a = Actor { uuid: w.uuid.as_u128(), name: &w.name, operator: w.operator };
        let name = block_name(if w.broken { w.before } else { now });
        hook.rp.observe_block(w.broken, &a, level, w.pos, name);
    }
    hook.rp.flush_observed();
}

impl SimPlugins {
    /// The region hooks for one parallel phase, by region.
    pub(crate) fn hooks(&mut self) -> BTreeMap<(crate::DimId, RegionId), RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt
            .regions_mut()
            .map(|((dim, r), rp)| ((dim as crate::DimId, RegionId(r)), RegionHook { rp, ops: ops.clone(), watch: Vec::new() }))
            .collect()
    }

    /// One region's hook (PX: a region packet queued behind a serial one).
    pub(crate) fn hook(&mut self, dim: crate::DimId, region: RegionId) -> Option<RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt.region_mut(dim as u32, region.0).map(|rp| RegionHook { rp, ops, watch: Vec::new() })
    }
}

impl Sim {
    /// Loads the plugins of `SimConfig::plugins` and registers their commands.
    pub(crate) fn load_plugins(&mut self) {
        let Some(PluginSettings { dir, call_budget }) = self.config.plugins.clone() else { return };
        let cfg = RuntimeConfig {
            data_dir: self.config.world.as_ref().map(|w| w.join("kiln").join("plugins")),
            levels: DIMENSIONS.iter().map(|(k, _)| (*k).to_owned()).collect(),
            spawn: self.spawn,
            call_budget,
            ..RuntimeConfig::default()
        };
        let rt = match PluginRuntime::load_dir(&dir, cfg) {
            Ok(rt) => rt,
            Err(e) => {
                warn!("plugins: {e:#}");
                return;
            }
        };
        info!("{} plugins loaded from {}", rt.ids().len(), dir.display());
        let dispatcher = std::sync::Arc::get_mut(&mut self.commands.dispatcher).expect("dispatcher not shared yet");
        for reg in rt.commands() {
            if dispatcher.find(&[reg.name.as_str()]).is_some() {
                warn!("plugin {}: command /{} already exists", rt.ids()[reg.plugin], reg.name);
                continue;
            }
            let (plugin, name) = (reg.plugin, reg.name.clone());
            let (plugin2, name2) = (reg.plugin, reg.name.clone());
            dispatcher.register(
                literal::<Sim>(&reg.name)
                    .requires(reg.permission)
                    .executes(move |_, sim| Ok(sim.run_plugin_command(plugin, &name, "")))
                    .then(
                        argument::<Sim>("args", ArgumentType::greedy_string())
                            .executes(move |ctx, sim: &mut Sim| Ok(sim.run_plugin_command(plugin2, &name2, ctx.arg_text("args").unwrap_or("")))),
                    ),
            );
        }
        self.plugins = Some(SimPlugins { rt, ops: Arc::new(HashSet::new()) });
        self.sync_plugin_regions();
    }

    fn run_plugin_command(&mut self, plugin: usize, name: &str, args: &str) -> i32 {
        let source = self.commands.source;
        let Some(pl) = self.plugins.as_mut() else { return 0 };
        let player = match source {
            crate::commands::CommandSource::Player(conn) => self.players.get(&conn),
            crate::commands::CommandSource::Console => None,
        };
        let a = player.map(|p| actor(p, &pl.ops));
        let reply = pl.rt.run_command(plugin, a.as_ref(), name, args);
        if !reply.is_empty() {
            let pkt = packets::system_chat(text_tag(&reply), false);
            match source {
                crate::commands::CommandSource::Player(conn) => {
                    if let Some(p) = self.players.get_mut(&conn) {
                        p.send(pkt);
                    }
                }
                crate::commands::CommandSource::Console => info!("{}", plain(&reply)),
            }
        }
        self.deliver_plugin_messages();
        1
    }

    /// Keeps each level's region instance sets in step with its regions.
    pub(crate) fn sync_plugin_regions(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for (dim, d) in self.dims.iter().enumerate() {
            pl.rt.sync_regions(dim as u32, d.regions.iter().map(|r| r.id().0));
        }
    }

    /// B0: the global operations of the last tick, and who is an operator.
    pub(crate) fn plugins_b0(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        pl.rt.begin_tick();
        let ops: HashSet<Uuid> = self.players.values().filter(|p| self.commands.is_op(&p.name)).map(|p| p.uuid).collect();
        if ops != *pl.ops {
            pl.ops = Arc::new(ops);
        }
    }

    /// PX: chat goes through the player's region instances. Returns whether a plugin handled
    /// it (cancelled, or rewrote and broadcast it).
    pub(crate) fn plugin_chat(&mut self, conn: ConnId, message: &str) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return false };
        let Some(p) = self.players.get(&conn) else { return false };
        let a = actor(p, &pl.ops);
        let Some(rp) = pl.rt.region_mut(p.dim as u32, p.region.0) else { return false };
        match rp.chat(&a, message) {
            ChatOutcome::Pass => false,
            ChatOutcome::Cancel => {
                info!("<{}> {message} (cancelled by a plugin)", p.name);
                true
            }
            ChatOutcome::Rewrite(spans) => {
                info!("{}", plain(&spans));
                self.broadcast(packets::system_chat(text_tag(&spans), false));
                true
            }
        }
    }

    /// PX: whether a region instance cancels a player's command.
    pub(crate) fn plugin_command_denied(&mut self, conn: ConnId, command: &str) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return false };
        let Some(p) = self.players.get_mut(&conn) else { return false };
        let a = Actor { uuid: p.uuid.as_u128(), name: &p.name, operator: pl.ops.contains(&p.uuid) };
        let Some(rp) = pl.rt.region_mut(p.dim as u32, p.region.0) else { return false };
        match rp.command(&a, command) {
            Verdict::Allow => false,
            Verdict::Deny(msg) => {
                if let Some(m) = msg {
                    p.send(packets::system_chat(text_tag(&m), false));
                }
                true
            }
        }
    }

    pub(crate) fn plugins_joined(&mut self, conn: ConnId) {
        let Some(pl) = self.plugins.as_mut() else { return };
        let Some(p) = self.players.get(&conn) else { return };
        if self.commands.is_op(&p.name) {
            Arc::make_mut(&mut pl.ops).insert(p.uuid);
        }
        pl.rt.player_joined(&actor(p, &pl.ops));
    }

    pub(crate) fn plugins_left(&mut self, p: &Player) {
        let Some(pl) = self.plugins.as_mut() else { return };
        pl.rt.player_left(&actor(p, &pl.ops));
    }

    /// Sends what plugins said (after P and after G).
    pub(crate) fn deliver_plugin_messages(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for m in pl.rt.take_messages() {
            let pkt = packets::system_chat(text_tag(&m.text), false);
            match m.to {
                None => self.broadcast(pkt),
                Some(u) => {
                    let uuid = Uuid::from_u128(u);
                    if let Some(p) = self.players.values_mut().find(|p| p.uuid == uuid) {
                        p.send(pkt);
                    }
                }
            }
        }
    }

    pub(crate) fn save_plugins(&self) {
        if let Some(pl) = &self.plugins {
            pl.rt.save();
        }
    }

    /// Plugin calls, traps and timeouts so far (tests and tools).
    pub fn plugin_stats(&self) -> (u64, u64, u64) {
        let Some(pl) = &self.plugins else { return (0, 0, 0) };
        let s = pl.rt.stats();
        let load = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
        (load(&s.calls), load(&s.traps), load(&s.timeouts))
    }

    /// A player's value of a plugin's key (tests and tools).
    pub fn plugin_player_value(&self, uuid: Uuid, plugin: &str, key: &str) -> Option<Vec<u8>> {
        self.plugins.as_ref()?.rt.player_value(uuid.as_u128(), plugin, key)
    }

    pub(crate) fn hash_plugins(&self, h: &mut impl std::hash::Hasher) {
        if let Some(pl) = &self.plugins {
            pl.rt.hash_state(h);
        }
    }
}
