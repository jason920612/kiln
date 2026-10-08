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
    /// `minecraft:elytra_movement_check`: whether a gliding player's moves are checked too.
    pub elytra_movement_check: bool,
    /// `minecraft:spectators_generate_chunks`: whether a spectator's view asks for chunks to
    /// be loaded and generated (`ChunkMap.skipPlayer`).
    pub spectators_generate_chunks: bool,
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
    /// Entities that came out of saved compounds this phase (what sat on a player's shoulder),
    /// to be loaded like the ones a chunk brings.
    pub saved_entities: Vec<kiln_proto::nbt::Tag>,
    /// Entities that may be touching a nether or end portal block at the end of the tick (the
    /// only ones [`crate::Sim::entity_portals`] looks at, with the entities spawned after).
    pub portal_candidates: Vec<i32>,
    /// CPU time per sub-phase, for the statistics.
    pub times: [Duration; SUB_PHASES.len()],
    /// Of which in split windows (owner's wall time).
    pub win: [Duration; SUB_PHASES.len()],
}

pub(crate) const SUB_WIN: [&str; 10] =
    ["menus~", "connections~", "chunks~", "blocks~", "entities~", "visibility~", "movement~", "light~", "egress~", "spawning~"];
pub(crate) const SUB_PHASES: [&str; 10] =
    ["menus", "connections", "chunks", "blocks", "entities", "visibility", "movement", "light", "egress", "spawning"];

pub(crate) struct RegionWork<'a> {
    /// The level the region is in.
    pub dim: crate::DimId,
    pub region: kiln_region::RegionId,
    pub cells: &'a mut CellSet<Cell>,
    pub entities: &'a mut Entities,
    pub blocks: &'a mut RegionBlocks,
    /// Sorted by connection id.
    pub players: Vec<&'a mut Player>,
    /// The players' connection ids, side by side (a search through the players themselves
    /// misses the cache at every step).
    pub conns: Vec<ConnId>,
    /// This region's packets for the tick, in arrival order.
    pub packets: Vec<(ConnId, PlayIn)>,
    /// The region's plugin instances.
    pub plugins: Option<crate::plugins::RegionHook<'a>>,
    /// Injected at the start of each tick ([`crate::SimConfig::inject_delay`], tests).
    pub delay: Duration,
    /// Chunks generated and installed since the region last ran, still to light (in the order
    /// they came in), before anything else reads them.
    pub unlit: Vec<kiln_world::ChunkPos>,
    pub out: RegionOut,
}

impl RegionWork<'_> {
    /// Lights the chunks installed since the region last ran ([`kiln_world::light::light_new_chunk`]):
    /// light spreads at most one chunk, so it stays within the region's own cells.
    pub(crate) fn light_new_chunks(&mut self) {
        if self.unlit.is_empty() {
            return;
        }
        let dt = std::time::Instant::now();
        for pos in std::mem::take(&mut self.unlit) {
            kiln_world::light::light_new_chunk(&mut *self.cells, pos);
        }
        crate::diag::lap("r.light_new", dt);
    }
}

