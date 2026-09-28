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
use kiln_sched::{Ctx, Window};
use kiln_world::{Blocks, Cell, CellStore, ChunkPos};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Read-only values of the global state that region work needs.
#[derive(Clone)]
pub(crate) struct Env {
    pub rules: std::sync::Arc<kiln_inventory::Rules>,
    /// The level the region is in.
    pub dim: crate::DimId,
    /// Portal game rules.
    pub portal: crate::portal::PortalRules,
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
    /// Whether keep-alives are sent (`SimConfig::keep_alive`).
    pub keep_alive: bool,
    pub blocks: blocks::BlockEnv,
    /// `/tick freeze`: only players tick (`TickRateManager.runsNormally` is false).
    pub frozen: bool,
    /// The level's world border, for players outside it.
    pub border: crate::world_state::BorderBox,
    /// Force-loaded chunks of the level, which tick without players near.
    pub forced: std::sync::Arc<Vec<ChunkPos>>,
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
    /// Players whose portal took them (they change level in a serial phase).
    pub portals: Vec<crate::portal::Travel>,
    /// CPU time per sub-phase, for the statistics.
    pub times: [Duration; SUB_PHASES.len()],
}

pub(crate) const SUB_PHASES: [&str; 9] =
    ["menus", "connections", "chunks", "blocks", "entities", "visibility", "movement", "light", "egress"];

pub(crate) struct RegionWork<'a> {
    /// The level the region is in.
    pub dim: crate::DimId,
    pub region: kiln_region::RegionId,
    pub cells: &'a mut CellSet<Cell>,
    pub entities: &'a mut Entities,
    pub blocks: &'a mut RegionBlocks,
    /// Sorted by connection id.
    pub players: Vec<&'a mut Player>,
    /// This region's packets for the tick, in arrival order.
    pub packets: Vec<(ConnId, PlayIn)>,
    /// The region's plugin instances.
    pub plugins: Option<crate::plugins::RegionHook<'a>>,
    /// Injected at the start of each tick ([`crate::SimConfig::inject_delay`], tests).
    pub delay: Duration,
    pub out: RegionOut,
}

