//! WASM plugins in the tick (design §11), through `kiln-plugin-host`.
//!
//! - Plugins load from `SimConfig::plugins` (`KILN_PLUGINS_DIR`) at start; their namespaces
//!   live in `<world>/kiln/plugins`, compiled components in `<world>/kiln/plugins/cache` (or
//!   `<plugins>/.cache` without a world). Commands they register join the dispatcher.
//! - Region instance sets follow the regionizer: after every topology change each level's
//!   regions get or lose theirs ([`Sim::sync_plugin_regions`]).
//! - B0: staged hot reloads swap in, the global atomic operations queued in the last tick
//!   apply, their results go back, due tasks run (player tasks in the region holding the
//!   player, position tasks in the region owning the position).
//! - P: a player's break (start and finish of digging), use-on-block (placement and
//!   interaction), bucket use (the block the bucket would fill or empty) and entity
//!   interaction are cancellable events in the region's instances, before the vanilla
//!   handling; a denial puts the client's blocks and inventory back. Block changes that went
//!   through are observed and sent in one batch per region at the end of the phase; a
//!   survival break the server completes later (the client finished early) is observed in
//!   the block phase that completes it.
//! - Entity data (the entity scope) lives in the entity's NBT under `kiln:plugin`, so it
//!   follows the entity across regions and is saved with its chunk.
//! - PX: chat (cancel or rewrite), commands (cancel), plugin commands (global instance), and
//!   region packets queued behind them (the same checks); `/kiln plugins` lists plugins and
//!   `/kiln plugins reload <id>` reloads one (compiled off the tick, swapped in B0).
//! - Join and leave run in the global instances; messages plugins sent go out after P and
//!   after G, in a deterministic order.

use crate::region::Env;
use crate::{DIMENSIONS, Player, Sim};
use kiln_command::arguments::ArgumentType;
use kiln_command::dispatcher::{argument, literal};
use kiln_link::{ConnId, PlayIn};
use kiln_plugin_host::{
    Actor, ChatOutcome, ClickKind, ContainerClick, EntityData, EntityRef, EventKind, ExecMode, ItemRef,
    BlockWindow, OnlinePlayer, PlayerAt, PlayerInfo, PluginRuntime, RegionPlugins, Registries, RegistryKind, RuntimeConfig, Span, Verdict, World,
};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_region::{CellPos, CellSet, RegionId};
use kiln_world::{Blocks, Cell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tracing::{info, warn};
use uuid::Uuid;

mod effects;

pub(crate) use kiln_plugin_host::SpawnReason;

/// Where plugins come from and how they are budgeted.
#[derive(Clone, Debug)]
pub struct PluginSettings {
    /// `<dir>/<plugin>/plugin.toml` + `plugin.wasm` (`KILN_PLUGINS_DIR`).
    pub dir: std::path::PathBuf,
    /// Ordered or strict (`KILN_PLUGIN_MODE=strict`); see `kiln_plugin_host` for the two.
    pub mode: ExecMode,
    /// Ordered mode: wall-clock budget of each cancellable call (`KILN_PLUGIN_BUDGET_US`,
    /// default 500 µs). Timeouts depend on the machine's load, so lockstep determinism tests
    /// either use a budget no call reaches or strict mode.
    pub call_budget: std::time::Duration,
    /// Strict mode: fuel of each call (`KILN_PLUGIN_FUEL`).
    pub call_fuel: u64,
    /// Seed of the plugins' random streams, tickets and task handles.
    pub seed: u64,
}

impl PluginSettings {
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        let d = RuntimeConfig::default();
        PluginSettings { dir: dir.into(), mode: ExecMode::Ordered, call_budget: d.call_budget, call_fuel: d.call_fuel, seed: 0 }
    }

    /// Strict mode with `fuel` per call: budgets that replay exactly.
    pub fn strict(dir: impl Into<std::path::PathBuf>, fuel: u64) -> Self {
        PluginSettings { mode: ExecMode::Strict, call_fuel: fuel, ..PluginSettings::new(dir) }
    }
}

pub(crate) struct SimPlugins {
    rt: PluginRuntime,
    /// Operators, for the events' `operator` flag (refreshed in B0).
    ops: Arc<HashSet<Uuid>>,
    /// Command names registered in the dispatcher.
    registered: HashSet<String>,
    /// What plugins put on each player's screen (to update and clear it).
    hud: HashMap<Uuid, Hud>,
    /// Who was online, and where, when plugins last heard of it.
    online: Vec<(Uuid, usize)>,
    /// Players that appeared in a level, to tell the plugins of their region once they have one.
    spawned: Vec<(ConnId, SpawnReason)>,
}

/// A player's plugin-made screen elements.
#[derive(Default)]
struct Hud {
    /// The sidebar's line count while one is shown.
    sidebar: Option<usize>,
    /// Boss bars shown (`<plugin id>:<id>`).
    bars: BTreeSet<String>,
}

/// A region's plugins for one phase of region work.
pub(crate) struct RegionHook<'a> {
    rp: &'a RegionPlugins,
    ops: Arc<HashSet<Uuid>>,
    /// Allowed breaks and placements, to observe if the block really changed.
    watch: Vec<Watch>,
}

impl RegionHook<'_> {
    /// The handle players carry to ask the region's plugins about damage, if any hears of it.
    pub(crate) fn damage_gate(&self) -> Option<RegionPlugins> {
        self.rp.subscribed(EventKind::PlayerDamage).then(|| self.rp.clone())
    }
}

