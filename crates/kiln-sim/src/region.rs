//! Region-local work: what one region does in the parallel phases (design §4.3).
//!
//! A [`RegionWork`] holds a region's cells and its players (disjoint `&mut` borrows, so
//! regions can run on different threads) plus the region's packets for this tick. Nothing
//! here can reach another region's cells or players; effects outside the region go through
//! the outputs the serial phases pick up (chunk requests and unloads).

use crate::blocks::{self, BlockOut, EntityBox, RegionBlocks, RegionLevel, Ticking};
use crate::entities::{self, Entities, Spawn};
use crate::{KEEP_ALIVE_INTERVAL, KEEP_ALIVE_TIMEOUT, MAX_UNACKED_BATCHES, Player, digging, movement};
use kiln_blocks::interact::{self, Actor};
use kiln_blocks::placement::{self, BlockItem, PlaceContext};
use kiln_blocks::{BlockPos, Level};
use kiln_link::{ConnId, PlayIn};
use kiln_proto::packets;
use kiln_region::CellSet;
use kiln_world::{Blocks, Cell, CellStore, ChunkPos};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Read-only values of the global state that region work needs.
#[derive(Clone)]
pub(crate) struct Env {
    pub rules: std::sync::Arc<kiln_inventory::Rules>,
    /// Bottom of the dimension (void damage starts 64 blocks below).
    pub min_y: i32,
    pub game_time: i64,
    /// The server's view distance: clients may ask for less.
    pub max_view: i32,
    /// `minecraft:player_movement_check`.
    pub movement_check: bool,
    /// `minecraft:natural_health_regeneration`.
    pub natural_regen: bool,
    pub biome_count: usize,
    pub now: Instant,
    /// Id for keep-alives sent this tick.
    pub keep_alive_id: i64,
    pub blocks: blocks::BlockEnv,
}

/// What a region leaves for the next serial phase.
#[derive(Default)]
pub(crate) struct RegionOut {
    /// Chunks its players need that are not loaded: (rank in the player's nearest-first
    /// list, player, chunk).
    pub wanted: Vec<(u32, ConnId, ChunkPos)>,
    /// Loaded chunks no player of the region is near any more.
    pub unload: Vec<ChunkPos>,
    /// Entities spawned this phase; ids are assigned afterwards in canonical order.
    pub spawns: Vec<Spawn>,
    /// Players that died this phase (the messages go to everyone afterwards).
    pub deaths: Vec<crate::health::Death>,
    /// CPU time per sub-phase, for the statistics.
    pub times: [Duration; SUB_PHASES.len()],
}

pub(crate) const SUB_PHASES: [&str; 9] =
    ["menus", "connections", "chunks", "blocks", "entities", "visibility", "movement", "light", "egress"];

pub(crate) struct RegionWork<'a> {
    pub cells: &'a mut CellSet<Cell>,
    pub entities: &'a mut Entities,
    pub blocks: &'a mut RegionBlocks,
    /// Sorted by connection id.
    pub players: Vec<&'a mut Player>,
    /// This region's packets for the tick, in arrival order.
    pub packets: Vec<(ConnId, PlayIn)>,
    pub out: RegionOut,
}