impl RegionWork<'_> {
    fn index_of(&self, conn: ConnId) -> Option<usize> {
        self.players.binary_search_by_key(&conn, |p| p.conn).ok()
    }

    /// P1: applies the region's packets in arrival order. Runs of packets that each touch only
    /// their player ([`is_player_packet`]) apply in windows, player by player in arrival
    /// order; what they leave behind merges in arrival order. With plugins (which may deny
    /// any packet) everything applies in order on this thread.
    pub fn apply_packets(&mut self, env: &Env, ctx: &Ctx<'_>) {
        let mut out = BlockOut::default();
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let mut packets = std::mem::take(&mut self.packets).into_iter().peekable();
        while let Some((conn, pkt)) = packets.next() {
            if self.plugins.is_none() && is_player_packet(&pkt) && !self.rod_use(conn, &pkt) {
                let mut run = vec![(conn, pkt)];
                while let Some(next) = packets.next_if(|(c, p)| is_player_packet(p) && !self.rod_use(*c, p)) {
                    run.push(next);
                }
                self.apply_player_packets(run, env, ctx);
                continue;
            }
            let Some(i) = self.index_of(conn) else { continue };
            if let Some(h) = self.plugins.as_mut()
                && crate::plugins::deny_packet(h, self.players[i], self.cells, env, &pkt, &mut self.out.spawns)
            {
                continue;
            }
            if let PlayIn::Attack { entity_id } = pkt {
                if !self.players[i].dead {
                    let attack_env = crate::combat::AttackEnv { cells: &*self.cells, game_time: env.game_time, seed: env.blocks.seed };
                    let mut ctx = damage_ctx(env, &mut self.out.spawns, &mut self.out.deaths);
                    let mut hits = Vec::new();
                    crate::combat::handle_attack(&mut self.players, i, entity_id, self.entities, &attack_env, &mut ctx, &mut hits);
                    for hit in hits {
                        let mut level = RegionLevel {
                            cells: &mut *self.cells,
                            blocks: &mut *self.blocks,
                            env: &env.blocks,
                            out: &mut out,
                            bodies: &bodies,
                            actor: None,
                        };
                        entities::hit_mob(self.entities, &mut level, &mut self.players, &mut self.out.spawns, &mut self.out.deaths, &hit);
                    }
                }
                continue;
            }
            // Fishing rods cast and reel in bobbers, which are the region's entities.
            if let PlayIn::UseItem { hand, sequence, .. } = pkt
                && self.rod_use(conn, &pkt)
            {
                let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
                let off = hand == kiln_proto::packets::serverbound::Hand::Off;
                crate::fishing::use_rod(self.entities, &mut level, &mut self.players, i, off, &mut self.out.spawns);
                let p = &mut *self.players[i];
                p.ack_block_changes = p.ack_block_changes.max(sequence);
                continue;
            }
            // Riding: the steered mount moves, the jump key makes it rear.
            if let PlayIn::MoveVehicle { pos, rot, on_ground } = pkt {
                entities::move_vehicle(self.entities, &mut self.players, i, pos, rot, on_ground, env.game_time);
                continue;
            }
            if let PlayIn::RidingJump { data } = pkt {
                entities::riding_jump(self.entities, &mut self.players, i, data, &env.blocks);
                continue;
            }
            if let PlayIn::Interact { entity_id, hand, sneaking, .. } = pkt {
                if let Some(h) = self.plugins.as_mut()
                    && crate::plugins::deny_interact(h, self.players[i], self.entities, entity_id)
                {
                    continue;
                }
                let p = &mut *self.players[i];
                if sneaking != p.sneaking {
                    p.sneaking = sneaking;
                    p.meta_dirty = true;
                }
                let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
                let off = hand == kiln_proto::packets::serverbound::Hand::Off;
                entities::interact_mob(self.entities, &mut level, &mut self.players, i, entity_id, off, &mut self.out.spawns, &mut self.out.deaths);
                crate::trading::open_if_requested(self.entities, self.players[i], entity_id, &env.rules, &mut self.out.spawns);
                continue;
            }
            // Beds and respawn anchors need the region's entities (monsters nearby, explosions).
            if let PlayIn::UseItemOn { hand: 0, pos, face, cursor, sequence, .. } = pkt
                && crate::sleep::intercepts(self.players[i], &*self.cells, &env.blocks, pos, face, cursor)
            {
                let mut level =
                    RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: Some(conn) };
                crate::sleep::use_item_on(self.entities, &mut level, &mut self.players, i, pos, face, &mut self.out.spawns, &mut self.out.deaths);
                let p = &mut *self.players[i];
                p.ack_block_changes = p.ack_block_changes.max(sequence);
                continue;
            }
            let mut world = World { cells: &mut *self.cells, blocks: &mut *self.blocks };
            let mut fx = Fx { blocks: &mut out, bodies: &bodies, spawns: &mut self.out.spawns, deaths: &mut self.out.deaths };
            local_packet(self.players[i], &mut world, env, pkt, &mut fx);
            if !self.players[i].merchant_events.is_empty() {
                let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
                crate::trading::apply_events(self.entities, &mut level, &mut self.players, i, &mut self.out.spawns, &mut self.out.deaths);
            }
        }
        if let Some(h) = self.plugins.as_mut() {
            crate::plugins::after_packets(h, self.cells, env);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
    }

    /// A Use Item with a fishing rod in that hand, from a living player.
    fn rod_use(&self, conn: ConnId, pkt: &PlayIn) -> bool {
        let PlayIn::UseItem { hand, .. } = pkt else { return false };
        let off = *hand == kiln_proto::packets::serverbound::Hand::Off;
        self.index_of(conn).is_some_and(|i| !self.players[i].dead && self.players[i].game_mode != 3 && crate::fishing::holds_rod(self.players[i], off))
    }

    /// A run of [`is_player_packet`] packets: grouped by player (each keeps its order) and
    /// applied in a window; drops and deaths merge in arrival order.
    fn apply_player_packets(&mut self, run: Vec<(ConnId, PlayIn)>, env: &Env, ctx: &Ctx<'_>) {
        let mut jobs: Vec<Vec<(usize, PlayIn)>> = (0..self.players.len()).map(|_| Vec::new()).collect();
        for (seq, (conn, pkt)) in run.into_iter().enumerate() {
            if let Some(i) = self.index_of(conn) {
                jobs[i].push((seq, pkt));
            }
        }
        let mut items: Vec<(&mut &mut Player, Vec<(usize, PlayIn)>)> =
            self.players.iter_mut().zip(jobs).filter(|(_, j)| !j.is_empty()).collect();
        let cells = &*self.cells;
        let left = ctx.map_mut_with(PACKET_WINDOW, &mut items, |_, (p, pkts)| {
            let mut left = Vec::new();
            for (seq, pkt) in std::mem::take(pkts) {
                let (mut spawns, mut deaths) = (Vec::new(), Vec::new());
                let rest = player_packet(p, cells, env, pkt, &mut spawns, &mut deaths);
                debug_assert!(rest.is_none(), "a player packet came back");
                if !spawns.is_empty() || !deaths.is_empty() {
                    left.push((seq, spawns, deaths));
                }
            }
            left
        });
        let mut left: Vec<_> = left.into_iter().flatten().collect();
        left.sort_unstable_by_key(|(seq, _, _)| *seq);
        for (_, spawns, deaths) in left {
            self.out.spawns.extend(spawns);
            self.out.deaths.extend(deaths);
        }
    }

    /// L: connection upkeep, chunk streaming, tracking, light, then egress.
    pub fn tick(&mut self, env: &Env, ctx: &Ctx<'_>) {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let mut lap = Instant::now();
        let mut mark = |times: &mut [Duration; SUB_PHASES.len()], i: usize| {
            let now = Instant::now();
            times[i] += now - lap;
            lap = now;
        };
        // Menu changes first, like vanilla's container broadcast at the start of a player tick,
        // then `stillValid`: a menu whose block went away or is out of reach closes.
        // A player without a block menu open touches only itself here, so those players
        // broadcast in windows first; the others then run serially in connection order, and
        // the drops land in that order either way.
        {
            let rules = &*env.blocks.menus;
            let own = ctx.map_mut_with(PLAYER_WINDOW, &mut self.players, |_, p| {
                p.containers.open.is_none().then(|| crate::container::open::own_menu_broadcast(p, rules))
            });
            let bodies = Vec::new();
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
                for (p, own) in self.players.iter_mut().zip(own) {
                    if let Some(spawns) = own {
                        self.out.spawns.extend(spawns);
                        continue;
                    }
                    crate::container::open::menu_op(p, &mut level, &mut self.out.spawns, |menu, _, env| menu.broadcast_changes(env));
                    if p.open_menu.is_some() && !p.menu_still_valid(&level) {
                        p.close_block_menu(&env.rules, &mut self.out.spawns, &mut level, true);
                    }
                }
            }
            blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        }
        mark(&mut self.out.times, 0);
        // The player tick in vanilla's order: base tick (fire, void, air, effects), using an
        // item, equipment, the blocks the player is in, then food. Each player touches only
        // itself and reads the region's blocks, so the players split into windows; what they
        // leave behind is merged in connection order, as a serial loop would have left it.
        let cells = &*self.cells;
        let ticked = ctx.map_mut_with(PLAYER_WINDOW, &mut self.players, |_, p| player_tick(p, cells, env));
        for t in ticked {
            self.out.spawns.extend(t.spawns);
            self.out.deaths.extend(t.deaths);
            self.out.portals.extend(t.portals);
        }
        mark(&mut self.out.times, 1);
        // Which chunks each player lacks is its own business (a window); sending them needs
        // the chunks' packet caches, so that part runs in connection order here.
        let missing = ctx.map_mut_with(PLAYER_WINDOW, &mut self.players, |_, p| if p.disconnected { Vec::new() } else { chunk_view(p) });
        for (p, missing) in self.players.iter_mut().zip(missing).filter(|(_, m)| !m.is_empty()) {
            send_chunks(p, missing, &mut *self.cells, env, &mut self.out.wanted);
        }
        // The same tick everywhere, so when chunks unload does not depend on the regions.
        if env.game_time % 20 == 0 {
            self.find_unloads();
        }
        mark(&mut self.out.times, 2);
        self.tick_blocks(env);
        mark(&mut self.out.times, 3);
        self.tick_entities(env);
        crate::trading::check_menus(self.entities, &mut self.players, &env.rules, &mut self.out.spawns);
        entities::pickups(self.entities, &mut self.players);
        crate::xp::pick_up_orbs(self.entities, &mut self.players);
        mark(&mut self.out.times, 4);
        let movers = crate::players::update_visibility(&mut self.players, ctx);
        mark(&mut self.out.times, 5);
        crate::players::broadcast_movement(&mut self.players, ctx);
        for p in self.players.iter_mut() {
            p.decay_velocity();
        }
        entities::track(self.entities, &mut self.players, &movers);
        mark(&mut self.out.times, 6);
        self.send_light_updates();
        mark(&mut self.out.times, 7);
        ctx.map_mut_with(PLAYER_WINDOW, &mut self.players, |_, p| p.flush());
        mark(&mut self.out.times, 8);
    }

    /// The block phases: players' digging, pressure plates under bodies, then scheduled
    /// ticks, random ticks, block events and moving pistons in chunks near players.
    fn tick_blocks(&mut self, env: &Env) {
        let ticking = ticking_chunks(&self.players, env);
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let mut out = BlockOut::default();
        if let Some(h) = self.plugins.as_mut() {
            crate::plugins::watch_delayed_breaks(h, &self.players, self.cells);
        }
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
            // `Player.tick`'s sleeping part and the insomnia statistic.
            for p in self.players.iter_mut().filter(|p| !p.disconnected) {
                crate::sleep::tick_player(p, &mut level);
            }
            // A frozen game (`/tick freeze`) ticks no blocks.
            if !env.frozen {
                blocks::press_plates(&mut level);
                blocks::tick_blocks(&mut level, &ticking);
                for pos in std::mem::take(&mut level.out.rechecks) {
                    crate::container::open::recheck_openers(&mut level, &self.players, pos);
                }
                blocks::tick_pistons(&mut level, &ticking);
            }
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        if let Some(h) = self.plugins.as_mut() {
            crate::plugins::after_packets(h, self.cells, env);
        }
    }

    /// The entity phase: the region's entities tick against its blocks; what they change
    /// goes out like block work.
    fn tick_entities(&mut self, env: &Env) {
        // `TickRateManager.isEntityFrozen`: nothing but players ticks while frozen.
        if env.frozen {
            return;
        }
        if self.entities.list.is_empty() && (self.players.is_empty() || env.blocks.spawn_table.is_none()) {
            self.tick_block_entities(env);
            return;
        }
        let ticking = ticking_chunks(&self.players, env);
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
            let any_player = !self.players.is_empty();
            crate::spawner::tick(&mut level, self.entities, &self.players, &ticking, &mut self.out.spawns);
            entities::tick(self.entities, &mut level, &ticking, &mut self.players, &mut self.out.spawns, &mut self.out.deaths, any_player);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        self.tick_block_entities(env);
    }

    /// `Level.tickBlockEntities`: hoppers and furnaces in ticking chunks. Hoppers take item
    /// entities; their viewers see the new counts.
    fn tick_block_entities(&mut self, env: &Env) {
        if self.blocks.containers.len() == 0 {
            return;
        }
        let ticking = ticking_chunks(&self.players, env);
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let mut out = BlockOut::default();
        let mut items = crate::container::hopper::EntityItems::new(self.entities);
        {
            let mut level = RegionLevel {
                cells: &mut *self.cells,
                blocks: &mut *self.blocks,
                env: &env.blocks,
                out: &mut out,
                bodies: &bodies,
                actor: None,
            };
            crate::container::tick_block_entities(&mut level, &mut items, &ticking);
        }
        let touched = items.touched();
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        crate::container::hopper::send_item_counts(self.entities, &mut self.players, &touched);
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

/// Players per window chunk in the per-player windows of a crowd (a few microseconds each).
const PLAYER_WINDOW: Window = Window::new();
/// A player's packets of one run (mostly a move and a tick end).
const PACKET_WINDOW: Window = Window::new();

/// What one player's tick leaves for its region, merged in connection order.
#[derive(Default)]
struct PlayerTicked {
    spawns: Vec<Spawn>,
    deaths: Vec<crate::health::Death>,
    portals: Vec<crate::portal::Travel>,
}

/// One player's part of the connection phase: connection upkeep and the player tick. It
/// changes only the player and reads the region's blocks.
fn player_tick(p: &mut Player, cells: &CellSet<Cell>, env: &Env) -> PlayerTicked {
    let mut t = PlayerTicked::default();
    let block = |pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0);
    tick_connection(p, env);
    p.tick_damage(env.game_time);
    let mut ctx = damage_ctx(env, &mut t.spawns, &mut t.deaths);
    p.base_tick(&block, env.min_y, &env.border, &mut ctx);
    // `Entity.handlePortal` (in `baseTick`).
    if let Some(travel) = p.handle_portal(env) {
        t.portals.push(travel);
    }
    p.tick_using(&block, &mut ctx);
    p.tick_combat();
    let (_, h, _) = p.dimensions();
    let in_rain = crate::weather::in_rain(cells, &env.blocks, p.pos, p.pos[1] + h as f64);
    p.block_effects(&block, env.dim, in_rain, &mut ctx);
    if let Some(travel) = p.pending_travel.take() {
        t.portals.push(travel);
    }
    p.tick_food(env.natural_regen, &mut ctx);
    p.tick_stats();
    let probe = crate::advancements::triggers::CellProbe::new(cells, &env.blocks);
    p.tick_triggers(&probe);
    // `onInsideBlock` (Kiln checks the block at the feet).
    let feet = kiln_entity::math::BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
    let inside = block(feet);
    if inside != 0 && !p.dead {
        p.entered_block(inside);
    }
    p.sync_health();
    p.sync_experience();
    t
}