struct Watch {
    uuid: Uuid,
    name: String,
    operator: bool,
    pos: [i32; 3],
    before: u16,
    broken: bool,
}

/// What plugins can ask about a player (`event.info`).
pub(crate) fn info_of(p: &Player) -> PlayerInfo {
    let held = p.inv.selected_item();
    PlayerInfo {
        level: p.dim as u32,
        pos: p.pos,
        rot: p.rot,
        health: p.health,
        food: p.food.max(0) as u32,
        game_mode: p.game_mode,
        on_ground: p.on_ground,
        sneaking: p.sneaking,
        sprinting: p.sprinting,
        flying: p.flying,
        held: (!held.is_empty()).then(|| held.item() as u32),
        held_count: held.count().max(0) as u32,
    }
}

fn actor<'a>(p: &'a Player, ops: &HashSet<Uuid>) -> Actor<'a> {
    Actor::new(p.uuid.as_u128(), &p.name, ops.contains(&p.uuid)).with_info(info_of(p))
}

impl Player {
    /// The damage gate: whether the plugins of the player's region let `amount` of damage
    /// through (`Player::hurt` asks once the hit would land). What bypasses invulnerability
    /// (`/kill`, the void) is not asked.
    pub(crate) fn plugin_allows_damage(&self, amount: f32, source: &crate::health::Source) -> bool {
        let Some(gate) = &self.plugin_gate else { return true };
        if source.is("minecraft:bypasses_invulnerability") {
            return true;
        }
        let victim = Actor::new(self.uuid.as_u128(), &self.name, self.permission >= 4).with_info(info_of(self));
        let attacker = source.attacker.as_ref().filter(|a| a.mob.is_none() && a.uuid != 0).map(|a| {
            Actor::new(a.uuid, &a.name, false).with_info(PlayerInfo { pos: a.pos, game_mode: if a.creative { 1 } else { 0 }, ..PlayerInfo::default() })
        });
        let pos = self.pos.map(|c| c.floor() as i32);
        match gate.player_damage(&victim, attacker.as_ref(), pos, source.type_id() as u32, amount) {
            Verdict::Allow => true,
            Verdict::Deny(_) => false,
        }
    }
}

/// The registries plugins see: levels, blocks, items and entity types by registry id, with
/// the synchronized tags.
fn registries() -> Arc<Registries> {
    static REG: OnceLock<Arc<Registries>> = OnceLock::new();
    REG.get_or_init(|| {
        let list = |r: &str| kiln_data::builtin_entries(r).unwrap_or(&[]).iter().map(|s| s.to_string()).collect();
        let levels = DIMENSIONS.iter().map(|(k, _)| (*k).to_owned()).collect();
        // Damage types by their network id (the order of the synchronized registry).
        let damage_types = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:damage_type")
            .map(|(_, e)| e.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        let reg = Registries::new(levels, list("minecraft:block"), list("minecraft:item"), list("minecraft:entity_type")).with_damage_types(damage_types);
        Arc::new(reg.with_tags(Arc::new(|kind, tag| {
            let registry = match kind {
                RegistryKind::Block => "minecraft:block",
                RegistryKind::Item => "minecraft:item",
                RegistryKind::EntityType => "minecraft:entity_type",
                RegistryKind::Level | RegistryKind::DamageType => return None,
            };
            let full = if tag.contains(':') { tag.to_owned() } else { format!("minecraft:{tag}") };
            let (_, tags) = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == registry)?;
            let (_, ids) = tags.iter().find(|(t, _)| *t == full)?;
            Some(ids.iter().map(|&i| i as u32).collect())
        })))
    })
    .clone()
}

/// `world-read`: when a plugin that reads blocks is going to hear of the event, the blocks around `pos` are copied for it.
fn provide_blocks(hook: &RegionHook, kind: EventKind, pos: [i32; 3], cells: &CellSet<Cell>) {
    if hook.rp.wants_blocks(kind) {
        hook.rp.provide_blocks(BlockWindow::new(pos, |x, y, z| cells.get_block(x, y, z).map_or(BlockWindow::UNLOADED, block_id)));
    }
}

/// `player-moved`: a player whose block position changed since the last tick (a level change is a spawn, not a move).
pub(crate) fn observe_moves(hook: &mut RegionHook, players: &mut [&mut Player]) {
    if !hook.rp.observing_kind(kiln_plugin_host::ObserveKinds::PLAYER_MOVED) {
        return;
    }
    for p in players.iter_mut() {
        let now = (p.dim, p.pos.map(|c| c.floor() as i32));
        if let Some((dim, before)) = p.plugin_block.replace(now)
            && dim == now.0
            && before != now.1
        {
            hook.rp.observe_move(&actor(p, &hook.ops), before, now.1);
        }
    }
}