impl RegionWork<'_> {
    fn index_of(&self, conn: ConnId) -> Option<usize> {
        self.conns.binary_search(&conn).ok()
    }

    /// P1: applies the region's packets in arrival order. Runs of packets that each touch only
    /// their player ([`is_player_packet`]) apply in windows, player by player in arrival
    /// order; what they leave behind merges in arrival order. With plugins (which may deny
    /// any packet) everything applies in order on this thread.
    pub fn apply_packets(&mut self, env: &Env, ctx: &Ctx<'_>) {
        let dt = Instant::now();
        let mut out = BlockOut::default();
        // The boxes at the start of the phase, for the packets that place or use things (a
        // crowd's ticks are mostly movement, which needs none).
        let needs_bodies = self.plugins.is_some() || self.packets.iter().any(|(c, p)| !is_player_packet(p) || self.rod_use(*c, p));
        let bodies = if needs_bodies { blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities) } else { Vec::new() };
        crate::diag::lap("ap.bodies", dt);
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
            // `ServerboundPlayerActionPacket.Action.STAB`: a spear's piercing attack.
            if let PlayIn::PlayerAction { action: STAB, .. } = pkt {
                if !self.players[i].dead {
                    let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
                    crate::spear::piercing_attack(self.entities, &mut level, &mut self.players, i, &mut self.out.spawns, &mut self.out.deaths);
                }
                continue;
            }
            if let PlayIn::Attack { entity_id } = pkt {
                if !self.players[i].dead {
                    let mut level = RegionLevel {
                        cells: &mut *self.cells,
                        blocks: &mut *self.blocks,
                        env: &env.blocks,
                        out: &mut out,
                        bodies: &bodies,
                        actor: None,
                    };
                    crate::combat::handle_attack(self.entities, &mut level, &mut self.players, i, entity_id, &mut self.out.spawns, &mut self.out.deaths);
                }
                continue;
            }
            // A glass bottle by a cloud of the dragon's breath fills with it.
            if let PlayIn::UseItem { hand, sequence, .. } = pkt {
                let off = hand == kiln_proto::packets::serverbound::Hand::Off;
                if crate::dragon_fight::bottle_breath(self.entities, self.players[i], off, &mut self.out.spawns, &env.blocks) {
                    let p = &mut *self.players[i];
                    p.ack_block_changes = p.ack_block_changes.max(sequence);
                    continue;
                }
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
            if let PlayIn::PaddleBoat { left, right } = pkt {
                entities::paddle_boat(self.entities, &self.players, i, left, right);
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
                let open = entities::interact_mob(self.entities, &mut level, &mut self.players, i, entity_id, off, &mut self.out.spawns, &mut self.out.deaths);
                if open {
                    crate::carts::open(self.entities, &mut level, self.players[i], entity_id, &mut self.out.spawns);
                }
                crate::trading::open_if_requested(self.entities, self.players[i], entity_id, &env.rules, &mut self.out.spawns);
                continue;
            }
            // A fence with leads tied to the player: they move to its knot.
            if let PlayIn::UseItemOn { hand, pos, face, cursor, sequence, .. } = pkt
                && crate::leash::intercepts(self.players[i], &*self.cells, &env.blocks, hand, pos, face, cursor)
            {
                let mut level =
                    RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: Some(conn) };
                if crate::leash::bind(self.entities, &mut level, &mut self.players, i, pos, &mut self.out.spawns, &mut self.out.deaths) {
                    let p = &mut *self.players[i];
                    let step = crate::blocks::direction(face).map_or([0; 3], |d| d.step());
                    p.resend_block(&mut level, pos);
                    p.resend_block(&mut level, [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]]);
                    p.ack_block_changes = p.ack_block_changes.max(sequence);
                    continue;
                }
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
            // A minecart menu works on the player's copy of its slots, brought up to date
            // first and handed back after.
            let cart = crate::carts::pull(self.entities, self.players[i]);
            local_packet(self.players[i], &mut world, env, pkt, &mut fx);
            crate::carts::push(self.entities, self.players[i], cart);
            if !self.players[i].merchant_events.is_empty() {
                let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
                crate::trading::apply_events(self.entities, &mut level, &mut self.players, i, &mut self.out.spawns, &mut self.out.deaths);
            }
        }
        if let Some(h) = self.plugins.as_mut() {
            crate::plugins::after_packets(h, self.cells, env);
        }
        let dt = Instant::now();
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        crate::diag::lap("ap.finish", dt);
    }

    /// `stopOpen` of the chest minecarts and chest boats whose menus `check_menus` closed: the
    /// `container_close` game events.
    fn post_cart_closes(&mut self, env: &Env) {
        if self.players.iter().all(|p| p.containers.cart_closed.is_none()) {
            return;
        }
        let bodies = Vec::new();
        let mut out = BlockOut::default();
        let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
        for p in self.players.iter_mut() {
            if let Some(at) = p.containers.cart_closed.take() {
                p.post_container_close(&mut level, at);
            }
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
    }

    /// `LivingEntity.checkAutoSpinAttack` for the players whose riptide spin counted down this
    /// tick: the first living entity their box meets is hit (`Player.attack` with the spin's
    /// damage and its trident) and the spin ends there; a finished spin forgets its damage.
    fn spin_attacks(&mut self, env: &Env) {
        for i in 0..self.players.len() {
            if !std::mem::take(&mut self.players[i].spin_check) {
                continue;
            }
            let touch = crate::combat::spin_touch(&self.players, i, self.entities);
            if !self.players[i].dead && touch.living.is_none() && !touch.any && self.players[i].horizontal_collision {
                self.players[i].end_spin_on_collision();
            }
            if !self.players[i].dead && let Some(target) = touch.living {
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
                    crate::combat::spin_attack(self.entities, &mut level, &mut self.players, i, target, &mut self.out.spawns, &mut self.out.deaths);
                }
                self.players[i].stop_spin_on_hit();
                blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
            }
            self.players[i].spin_finished();
        }
    }

    /// `KineticWeapon.damageEntities` for the players using a charging weapon this tick (it is
    /// part of the item's use tick): the entities in the way of the charge are hurt, pushed or
    /// thrown off their mounts by how fast the player moves relative to them.
    fn kinetic_attacks(&mut self, env: &Env) {
        for i in 0..self.players.len() {
            let Some(ticks) = self.players[i].kinetic_ticks.take() else { continue };
            if self.players[i].dead {
                continue;
            }
            let bodies = Vec::new();
            let mut out = BlockOut::default();
            let mut level = RegionLevel { cells: &mut *self.cells, blocks: &mut *self.blocks, env: &env.blocks, out: &mut out, bodies: &bodies, actor: None };
            crate::spear::kinetic_attack(self.entities, &mut level, &mut self.players, i, ticks, &mut self.out.spawns, &mut self.out.deaths);
            blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        }
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
        let dt = Instant::now();
        let mut jobs: Vec<Vec<(usize, PlayIn)>> = (0..self.players.len()).map(|_| Vec::new()).collect();
        let mut last: Option<(ConnId, Option<usize>)> = None;
        for (seq, (conn, pkt)) in run.into_iter().enumerate() {
            let at = match last {
                Some((c, at)) if c == conn => at,
                _ => self.index_of(conn),
            };
            last = Some((conn, at));
            if let Some(i) = at {
                jobs[i].push((seq, pkt));
            }
        }
        let mut items: Vec<(&mut &mut Player, Vec<(usize, PlayIn)>)> =
            self.players.iter_mut().zip(jobs).filter(|(_, j)| !j.is_empty()).collect();
        let dt = crate::diag::lap("ap.jobs", dt);
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
        let dt = crate::diag::lap("ap.window", dt);
        let mut left: Vec<_> = left.into_iter().flatten().collect();
        crate::diag::lap("ap.left", dt);
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
        let mut wlap = kiln_sched::window_ns();
        let mut win = [Duration::ZERO; SUB_PHASES.len()];
        let mut mark = |times: &mut [Duration; SUB_PHASES.len()], i: usize| {
            let now = Instant::now();
            times[i] += now - lap;
            lap = now;
            let w = kiln_sched::window_ns();
            win[i] += Duration::from_nanos(w - wlap);
            wlap = w;
        };
        // Menu changes first, like vanilla's container broadcast at the start of a player tick,
        // then `stillValid`: a menu whose block went away or is out of reach closes.
        // A player without a block menu open touches only itself here, so those players
        // broadcast in windows first; the others then run serially in connection order, and
        // the drops land in that order either way.
        {
            let rules = &*env.blocks.menus;
            let own = ctx.map_mut_with(MENU_WINDOW, &mut self.players, |_, p| {
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
                    let cart = crate::carts::pull(self.entities, p);
                    crate::container::open::menu_op(p, &mut level, &mut self.out.spawns, |menu, _, env| menu.broadcast_changes(env));
                    crate::carts::push(self.entities, p, cart);
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
        let ticked = ctx.map_mut_with(PLAYER_TICK_WINDOW, &mut self.players, |_, p| player_tick(p, cells, env));
        for t in ticked {
            self.out.spawns.extend(t.spawns);
            self.out.deaths.extend(t.deaths);
            self.out.portals.extend(t.portals);
        }
        self.spin_attacks(env);
        self.kinetic_attacks(env);
        mark(&mut self.out.times, 1);
        // Which chunks each player lacks is its own business (a window); sending them needs
        // the chunks' packet caches, so that part runs in connection order here.
        let missing = ctx.map_mut_with(CHUNK_VIEW_WINDOW, &mut self.players, |_, p| if p.disconnected { Vec::new() } else { chunk_view(p) });
        for (p, missing) in self.players.iter_mut().zip(missing).filter(|(_, m)| !m.is_empty()) {
            send_chunks(p, missing, &mut *self.cells, env, &mut self.out.wanted);
        }
        // The same tick everywhere, so when chunks unload does not depend on the regions.
        if env.game_time % 20 == 0 {
            self.find_unloads();
        }
        // Encoded chunk packets are shared by the players a chunk goes to around the same time;
        // a minute on they only take memory (a later viewer gets the chunk encoded again, the
        // same bytes).
        if env.game_time % 1200 == 600 {
            for (cell_pos, cell) in self.cells.iter_mut() {
                for (_, chunk) in cell.chunks_mut(cell_pos) {
                    chunk.drop_packet_cache();
                }
            }
        }
        mark(&mut self.out.times, 2);
        // The chunks that tick, for the block and entity phases (no player changes chunk
        // between them).
        let dt = std::time::Instant::now();
        let ticking = ticking_chunks(&self.players, env);
        crate::diag::lap("b.ticking", dt);
        self.tick_blocks(env, &ticking);
        mark(&mut self.out.times, 3);
        let spawned_before = self.out.times[9];
        self.tick_entities(env, ctx, &ticking);
        let dt = std::time::Instant::now();
        crate::trading::check_menus(self.entities, &mut self.players, &env.rules, &mut self.out.spawns);
        crate::carts::check_menus(self.entities, &mut self.players, &env.rules, &mut self.out.spawns);
        self.post_cart_closes(env);
        entities::pickups(self.entities, &mut self.players);
        crate::xp::pick_up_orbs(self.entities, &mut self.players);
        crate::diag::lap("ent.after", dt);
        mark(&mut self.out.times, 4);
        // The spawner's share of the entity phase is its own sub-phase.
        self.out.times[4] = self.out.times[4].saturating_sub(self.out.times[9] - spawned_before);
        let movers = crate::players::update_visibility(&mut self.players, ctx);
        mark(&mut self.out.times, 5);
        let dt = std::time::Instant::now();
        crate::players::broadcast_movement(&mut self.players, ctx);
        let dt = crate::diag::lap("mv.broadcast", dt);
        for p in self.players.iter_mut() {
            p.decay_velocity();
        }
        let dt = crate::diag::lap("mv.decay", dt);
        entities::track(self.entities, &mut self.players, &movers, ctx);
        crate::diag::lap("mv.track", dt);
        mark(&mut self.out.times, 6);
        self.send_light_updates();
        self.out.portal_candidates = crate::portal::portal_candidates(self.entities, self.cells, env.min_y);
        mark(&mut self.out.times, 7);
        for p in self.players.iter_mut() {
            self.out.saved_entities.append(&mut p.released_shoulders);
        }
        ctx.map_mut_with(FLUSH_WINDOW, &mut self.players, |_, p| p.flush());
        mark(&mut self.out.times, 8);
        self.out.win = win;
    }

    /// The block phases: players' digging, pressure plates under bodies, then scheduled
    /// ticks, random ticks, block events and moving pistons in chunks near players.
    fn tick_blocks(&mut self, env: &Env, ticking: &Ticking) {
        let dt = std::time::Instant::now();
        let bodies = blocks::entity_boxes(self.players.iter().map(|p| &**p), self.entities);
        let dt = crate::diag::lap("b.bodies", dt);
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
            // What the players' own ticks asked for (melted powder snow, trampled farmland).
            for p in self.players.iter_mut().filter(|p| !p.block_edits.is_empty()) {
                for edit in std::mem::take(&mut p.block_edits) {
                    match edit {
                        crate::fall::BlockEdit::Destroy(pos) => {
                            let pos = BlockPos::new(pos.x, pos.y, pos.z);
                            kiln_blocks::destroy_block(&mut level, pos, false, 512);
                        }
                        crate::fall::BlockEdit::Dirt(pos) => {
                            let pos = BlockPos::new(pos.x, pos.y, pos.z);
                            // `FarmBlock.turnToDirt`.
                            if kiln_entity::blocks::kind(level.block(pos)) == kiln_entity::blocks::Kind::Farmland {
                                kiln_blocks::set_block(&mut level, pos, kiln_data::blocks::default_state::DIRT, 3);
                            }
                        }
                    }
                }
            }
            // `Player.tick`'s sleeping part and the insomnia statistic.
            for p in self.players.iter_mut().filter(|p| !p.disconnected) {
                crate::sleep::tick_player(p, &mut level);
            }
            let dt = crate::diag::lap("b.dig_sleep", dt);
            // A frozen game (`/tick freeze`) ticks no blocks.
            if !env.frozen {
                blocks::press_plates(&mut level);
                let dt = crate::diag::lap("b.plates", dt);
                crate::sculk::players_step_on(&mut level, &self.players);
                let dt = crate::diag::lap("b.sculk_step", dt);
                blocks::tick_blocks(&mut level, &ticking);
                crate::diag::lap("b.tick_blocks", dt);
                for pos in std::mem::take(&mut level.out.rechecks) {
                    crate::container::open::recheck_openers(&mut level, &self.players, pos);
                }
                blocks::tick_pistons(&mut level, &ticking);
                crate::sculk::requests(&mut level, &mut self.players, self.entities, &mut self.out.spawns);
            }
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        if let Some(h) = self.plugins.as_mut() {
            crate::plugins::after_packets(h, self.cells, env);
        }
    }

    /// The entity phase: the region's entities tick against its blocks; what they change
    /// goes out like block work.
    fn tick_entities(&mut self, env: &Env, ctx: &Ctx<'_>, ticking_now: &Ticking) {
        // `TickRateManager.isEntityFrozen`: nothing but players ticks while frozen.
        if env.frozen {
            return;
        }
        if !self.blocks.sculk.wardens.is_empty() {
            let list = &self.entities.list;
            self.blocks.sculk.retain_wardens(|id| list.binary_search_by_key(&id, |e| e.id).is_ok_and(|i| !list[i].removed));
        }
        if !self.blocks.sculk.allays.is_empty() {
            let list = &self.entities.list;
            self.blocks.sculk.retain_allays(|id| list.binary_search_by_key(&id, |e| e.id).is_ok_and(|i| !list[i].removed));
        }
        if self.entities.list.is_empty() && self.blocks.hearts.is_empty() && (self.players.is_empty() || (env.blocks.spawn_table.is_none() && self.blocks.spawners.is_empty())) {
            self.tick_block_entities(env, ticking_now);
            return;
        }
        // `TicketType.DRAGON`: the fight's arena ticks while its boss bar has players.
        let with_arena;
        let ticking = match env.blocks.dragon_fight.as_ref().filter(|f| f.active) {
            Some(f) => {
                with_arena = ticking_now.with_arena(f.arena_center, f.arena_radius);
                &with_arena
            }
            None => ticking_now,
        };
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
            let spawning = Instant::now();
            crate::spawner::tick(&mut level, self.entities, &self.players, &ticking, &mut self.out.spawns, ctx);
            self.out.times[9] += spawning.elapsed();
            entities::tick(self.entities, &mut level, &ticking, &mut self.players, &mut self.out.spawns, &mut self.out.deaths, any_player, ctx);
            crate::sculk::requests(&mut level, &mut self.players, self.entities, &mut self.out.spawns);
        }
        blocks::finish(self.cells, out, &mut self.players, &mut self.out.spawns, &env.blocks);
        self.tick_block_entities(env, ticking_now);
    }

    /// `Level.tickBlockEntities`: hoppers and furnaces in ticking chunks. Hoppers take item
    /// entities; their viewers see the new counts.
    fn tick_block_entities(&mut self, env: &Env, ticking: &Ticking) {
        if self.blocks.containers.len() == 0 && self.blocks.sculk.len() == 0 {
            return;
        }
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
            crate::bell::requests(&mut level, items.entities_mut());
            crate::bell::tick(&mut level, items.entities_mut(), &ticking);
            crate::sculk::tick_block_entities(&mut level, &ticking);
            crate::sculk::requests(&mut level, &mut self.players, items.entities(), &mut self.out.spawns);
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

/// The per-player windows of a crowd, with what one player costs (measured with the vanilla
/// datapack; a hint saves the timed prefix, wp40): menu broadcast, player tick, chunk view,
/// egress.
const MENU_WINDOW: Window = Window::new().item_ns(700);
const PLAYER_TICK_WINDOW: Window = Window::new().item_ns(2_000);
const CHUNK_VIEW_WINDOW: Window = Window::new().item_ns(150);
const FLUSH_WINDOW: Window = Window::new().item_ns(400);
/// A player's packets of one run (mostly a move and a tick end).
const PACKET_WINDOW: Window = Window::new().item_ns(600);

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
    // What bad omen asks of the level (only looked up while the player has it).
    if p.has_effect("minecraft:bad_omen") {
        let at = [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32];
        p.omen_village = crate::poi::sections_to_village(cells, at) <= 1;
        let bp = kiln_entity::math::BlockPos::new(at[0], at[1], at[2]);
        p.omen_raid_full = crate::raid::raid_at_view(&env.blocks.raids, bp).is_some_and(|r| r.omen_level >= 5);
    }
    p.base_tick(&block, env.min_y, &env.border, &mut ctx);
    p.tick_peaceful_regeneration(env.natural_regen, ctx.rules.difficulty);
    p.tick_fall_resets(&block);
    p.tick_glide();
    p.tick_spin();
    {
        // `handleShoulderEntities`, at the end of `Player.aiStep`.
        let in_water = p.fluids(&|pos: kiln_entity::math::BlockPos| block(pos)).in_water;
        let feet = kiln_entity::math::BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
        let in_powder_snow = block(feet) == kiln_data::blocks::default_state::POWDER_SNOW;
        p.handle_shoulder_entities(env.game_time, in_water, in_powder_snow);
    }
    // `Entity.handlePortal` (in `baseTick`).
    if let Some(travel) = p.handle_portal(env) {
        t.portals.push(travel);
    }
    p.tick_using(&block, &mut ctx);
    p.tick_cooldowns();
    p.tick_combat();
    // The server's body moves on its own (gravity, drag, a ladder's grip) and the blocks it
    // passes through take effect, then the connection puts the position back (`doTick`).
    let snap = p.pos;
    p.phantom_travel(cells, env.game_time, env.min_y, env.dim == crate::NETHER_ID);
    let (_, h, _) = p.dimensions();
    let in_rain = crate::weather::in_rain(cells, &env.blocks, p.pos, p.pos[1] + h as f64);
    p.block_effects(&block, env.dim, in_rain, &mut ctx);
    p.tick_freezing(&block, &mut ctx);
    // `Player.tick`'s last step.
    p.update_pose(cells, env.game_time, env.min_y);
    p.pos = snap;
    if let Some(travel) = p.pending_travel.take() {
        t.portals.push(travel);
    }
    p.tick_food(env.natural_regen, &mut ctx);
    p.tick_stats();
    // `ServerPlayer.tick`.
    p.warden_tracker.tick();
    let probe = crate::advancements::triggers::CellProbe::new(cells, &env.blocks);
    p.tick_triggers(&probe);
    // `onInsideBlock` (Kiln checks the block at the feet).
    let feet = kiln_entity::math::BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
    let inside = block(feet);
    if inside != 0 && !p.dead {
        p.entered_block(inside);
    }
    p.tick_honey_slide(&block, env.game_time);
    p.sync_health();
    p.sync_experience();
    t
}

/// Chunks that tick: those within the simulation distance of the region's players and the
/// level's force-loaded chunks.
fn ticking_chunks(players: &[&mut Player], env: &Env) -> Ticking {
    let mut t = Ticking::around(players.iter().map(|p| p.center), env.blocks.simulation_distance);
    for &c in env.forced.iter() {
        t.add_chunk(c);
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

/// `ServerboundPlayerActionPacket.Action.STAB`.
const STAB: i32 = 8;

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
        PlayIn::Move { pos, rot, on_ground, horizontal_collision } => {
            let from = p.pos;
            // `player.onGround()` as the server holds it (its own body's, not the client's).
            let was_on_ground = p.on_ground;
            let y0 = p.pos[1];
            if handle_move(p, cells, env, pos, rot, on_ground) {
                // `setOnGroundWithMovement`: the client's own report of running into a wall.
                p.horizontal_collision = horizontal_collision;
                let feet = p.pos.map(|c| c.floor() as i32);
                let d = [p.pos[0] - from[0], p.pos[1] - from[1], p.pos[2] - from[2]];
                // `handlePlayerKnownMovement`.
                p.known_movement = d;
                p.moved_this_tick = true;
                // `Block.updateEntityMovementAfterFallOn`: landing stops the fall.
                if on_ground {
                    p.vel[1] = 0.0;
                }
                // `jumpFromGround` (when the server holds the player on the ground; a body that
                // bounced off slime or a bed is not), then the fall, then `checkMovementStatistics`.
                if was_on_ground && !on_ground && d[1] > 0.0 {
                    p.award_stat(*crate::player_stats::stat::JUMP, 1);
                }
                p.exhaust_for_jump(d, was_on_ground);
                if was_on_ground && !on_ground && d[1] > 0.0 {
                    p.server_jump(from, cells, env.game_time, env.min_y);
                }
                p.server_packet_move(from, d, was_on_ground, cells, env.game_time, env.min_y, env.dim == crate::NETHER_ID);
                let mut ctx = damage_ctx(env, spawns, deaths);
                let blocks = |pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z);
                p.after_move_fall(d, on_ground, p.pos[1] - y0 > 0.0, &blocks, &mut ctx);
                let climbing = cells.get_block(feet[0], feet[1], feet[2]).is_some_and(crate::player_stats::climbable);
                let (in_water, eyes_in_water) = (p.was_touching_water, p.was_eye_in_water);
                p.movement_stats(d, in_water, eyes_in_water, climbing);
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
        PlayIn::PlayerCommand { action } if action == crate::glide::START_FALL_FLYING => {
            let fluids = p.fluids(&|pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0));
            p.try_start_fall_flying(fluids.in_water || fluids.in_lava);
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
                RELEASE_USE_ITEM => {
                    let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                    crate::ranged::release_using(p, &mut level, fx.spawns);
                }
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
        PlayIn::UseItem { hand, sequence, yaw, pitch } => {
            // `handleUseItem`: the rotation the client used applies first.
            if yaw.is_finite() && pitch.is_finite() {
                p.rot = crate::movement::normalize_rotation([yaw, pitch]);
            }
            let off = hand == kiln_proto::packets::serverbound::Hand::Off;
            let held = p.in_hand(off).clone();
            let name = if held.is_empty() { "minecraft:air" } else { held.item_name() };
            // `ServerPlayerGameMode.useItem`: nothing for spectators or items cooling down.
            if p.game_mode == 3 || p.dead || held.is_empty() || p.on_cooldown(&held) {
            } else if crate::buckets::is_bucket(name) {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::buckets::use_bucket(p, &mut level, off, fx.spawns);
            } else if name == crate::firework::ITEM {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::firework::use_item(p, &mut level, off, fx.spawns);
            } else if matches!(name, "minecraft:writable_book" | "minecraft:written_book") {
                p.use_book(off);
            } else if name == crate::end_eye::ITEM {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::end_eye::use_item(p, &mut level, off, fx.spawns);
            } else if crate::boats::is_boat_item(name) {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::boats::use_item(p, &mut level, off, fx.spawns);
            } else if crate::ranged::handles(name) {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::ranged::use_item(p, &mut level, off, fx.spawns);
            } else if name == "minecraft:crossbow" {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::crossbow::use_item(p, &mut level, off, fx.spawns);
            } else if name == "minecraft:trident" {
                let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
                crate::trident::use_item(p, &mut level, off);
            } else if crate::ranged::use_held(p, off, &held) {
            } else if held.get(kiln_item::keys::CONSUMABLE).is_none() && p.use_equippable(off, &env.blocks.menus, fx.spawns) {
                // `Item.use` of armor and the like: swapped with what is worn.
            } else {
                let cells = &*world.cells;
                let block = |pos: kiln_entity::math::BlockPos| cells.get_block(pos.x, pos.y, pos.z).unwrap_or(0);
                let mut ctx = damage_ctx(env, fx.spawns, fx.deaths);
                p.use_item(off, &block, &mut ctx);
            }
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::UseItemOn { hand, pos, face, cursor, sequence, .. } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            let may_interact = env.border.contains(pos[0] as f64, pos[2] as f64);
            use_item_on(p, &mut level, hand, pos, face, cursor, may_interact, fx.spawns);
            p.ack_block_changes = p.ack_block_changes.max(sequence);
        }
        PlayIn::EditBook { slot, pages, title } => p.edit_book(slot, &pages, title.as_deref()),
        PlayIn::PickItemFromBlock { pos, .. } => {
            let state = world.cells.get_block(pos[0], pos[1], pos[2]);
            p.pick_item_from_block(BlockPos::new(pos[0], pos[1], pos[2]), state);
        }
        PlayIn::SignUpdate { pos, lines, front } => {
            let mut level = world.level(env, fx.blocks, fx.bodies, p.conn);
            crate::signs::update_text(p, &mut level, BlockPos::new(pos[0], pos[1], pos[2]), &lines, front);
        }
        // `handlePunch`: the swing resets the attack strength.
        PlayIn::Punch => {
            p.swing_main_hand();
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
    let have_something = !p.inv.selected_item().is_empty() || !p.inv.equipped(EquipmentSlot::OffHand).is_empty();
    let bp = BlockPos::new(pos[0], pos[1], pos[2]);
    let actor = Actor { yaw: p.rot[0], may_build: p.game_mode <= 1, creative: p.game_mode == 1 };
    // `SignBlock.useItemOn`, then `useWithoutItem` for the main hand: dyes and honeycomb, the
    // editor, the refusal of waxed signs.
    if !(p.sneaking && have_something) && crate::signs::use_on(p, level, bp, !main_hand) {
        return;
    }
    let held = if main_hand { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    // `BlockState.useItemOn` of blocks that react to the item itself (either hand).
    // (`ServerPlayerGameMode.useItemOn` does not ask whether the player may build: pots, campfires and
    // composters work in adventure mode.)
    if !(p.sneaking && have_something) && !held.is_empty() {
        let used = held.clone();
        if let Some(true) = crate::tools::block_use_item_on(p, level, bp, dir, cursor, !main_hand, spawns) {
            let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
            p.used_on_block("minecraft:item_used_on_block", pos, level.block(bp), &used, &probe);
            return;
        }
    }
    let held = if main_hand { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    let item_name = if held.is_empty() {
        None
    } else {
        kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(held.item() as usize).copied())
    };
    if !(p.sneaking && have_something) && main_hand && !interact::passes_to_item(level.block(bp), item_name, dir) {
        if let Some(consumed) = crate::container::open::use_block(p, level, bp, spawns) {
            if consumed {
                return;
            }
        } else if crate::tools::block_use_without_item(p, level, bp, dir, cursor, spawns) || interact::use_without_item(level, bp, &actor) {
            return;
        }
    }
    // `Item.useOn` of tools (hoes, shovels, axes, shears, honeycomb, bone meal, fire charges,
    // flint and steel on campfires and candles).
    if item_name.is_some_and(crate::boats::is_minecart_item) && actor.may_build && crate::boats::use_minecart_on(p, level, bp, !main_hand, spawns) {
        return;
    }
    if item_name == Some(crate::firework::ITEM) && actor.may_build && crate::firework::use_on(p, level, bp, dir, cursor, !main_hand, spawns) {
        return;
    }
    if item_name == Some(crate::end_eye::ITEM) && actor.may_build && crate::end_eye::use_on(p, level, bp, !main_hand) {
        return;
    }
    if actor.may_build && crate::tools::item_use_on(p, level, bp, dir, !main_hand, spawns) {
        return;
    }
    if item_name == Some("minecraft:flint_and_steel") && actor.may_build {
        light_fire(p, level, main_hand, pos, dir);
        return;
    }
    // `SpawnEggItem.useOn` on a mob spawner: its next spawn data's entity becomes the egg's.
    if let Some(entity) = item_name.and_then(|n| n.strip_suffix("_spawn_egg"))
        && p.game_mode != 3
        && let Some(worked) = crate::mob_spawner::use_egg(level, bp, entity)
    {
        if !worked {
            // `advMode.notEnabled.spawner`.
            p.send(kiln_proto::packets::system_chat(crate::container::translatable("advMode.notEnabled.spawner"), false));
            return;
        }
        let egg = if main_hand { p.inv.selected_item().item() } else { p.inv.equipped(EquipmentSlot::OffHand).item() };
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, egg), 1);
        if p.game_mode != 1 {
            let slot = kiln_inventory::inventory::equipment_index(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, p.inv.selected);
            kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
        }
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
        let finalize = crate::mobs::Finalize::command(
            crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, 1.0),
            crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (at.x as u64) << 32 ^ at.z as u64 ^ (at.y as u64) << 16),
            false,
            env.mobs.difficulty == 0 || !env.mobs.spawn_monsters,
        );
        spawns.push(crate::mobs::spawn(kind, [at.x as f64 + 0.5, at.y as f64, at.z as f64 + 0.5], Some(yaw), Some(finalize)));
        let egg = if main_hand { p.inv.selected_item().item() } else { p.inv.equipped(EquipmentSlot::OffHand).item() };
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, egg), 1);
        if p.game_mode != 1 {
            let slot = kiln_inventory::inventory::equipment_index(if main_hand { EquipmentSlot::MainHand } else { EquipmentSlot::OffHand }, p.inv.selected);
            kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
        }
        return;
    }
    // `HangingEntityItem.useOn`: item frames and paintings.
    if item_name.is_some_and(crate::frames::is_hanging_item) && crate::frames::use_on(p, level, bp, dir, !main_hand, spawns) {
        return;
    }
    // `EndCrystalItem.useOn`: on obsidian or bedrock with air above and no entity in the two
    // blocks there; the fight looks for its respawn crystals.
    if item_name == Some("minecraft:end_crystal") {
        let s = level.block(bp);
        if !kiln_blocks::state::is(s, kiln_data::blocks::default_state::OBSIDIAN) && !kiln_blocks::state::is(s, kiln_data::blocks::default_state::BEDROCK) {
            return;
        }
        let above = bp.relative(kiln_blocks::Direction::Up);
        if !kiln_data::blocks_types::is_air(level.block(above)) {
            return;
        }
        let (x, y, z) = (above.x as f64, above.y as f64, above.z as f64);
        if level.bodies.iter().any(|b| b.intersects([x, y, z], [x + 1.0, y + 2.0, z + 1.0])) {
            return;
        }
        let env = level.env;
        let seed = crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (above.x as u64) << 32 ^ above.z as u64 ^ (above.y as u64) << 16);
        let crystal = kiln_entity::ext_entity::end_crystal::new(0, kiln_entity::math::Vec3::new(x + 0.5, y, z + 0.5), false, seed);
        if let Some(kind) = kiln_data::entities::by_name("minecraft:end_crystal") {
            spawns.push(Spawn { kind, pos: [x + 0.5, y, z + 0.5], vel: [0.0; 3], body: crate::entities::Body::Ready(Box::new(crystal)) });
        }
        if let Some(f) = &env.dragon_fight {
            f.send(crate::dragon_fight::FightMsg::TryRespawn);
        }
        let item = if main_hand { p.inv.selected_item().item() } else { p.inv.equipped(EquipmentSlot::OffHand).item() };
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
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
    // `SignBlock.setPlacedBy`: the placer edits the new sign.
    crate::signs::placed_by(p, level, placed_at);
    crate::golems::try_spawn_golem(p, level, placed_at, spawns);
    // `WitherSkullBlock.setPlacedBy`.
    crate::wither::check_spawn(level, placed_at, spawns);
    // `ItemStack.useOn`: a successful item interaction counts as a use; `BlockItem.place`
    // and `ServerPlayerGameMode.useItemOn` fire their triggers.
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, placed_from.item()), 1);
    let placed_state = level.block(placed_at);
    // `CarvedPumpkinBlock.onPlace`: a pumpkin may finish a golem.
    if matches!(kiln_data::blocks_types::block_of(placed_state).name, "minecraft:carved_pumpkin" | "minecraft:jack_o_lantern") {
        crate::golem::try_spawn(level, placed_at, p, spawns);
    }
    let placed_state = level.block(placed_at);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    let at = [placed_at.x, placed_at.y, placed_at.z];
    p.used_on_block("minecraft:placed_block", at, placed_state, &placed_from, &probe);
    p.used_on_block("minecraft:item_used_on_block", pos, level.block(bp), &placed_from, &probe);
    // `SolidBucketItem.useOn`: the powder snow bucket leaves an empty bucket.
    if placed_from.item_name() == "minecraft:powder_snow_bucket" {
        if !p.infinite_materials() {
            p.set_in_hand(!main_hand, kiln_item::ItemStack::of("minecraft:bucket", 1).unwrap_or_else(kiln_item::ItemStack::empty));
        }
        return;
    }
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
    let h = p.dimensions().1 as f64;
    let me = EntityBox {
        min: [p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3],
        max: [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3],
        living: true,
        blocks_building: p.game_mode != 3,
        conn: Some(p.conn),
        prevents_rest: false,
        player_source: None,
        hanging: None,
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
    // `ServerGamePacketListenerImpl.shouldCheckPlayerMovement`.
    if env.movement_check && (!p.fall_flying || env.elytra_movement_check) && movement::too_fast(p.first_good, to, 0.0, p.move_packets, p.fall_flying) {
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
    let skipped = p.game_mode == 3 && !env.spectators_generate_chunks;
    for c in missing {
        match cells.chunk_mut(c) {
            Some(chunk) if batch.len() < budget => batch.push((c, chunk.packet(c.x, c.z, env.biome_count))),
            Some(_) => {}
            None if asked < 2 * budget && !skipped => {
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