/// Chunks that tick: those within the simulation distance of the region's players and the
/// level's force-loaded chunks.
fn ticking_chunks(players: &[&mut Player], env: &Env) -> Ticking {
    let mut t = Ticking::around(players.iter().map(|p| p.center), env.blocks.simulation_distance);
    for &c in env.forced.iter() {
        t.add(c);
    }
    t
}

/// Damage context for region work.
pub(crate) fn damage_ctx<'a>(
    env: &Env,
    spawns: &'a mut Vec<Spawn>,
    deaths: &'a mut Vec<crate::health::Death>,
) -> crate::health::DamageCtx<'a> {
    crate::health::DamageCtx { rules: env.blocks.damage, game_time: env.game_time, spawns, deaths, level_rng: None }
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

/// Whether [`player_packet`] handles the packet: it changes only its player and reads the
/// region's blocks (movement, keep-alives, client settings, ...), so packets like these from
/// different players may apply in parallel.
pub(crate) fn is_player_packet(pkt: &PlayIn) -> bool {
    match pkt {
        PlayIn::AcceptTeleport { .. }
        | PlayIn::KeepAlive { .. }
        | PlayIn::Move { .. }
        | PlayIn::PlayerAbilities { .. }
        | PlayIn::ChunkBatchReceived { .. }
        | PlayIn::ClientInformation(_)
        | PlayIn::PlayerInput { .. }
        | PlayIn::PlayerLoaded
        | PlayIn::SetCarriedItem { .. }
        | PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::RequestStats)
        | PlayIn::ClientTickEnd => true,
        PlayIn::PlayerCommand { action } => *action != STOP_SLEEPING,
        _ => false,
    }
}