/// The block registry id of a state.
fn block_id(state: u16) -> u32 {
    static IDS: OnceLock<Vec<u32>> = OnceLock::new();
    let ids = IDS.get_or_init(|| {
        kiln_data::blocks::BLOCKS
            .iter()
            .map(|b| kiln_data::builtin_id("minecraft:block", b.name).map_or(u32::MAX, |i| i as u32))
            .collect()
    });
    ids.get(kiln_blocks::BlockId::of(state).0 as usize).copied().unwrap_or(u32::MAX)
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

/// The block a bucket aims at (`BucketItem.use`'s ray, [`crate::use_item::pov_hit`]; an empty
/// bucket stops at fluid sources) and the block in front of the hit face (where a filled
/// bucket empties).
fn bucket_target(p: &Player, rot: [f32; 2], cells: &CellSet<Cell>, empty: bool) -> Option<([i32; 3], [i32; 3])> {
    use crate::use_item::FluidMode;
    let block = |pos: kiln_blocks::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR);
    let fluid = if empty { FluidMode::SourceOnly } else { FluidMode::None };
    let hit = crate::use_item::pov_hit_rot(p, rot, &block, fluid)?;
    let next = hit.pos.relative(hit.face);
    Some(([hit.pos.x, hit.pos.y, hit.pos.z], [next.x, next.y, next.z]))
}

/// The item id in a hand (`None` when empty).
fn held_item(p: &Player, hand: i32) -> (Option<u32>, &'static str) {
    use kiln_item::component::EquipmentSlot;
    let held = if hand == 0 { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    if held.is_empty() {
        return (None, "minecraft:air");
    }
    let name = kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(held.item() as usize).copied()).unwrap_or("minecraft:air");
    (Some(held.item() as u32), name)
}

/// A denied use: the client's view of the blocks and its inventory are put back.
fn restore(p: &mut Player, env: &Env, spawns: &mut Vec<crate::entities::Spawn>, blocks: &[([i32; 3], u16)], sequence: i32, msg: Option<Vec<Span>>) {
    for &(pos, state) in blocks {
        p.send(packets::block_update(pos, state));
    }
    p.ack_block_changes = p.ack_block_changes.max(sequence);
    // The client may have used up the item in its prediction.
    p.with_menu(&env.rules, spawns, |menu, _, env| menu.send_all_data_to_remote(env));
    if let Some(m) = msg {
        p.send(packets::system_chat(text_tag(&m), false));
    }
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
            provide_blocks(hook, EventKind::BlockBreak, pos, cells);
            match hook.rp.block_break(&a, pos, block_id(state)) {
                Verdict::Allow => {
                    hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos, before: state, broken: true });
                    false
                }
                Verdict::Deny(msg) => {
                    p.digging = None;
                    p.delayed_destroy = None;
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
            if let Some(Verdict::Deny(msg)) = item_use_verdict(hook, p, hand != 0, Some(pos)) {
                restore(p, env, spawns, &[(pos, block(pos)), (next, block(next))], sequence, msg);
                return true;
            }
            let (item, _) = held_item(p, hand);
            let a = actor(p, &hook.ops);
            provide_blocks(hook, EventKind::BlockPlace, next, cells);
            match hook.rp.block_place(&a, next, pos, item) {
                Verdict::Allow => {
                    for at in [pos, next] {
                        hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos: at, before: block(at), broken: false });
                    }
                    false
                }
                Verdict::Deny(msg) => {
                    restore(p, env, spawns, &[(pos, block(pos)), (next, block(next))], sequence, msg);
                    true
                }
            }
        }
        // Buckets act on the block they aim at (`BucketItem.use` ray-traces from the eye):
        // the same placement check applies to the block they would fill or empty.
        PlayIn::UseItem { hand, sequence, yaw, pitch } => {
            let hand = if hand == kiln_proto::packets::serverbound::Hand::Off { 1 } else { 0 };
            if let Some(Verdict::Deny(msg)) = item_use_verdict(hook, p, hand != 0, None) {
                p.ack_block_changes = p.ack_block_changes.max(sequence);
                resync_menu(p, env, spawns);
                if let Some(m) = msg {
                    p.send(packets::system_chat(text_tag(&m), false));
                }
                return true;
            }
            let (item, name) = held_item(p, hand);
            if !crate::buckets::is_bucket(name) {
                return false;
            }
            // The packet carries the rotation the client used (`handleUseItem` applies it).
            let Some((hit, before)) = bucket_target(p, [yaw, pitch], cells, name == "minecraft:bucket") else { return false };
            let target = if name == "minecraft:bucket" { hit } else { before };
            let a = actor(p, &hook.ops);
            provide_blocks(hook, EventKind::BlockPlace, target, cells);
            match hook.rp.block_place(&a, target, hit, item) {
                Verdict::Allow => {
                    hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator: a.operator, pos: target, before: block(target), broken: false });
                    false
                }
                Verdict::Deny(msg) => {
                    restore(p, env, spawns, &[(hit, block(hit)), (before, block(before))], sequence, msg);
                    true
                }
            }
        }
        PlayIn::ContainerClick { ref body } => container_click(hook, p, env, body, spawns),
        _ => false,
    }
}

/// The plugin tag of a stack (`kiln:tag` in its custom data: `<plugin id>:<tag>`).
fn plugin_tag(stack: &kiln_item::ItemStack) -> Option<String> {
    let data = stack.get(kiln_item::keys::CUSTOM_DATA)?;
    data.0.get("kiln:tag").and_then(Tag::as_str).map(str::to_owned)
}