impl RegionWork<'_> {
    fn index_of(&self, conn: ConnId) -> Option<usize> {
        self.players.binary_search_by_key(&conn, |p| p.conn).ok()
    }

    /// P1: applies the region's packets in arrival order.
    pub fn apply_packets(&mut self, env: &Env) {
        let mut out = BlockOut::default();
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        for (conn, pkt) in std::mem::take(&mut self.packets) {
            let Some(i) = self.index_of(conn) else { continue };
            if let PlayIn::Attack { entity_id } = pkt {
                if !self.players[i].dead {
                    let attack_env = crate::combat::AttackEnv { cells: &*self.cells, game_time: env.game_time, seed: env.blocks.seed };
                    let mut ctx = damage_ctx(env, &mut self.out.spawns, &mut self.out.deaths);
                    crate::combat::handle_attack(&mut self.players, i, entity_id, self.entities, &attack_env, &mut ctx);
                }
                continue;
            }
            let mut world = World { cells: &mut *self.cells, blocks: &mut *self.blocks };
            let mut fx = Fx { blocks: &mut out, bodies: &bodies, spawns: &mut self.out.spawns, deaths: &mut self.out.deaths };
            local_packet(self.players[i], &mut world, env, pkt, &mut fx);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
    }

    /// L: connection upkeep, chunk streaming, tracking, light, then egress.
    pub fn tick(&mut self, env: &Env) {
        let mut lap = Instant::now();
        let mut mark = |times: &mut [Duration; SUB_PHASES.len()], i: usize| {
            let now = Instant::now();
            times[i] += now - lap;
            lap = now;
        };
        // Menu changes first, like vanilla's container broadcast at the start of a player tick.
        for p in self.players.iter_mut() {
            p.with_menu(&env.rules, &mut self.out.spawns, |menu, _, env| menu.broadcast_changes(env));
        }
        mark(&mut self.out.times, 0);
        for p in self.players.iter_mut() {
            tick_connection(p, env);
            p.tick_damage(env.game_time);
            p.tick_combat();
            let mut ctx = damage_ctx(env, &mut self.out.spawns, &mut self.out.deaths);
            p.check_void(env.min_y, &mut ctx);
            p.tick_using(&mut self.out.spawns);
            let mut ctx = damage_ctx(env, &mut self.out.spawns, &mut self.out.deaths);
            p.tick_food(env.natural_regen, &mut ctx);
            p.sync_health();
        }
        mark(&mut self.out.times, 1);
        for p in self.players.iter_mut().filter(|p| !p.disconnected) {
            update_chunks(p, &mut *self.cells, env, &mut self.out.wanted);
        }
        // The same tick everywhere, so when chunks unload does not depend on the regions.
        if env.game_time % 20 == 0 {
            self.find_unloads();
        }
        mark(&mut self.out.times, 2);
        self.tick_blocks(env);
        mark(&mut self.out.times, 3);
        self.tick_entities(env);
        entities::pickups(self.entities, &mut self.players);
        mark(&mut self.out.times, 4);
        let movers = crate::players::update_visibility(&mut self.players);
        mark(&mut self.out.times, 5);
        crate::players::broadcast_movement(&mut self.players);
        for p in self.players.iter_mut() {
            p.decay_velocity();
        }
        entities::track(self.entities, &mut self.players, &movers);
        mark(&mut self.out.times, 6);
        self.send_light_updates();
        mark(&mut self.out.times, 7);
        for p in self.players.iter_mut() {
            p.flush();
        }
        mark(&mut self.out.times, 8);
    }

    /// The block phases: players' digging, pressure plates under bodies, then scheduled
    /// ticks, random ticks, block events and moving pistons in chunks near players.
    fn tick_blocks(&mut self, env: &Env) {
        let ticking = Ticking::around(self.players.iter().map(|p| p.center), env.blocks.simulation_distance);
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let mut out = BlockOut::default();
        {
            let mut level = RegionLevel {
                cells: &mut *self.cells,
                blocks: &mut *self.blocks,
                env: &env.blocks,
                out: &mut out,
                bodies: &bodies,
                actor: None,
            };
            for p in self.players.iter_mut().filter(|p| p.digging.is_some() || p.delayed_destroy.is_some()) {
                digging::tick(p, &mut level);
            }
            blocks::press_plates(&mut level);
            blocks::tick_blocks(&mut level, &ticking);
            blocks::tick_pistons(&mut level, &ticking);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
    }

    /// The entity phase: the region's entities tick against its blocks; what they change
    /// goes out like block work.
    fn tick_entities(&mut self, env: &Env) {
        if self.entities.list.is_empty() {
            return;
        }
        let ticking = Ticking::around(self.players.iter().map(|p| p.center), env.blocks.simulation_distance);
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let mut out = BlockOut::default();
        {
            let mut level = RegionLevel {
                cells: &mut *self.cells,
                blocks: &mut *self.blocks,
                env: &env.blocks,
                out: &mut out,
                bodies: &bodies,
                actor: None,
            };
            entities::tick(self.entities, &mut level, &ticking, &mut self.players, &mut self.out.spawns, &mut self.out.deaths);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
    }

    /// Chunks outside every player's view (plus one chunk of margin) can go.
    fn find_unloads(&mut self) {
        let mut centers: Vec<(ChunkPos, i32)> = self.players.iter().map(|p| (p.center, p.view_distance + 1)).collect();
        centers.sort_unstable_by_key(|&(c, r)| (c, r));
        centers.dedup();
        let mut near = HashSet::new();
        for (o, r) in centers {
            for x in o.x - r..=o.x + r {
                for z in o.z - r..=o.z + r {
                    near.insert(ChunkPos::new(x, z));
                }
            }
        }
        let mut unload = Vec::new();
        self.cells.for_each_cell(&mut |pos, cell| {
            unload.extend(cell.chunks(pos).map(|(c, _)| c).filter(|c| !near.contains(c)));
        });
        self.out.unload = unload;
    }

    /// Update Light for every chunk whose light changed, to players who have it.
    fn send_light_updates(&mut self) {
        for (pos, sky, block) in self.cells.take_light_changes() {
            if !self.players.iter().any(|p| p.sent_chunks.contains(&pos)) {
                continue;
            }
            let Some(body) = self.cells.light_update_body(pos, sky, block) else { continue };
            let pkt = packets::light_update(pos.x, pos.z, &body);
            for p in self.players.iter_mut().filter(|p| p.sent_chunks.contains(&pos)) {
                p.send(pkt.clone());
            }
        }
    }
}

/// Damage context for region work.
pub(crate) fn damage_ctx<'a>(
    env: &Env,
    spawns: &'a mut Vec<Spawn>,
    deaths: &'a mut Vec<crate::health::Death>,
) -> crate::health::DamageCtx<'a> {
    crate::health::DamageCtx { rules: env.blocks.damage, game_time: env.game_time, spawns, deaths }
}