/// `PlayerCommand`'s "Leave Bed" action.
const STOP_SLEEPING: i32 = 0;

/// The packets of [`is_player_packet`]; any other packet comes back for [`local_packet`]
/// (`None`: handled, or ignored because the player is dead).
pub(crate) fn player_packet(
    p: &mut Player,
    cells: &CellSet<Cell>,
    env: &Env,
    pkt: PlayIn,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<crate::health::Death>,
) -> Option<PlayIn> {
    if p.dead && !matches!(pkt, PlayIn::KeepAlive { .. } | PlayIn::ChunkBatchReceived { .. } | PlayIn::ClientTickEnd) {
        return None;
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
        // A passenger's moves only turn it (`handlePlayerPositionChange` while riding).
        PlayIn::Move { rot, .. } if p.vehicle.is_some() => {
            if let Some(r) = rot.filter(|r| r.iter().all(|a| a.is_finite())) {
                p.rot = crate::movement::normalize_rotation(r);
            }
        }
        PlayIn::Move { pos, rot, on_ground } => {
            let (from, was_on_ground) = (p.pos, p.on_ground);
            let y0 = p.pos[1];
            if handle_move(p, cells, env, pos, rot, on_ground) {
                let feet = p.pos.map(|c| c.floor() as i32);
                let in_fluid = cells.get_block(feet[0], feet[1], feet[2]).is_some_and(kiln_data::blocks_types::has_fluid);
                let d = [p.pos[0] - from[0], p.pos[1] - from[1], p.pos[2] - from[2]];
                // `handlePlayerKnownMovement`.
                p.known_movement = d;
                p.moved_this_tick = true;
                // `Block.updateEntityMovementAfterFallOn`: landing stops the fall.
                if on_ground {
                    p.vel[1] = 0.0;
                }
                p.exhaust_for_move(d, was_on_ground, in_fluid);
                // `jumpFromGround` and `checkMovementStatistics`.
                if was_on_ground && !on_ground && d[1] > 0.0 {
                    p.award_stat(*crate::player_stats::stat::JUMP, 1);
                }
                let eye = [feet[0], (p.pos[1] + if p.sneaking { 1.27 } else { 1.62 }).floor() as i32, feet[2]];
                let eyes_in_water = cells.get_block(eye[0], eye[1], eye[2]).is_some_and(kiln_data::blocks_types::has_fluid);
                let climbing = cells.get_block(feet[0], feet[1], feet[2]).is_some_and(crate::player_stats::climbable);
                p.movement_stats(d, in_fluid, eyes_in_water, climbing);
                let mut ctx = damage_ctx(env, spawns, deaths);
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
        PlayIn::PlayerCommand { action } if action != STOP_SLEEPING => {
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
        PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::RequestStats) => {
            let pkt = p.stats.take_award_packet();
            p.send(pkt);
        }
        PlayIn::ClientTickEnd => {
            p.position_this_tick = false;
            if !std::mem::take(&mut p.moved_this_tick) {
                p.known_movement = [0.0; 3];
            }
        }
        other => return Some(other),
    }
    None
}

/// A packet that touches only its player and the world around it.
pub(crate) fn local_packet(p: &mut Player, world: &mut World, env: &Env, pkt: PlayIn, fx: &mut Fx) {
    let Some(pkt) = player_packet(p, world.cells, env, pkt, fx.spawns, fx.deaths) else { return };
    match pkt {
        // `handlePlayerCommand`: the "Leave Bed" button (the other actions are the player's own).
        PlayIn::PlayerCommand { .. } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            crate::sleep::stop_sleep_in_bed(p, &mut level, false, true);
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
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            let crashed = crate::container::open::menu_op(p, &mut level, fx.spawns, |menu, _, env| {
                kiln_inventory::handle_container_click(menu, env, &click, true).is_err()
            });
            if crashed {
                p.disconnect("Invalid container click");
            }
        }
        PlayIn::ContainerClose { .. } => {
            if p.open_menu.is_some() {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                p.close_block_menu(&env.rules, fx.spawns, &mut level, false);
            } else {
                p.with_menu(&env.rules, fx.spawns, |menu, _, env| kiln_inventory::click::close_container(menu, None, env));
            }
        }
        // `handleSelectTrade`: only a merchant screen reacts.
        PlayIn::SelectTrade { offer } => p.with_menu(&env.rules, fx.spawns, |menu, _, env| menu.select_trade(env, offer)),
        PlayIn::RenameItem { name } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            crate::container::open::menu_op(p, &mut level, fx.spawns, |menu, _, env| {
                kiln_inventory::click::handle_rename_item(menu, env, &name, true)
            });
        }
        PlayIn::ContainerButtonClick { container_id, button_id } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            crate::container::open::menu_op(p, &mut level, fx.spawns, |menu, _, env| {
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
                    // `Level.mayInteract`: nothing outside the world border breaks.
                    if env.border.contains(pos[0] as f64, pos[2] as f64) {
                        digging::player_action(p, &mut level, action, pos);
                    } else {
                        p.resend_block(&level, pos);
                    }
                }
                _ => {}
            }
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItem { hand, sequence, .. } => {
            let cells = &*world.cells;
            let block = |pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0);
            let mut ctx = damage_ctx(env, fx.spawns, fx.deaths);
            p.use_item(hand == kiln_proto::packets::serverbound::Hand::Off, &block, &mut ctx);
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItemOn { hand, pos, face, cursor, sequence, .. } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            let may_interact = env.border.contains(pos[0] as f64, pos[2] as f64);
            use_item_on(p, &mut level, hand, pos, face, cursor, may_interact, fx.spawns);
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
        PlayIn::PlaceRecipe { container_id, recipe, use_max_items } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            p.place_recipe(&mut level, fx.spawns, container_id, recipe, use_max_items);
        }
        PlayIn::SetBeacon { primary, secondary } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            crate::container::open::set_beacon(p, &mut level, fx.spawns, primary, secondary);
        }
        PlayIn::RecipeBookChangeSettings { book, open, filtering } => p.recipe_book_settings(book, open, filtering),
        PlayIn::RecipeBookSeenRecipe { recipe } => p.recipe_seen(&env.rules, recipe),
        // `handleSeenAdvancements`: opening a tab selects it.
        PlayIn::SeenAdvancements { tab: Some(tab) } => {
            if let Some(pkt) = p.advancements.select_tab(Some(&tab)) {
                p.send(pkt);
            }
        }
        _ => {}
    }
}