/// The plugins' answer to the held item being used (none: nobody hears of it).
fn item_use_verdict(hook: &RegionHook, p: &Player, off_hand: bool, target: Option<[i32; 3]>) -> Option<Verdict> {
    if !hook.rp.subscribed(EventKind::ItemUse) {
        return None;
    }
    let stack = if off_hand { p.inv.equipped(kiln_item::component::EquipmentSlot::OffHand) } else { p.inv.selected_item() };
    if stack.is_empty() {
        return None;
    }
    let tag = plugin_tag(stack);
    let a = actor(p, &hook.ops);
    let item = ItemRef { item: stack.item() as u32, count: stack.count().max(0) as u32, tag: tag.as_deref() };
    Some(hook.rp.item_use(&a, item, off_hand, target))
}

/// Sends the client the whole open menu again (what it predicted did not happen).
fn resync_menu(p: &mut Player, env: &Env, spawns: &mut Vec<crate::entities::Spawn>) {
    p.with_menu(&env.rules, spawns, |menu, _, env| menu.send_all_data_to_remote(env));
}

/// The stack in a slot of a menu the player sees (the player's own slots and a plugin menu's).
fn clicked_stack(p: &Player, menu: &kiln_inventory::Menu, slot: i16) -> Option<kiln_item::ItemStack> {
    let s = menu.slots().get(usize::try_from(slot).ok()?)?;
    let stack = match s.source {
        kiln_inventory::Source::Player => p.inv.items.get(s.index)?.clone(),
        kiln_inventory::Source::Block if matches!(p.containers.open, Some(crate::container::open::OpenBlock::Plugin(_))) => {
            p.containers.cart.items.get(s.index)?.clone()
        }
        _ => return None,
    };
    (!stack.is_empty()).then_some(stack)
}

/// P: a click in a container screen. Clicks in a plugin's menu are always consumed (the menu
/// is locked: nothing moves) after the plugin heard of them; a vanilla container's click
/// goes to plugins subscribed with `vanilla`, and a denial puts the client's view back.
fn container_click(hook: &mut RegionHook, p: &mut Player, env: &Env, body: &bytes::Bytes, spawns: &mut Vec<crate::entities::Spawn>) -> bool {
    use kiln_inventory::click::ContainerInput;
    let plugin_menu = match &p.containers.open {
        Some(crate::container::open::OpenBlock::Plugin(id)) => Some(id.clone()),
        _ => None,
    };
    if plugin_menu.is_none() && !hook.rp.subscribed(EventKind::ContainerClick) {
        return false;
    }
    let Ok(click) = kiln_inventory::ContainerClick::decode(body) else { return false };
    let (menu, menu_id) = if click.container_id == 0 {
        (&p.menu, None)
    } else {
        match &p.open_menu {
            Some(m) if m.container_id == click.container_id => (m, plugin_menu.clone()),
            // A click for a screen that is no longer open: vanilla ignores it.
            _ => return false,
        }
    };
    let clicked = clicked_stack(p, menu, click.slot);
    let tag = clicked.as_ref().and_then(plugin_tag);
    let kind = match (click.input, click.button) {
        (ContainerInput::Pickup, 0) => ClickKind::Left,
        (ContainerInput::Pickup, 1) => ClickKind::Right,
        (ContainerInput::QuickMove, 0) => ClickKind::ShiftLeft,
        (ContainerInput::QuickMove, _) => ClickKind::ShiftRight,
        (ContainerInput::Swap, _) => ClickKind::Swap,
        (ContainerInput::Clone, _) => ClickKind::Middle,
        (ContainerInput::Throw, _) => ClickKind::Drop,
        (ContainerInput::QuickCraft, _) => ClickKind::Drag,
        (ContainerInput::PickupAll, _) => ClickKind::Double,
        _ => ClickKind::Other,
    };
    let event = ContainerClick {
        menu: menu_id.as_deref(),
        container: menu.kind.menu_type().unwrap_or("minecraft:inventory"),
        slot: click.slot as i32,
        button: click.button as u8,
        kind,
        clicked: clicked.as_ref().map(|s| ItemRef { item: s.item() as u32, count: s.count().max(0) as u32, tag: tag.as_deref() }),
    };
    let a = actor(p, &hook.ops);
    let verdict = hook.rp.container_click(&a, &event);
    if plugin_menu.is_some() && click.container_id != 0 || matches!(verdict, Verdict::Deny(_)) {
        resync_menu(p, env, spawns);
        return true;
    }
    false
}

/// P: a player hits an entity (not another player: that is damage, see
/// [`Player::plugin_allows_damage`]). Returns whether a plugin denied it.
pub(crate) fn deny_attack(hook: &mut RegionHook, p: &mut Player, entities: &mut crate::entities::Entities, entity_id: i32) -> bool {
    if !hook.rp.subscribed(EventKind::EntityAttack) {
        return false;
    }
    let Ok(idx) = entities.list.binary_search_by_key(&entity_id, |e| e.id) else { return false };
    let e = &mut entities.list[idx];
    let Some(kind) = kiln_data::builtin_id("minecraft:entity_type", e.kind.name) else { return false };
    let (uuid, pos) = (e.uuid.as_u128(), e.pos);
    let eye = p.eye_position();
    if (0..3).map(|i| (pos[i] - eye[i]).powi(2)).sum::<f64>() > 8.0 * 8.0 {
        return false;
    }
    let Some(phys) = e.phys.as_deref_mut() else { return false };
    let mut data = entity_data(&phys.extra);
    let before = data.clone();
    let a = actor(p, &hook.ops);
    let mut r = EntityRef { uuid, kind: kind as u32, pos, data: &mut data };
    let v = hook.rp.entity_attack(&a, &mut r);
    if data != before {
        set_entity_data(&mut phys.extra, &data);
    }
    match v {
        Verdict::Allow => false,
        Verdict::Deny(msg) => {
            if let Some(m) = msg {
                p.send(packets::system_chat(text_tag(&m), false));
            }
            true
        }
    }
}

