//! What plugins asked the game to do, applied at the serial points (after P, after G, after a
//! plugin command): messages, titles, a private sidebar and boss bars, teleports, game
//! modes, healing, kicks, items, locked menus, owned entities and blocks of a cell.
//!
//! The effects come out of the host in a deterministic order (tick, acting player, call
//! order); each is applied through the same code the vanilla commands use (`kiln_command::Host`)
//! or the same packets, and its outcome goes back to the plugin ([`PluginRuntime::effect_done`]).

use super::{Hud, entity_data, plain, text_tag};
use crate::container::open::OpenBlock;
use crate::{Player, Sim};
use kiln_command::{BlockInput, GameMode, Host, Identifier, StringReader, Teleport, Text, UpdateFlags};
use kiln_world::Blocks;
use kiln_link::ConnId;
use kiln_plugin_host::{BlockChange, Effect, EffectKind, ItemSpec, MenuSpec, Span, SpawnSpec};
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_proto::packets::hud as hud_packets;
use kiln_proto::packets::scoreboard as sb;
use std::collections::HashMap;
use uuid::Uuid;

/// The objective a plugin sidebar shows under (the client keeps it apart from the server's).
const SIDEBAR: &str = "kiln_sb";

/// A string as SNBT text.
fn snbt_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// A text component as SNBT. Plugin text is not italic unless it says so (item names and
/// lore are italic by default).
fn snbt_text(spans: &[Span]) -> String {
    let part = |s: &Span| {
        let mut out = format!("{{text:{}", snbt_str(&s.text));
        if let Some(c) = &s.color {
            out.push_str(&format!(",color:{}", snbt_str(c)));
        }
        if s.bold {
            out.push_str(",bold:1b");
        }
        out.push_str(if s.italic { ",italic:1b" } else { ",italic:0b" });
        out.push('}');
        out
    };
    match spans {
        [one] => part(one),
        _ => format!("{{text:\"\",italic:0b,extra:[{}]}}", spans.iter().map(part).collect::<Vec<_>>().join(",")),
    }
}

/// The component changes of an item spec, as `(component, SNBT)`.
fn components(item: &ItemSpec) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(name) = &item.name {
        out.push(("minecraft:custom_name", snbt_text(name)));
    }
    if !item.lore.is_empty() {
        out.push(("minecraft:lore", format!("[{}]", item.lore.iter().map(|l| snbt_text(l)).collect::<Vec<_>>().join(","))));
    }
    if let Some(tag) = &item.tag {
        out.push(("minecraft:custom_data", format!("{{\"kiln:tag\":{}}}", snbt_str(tag))));
    }
    if let Some(model) = &item.model {
        out.push(("minecraft:item_model", snbt_str(model)));
    }
    if item.glint {
        out.push(("minecraft:enchantment_glint_override", "1b".to_owned()));
    }
    out
}

/// The stack an item spec describes; none if the item does not exist.
fn build_stack(item: &ItemSpec) -> Option<kiln_item::ItemStack> {
    let id = kiln_data::builtin_id("minecraft:item", Identifier::parse(&item.item)?.as_str())?;
    let mut m = kiln_item::value::MapBuilder::new();
    for (component, snbt) in components(item) {
        if let Ok(v) = kiln_item::component::predicate::parse_snbt(&snbt) {
            m.put(component, v);
        }
    }
    let patch = kiln_item::DataComponentPatch::from_value(&m.build()).unwrap_or_default();
    Some(kiln_item::ItemStack::from_parts(id, item.count as i32, patch))
}