/// `ServerGamePacketListenerImpl.handleUseItemOn` and `ServerPlayerGameMode.useItemOn`: the
/// clicked block reacts (levers, doors, ...) unless the player sneaks with something in hand;
/// otherwise a held block item is placed. The player always gets the clicked block and the
/// one next to it back, to settle its prediction.
#[allow(clippy::too_many_arguments)]
fn use_item_on(
    p: &mut Player,
    level: &mut RegionLevel,
    hand: i32,
    pos: [i32; 3],
    face: i32,
    cursor: [f32; 3],
    may_interact: bool,
    spawns: &mut Vec<Spawn>,
) {
    let Some(dir) = blocks::direction(face) else { return };
    if !p.can_reach_block(pos, 1.0) || cursor.iter().any(|&c| (c as f64 - 0.5).abs() >= 1.0000001) {
        return;
    }
    let step = dir.step();
    let next = [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]];
    let top = level.env.min_y + level.env.height - 1;
    // `Level.mayInteract`: blocks outside the world border do not react.
    if pos[1] <= top && p.awaiting_teleport.is_none() && may_interact {
        if p.game_mode == 3 {
            crate::container::open::spectator_use(p, level, BlockPos::new(pos[0], pos[1], pos[2]), spawns);
        } else {
            use_on_block(p, level, hand, pos, dir, cursor, spawns);
        }
    }
    p.resend_block(level, pos);
    p.resend_block(level, next);
}