/// An entity's plugin data from its NBT (`kiln:plugin`: plugin id → key → bytes).
fn entity_data(extra: &[(String, Tag)]) -> EntityData {
    let mut data = EntityData::new();
    let Some((_, Tag::Compound(plugins))) = extra.iter().find(|(k, _)| k == KEY) else { return data };
    for (id, kv) in plugins {
        let Tag::Compound(kv) = kv else { continue };
        let map = kv
            .iter()
            .filter_map(|(k, v)| match v {
                Tag::ByteArray(b) => Some((k.clone(), b.iter().map(|&x| x as u8).collect())),
                _ => None,
            })
            .collect();
        data.insert(id.clone(), map);
    }
    data
}

fn set_entity_data(extra: &mut Vec<(String, Tag)>, data: &EntityData) {
    extra.retain(|(k, _)| k != KEY);
    if data.is_empty() {
        return;
    }
    let plugins = data
        .iter()
        .map(|(id, kv)| {
            let kv = kv.iter().map(|(k, v)| (k.clone(), Tag::ByteArray(v.iter().map(|&x| x as i8).collect()))).collect();
            (id.clone(), Tag::Compound(kv))
        })
        .collect();
    extra.push((KEY.to_owned(), Tag::Compound(plugins)));
}

const KEY: &str = "kiln:plugin";

/// P: a right click on an entity goes to the region's plugins first (they may read and write
/// the entity's data). Returns whether a plugin denied it.
pub(crate) fn deny_interact(hook: &mut RegionHook, p: &mut Player, entities: &mut crate::entities::Entities, entity_id: i32) -> bool {
    let Ok(idx) = entities.list.binary_search_by_key(&entity_id, |e| e.id) else { return false };
    let e = &mut entities.list[idx];
    let Some(kind) = kiln_data::builtin_id("minecraft:entity_type", e.kind.name) else { return false };
    let (uuid, pos) = (e.uuid.as_u128(), e.pos);
    // Out of reach: vanilla drops the packet before any interaction (`canInteractWithEntity`).
    let eye = p.eye_position();
    if (0..3).map(|i| (pos[i] - eye[i]).powi(2)).sum::<f64>() > 8.0 * 8.0 {
        return false;
    }
    // The vanilla state carries the NBT fields Kiln does not model, `kiln:plugin` among them.
    let Some(phys) = e.phys.as_deref_mut() else { return false };
    let mut data = entity_data(&phys.extra);
    let before = data.clone();
    let a = actor(p, &hook.ops);
    let mut r = EntityRef { uuid, kind: kind as u32, pos, data: &mut data };
    let v = hook.rp.entity_interact(&a, &mut r);
    if data != before {
        set_entity_data(&mut phys.extra, &data);
    }
    match v {
        Verdict::Allow => false,
        Verdict::Deny(msg) => {
            if let Some(m) = msg {
                p.send(packets::system_chat(text_tag(&m), false));
            }
            true
        }
    }
}

/// End of a phase's packets (or of the block phase): allowed breaks and placements that
/// changed their block are observed, and the batch goes to the region's observers.
pub(crate) fn after_packets(hook: &mut RegionHook, cells: &CellSet<Cell>, _env: &Env) {
    for w in std::mem::take(&mut hook.watch) {
        let now = cells.get_block(w.pos[0], w.pos[1], w.pos[2]).unwrap_or(0);
        if now == w.before || (w.broken && !kiln_data::blocks_types::is_air(now)) {
            continue;
        }
        let a = Actor::new(w.uuid.as_u128(), &w.name, w.operator);
        hook.rp.observe_block(w.broken, &a, w.pos, block_id(if w.broken { w.before } else { now }));
    }
    hook.rp.flush_observed();
}

/// Before the block phase: a survival break the client finished early completes in it
/// (`delayed_destroy`); watch those blocks so the break is observed when it happens.
pub(crate) fn watch_delayed_breaks(hook: &mut RegionHook, players: &[&mut Player], cells: &CellSet<Cell>) {
    if !hook.rp.observing() {
        return;
    }
    for p in players {
        if let Some(dig) = p.delayed_destroy {
            let before = cells.get_block(dig.pos[0], dig.pos[1], dig.pos[2]).unwrap_or(0);
            let operator = hook.ops.contains(&p.uuid);
            hook.watch.push(Watch { uuid: p.uuid, name: p.name.clone(), operator, pos: dig.pos, before, broken: true });
        }
    }
}

impl SimPlugins {
    /// The region hooks for one parallel phase, by region.
    pub(crate) fn hooks(&mut self) -> BTreeMap<(crate::DimId, RegionId), RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt
            .regions_mut()
            .map(|((dim, r), rp)| ((dim as crate::DimId, RegionId(r)), RegionHook { rp: &*rp, ops: ops.clone(), watch: Vec::new() }))
            .collect()
    }

    /// One region's hook (PX: a region packet queued behind a serial one).
    pub(crate) fn hook(&mut self, dim: crate::DimId, region: RegionId) -> Option<RegionHook<'_>> {
        let ops = self.ops.clone();
        self.rt.region_mut(dim as u32, region.0).map(|rp| RegionHook { rp: &*rp, ops, watch: Vec::new() })
    }
}