/// Whether a packet needs the whole server (chat, commands): it and everything its region
/// receives after it this tick run in the serial PX phase.
pub(crate) fn is_exclusive(pkt: &PlayIn) -> bool {
    matches!(
        pkt,
        PlayIn::ChatCommand { .. }
            | PlayIn::CommandSuggestion { .. }
            | PlayIn::Chat { .. }
            // Disconnects and per-player protocol state.
            | PlayIn::ResourcePack { .. }
            | PlayIn::CookieResponse(_)
            // Respawning finds a spawn point anywhere and restarts tracking.
            | PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn)
    )
}

/// A region's cells and block machinery, which a packet may change.
pub(crate) struct World<'a> {
    pub cells: &'a mut CellSet<Cell>,
    pub blocks: &'a mut RegionBlocks,
}

/// Where a packet's side effects go.
pub(crate) struct Fx<'a> {
    pub blocks: &'a mut BlockOut,
    /// Entity boxes at the start of the phase (placement must not overlap them).
    pub bodies: &'a [EntityBox],
    pub spawns: &'a mut Vec<Spawn>,
    pub deaths: &'a mut Vec<crate::health::Death>,
}

impl World<'_> {
    /// The world as block behaviour sees it, with `actor` acting.
    fn level<'l>(&'l mut self, env: &'l Env, out: &'l mut BlockOut, bodies: &'l [EntityBox], actor: ConnId) -> RegionLevel<'l> {
        RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out, bodies, actor: Some(actor) }
    }
}