fn use_on_block(
    p: &mut Player,
    level: &mut RegionLevel,
    hand: i32,
    pos: [i32; 3],
    dir: kiln_blocks::Direction,
    cursor: [f32; 3],
    spawns: &mut Vec<Spawn>,
) {
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
        if let Some(consumed) = crate::container::open::use_block(p, level, bp, spawns) {
            if consumed {
                return;
            }
        } else if interact::use_without_item(level, bp, &actor) {
            return;
        }
    }
    if item_name == Some("minecraft:flint_and_steel") && actor.may_build {
        light_fire(p, level, main_hand, pos, dir);
        return;
    }
    // `SpawnEggItem.useOn`: the mob appears in the clicked block if it has no collision,
    // else next to it, facing a random way.
    if let Some(kind) = item_name.and_then(|n| n.strip_suffix("_spawn_egg")).and_then(kiln_entity::mob::MobKind::by_name)
        && p.game_mode != 3
    {
        let clicked_empty = kiln_data::block_props::collision(level.block(bp)).is_empty();
        let at = if clicked_empty { bp } else { bp.relative(dir) };
        let yaw = kiln_entity::mob::mth::wrap_degrees(kiln_javamath::random::RandomSource::next_float(level.random()) * 360.0);
        let env = level.env;
        let finalize = crate::mobs::Finalize {
            ctx: crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, 1.0),
            seed: crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (at.x as u64) << 32 ^ at.z as u64 ^ (at.y as u64) << 16),
            persistent: false,
        };
        spawns.push(crate::mobs::spawn(kind, [at.x as f64 + 0.5, at.y as f64, at.z as f64 + 0.5], Some(yaw), Some(finalize)));
        let egg = if main_hand { p.inv.selected_item().item() } else { p.inv.equipped(EquipmentSlot::OffHand).item() };
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, egg), 1);
        if p.game_mode != 1 {
            let slot = kiln_inventory::inventory::equipment_index(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, p.inv.selected);
            kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
        }
        return;
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
    let placed_from = if main_hand { p.inv.selected_item().clone() } else { p.inv.equipped(EquipmentSlot::OffHand).clone() };
    let Some((placed_at, _)) = placement::place(level, &item, &ctx) else { return };
    crate::container::open::apply_item_components(level, placed_at, &placed_from);
    // `ItemStack.useOn`: a successful item interaction counts as a use; `BlockItem.place`
    // and `ServerPlayerGameMode.useItemOn` fire their triggers.
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, placed_from.item()), 1);
    let placed_state = level.block(placed_at);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    let at = [placed_at.x, placed_at.y, placed_at.z];
    p.used_on_block("minecraft:placed_block", at, placed_state, &placed_from, &probe);
    p.used_on_block("minecraft:item_used_on_block", pos, level.block(bp), &placed_from, &probe);
    if p.game_mode != 1 {
        let slot = kiln_inventory::inventory::equipment_index(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, p.inv.selected);
        kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
    }
}