/// What B0 routes tasks and results with: the players and the level regions.
struct SimWorld<'a> {
    players: &'a crate::FastMap<ConnId, Player>,
    dims: &'a [crate::Dim],
    ops: &'a HashSet<Uuid>,
}

impl World for SimWorld<'_> {
    fn player(&self, uuid: u128) -> Option<PlayerAt> {
        let u = Uuid::from_u128(uuid);
        let p = self.players.values().find(|p| p.uuid == u && !p.disconnected)?;
        Some(PlayerAt { uuid, level: p.dim as u32, region: p.region.0, name: p.name.clone(), operator: self.ops.contains(&u), info: info_of(p) })
    }

    fn owner(&self, level: u32, x: i32, z: i32) -> Option<u64> {
        self.dims.get(level as usize)?.regions.owner(CellPos::of_block(x, z)).map(|r| r.0)
    }
}

/// Plugin cell data of a native world, in its dimensions' cell files.
struct NativeSidecars(Vec<(&'static str, std::sync::Arc<std::sync::Mutex<kiln_storage::NativeStore>>)>);

impl kiln_plugin_host::CellSidecars for NativeSidecars {
    fn read(&self, level: &str, rx: i32, rz: i32) -> Option<Vec<u8>> {
        let (_, store) = self.0.iter().find(|(l, _)| *l == level)?;
        store.lock().unwrap().read_sidecar(rx, rz)
    }

    fn write(&self, level: &str, rx: i32, rz: i32, data: Option<&[u8]>) {
        let Some((_, store)) = self.0.iter().find(|(l, _)| *l == level) else { return };
        if let Err(e) = store.lock().unwrap().write_sidecar(rx, rz, data) {
            warn!("cannot save plugin cell data of {level}: {e}");
        }
    }
}

impl Sim {
    /// Where plugin cell data goes in a native world (sidecar files otherwise).
    fn native_sidecars(&self) -> Option<std::sync::Arc<dyn kiln_plugin_host::CellSidecars>> {
        let stores: Vec<_> = self.dims.iter().filter_map(|d| Some((d.key, d.native.clone()?))).collect();
        (!stores.is_empty()).then(|| std::sync::Arc::new(NativeSidecars(stores)) as std::sync::Arc<dyn kiln_plugin_host::CellSidecars>)
    }

    /// Loads the plugins of `SimConfig::plugins` and registers their commands.
    pub(crate) fn load_plugins(&mut self) {
        let Some(settings) = self.config.plugins.clone() else { return };
        let data_dir = self.config.world.as_ref().map(|w| w.join("kiln").join("plugins"));
        let cache_dir = data_dir.as_ref().map_or_else(|| settings.dir.join(".cache"), |d| d.join("cache"));
        let cfg = RuntimeConfig {
            data_dir,
            cache_dir: Some(cache_dir),
            registries: registries(),
            spawn: self.spawn,
            mode: settings.mode,
            seed: settings.seed,
            call_budget: settings.call_budget,
            call_fuel: settings.call_fuel,
            cell_sidecars: self.native_sidecars(),
            ..RuntimeConfig::default()
        };
        let rt = match PluginRuntime::load_dir(&settings.dir, cfg) {
            Ok(rt) => rt,
            Err(e) => {
                warn!("plugins: {e:#}");
                return;
            }
        };
        info!("{} plugins loaded from {} ({:?} mode)", rt.ids().len(), settings.dir.display(), settings.mode);
        let dispatcher = Arc::get_mut(&mut self.commands.dispatcher).expect("dispatcher not shared yet");
        dispatcher.register(
            literal::<Sim>("kiln").requires(kiln_command::vanilla::LEVEL_GAMEMASTERS).then(
                literal::<Sim>("plugins").executes(|_, sim: &mut Sim| Ok(sim.kiln_plugins())).then(literal::<Sim>("reload").then(
                    argument::<Sim>("plugin", ArgumentType::word()).executes(|ctx, sim: &mut Sim| {
                        let id = ctx.arg_text("plugin").unwrap_or("").to_owned();
                        Ok(sim.kiln_plugins_reload(&id))
                    }),
                )),
            ),
        );
        self.plugins = Some(SimPlugins {
            rt,
            ops: Arc::new(HashSet::new()),
            registered: HashSet::new(),
            hud: HashMap::new(),
            online: Vec::new(),
            spawned: Vec::new(),
        });
        self.register_plugin_commands();
        self.sync_plugin_regions();
    }

    /// Registers plugin commands the dispatcher does not have yet (at load, and after a
    /// reload added some; commands are looked up by name when they run). Needs the only
    /// reference to the dispatcher: false if it is shared right now.
    fn register_plugin_commands(&mut self) -> bool {
        let Some(pl) = self.plugins.as_mut() else { return true };
        let new: Vec<(String, u8, usize)> =
            pl.rt.commands().iter().filter(|c| !pl.registered.contains(&c.name)).map(|c| (c.name.clone(), c.permission, c.plugin)).collect();
        if new.is_empty() {
            return true;
        }
        let Some(dispatcher) = Arc::get_mut(&mut self.commands.dispatcher) else { return false };
        for (name, permission, plugin) in new {
            pl.registered.insert(name.clone());
            if dispatcher.find(&[name.as_str()]).is_some() {
                warn!("plugin {}: command /{name} already exists", pl.rt.ids()[plugin]);
                continue;
            }
            let name2 = name.clone();
            let name3 = name.clone();
            dispatcher.register(
                literal::<Sim>(&name)
                    .requires(permission)
                    .executes(move |_, sim| Ok(sim.run_plugin_command(&name2, "")))
                    .then(
                        argument::<Sim>("args", ArgumentType::greedy_string())
                            .executes(move |ctx, sim: &mut Sim| Ok(sim.run_plugin_command(&name3, ctx.arg_text("args").unwrap_or("")))),
                    ),
            );
        }
        true
    }

    fn plugin_reply(&mut self, spans: &[Span]) {
        let pkt = packets::system_chat(text_tag(spans), false);
        match self.commands.source {
            crate::commands::CommandSource::Player(conn) => {
                if let Some(p) = self.players.get_mut(&conn) {
                    p.send(pkt);
                }
            }
            crate::commands::CommandSource::Console | crate::commands::CommandSource::Block { .. } => info!("{}", plain(spans)),
        }
    }

    fn run_plugin_command(&mut self, name: &str, args: &str) -> i32 {
        let source = self.commands.source;
        let Some(pl) = self.plugins.as_mut() else { return 0 };
        let Some(plugin) = pl.rt.command_plugin(name).map(|c| c.plugin) else {
            self.plugin_reply(&[Span::colored(format!("/{name} is no longer provided by a plugin."), "red")]);
            return 0;
        };
        let player = match source {
            crate::commands::CommandSource::Player(conn) => self.players.get(&conn),
            crate::commands::CommandSource::Console | crate::commands::CommandSource::Block { .. } => None,
        };
        let a = player.map(|p| actor(p, &pl.ops));
        let reply = pl.rt.run_command(plugin, a.as_ref(), name, args);
        if !reply.is_empty() {
            self.plugin_reply(&reply);
        }
        self.deliver_plugin_messages();
        1
    }

    /// `/kiln plugins`.
    fn kiln_plugins(&mut self) -> i32 {
        let Some(pl) = self.plugins.as_ref() else {
            self.plugin_reply(&[Span::colored("No plugins are loaded.", "gray")]);
            return 0;
        };
        let lines = pl.rt.describe();
        let stats: Vec<String> = pl.rt.stat_values().iter().map(|(k, v)| format!("{k} {v}")).collect();
        for l in &lines {
            self.plugin_reply(&[Span::colored(l.clone(), "gray")]);
        }
        self.plugin_reply(&[Span::colored(stats.join(", "), "dark_gray")]);
        lines.len() as i32
    }

    /// `/kiln plugins reload <id>`: compiles the plugin again from its directory off the
    /// tick; the swap and its report follow in a later B0.
    fn kiln_plugins_reload(&mut self, id: &str) -> i32 {
        let requester = match self.commands.source {
            crate::commands::CommandSource::Player(conn) => self.players.get(&conn).map(|p| p.uuid.as_u128()),
            crate::commands::CommandSource::Console | crate::commands::CommandSource::Block { .. } => None,
        };
        let Some(pl) = self.plugins.as_mut() else { return 0 };
        match pl.rt.request_reload(id, requester) {
            Ok(()) => {
                self.plugin_reply(&[Span::colored(format!("Reloading plugin {id}..."), "gray")]);
                1
            }
            Err(e) => {
                self.plugin_reply(&[Span::colored(format!("Cannot reload {id}: {e:#}"), "red")]);
                0
            }
        }
    }

    /// Keeps each level's region instance sets in step with its regions.
    pub(crate) fn sync_plugin_regions(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for (dim, d) in self.dims.iter().enumerate() {
            pl.rt.sync_regions(dim as u32, d.regions.iter().map(|r| r.id().0));
        }
    }

    /// B0: who is an operator; staged reloads, the global operations of the last tick, their
    /// results, due tasks; commands a reload added.
    pub(crate) fn plugins_b0(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        let ops: HashSet<Uuid> = self.players.values().filter(|p| self.commands.is_op(&p.name)).map(|p| p.uuid).collect();
        if ops != *pl.ops {
            pl.ops = Arc::new(ops);
        }
        // Who is online and where, for `event.online` (when it changed).
        let mut online: Vec<(Uuid, usize)> = self.players.values().filter(|p| !p.disconnected).map(|p| (p.uuid, p.dim)).collect();
        online.sort_unstable();
        if online != pl.online {
            let mut list: Vec<OnlinePlayer> = self
                .players
                .values()
                .filter(|p| !p.disconnected)
                .map(|p| OnlinePlayer { uuid: p.uuid.as_u128(), name: p.name.clone(), level: p.dim as u32 })
                .collect();
            list.sort_by_key(|p| p.uuid);
            pl.rt.set_online(list);
            pl.online = online;
        }
        let world = SimWorld { players: &self.players, dims: &self.dims, ops: &pl.ops };
        pl.rt.begin_tick_in(&world);
        // Players that appeared are told to the region they ended up in.
        let spawned = std::mem::take(&mut pl.spawned);
        for (conn, reason) in spawned {
            let Some(p) = self.players.get(&conn) else { continue };
            if p.disconnected {
                continue;
            }
            match pl.rt.region_mut(p.dim as u32, p.region.0) {
                Some(rp) => {
                    if rp.observing_kind(kiln_plugin_host::ObserveKinds::PLAYER_SPAWNED) {
                        rp.observe_spawn(&actor(p, &pl.ops), p.pos.map(|c| c.floor() as i32), reason);
                        rp.flush_observed();
                    }
                }
                // Not in a region yet: try again next tick.
                None => pl.spawned.push((conn, reason)),
            }
        }
        let reloaded = pl.rt.take_reloaded();
        if reloaded.iter().any(|r| r.commands_changed) && self.register_plugin_commands() {
            let conns: Vec<ConnId> = self.players.keys().copied().collect();
            for conn in conns {
                self.send_command_tree(conn);
            }
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
        let a = actor(p, &pl.ops);
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
        pl.spawned.push((conn, SpawnReason::Join));
    }

    pub(crate) fn plugins_left(&mut self, p: &Player) {
        let Some(pl) = self.plugins.as_mut() else { return };
        pl.rt.player_left(&actor(p, &pl.ops));
        pl.hud.remove(&p.uuid);
    }

    /// Tells the plugins of the region a player is in (once it has one) that the player
    /// appeared: after joining, dying, or changing level.
    pub(crate) fn plugin_spawned(&mut self, conn: ConnId, reason: SpawnReason) {
        let Some(pl) = self.plugins.as_mut() else { return };
        pl.spawned.push((conn, reason));
    }

    /// Tells the plugins that players died (the regions they died in hear of it at once).
    pub(crate) fn plugin_deaths(&mut self, deaths: &[crate::health::Death]) {
        let Some(pl) = self.plugins.as_mut() else { return };
        for d in deaths {
            let Some(p) = self.players.get(&d.conn) else { continue };
            let Some(rp) = pl.rt.region_mut(p.dim as u32, p.region.0) else { continue };
            if !rp.observing_kind(kiln_plugin_host::ObserveKinds::PLAYER_DIED) {
                continue;
            }
            let killer = d.killer.as_ref().and_then(|name| self.players.values().find(|q| q.name == *name)).map(|q| q.uuid.as_u128());
            let a = actor(p, &pl.ops);
            rp.observe_death(&a, p.pos.map(|c| c.floor() as i32), d.cause.max(0) as u32, killer);
            rp.flush_observed();
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
        (pl.rt.stat("calls"), pl.rt.stat("traps"), pl.rt.stat("timeouts"))
    }

    /// Every plugin statistic by name (tests and tools).
    pub fn plugin_stat(&self, name: &str) -> u64 {
        self.plugins.as_ref().map_or(0, |pl| pl.rt.stat(name))
    }

    /// A player's value of a plugin's key (tests and tools).
    pub fn plugin_player_value(&self, uuid: Uuid, plugin: &str, key: &str) -> Option<Vec<u8>> {
        self.plugins.as_ref()?.rt.player_value(uuid.as_u128(), plugin, key)
    }

    /// A plugin's current generation (tests and tools).
    pub fn plugin_generation(&self, plugin: &str) -> Option<u32> {
        let rt = &self.plugins.as_ref()?.rt;
        Some(rt.generation(rt.plugin_index(plugin)?))
    }

    pub(crate) fn hash_plugins(&self, h: &mut impl std::hash::Hasher) {
        if let Some(pl) = &self.plugins {
            pl.rt.hash_state(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Client, join};
    use crate::{OVERWORLD_ID, SimConfig};
    use std::hint::black_box;
    use std::time::Instant;

    /// What `world-read` costs an event: the copy of the 9x9x9 blocks around it (`provide_blocks`, 729 `get_block` calls and
    /// a `Vec`), measured over the chunks of a running level. Prints the numbers (`--nocapture`); the bound only catches a
    /// copy that went wrong by an order of magnitude.
    #[test]
    fn block_window_copy_cost() {
        let mut sim = Sim::new(SimConfig::new(2, 2, None));
        let (msg, stats) = join(1, "Cost", 2);
        assert!(sim.step([msg]));
        let mut client = Client::new(1, stats);
        for _ in 0..10 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let centre = sim.players.get(&1).unwrap().pos.map(|c| c.floor() as i32);
        let region = sim.dims[OVERWORLD_ID].regions.iter().next().expect("a region");
        let cells = region.cells();
        let window = |c: [i32; 3]| BlockWindow::new(c, |x, y, z| cells.get_block(x, y, z).map_or(BlockWindow::UNLOADED, block_id));
        let loaded = (0..729).filter(|i| window(centre).get(centre[0] - 4 + i % 9, centre[1] - 4 + i / 81, centre[2] - 4 + i / 9 % 9).is_some()).count();
        assert!(loaded > 100, "the window is over loaded chunks ({loaded} blocks)");
        const N: u32 = 20_000;
        let start = Instant::now();
        for i in 0..N {
            // (The centre moves a little, as events do.)
            black_box(window(black_box([centre[0] + (i % 5) as i32, centre[1], centre[2] + (i % 3) as i32])));
        }
        let per_window = start.elapsed().as_nanos() as f64 / N as f64;
        println!("world-read window: {per_window:.0} ns per event ({:.2} ns per block, {loaded} of 729 loaded)", per_window / 729.0);
        assert!(per_window < 200_000.0, "{per_window} ns");
    }
}