/// A packet that touches only its player and the world around it.
pub(crate) fn local_packet(p: &mut Player, world: &mut World, env: &Env, pkt: PlayIn, fx: &mut Fx) {
    if p.dead && !matches!(pkt, PlayIn::KeepAlive { .. } | PlayIn::ChunkBatchReceived { .. } | PlayIn::ClientTickEnd) {
        return;
    }
    match pkt {
        PlayIn::AcceptTeleport { id } => {
            if p.awaiting_teleport == Some(id) {
                p.awaiting_teleport = None;
            }
        }
        PlayIn::KeepAlive { id } => {
            if p.keep_alive.is_some_and(|(k, _)| k == id) {
                p.keep_alive = None;
            }
        }
        PlayIn::Move { pos, rot, on_ground } => {
            let (from, was_on_ground) = (p.pos, p.on_ground);
            let y0 = p.pos[1];
            if handle_move(p, &*world.cells, env, pos, rot, on_ground) {
                let feet = p.pos.map(|c| c.floor() as i32);
                let in_fluid = world.cells.get_block(feet[0], feet[1], feet[2]).is_some_and(kiln_data::blocks_types::has_fluid);
                let d = [p.pos[0] - from[0], p.pos[1] - from[1], p.pos[2] - from[2]];
                // `handlePlayerKnownMovement`.
                p.known_movement = d;
                p.moved_this_tick = true;
                // `Block.updateEntityMovementAfterFallOn`: landing stops the fall.
                if on_ground {
                    p.vel[1] = 0.0;
                }
                p.exhaust_for_move(d, was_on_ground, in_fluid);
                let mut ctx = damage_ctx(env, fx.spawns, fx.deaths);
                p.check_fall(p.pos[1] - y0, on_ground, in_fluid, &mut ctx);
            }
        }
        PlayIn::PlayerAbilities { flying } => p.flying = flying && matches!(p.game_mode, 1 | 3),
        PlayIn::ChunkBatchReceived { chunks_per_tick } => {
            p.unacked_batches = p.unacked_batches.saturating_sub(1);
            if chunks_per_tick.is_finite() {
                p.chunks_per_tick = chunks_per_tick.clamp(0.01, 64.0);
            }
        }
        PlayIn::ClientInformation(info) => {
            p.view_distance = (info.view_distance as i32).min(env.max_view);
            p.section = None;
            p.client = info;
        }
        PlayIn::PlayerInput { flags } => {
            let sneaking = flags & 0x20 != 0;
            if sneaking != p.sneaking {
                p.sneaking = sneaking;
                p.meta_dirty = true;
            }
        }
        PlayIn::PlayerCommand { action } => {
            const START_SPRINTING: i32 = 1;
            const STOP_SPRINTING: i32 = 2;
            let sprinting = match action {
                START_SPRINTING => true,
                STOP_SPRINTING => false,
                _ => p.sprinting,
            };
            if sprinting != p.sprinting {
                p.sprinting = sprinting;
                p.meta_dirty = true;
            }
        }
        // Sent by the client when its "Loading terrain" screen closes.
        PlayIn::PlayerLoaded => {
            p.load_timeout = 0;
            info!("{} finished loading terrain", p.name);
        }
        PlayIn::SetCarriedItem { slot } => {
            if (0..9).contains(&slot) {
                p.inv.selected = slot as usize;
            }
        }
        PlayIn::SetCreativeSlot { slot, item } => {
            let Ok(stack) = kiln_inventory::click::creative_stack(item.as_ref()) else {
                p.disconnect("Invalid item");
                return;
            };
            // Creative slots always address the inventory menu, even with another one open.
            p.with_menu(&env.rules, fx.spawns, |menu, inventory_menu, env| {
                kiln_inventory::handle_set_creative_slot(inventory_menu.unwrap_or(menu), env, slot, stack, true)
            });
        }
        PlayIn::ContainerClick { body } => {
            let Ok(click) = kiln_inventory::ContainerClick::decode(&body) else {
                p.disconnect("Invalid container click");
                return;
            };
            let crashed = p.with_menu(&env.rules, fx.spawns, |menu, _, env| {
                kiln_inventory::handle_container_click(menu, env, &click, true).is_err()
            });
            if crashed {
                p.disconnect("Invalid container click");
            }
        }
        PlayIn::ContainerClose { .. } => {
            if p.open_menu.is_some() {
                p.with_menu(&env.rules, fx.spawns, |open, inventory_menu, env| {
                    kiln_inventory::click::close_container(open, inventory_menu, env)
                });
                p.open_menu = None;
            } else {
                p.with_menu(&env.rules, fx.spawns, |menu, _, env| kiln_inventory::click::close_container(menu, None, env));
            }
        }
        PlayIn::ContainerButtonClick { container_id, button_id } => {
            p.with_menu(&env.rules, fx.spawns, |menu, _, env| {
                kiln_inventory::click::handle_container_button_click(menu, env, container_id, button_id, true)
            });
        }
        PlayIn::PlayerAction { action, pos, sequence, .. } => {
            // `ServerboundPlayerActionPacket.Action` ordinals.
            const DROP_ALL_ITEMS: i32 = 4;
            const DROP_ITEM: i32 = 5;
            const RELEASE_USE_ITEM: i32 = 6;
            match action {
                RELEASE_USE_ITEM => p.stop_using(),
                DROP_ITEM | DROP_ALL_ITEMS => {
                    if let Some(spawn) = p.drop_held(action == DROP_ALL_ITEMS) {
                        fx.spawns.push(spawn);
                        // `ServerPlayer.drop(boolean)`.
                        p.attack_ticker = 0;
                    }
                    return;
                }
                digging::START_DESTROY_BLOCK | digging::STOP_DESTROY_BLOCK | digging::ABORT_DESTROY_BLOCK => {
                    let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                    digging::player_action(p, &mut level, action, pos);
                }
                _ => {}
            }
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItem { hand, sequence, .. } => {
            p.use_item(hand == kiln_proto::packets::serverbound::Hand::Off, fx.spawns);
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItemOn { hand, pos, face, cursor, sequence, .. } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            use_item_on(p, &mut level, hand, pos, face, cursor);
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        // `handlePunch`: the swing resets the attack strength.
        PlayIn::Punch => {
            p.swung = true;
            p.attack_ticker = 0;
        }
        // `handleInteract`: nothing Kiln simulates reacts to a right click on an entity yet
        // (players, items and projectiles pass); the reach check still applies.
        PlayIn::Interact { sneaking, .. } => {
            if sneaking != p.sneaking {
                p.sneaking = sneaking;
                p.meta_dirty = true;
            }
        }
        PlayIn::ClientTickEnd => {
            p.position_this_tick = false;
            if !std::mem::take(&mut p.moved_this_tick) {
                p.known_movement = [0.0; 3];
            }
        }
        _ => {}
    }
}

/// `ServerGamePacketListenerImpl.handleUseItemOn` and `ServerPlayerGameMode.useItemOn`: the
/// clicked block reacts (levers, doors, ...) unless the player sneaks with something in hand;
/// otherwise a held block item is placed. The player always gets the clicked block and the
/// one next to it back, to settle its prediction.
fn use_item_on(p: &mut Player, level: &mut RegionLevel, hand: i32, pos: [i32; 3], face: i32, cursor: [f32; 3]) {
    let Some(dir) = blocks::direction(face) else { return };
    if !p.can_reach_block(pos, 1.0) || cursor.iter().any(|&c| (c as f64 - 0.5).abs() >= 1.0000001) {
        return;
    }
    let step = dir.step();
    let next = [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]];
    let top = level.env.min_y + level.env.height - 1;
    if pos[1] <= top && p.awaiting_teleport.is_none() && p.game_mode != 3 {
        use_on_block(p, level, hand, pos, dir, cursor);
    }
    p.resend_block(level, pos);
    p.resend_block(level, next);
}

fn use_on_block(p: &mut Player, level: &mut RegionLevel, hand: i32, pos: [i32; 3], dir: kiln_blocks::Direction, cursor: [f32; 3]) {
    use kiln_item::component::EquipmentSlot;
    let main_hand = hand == 0;
    let held = if main_hand { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    let have_something = !p.inv.selected_item().is_empty() || !p.inv.equipped(EquipmentSlot::OffHand).is_empty();
    let bp = BlockPos::new(pos[0], pos[1], pos[2]);
    let item_name = if held.is_empty() {
        None
    } else {
        kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(held.item() as usize).copied())
    };
    let actor = Actor { yaw: p.rot[0], may_build: p.game_mode <= 1, creative: p.game_mode == 1 };
    if !(p.sneaking && have_something) && main_hand && !interact::passes_to_item(level.block(bp), item_name, dir) {
        let used = interact::use_without_item(level, bp, &actor);
        if used {
            return;
        }
    }
    // `ItemStack.useOn` for block items (adventure players cannot place).
    let Some(item) = item_name.and_then(BlockItem::of_item) else { return };
    if !actor.may_build {
        return;
    }
    let click = [pos[0] as f64 + cursor[0] as f64, pos[1] as f64 + cursor[1] as f64, pos[2] as f64 + cursor[2] as f64];
    let ctx = PlaceContext { hit: bp, face: dir, click, yaw: p.rot[0], pitch: p.rot[1], sneaking: p.sneaking };
    let Some((at, state)) = placement::placement(level, &item, &ctx) else { return };
    if obstructed(p, level.bodies, at, state) {
        return;
    }
    if placement::place(level, &item, &ctx).is_none() {
        return;
    }
    if p.game_mode != 1 {
        let slot = kiln_inventory::inventory::equipment_index(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, p.inv.selected);
        kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
    }
}

/// `Level.isUnobstructed`: the placed block's collision boxes would overlap a player (this one
/// where it is now, others where they were at the start of the phase).
fn obstructed(p: &Player, bodies: &[EntityBox], at: BlockPos, state: u16) -> bool {
    let boxes = kiln_data::block_props::collision(state);
    if boxes.is_empty() {
        return false;
    }
    let h = if p.sneaking { 1.5 } else { 1.8 };
    let me = EntityBox {
        min: [p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3],
        max: [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3],
        living: true,
        blocks_building: p.game_mode != 3,
        conn: Some(p.conn),
    };
    let origin = [at.x as f64, at.y as f64, at.z as f64];
    let others = bodies.iter().filter(|b| b.conn != Some(p.conn));
    std::iter::once(&me).chain(others).filter(|b| b.blocks_building).any(|b| {
        boxes.iter().any(|a| {
            (0..3).all(|i| b.min[i] < origin[i] + a[i + 3] as f64 && b.max[i] > origin[i] + a[i] as f64)
        })
    })
}

/// Returns whether the move was accepted.
fn handle_move(
    p: &mut Player,
    world: &CellSet<Cell>,
    env: &Env,
    pos: Option<[f64; 3]>,
    rot: Option<[f32; 2]>,
    on_ground: bool,
) -> bool {
    let now = env.game_time;
    if movement::invalid(pos, rot) {
        p.disconnect("Invalid movement");
        return false;
    }
    if pos.is_some() {
        // The 26.3 client sends at most one position per client tick.
        if p.position_this_tick {
            p.disconnect("Invalid movement");
            return false;
        }
        p.position_this_tick = true;
    }
    if p.load_timeout > 0 {
        return false;
    }
    let rot = rot.map_or(p.rot, movement::normalize_rotation);
    if p.awaiting_teleport.is_some() {
        // Movement sent before the client saw our teleport is stale; only the view turns.
        p.rot = rot;
        if now - p.teleport_sent > movement::TELEPORT_RESEND_TICKS {
            p.teleport(p.pos, rot, now);
        }
        return false;
    }
    let to = pos.map_or(p.pos, movement::clamp_position);
    p.move_packets += 1;
    if env.movement_check && movement::too_fast(p.first_good, to, 0.0, p.move_packets, false) {
        let d = [to[0] - p.first_good[0], to[1] - p.first_good[1], to[2] - p.first_good[2]];
        warn!("{} moved too quickly! {d:?}", p.name);
        p.teleport(p.pos, p.rot, now);
        return false;
    }
    // Spectators have no physics.
    if p.game_mode != 3 && to != p.pos {
        let old = movement::Aabb::player(p.pos, movement::MIN_POSE_HEIGHT);
        let new = movement::Aabb::player(to, movement::MIN_POSE_HEIGHT);
        if movement::collides_with_anything_new(world, old, new) {
            p.teleport(p.pos, rot, now);
            return false;
        }
    }
    p.pos = to;
    p.rot = rot;
    p.on_ground = on_ground;
    true
}

/// Start of a connection's tick: block change acks (after the block updates they
/// acknowledge, like vanilla's connection tick), movement bookkeeping and keep-alives.
fn tick_connection(p: &mut Player, env: &Env) {
    if p.ack_block_changes >= 0 {
        p.send(packets::block_changed_ack(p.ack_block_changes));
        p.ack_block_changes = -1;
    }
    p.first_good = p.pos;
    p.move_packets = 0;
    p.load_timeout = p.load_timeout.saturating_sub(1);
    if let Some((_, sent)) = p.keep_alive {
        if env.now - sent > KEEP_ALIVE_TIMEOUT {
            warn!("{} timed out", p.name);
            p.disconnect("Timed out");
        }
    } else if env.now - p.last_keep_alive > KEEP_ALIVE_INTERVAL {
        p.keep_alive = Some((env.keep_alive_id, env.now));
        p.last_keep_alive = env.now;
        p.send(packets::keep_alive(env.keep_alive_id));
    }
}

/// Recenters the player's chunk view and streams missing loaded chunks, nearest first;
/// missing chunks that are not loaded yet are requested.
fn update_chunks(p: &mut Player, cells: &mut CellSet<Cell>, env: &Env, wanted: &mut Vec<(u32, ConnId, ChunkPos)>) {
    let center = ChunkPos::of_block(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
    let r = p.view_distance;
    // A smaller view distance forgets chunks too (vanilla `updateChunkTracking`).
    if center != p.center || r != p.applied_view {
        if center != p.center {
            p.center = center;
            p.send(packets::set_chunk_cache_center(center.x, center.z));
        }
        p.applied_view = r;
        let mut stale: Vec<_> =
            p.sent_chunks.iter().copied().filter(|c| (c.x - center.x).abs() > r || (c.z - center.z).abs() > r).collect();
        stale.sort_unstable();
        for c in stale {
            p.sent_chunks.remove(&c);
            p.send(packets::forget_level_chunk(c.x, c.z));
        }
    }
    if p.unacked_batches >= MAX_UNACKED_BATCHES {
        return;
    }
    let mut missing: Vec<ChunkPos> = Vec::new();
    for x in center.x - r..=center.x + r {
        for z in center.z - r..=center.z + r {
            let c = ChunkPos::new(x, z);
            if !p.sent_chunks.contains(&c) {
                missing.push(c);
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    missing.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));
    let budget = (p.chunks_per_tick.ceil() as usize).max(1);
    let mut batch = Vec::new();
    // Ask for about what the client takes in the next tick or two, nearest first.
    let mut asked = 0;
    for c in missing {
        match cells.chunk_mut(c) {
            Some(chunk) if batch.len() < budget => batch.push((c, chunk.packet(c.x, c.z, env.biome_count))),
            Some(_) => {}
            None if asked < 2 * budget => {
                wanted.push((asked as u32, p.conn, c));
                asked += 1;
            }
            None => {}
        }
    }
    if batch.is_empty() {
        return;
    }
    p.send(packets::chunk_batch_start());
    for (c, body) in &batch {
        p.send(body.clone());
        p.sent_chunks.insert(*c);
    }
    p.send(packets::chunk_batch_finished(batch.len() as i32));
    p.unacked_batches += 1;
}

/// De-duplicated union of chunk requests, keeping the first occurrence.
pub(crate) fn merge_requests(requests: impl IntoIterator<Item = ChunkPos>) -> Vec<ChunkPos> {
    let mut seen = HashSet::new();
    requests.into_iter().filter(|c| seen.insert(*c)).collect()
}