/// `FlintAndSteelItem.useOn` beside a block (campfires and candles are not lit by Kiln): fire
/// where it can burn or where it lights a nether portal frame, and a point of durability.
fn light_fire(p: &mut Player, level: &mut RegionLevel, main_hand: bool, pos: [i32; 3], dir: kiln_blocks::Direction) {
    use kiln_blocks::behaviour::portal;
    use kiln_item::component::EquipmentSlot;
    let at = BlockPos::new(pos[0], pos[1], pos[2]).relative(dir);
    let forward = kiln_blocks::Direction::from_yaw(p.rot[0] as f64);
    if !portal::fire_can_be_placed_at(level, at, forward) {
        return;
    }
    // `level.getRandom().nextFloat() * 0.4F + 0.8F` for the sound's pitch.
    let pitch = {
        use kiln_javamath::random::RandomSource;
        level.random().next_float() * 0.4 + 0.8
    };
    level.effect(kiln_blocks::level::Effect::ActorSound { pos: at, sound: "minecraft:item.flintandsteel.use", volume: 1.0, pitch });
    let fire = portal::fire_state(level, at);
    kiln_blocks::set_block(level, at, fire, kiln_blocks::flags::ALL_IMMEDIATE);
    let flint = if main_hand { p.inv.selected_item().item() } else { p.inv.equipped(EquipmentSlot::OffHand).item() };
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, flint), 1);
    p.hurt_and_break(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, 1, None);
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
        prevents_rest: false,
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
    } else if env.keep_alive && env.now - p.last_keep_alive > KEEP_ALIVE_INTERVAL {
        p.keep_alive = Some((env.keep_alive_id, env.now));
        p.last_keep_alive = env.now;
        p.send(packets::keep_alive(env.keep_alive_id));
    }
}

/// Recenters the player's chunk view (forgetting chunks now out of it) and returns the chunks
/// it lacks, nearest first (none while the client is behind on batches).
fn chunk_view(p: &mut Player) -> Vec<ChunkPos> {
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
        return Vec::new();
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
    missing.sort_by_key(|c| (c.x - center.x).pow(2) + (c.z - center.z).pow(2));
    missing
}

/// Streams the missing chunks that are loaded, nearest first; missing chunks that are not
/// loaded yet are requested.
fn send_chunks(p: &mut Player, missing: Vec<ChunkPos>, cells: &mut CellSet<Cell>, env: &Env, wanted: &mut Vec<(u32, ConnId, ChunkPos)>) {
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