/// A boss bar's uuid: made from its name (`<plugin id>:<id>`), the same on every run.
fn bar_uuid(id: &str) -> Uuid {
    let (mut a, mut b) = (0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4u64);
    for byte in id.bytes() {
        a = (a ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b.rotate_left(5) ^ byte as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    Uuid::from_u128(((a as u128) << 64) | b as u128)
}

fn game_mode(m: u8) -> GameMode {
    match m {
        0 => GameMode::Survival,
        2 => GameMode::Adventure,
        3 => GameMode::Spectator,
        _ => GameMode::Creative,
    }
}

fn boss_color(c: u8) -> hud_packets::BossBarColor {
    use hud_packets::BossBarColor as C;
    [C::Pink, C::Blue, C::Red, C::Green, C::Yellow, C::Purple, C::White][c.min(6) as usize]
}

fn boss_overlay(s: u8) -> hud_packets::BossBarOverlay {
    use hud_packets::BossBarOverlay as O;
    [O::Progress, O::Notched6, O::Notched10, O::Notched12, O::Notched20][s.min(4) as usize]
}

impl Sim {
    /// Applies what plugins asked for since the last serial point.
    pub(crate) fn deliver_plugin_messages(&mut self) {
        let Some(pl) = self.plugins.as_mut() else { return };
        let effects = pl.rt.take_effects();
        if effects.is_empty() {
            return;
        }
        let conns: HashMap<u128, ConnId> = self.players.iter().filter(|(_, p)| !p.disconnected).map(|(&c, p)| (p.uuid.as_u128(), c)).collect();
        let mut states: HashMap<String, Option<BlockInput>> = HashMap::new();
        for e in effects {
            let applied = self.apply_plugin_effect(&e, &conns, &mut states);
            if let Some(pl) = self.plugins.as_mut() {
                pl.rt.effect_done(&e, applied);
            }
        }
    }

    fn apply_plugin_effect(&mut self, e: &Effect, conns: &HashMap<u128, ConnId>, states: &mut HashMap<String, Option<BlockInput>>) -> bool {
        let conn = |who: &u128| conns.get(who).copied();
        match &e.kind {
            EffectKind::Message { to, text } => {
                let pkt = packets::system_chat(text_tag(text), false);
                match to {
                    None => self.broadcast(pkt),
                    Some(u) => match conn(u).and_then(|c| self.players.get_mut(&c)) {
                        Some(p) => p.send(pkt),
                        None => return false,
                    },
                }
                true
            }
            EffectKind::Title { to, title, subtitle, fade_in, stay, fade_out } => {
                let Some(p) = conn(to).and_then(|c| self.players.get_mut(&c)) else { return false };
                p.send(hud_packets::set_titles_animation(*fade_in as i32, *stay as i32, *fade_out as i32));
                p.send(hud_packets::set_subtitle_text(&text_tag(subtitle)));
                p.send(hud_packets::set_title_text(&text_tag(title)));
                true
            }
            EffectKind::ActionBar { to, text } => {
                let Some(p) = conn(to).and_then(|c| self.players.get_mut(&c)) else { return false };
                p.send(hud_packets::set_action_bar_text(&text_tag(text)));
                true
            }
            EffectKind::Sidebar { to, title, lines } => {
                let Some(c) = conn(to) else { return false };
                let Some(pl) = self.plugins.as_mut() else { return false };
                let Some(p) = self.players.get_mut(&c) else { return false };
                let hud = pl.hud.entry(p.uuid).or_default();
                let title_tag = text_tag(title);
                let objective =
                    sb::Objective { display_name: &title_tag, render_type: sb::RenderType::Integer, number_format: Some(sb::NumberFormat::Blank) };
                let previous = hud.sidebar;
                if previous.is_none() {
                    p.send(sb::set_objective(SIDEBAR, &sb::ObjectiveMethod::Add(objective)));
                    p.send(sb::set_display_objective(sb::display_slot::SIDEBAR, SIDEBAR));
                } else {
                    p.send(sb::set_objective(SIDEBAR, &sb::ObjectiveMethod::Change(objective)));
                }
                for (i, line) in lines.iter().enumerate() {
                    let tag = text_tag(line);
                    p.send(sb::set_score(&format!("kiln_l{i}"), SIDEBAR, (lines.len() - i) as i32, Some(&tag), Some(&sb::NumberFormat::Blank)));
                }
                for i in lines.len()..previous.unwrap_or(0) {
                    p.send(sb::reset_score(&format!("kiln_l{i}"), Some(SIDEBAR)));
                }
                hud.sidebar = Some(lines.len());
                true
            }
            EffectKind::ClearSidebar { to } => {
                let Some(c) = conn(to) else { return false };
                let Some(pl) = self.plugins.as_mut() else { return false };
                let Some(p) = self.players.get_mut(&c) else { return false };
                let shown = pl.hud.get_mut(&p.uuid).and_then(|h| h.sidebar.take()).is_some();
                if shown {
                    p.send(sb::set_display_objective(sb::display_slot::SIDEBAR, ""));
                    p.send(sb::set_objective(SIDEBAR, &sb::ObjectiveMethod::Remove));
                }
                shown
            }
            EffectKind::Bossbar { to, id, text, progress, color, style } => {
                let Some(c) = conn(to) else { return false };
                let Some(pl) = self.plugins.as_mut() else { return false };
                let Some(p) = self.players.get_mut(&c) else { return false };
                let hud: &mut Hud = pl.hud.entry(p.uuid).or_default();
                let (name, uuid) = (text_tag(text), bar_uuid(id));
                let (color, overlay) = (boss_color(*color), boss_overlay(*style));
                if hud.bars.insert(id.clone()) {
                    p.send(hud_packets::boss_event(uuid, &hud_packets::BossEvent::Add { name: &name, progress: *progress, color, overlay, flags: 0 }));
                } else {
                    p.send(hud_packets::boss_event(uuid, &hud_packets::BossEvent::Name(&name)));
                    p.send(hud_packets::boss_event(uuid, &hud_packets::BossEvent::Progress(*progress)));
                    p.send(hud_packets::boss_event(uuid, &hud_packets::BossEvent::Style { color, overlay }));
                }
                true
            }
            EffectKind::ClearBossbar { to, id } => {
                let Some(c) = conn(to) else { return false };
                let Some(pl) = self.plugins.as_mut() else { return false };
                let Some(p) = self.players.get_mut(&c) else { return false };
                let shown = pl.hud.get_mut(&p.uuid).is_some_and(|h| h.bars.remove(id));
                if shown {
                    p.send(hud_packets::boss_event(bar_uuid(id), &hud_packets::BossEvent::Remove));
                }
                shown
            }
            EffectKind::Teleport { who, level, pos, rot } => {
                let Some(c) = conn(who) else { return false };
                let Some(&(dimension, _)) = crate::DIMENSIONS.get(*level as usize) else { return false };
                let Some(p) = self.players.get(&c) else { return false };
                let target = crate::commands::PlayerRef::of(c, p, &self.commands.scoreboard);
                let to = Teleport {
                    dimension: dimension.to_owned(),
                    pos: *pos,
                    relative: [false; 3],
                    rotation: Some(*rot),
                    relative_rotation: [false; 2],
                    facing: None,
                };
                Host::teleport(self, &target, &to).is_ok()
            }
            EffectKind::GameMode { who, mode } => {
                let Some(c) = conn(who) else { return false };
                let Some(p) = self.players.get(&c) else { return false };
                let target = crate::commands::PlayerRef::of(c, p, &self.commands.scoreboard);
                Host::set_game_mode(self, &target, game_mode(*mode))
            }
            EffectKind::Heal { who } => {
                let Some(p) = conn(who).and_then(|c| self.players.get_mut(&c)) else { return false };
                if p.dead {
                    return false;
                }
                p.health = 20.0;
                p.food = 20;
                p.saturation = 5.0;
                p.clear_fire();
                p.sync_health();
                true
            }
            EffectKind::Kill { who } => {
                let Some(c) = conn(who) else { return false };
                let Some(p) = self.players.get(&c) else { return false };
                if p.dead {
                    return false;
                }
                let target = crate::commands::PlayerRef::of(c, p, &self.commands.scoreboard);
                Host::kill(self, &target);
                true
            }
            EffectKind::Kick { who, reason } => {
                let Some(c) = conn(who) else { return false };
                let Some(p) = self.players.get(&c) else { return false };
                let target = crate::commands::PlayerRef::of(c, p, &self.commands.scoreboard);
                Host::kick(self, &target, Text::literal(plain(reason)));
                true
            }
            EffectKind::Give { who, item } => {
                let Some(c) = conn(who) else { return false };
                self.give_plugin_stack(c, item)
            }
            EffectKind::Take { who, item, count } => {
                let Some(c) = conn(who) else { return false };
                self.take_plugin_items(c, item, *count as i32)
            }
            EffectKind::Clear { who } => {
                let Some(c) = conn(who) else { return false };
                let rules = self.rules.clone();
                let Some(p) = self.players.get_mut(&c) else { return false };
                for s in p.inv.items.iter_mut().chain(p.inv.equipment.iter_mut()) {
                    *s = kiln_item::ItemStack::empty();
                }
                p.inv.times_changed += 1;
                let mut spawns = Vec::new();
                p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
                true
            }
            EffectKind::OpenMenu { who, menu } => {
                let Some(c) = conn(who) else { return false };
                self.open_plugin_menu(c, menu)
            }
            EffectKind::SetSlot { who, menu, slot, item } => {
                let Some(c) = conn(who) else { return false };
                let rules = self.rules.clone();
                let Some(p) = self.players.get_mut(&c) else { return false };
                let open = matches!(&p.containers.open, Some(OpenBlock::Plugin(id)) if **id == **menu);
                if !open || p.containers.cart.items.len() <= *slot as usize {
                    return false;
                }
                let stack = match item {
                    Some(spec) => match build_stack(spec) {
                        Some(s) => s,
                        None => return false,
                    },
                    None => kiln_item::ItemStack::empty(),
                };
                p.containers.cart.items[*slot as usize] = stack;
                let mut spawns = Vec::new();
                p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
                true
            }
            EffectKind::CloseMenu { who } => {
                let Some(c) = conn(who) else { return false };
                self.close_plugin_menu(c)
            }
            EffectKind::Spawn(spec) => self.spawn_plugin_entity(&e.plugin_id, spec),
            EffectKind::Remove { level, uuid } => self.remove_plugin_entity(&e.plugin_id, *level as usize, *uuid),
            EffectKind::SetBlocks { level, changes } => self.set_plugin_blocks(*level as usize, changes, states),
        }
    }

    fn give_plugin_stack(&mut self, conn: ConnId, item: &ItemSpec) -> bool {
        let Some(mut stack) = build_stack(item) else { return false };
        let rules = self.rules.clone();
        let Some(p) = self.players.get_mut(&conn) else { return false };
        let dim = p.dim;
        p.add_to_inventory(&mut stack);
        if !stack.is_empty() {
            let spawn = p.throw(stack);
            self.dims[dim].spawns.push(spawn);
        }
        let Some(p) = self.players.get_mut(&conn) else { return false };
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
        self.dims[dim].spawns.extend(spawns);
        true
    }

    fn take_plugin_items(&mut self, conn: ConnId, key: &str, count: i32) -> bool {
        let Some(id) = Identifier::parse(key).and_then(|i| kiln_data::builtin_id("minecraft:item", i.as_str())) else { return false };
        let rules = self.rules.clone();
        let Some(p) = self.players.get_mut(&conn) else { return false };
        let has: i32 = p.inv.items.iter().filter(|s| !s.is_empty() && s.item() == id).map(|s| s.count()).sum();
        if has < count {
            return false;
        }
        let mut left = count;
        for s in p.inv.items.iter_mut().filter(|s| !s.is_empty() && s.item() == id) {
            let n = left.min(s.count());
            s.shrink(n);
            if s.count() <= 0 {
                *s = kiln_item::ItemStack::empty();
            }
            left -= n;
            if left == 0 {
                break;
            }
        }
        p.inv.times_changed += 1;
        let dim = p.dim;
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
        self.dims[dim].spawns.extend(spawns);
        true
    }

    /// Opens a plugin's locked menu for a player (what they had open closes first).
    fn open_plugin_menu(&mut self, conn: ConnId, spec: &MenuSpec) -> bool {
        let Some(mut p) = self.players.remove(&conn) else { return false };
        let rules = self.rules.clone();
        let mut spawns = Vec::new();
        if p.open_menu.is_some() {
            let (dim, pos) = (p.dim, p.pos.map(|c| c.floor() as i32));
            let r = self.with_level_in(dim, pos, |level| p.close_block_menu(&rules, &mut spawns, level, true));
            if r.is_none() {
                // Its block's region is gone: forget the menu.
                p.open_menu = None;
                p.containers.open = None;
            }
        }
        let rows = spec.rows.clamp(1, 6);
        p.containers.counter = p.containers.counter % 100 + 1;
        let id = p.containers.counter;
        let menu = kiln_inventory::Menu::generic(id, rows);
        let mut items = vec![kiln_item::ItemStack::empty(); rows as usize * 9];
        for (slot, item) in &spec.items {
            if let (Some(s), Some(stack)) = (items.get_mut(*slot as usize), build_stack(item)) {
                *s = stack;
            }
        }
        p.containers.cart = kiln_inventory::SimpleContainer::from_items(items);
        if let Some(ty) = menu.kind.menu_type_id() {
            p.send(kiln_inventory::effect::open_screen(id, ty, &text_tag(&spec.title)));
        }
        p.containers.open = Some(OpenBlock::Plugin(spec.id.as_str().into()));
        p.open_menu = Some(menu);
        p.with_menu_at(&rules, &mut spawns, None, |menu, _, env| menu.open(env));
        let dim = p.dim;
        self.players.insert(conn, p);
        self.dims[dim].spawns.extend(spawns);
        true
    }

    /// Closes the menu a player has open, if a plugin opened it.
    fn close_plugin_menu(&mut self, conn: ConnId) -> bool {
        let Some(mut p) = self.players.remove(&conn) else { return false };
        let rules = self.rules.clone();
        let mut spawns = Vec::new();
        let ours = matches!(p.containers.open, Some(OpenBlock::Plugin(_)));
        if ours {
            let (dim, pos) = (p.dim, p.pos.map(|c| c.floor() as i32));
            if self.with_level_in(dim, pos, |level| p.close_block_menu(&rules, &mut spawns, level, true)).is_none() {
                p.open_menu = None;
                p.containers.open = None;
            }
        }
        let dim = p.dim;
        self.players.insert(conn, p);
        self.dims[dim].spawns.extend(spawns);
        ours
    }

    /// Spawns an entity for a plugin: the uuid it was promised, the owner mark that lets only
    /// that plugin remove it.
    fn spawn_plugin_entity(&mut self, plugin: &str, s: &SpawnSpec) -> bool {
        let dim = s.level as usize;
        if dim >= self.dims.len() {
            return false;
        }
        let u = s.uuid;
        let flag = |on: bool, name: &str| on.then(|| (name.to_owned(), Tag::Byte(1)));
        let mut fields: Vec<(String, Tag)> = vec![
            ("UUID".into(), Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32])),
            ("Rotation".into(), Tag::List(vec![Tag::Float(s.yaw), Tag::Float(0.0)])),
            (
                "kiln:plugin".into(),
                Tag::Compound(vec![(plugin.to_owned(), Tag::Compound(vec![("kiln:owner".into(), Tag::ByteArray(vec![1]))]))]),
            ),
        ];
        fields.extend(flag(s.no_ai, "NoAI"));
        fields.extend(flag(s.invulnerable, "Invulnerable"));
        fields.extend(flag(s.silent, "Silent"));
        fields.extend(flag(s.no_gravity, "NoGravity"));
        if let Some(name) = &s.name {
            fields.push(("CustomName".into(), text_tag(name)));
            fields.push(("CustomNameVisible".into(), Tag::Byte(1)));
        }
        let nbt = Tag::Compound(fields);
        let Some(kind) = Identifier::parse(&s.kind) else { return false };
        let seed = crate::mobs::loot_seed(self.config.noise.as_ref().map_or(0, |n| n.seed), self.game_time, self.dims[dim].spawns.len() as i32, 0x706c_7567);
        let inhabited = self.dims[dim]
            .regions
            .chunk(kiln_world::ChunkPos::of_block(s.pos[0].floor() as i32, s.pos[2].floor() as i32))
            .map_or(0, |c| c.inhabited_time());
        crate::mobs::summon(&mut self.dims[dim].spawns, kind.as_str(), s.pos, Some(&nbt), true, self.commands.difficulty as u8, self.game_time, inhabited, seed)
            .is_some()
    }

    /// Removes an entity the plugin spawned (the owner mark says so).
    fn remove_plugin_entity(&mut self, plugin: &str, dim: usize, uuid: u128) -> bool {
        let Some(d) = self.dims.get_mut(dim) else { return false };
        let target = Uuid::from_u128(uuid);
        for r in d.regions.iter_mut() {
            if let Some(e) = r.part_mut().0.list.iter_mut().find(|e| e.uuid == target && !e.removed) {
                let Some(phys) = e.phys.as_deref_mut() else { return false };
                let owned = entity_data(&phys.extra).get(plugin).is_some_and(|kv| kv.contains_key("kiln:owner"));
                if !owned {
                    return false;
                }
                phys.removed = Some(kiln_entity::entity::RemovalReason::Discarded);
                return true;
            }
        }
        false
    }

    /// Sets the blocks of a cell-bound edit with the flags of a player placing blocks.
    fn set_plugin_blocks(&mut self, dim: usize, changes: &[BlockChange], states: &mut HashMap<String, Option<BlockInput>>) -> bool {
        let Some(&(key, _)) = crate::DIMENSIONS.get(dim) else { return false };
        let mut all = true;
        for c in changes {
            let input = states
                .entry(c.state.clone())
                .or_insert_with(|| kiln_command::blocks::parse_block_state(&mut StringReader::new(&c.state)).ok())
                .clone();
            match input {
                Some(input) => {
                    self.place_block(key, c.pos, &input, UpdateFlags::ALL);
                }
                None => all = false,
            }
        }
        all
    }
}

impl Player {
    /// Whether the player has a plugin menu open.
    pub(crate) fn has_plugin_menu(&self) -> bool {
        matches!(self.containers.open, Some(OpenBlock::Plugin(_)))
    }
}
