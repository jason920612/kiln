//! Travel between levels: nether portals, end portals and end gateways (vanilla
//! `PortalProcessor`, `Entity.handlePortal`, `NetherPortalBlock.getPortalDestination`,
//! `PortalForcer`, `EndPortalBlock`, `TheEndGatewayBlockEntity`), and the dimension change
//! itself (`ServerPlayer.teleport` to another level).
//!
//! Standing in a portal is noticed in the region tick (`entityInside`); when the portal time
//! runs out the region hands a [`Travel`] to the serial phase after L, where the destination
//! level's chunks can be loaded and changed (the exit portal is found or built there).
//!
//! Kiln has no point-of-interest storage: the portal search scans the chunks of the search
//! square that exist (loaded, or saved), which finds the same portals vanilla's POI records
//! would (portals only exist in chunks that were generated and saved).

use crate::{DIMENSIONS, DimId, END_ID, NETHER_ID, OVERWORLD_ID, Player, Sim, player_chunk};
use kiln_blocks::BlockPos;
use kiln_blocks::behaviour::portal as shape;
use kiln_blocks::pos::{Axis, Direction};
use kiln_data::blocks::default_state as block;
use kiln_link::ConnId;
use kiln_proto::packets;
use kiln_world::{Blocks, ChunkPos};
use tracing::{info, warn};

/// `Player.getDimensionChangingDelay`: the portal cooldown after a teleport.
const PLAYER_PORTAL_COOLDOWN: i32 = 10;
/// `LevelEvent.SOUND_PORTAL_TRAVEL`.
const SOUND_PORTAL_TRAVEL: i32 = 1032;
/// `ClientboundGameEventPacket.WIN_GAME`.
const WIN_GAME: u8 = 4;
/// `ServerLevel.END_SPAWN_POINT`.
pub(crate) const END_SPAWN_POINT: [i32; 3] = [100, 50, 0];
/// `TheEndGatewayBlockEntity.COOLDOWN_TIME`.
const GATEWAY_COOLDOWN: i32 = 40;
/// Half the default world border (59999968): the horizontal limit of portal destinations.
const BORDER: f64 = 29_999_984.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PortalKind {
    Nether,
    End,
    Gateway,
}

/// `PortalProcessor`.
#[derive(Clone, Debug)]
pub(crate) struct PortalProcess {
    pub kind: PortalKind,
    pub entry: [i32; 3],
    /// `portalTime`.
    pub time: i32,
    /// `insidePortalThisTick`.
    pub inside: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TravelKind {
    Portal(PortalKind),
    /// The End's exit portal before the credits were seen (`showEndCredits`).
    Credits,
}

/// A player to move to another level (or across the End through a gateway).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Travel {
    pub conn: ConnId,
    pub kind: TravelKind,
    /// The portal block the player stood in.
    pub entry: [i32; 3],
}

/// The portal game rules.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PortalRules {
    /// `players_nether_portal_default_delay`.
    pub survival_delay: i32,
    /// `players_nether_portal_creative_delay`.
    pub creative_delay: i32,
}

impl Sim {
    pub(crate) fn portal_rules(&self) -> PortalRules {
        PortalRules {
            survival_delay: self.rule_int("minecraft:players_nether_portal_default_delay"),
            creative_delay: self.rule_int("minecraft:players_nether_portal_creative_delay"),
        }
    }
}

impl Player {
    /// `Entity.canUsePortal(false)`: alive, and not a passenger (what it rides goes, with it).
    fn can_use_portal(&self) -> bool {
        !self.dead && self.health > 0.0 && !self.disconnected && self.vehicle.is_none()
    }

    /// `entityInside` of the portal blocks, for a block the player's box touches.
    pub(crate) fn portal_inside(&mut self, state: u16, pos: [i32; 3], dim: DimId) {
        let kind = match kiln_data::blocks_types::block_of(state).first {
            s if s == block::NETHER_PORTAL => PortalKind::Nether,
            s if s == block::END_PORTAL => PortalKind::End,
            s if s == block::END_GATEWAY => PortalKind::Gateway,
            _ => return,
        };
        if !self.can_use_portal() {
            return;
        }
        if kind == PortalKind::End && dim == END_ID && !self.seen_credits {
            if !self.won_game && self.pending_travel.is_none() {
                self.pending_travel = Some(Travel { conn: self.conn, kind: TravelKind::Credits, entry: pos });
            }
            return;
        }
        self.set_inside_portal(kind, pos);
    }

    /// `Entity.setAsInsidePortal`.
    fn set_inside_portal(&mut self, kind: PortalKind, pos: [i32; 3]) {
        if self.portal_cooldown > 0 {
            self.portal_cooldown = PLAYER_PORTAL_COOLDOWN;
            return;
        }
        match &mut self.portal {
            Some(p) if p.kind == kind => {
                if !p.inside {
                    p.entry = pos;
                    p.inside = true;
                }
            }
            // A new `PortalProcessor` counts as inside this tick.
            _ => self.portal = Some(PortalProcess { kind, entry: pos, time: 0, inside: true }),
        }
    }

    /// `NetherPortalBlock.getPortalTransitionTime` (end portals and gateways take none).
    fn transition_time(&self, kind: PortalKind, rules: &crate::portal::PortalRules) -> i32 {
        match kind {
            // `Abilities.invulnerable`: creative and spectator.
            PortalKind::Nether if matches!(self.game_mode, 1 | 3) => rules.creative_delay.max(0),
            PortalKind::Nether => rules.survival_delay.max(0),
            _ => 0,
        }
    }

    /// `Entity.handlePortal`: the cooldown runs down; standing in a portal long enough starts a
    /// teleport (returned for the serial phase), leaving one lets the time decay.
    pub(crate) fn handle_portal(&mut self, env: &crate::region::Env) -> Option<Travel> {
        if self.portal_cooldown > 0 {
            self.portal_cooldown -= 1;
        }
        let can_use = self.can_use_portal();
        let proc = self.portal.as_ref()?;
        let kind = proc.kind;
        let delay = self.transition_time(kind, &env.portal);
        let proc = self.portal.as_mut().unwrap();
        if proc.inside {
            proc.inside = false;
            if can_use {
                let t = proc.time;
                proc.time += 1;
                if t >= delay {
                    let entry = proc.entry;
                    self.portal_cooldown = PLAYER_PORTAL_COOLDOWN;
                    return Some(Travel { conn: self.conn, kind: TravelKind::Portal(kind), entry });
                }
            }
        } else {
            proc.time = (proc.time - 4).max(0);
            if proc.time <= 0 {
                self.portal = None;
            }
        }
        None
    }
}

/// `BlockUtil.FoundRectangle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rectangle {
    pub min: BlockPos,
    pub size1: i32,
    pub size2: i32,
}

fn axis_step(axis: Axis) -> Direction {
    match axis {
        Axis::X => Direction::East,
        Axis::Y => Direction::Up,
        Axis::Z => Direction::South,
    }
}

/// `BlockUtil.getLimit`.
fn limit(test: &mut dyn FnMut(BlockPos) -> bool, from: BlockPos, dir: Direction, max: i32) -> i32 {
    let mut n = 0;
    let mut at = from;
    while n < max {
        at = at.relative(dir);
        if !test(at) {
            break;
        }
        n += 1;
    }
    n
}

/// `BlockUtil.getMaxRectangleLocation`: the largest rectangle under a histogram, as (first
/// column, last column, height).
fn max_rectangle(columns: &[i32]) -> (i32, i32, i32) {
    let (mut start, mut end, mut height) = (0i32, 0i32, 0i32);
    let mut stack: Vec<usize> = vec![0];
    for i in 1..=columns.len() {
        let h = if i == columns.len() { 0 } else { columns[i] };
        while let Some(&top) = stack.last() {
            let top_height = columns[top];
            if h >= top_height {
                stack.push(i);
                break;
            }
            stack.pop();
            let s = stack.last().map_or(0, |&t| t + 1) as i32;
            if top_height * (i as i32 - s) > height * (end - start) {
                end = i as i32;
                start = s;
                height = top_height;
            }
        }
        if stack.is_empty() {
            stack.push(i);
        }
    }
    (start, end - 1, height)
}

/// `BlockUtil.getLargestRectangleAround`.
pub(crate) fn largest_rectangle(
    center: BlockPos,
    axis1: Axis,
    limit1: i32,
    axis2: Axis,
    limit2: i32,
    test: &mut dyn FnMut(BlockPos) -> bool,
) -> Rectangle {
    let pos1 = axis_step(axis1);
    let neg1 = pos1.opposite();
    let pos2 = axis_step(axis2);
    let neg2 = pos2.opposite();
    let neg_len = limit(test, center, neg1, limit1);
    let pos_len = limit(test, center, pos1, limit1);
    let left = neg_len as usize;
    let mut columns = vec![(0, 0); neg_len as usize + 1 + pos_len as usize];
    columns[left] = (limit(test, center, neg2, limit2), limit(test, center, pos2, limit2));
    let center_bottom = columns[left].0;
    for i in 1..=neg_len as usize {
        let prev = columns[left - (i - 1)];
        let at = center.relative_by(neg1, i as i32);
        columns[left - i] = (limit(test, at, neg2, prev.0), limit(test, at, pos2, prev.1));
    }
    for i in 1..=pos_len as usize {
        let prev = columns[left + i - 1];
        let at = center.relative_by(pos1, i as i32);
        columns[left + i] = (limit(test, at, neg2, prev.0), limit(test, at, pos2, prev.1));
    }
    let (mut best_left, mut best_bottom, mut best_width, mut best_height) = (0, 0, 0, 0);
    let mut heights = vec![0; columns.len()];
    let mut bottom = center_bottom;
    while bottom >= 0 {
        for (h, &(min, max)) in heights.iter_mut().zip(&columns) {
            let (start, end) = (center_bottom - min, center_bottom + max);
            *h = if bottom >= start && bottom <= end { end + 1 - bottom } else { 0 };
        }
        let (lo, hi, height) = max_rectangle(&heights);
        let width = 1 + hi - lo;
        if width * height > best_width * best_height {
            (best_left, best_bottom, best_width, best_height) = (lo, bottom, width, height);
        }
        bottom -= 1;
    }
    let min = center.relative_by(pos1, best_left - left as i32).relative_by(pos2, best_bottom - center_bottom);
    Rectangle { min, size1: best_width, size2: best_height }
}

/// `PortalShape.getRelativePosition`: where in the entry portal the player stands, as
/// fractions of its free width and height plus the offset across it.
fn relative_position(rect: Rectangle, axis: Axis, pos: [f64; 3], width: f64, height: f64) -> [f64; 3] {
    let free_w = rect.size1 as f64 - width;
    let free_h = rect.size2 as f64 - height;
    let get = |p: BlockPos, a: Axis| match a {
        Axis::X => p.x,
        Axis::Y => p.y,
        Axis::Z => p.z,
    } as f64;
    let getv = |a: Axis| match a {
        Axis::X => pos[0],
        Axis::Y => pos[1],
        Axis::Z => pos[2],
    };
    let inverse_lerp = |v: f64, lo: f64, hi: f64| (v - lo) / (hi - lo);
    let fx = if free_w > 0.0 {
        let start = get(rect.min, axis) + width / 2.0;
        inverse_lerp(getv(axis) - start, 0.0, free_w).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let fy = if free_h > 0.0 { inverse_lerp(getv(Axis::Y) - get(rect.min, Axis::Y), 0.0, free_h).clamp(0.0, 1.0) } else { 0.0 };
    let across = if axis == Axis::X { Axis::Z } else { Axis::X };
    let fz = getv(across) - (get(rect.min, across) + 0.5);
    [fx, fy, fz]
}

/// `BlockPos.spiralAround(center, radius, first, second)` in iteration order.
fn spiral(center: BlockPos, radius: i32, first: Direction, second: Direction) -> Vec<BlockPos> {
    let dirs = [first, second, first.opposite(), second.opposite()];
    let legs = 4 * radius;
    let (mut leg, mut leg_size, mut leg_index) = (-1i32, 0i32, 0i32);
    let mut last = center.relative(second);
    let mut out = Vec::new();
    loop {
        let cursor = last.relative(dirs[((leg + 4) % 4) as usize]);
        last = cursor;
        if leg_index >= leg_size {
            if leg >= legs {
                return out;
            }
            leg += 1;
            leg_index = 0;
            leg_size = leg / 2 + 1;
        }
        leg_index += 1;
        out.push(cursor);
    }
}

fn floor_pos(p: [f64; 3]) -> BlockPos {
    BlockPos::new(p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32)
}

impl Sim {
    /// A block of level `dim`, loading (or generating) its chunk if needed.
    pub(crate) fn block_loading(&mut self, dim: DimId, p: BlockPos) -> u16 {
        use kiln_world::spawn::LoadChunks;
        let chunk = self.dims[dim].load_chunk(ChunkPos::of_block(p.x, p.z));
        chunk.get((p.x & 15) as usize, p.y, (p.z & 15) as usize)
    }

    /// Loads the chunks around a block (`r` blocks each way) and puts them in regions, so
    /// block work there can run.
    pub(crate) fn load_area(&mut self, dim: DimId, center: BlockPos, r: i32) {
        use kiln_world::spawn::LoadChunks;
        for cx in (center.x - r) >> 4..=(center.x + r) >> 4 {
            for cz in (center.z - r) >> 4..=(center.z + r) >> 4 {
                self.dims[dim].load_chunk(ChunkPos::new(cx, cz));
            }
        }
        self.apply_topology();
    }

    /// Sets a block in a loaded chunk of `dim` through block behaviour (`Level.setBlock`).
    pub(crate) fn set_level_block(&mut self, dim: DimId, p: BlockPos, state: u16, flags: u32) {
        self.with_level_in(dim, [p.x, p.y, p.z], |level| kiln_blocks::set_block(level, p, state, flags));
    }

    /// `ServerLevel.getHeight(MOTION_BLOCKING)`: one above the highest motion-blocking block.
    pub(crate) fn motion_blocking_height(&mut self, dim: DimId, x: i32, z: i32) -> i32 {
        let d = self.dims[dim].provider.dimension;
        for y in (d.min_y..d.min_y + d.height).rev() {
            if kiln_data::block_props::motion_blocking(self.block_loading(dim, BlockPos::new(x, y, z))) {
                return y + 1;
            }
        }
        d.min_y
    }

    /// The serial half of a portal trip.
    pub(crate) fn travel(&mut self, t: Travel) {
        let Some(p) = self.players.get(&t.conn) else { return };
        if !p.can_use_portal() {
            return;
        }
        let from = p.dim;
        match t.kind {
            TravelKind::Credits => self.show_end_credits(t.conn),
            TravelKind::Portal(PortalKind::Nether) => {
                let to = if from == NETHER_ID { OVERWORLD_ID } else { NETHER_ID };
                // `ServerLevel.isAllowedToEnterPortal`.
                if to == NETHER_ID && !self.rule_bool("minecraft:allow_entering_nether_using_portals") {
                    return;
                }
                if let Some((pos, rot)) = self.nether_destination(t.conn, from, to, BlockPos::new(t.entry[0], t.entry[1], t.entry[2])) {
                    self.change_dimension(t.conn, to, pos, rot);
                    self.portal_sound(t.conn);
                }
            }
            TravelKind::Portal(PortalKind::End) => {
                if from == END_ID {
                    // `findRespawnPositionAndUseSpawnBlock`: the respawn point, else the world
                    // spawn.
                    let (dim, pos) = self.respawn_position(t.conn);
                    let rot = self.spawn_rot;
                    self.change_dimension(t.conn, dim, pos, rot);
                } else {
                    let [x, y, z] = END_SPAWN_POINT;
                    self.end_platform(BlockPos::new(x, y - 1, z));
                    let pitch = self.players[&t.conn].rot[1];
                    // A player lands one block lower than other entities.
                    let pos = [x as f64 + 0.5, y as f64 - 1.0, z as f64 + 0.5];
                    self.change_dimension(t.conn, END_ID, pos, [90.0, pitch]);
                }
                self.portal_sound(t.conn);
            }
            TravelKind::Portal(PortalKind::Gateway) => {
                let entry = BlockPos::new(t.entry[0], t.entry[1], t.entry[2]);
                if let Some(pos) = self.gateway_destination(from, entry) {
                    let rot = self.players[&t.conn].rot;
                    self.change_dimension(t.conn, from, pos, rot);
                }
            }
        }
    }

    /// Non-player entities in portals (basic): an entity touching a nether or end portal block
    /// moves to the other level at once (their transition time is 0) and then waits out
    /// `Entity.getDimensionChangingDelay` (300 ticks) before another trip. It arrives as a
    /// copy with a new network id, as vanilla's `teleportCrossDimension` makes one; end
    /// gateways and riders are not handled.
    ///
    /// `only`: the entities the regions found may touch a portal block ([`portal_candidates`]),
    /// sorted, and the first id given out after the regions ran: only those and the newer ones
    /// are looked at (`None`: every entity, when blocks or entities may have changed since).
    pub(crate) fn entity_portals(&mut self, only: Option<(&[(DimId, i32)], i32)>) {
        const ENTITY_PORTAL_COOLDOWN: i64 = 300;
        let now = self.game_time;
        let mut trips = Vec::new();
        for (dim, d) in self.dims.iter_mut().enumerate() {
            d.portal_cooldowns.retain(|_, until| *until > now);
            for r in d.regions.iter() {
                // `EnderDragon.canUsePortal`: never.
                // `Creaking.canUsePortal`: not while bound to a heart.
                for e in r.part().0.list.iter().filter(|e| {
                    only.is_none_or(|(ids, first_new)| e.id >= first_new || ids.binary_search(&(dim, e.id)).is_ok())
                        && !e.removed
                        && e.kind.name != "minecraft:ender_dragon"
                        && !d.portal_cooldowns.contains_key(&e.uuid.as_u128())
                        // `canUsePortal(false)`: a passenger does not (its vehicle does, with it).
                        && e.phys.as_deref().is_none_or(|p| p.vehicle.is_none())
                        && !e.phys.as_deref().is_some_and(kiln_entity::mob::kinds::creaking::is_heart_bound)
                }) {
                    let Some(phys) = e.phys.as_deref() else { continue };
                    let half = phys.width as f64 / 2.0 - 1.0e-5;
                    let (min, max) = ([e.pos[0] - half, e.pos[1] + 1.0e-5, e.pos[2] - half], [e.pos[0] + half, e.pos[1] + phys.height as f64 - 1.0e-5, e.pos[2] + half]);
                    'find: for x in min[0].floor() as i32..=max[0].floor() as i32 {
                        for y in min[1].floor() as i32..=max[1].floor() as i32 {
                            for z in min[2].floor() as i32..=max[2].floor() as i32 {
                                let s = d.regions.get_block(x, y, z).unwrap_or(0);
                                let first = kiln_data::blocks_types::block_of(s).first;
                                let kind = if first == block::NETHER_PORTAL && dim != END_ID {
                                    PortalKind::Nether
                                } else if first == block::END_PORTAL {
                                    PortalKind::End
                                } else {
                                    continue;
                                };
                                trips.push((dim, e.id, kind, BlockPos::new(x, y, z)));
                                break 'find;
                            }
                        }
                    }
                }
            }
        }
        // The destination may be a region ticking away (independent mode).
        if !trips.is_empty() {
            self.rendezvous();
        }
        for (from, id, kind, entry) in trips {
            let Some((pos, rot, size)) = self.dims[from]
                .regions
                .iter()
                .flat_map(|r| r.part().0.list.iter())
                .find(|e| e.id == id)
                .and_then(|e| e.phys.as_deref().map(|p| (e.pos, [p.y_rot, p.x_rot], [p.width, p.height])))
            else {
                continue;
            };
            let dest = match kind {
                PortalKind::Nether => {
                    let to = if from == NETHER_ID { OVERWORLD_ID } else { NETHER_ID };
                    if to == NETHER_ID && !self.rule_bool("minecraft:allow_entering_nether_using_portals") {
                        continue;
                    }
                    self.nether_exit(from, to, entry, pos, rot, size, false).map(|(p, r)| (to, p, r))
                }
                PortalKind::End if from == END_ID => {
                    // `adjustSpawnLocation`: the world spawn's column.
                    let [x, y, z] = self.spawn;
                    Some((OVERWORLD_ID, [x as f64 + 0.5, y as f64, z as f64 + 0.5], rot))
                }
                PortalKind::End => {
                    let [x, y, z] = END_SPAWN_POINT;
                    self.end_platform(BlockPos::new(x, y - 1, z));
                    Some((END_ID, [x as f64 + 0.5, y as f64, z as f64 + 0.5], [90.0, rot[1]]))
                }
                PortalKind::Gateway => None,
            };
            let Some((to, pos, rot)) = dest else { continue };
            // A vehicle takes its riders along (`Entity.teleport`: passengers are teleported
            // first and sit down again on the new entity).
            if self.entity_has_riders(from, id) {
                self.stack_changes_level(from, id, to, pos, Some(rot), Some(now + ENTITY_PORTAL_COOLDOWN));
                continue;
            }
            let mut taken = None;

            for r in self.dims[from].regions.iter_mut() {
                let list = &mut r.part_mut().0.list;
                if let Some(i) = list.iter().position(|e| e.id == id) {
                    taken = Some(list.remove(i));
                    break;
                }
            }
            let Some(mut e) = taken else { continue };
            let viewers = std::mem::take(&mut e.seen_by);
            self.forget_entities(vec![(id, viewers)]);
            let Some(mut phys) = e.phys.take() else { continue };
            let v = kiln_entity::math::Vec3::new(pos[0], pos[1], pos[2]);
            phys.set_pos(v);
            phys.set_old_pos_and_rot();
            phys.delta = kiln_entity::math::Vec3::ZERO;
            (phys.y_rot, phys.x_rot) = (rot[0], rot[1]);
            // `placePortalTicket`: the arrival chunks load.
            self.load_area(to, floor_pos(pos), 1);
            self.dims[to].portal_cooldowns.insert(e.uuid.as_u128(), now + ENTITY_PORTAL_COOLDOWN);
            self.dims[to].spawns.push(crate::entities::Spawn { kind: e.kind, pos, vel: [0.0; 3], body: crate::entities::Body::Loaded(phys) });
            info!("{} went from {} to {} at {pos:?}", e.kind.name, DIMENSIONS[from].0, DIMENSIONS[to].0);
        }
    }

}

/// The region's entities whose box may meet a nether or end portal block, as the region's cells
/// stand (a box in a chunk the region does not have counts): only the sections whose palette
/// has a portal block are looked at, so a region without portals finds none at once.
pub(crate) fn portal_candidates(entities: &crate::entities::Entities, cells: &kiln_region::CellSet<kiln_world::Cell>, min_y: i32) -> Vec<i32> {
    let is_portal = |s: u16| {
        let first = kiln_data::blocks_types::block_of(s).first;
        first == block::NETHER_PORTAL || first == block::END_PORTAL
    };
    let mut may_have: crate::FastMap<(i32, i32, i32), bool> = Default::default();
    let mut out = Vec::new();
    for e in entities.list.iter().filter(|e| !e.removed) {
        let Some(phys) = e.phys.as_deref() else { continue };
        // The box `entity_portals` looks in.
        let half = phys.width as f64 / 2.0 - 1.0e-5;
        let lo = [e.pos[0] - half, e.pos[1] + 1.0e-5, e.pos[2] - half].map(|c| c.floor() as i32);
        let hi = [e.pos[0] + half, e.pos[1] + phys.height as f64 - 1.0e-5, e.pos[2] + half].map(|c| c.floor() as i32);
        let mut any = false;
        'sections: for sx in lo[0] >> 4..=hi[0] >> 4 {
            for sz in lo[2] >> 4..=hi[2] >> 4 {
                for sy in (lo[1] - min_y) >> 4..=(hi[1] - min_y) >> 4 {
                    any |= *may_have.entry((sx, sy, sz)).or_insert_with(|| match cells.chunk(ChunkPos::new(sx, sz)) {
                        // Outside the level's height: air.
                        Some(c) => usize::try_from(sy).ok().and_then(|i| c.sections.get(i)).is_some_and(|s| s.blocks.maybe_has(is_portal)),
                        None => true,
                    });
                    if any {
                        break 'sections;
                    }
                }
            }
        }
        if any {
            out.push(e.id);
        }
    }
    out
}

impl Sim {
    /// `TeleportTransition.PLAY_PORTAL_SOUND`.
    fn portal_sound(&mut self, conn: ConnId) {
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(packets::world_fx::level_event(SOUND_PORTAL_TRAVEL, [0, 0, 0], 0, false));
        }
    }

    /// Where a player respawns: the respawn point's level and a free spot there, else the
    /// overworld spawn.
    pub(crate) fn respawn_position(&mut self, conn: ConnId) -> (DimId, [f64; 3]) {
        let p = &self.players[&conn];
        match p.respawn {
            Some(r) => {
                let d = p.respawn_dim;
                (d, kiln_world::spawn::free_spawn_at(&mut self.dims[d], r))
            }
            None => {
                let uuid = p.uuid;
                (OVERWORLD_ID, self.new_player_position(uuid))
            }
        }
    }

    /// `ServerPlayer.showEndCredits`: the player leaves the End's level and the client rolls
    /// the credits; its respawn request brings it back with everything it had.
    fn show_end_credits(&mut self, conn: ConnId) {
        self.untrack_everywhere(conn);
        let Some(p) = self.players.get_mut(&conn) else { return };
        if !p.won_game {
            p.won_game = true;
            p.send(packets::game_event(WIN_GAME, 0.0));
            p.seen_credits = true;
            info!("{} left the End", p.name);
        }
    }

    /// `EndPlatformFeature.createEndPlatform(level, origin, true)`: a 5×5 obsidian floor below
    /// `origin` with air above, breaking (and dropping) what was there.
    pub(crate) fn end_platform(&mut self, origin: BlockPos) {
        self.load_area(END_ID, origin, 2);
        for dz in -2..=2 {
            for dx in -2..=2 {
                for dy in -1..3 {
                    let p = origin.offset(dx, dy, dz);
                    let want = if dy == -1 { block::OBSIDIAN } else { block::AIR };
                    let have = self.block_loading(END_ID, p);
                    if kiln_data::blocks_types::block_of(have).first == want {
                        continue;
                    }
                    self.with_level_in(END_ID, [p.x, p.y, p.z], |level| {
                        kiln_blocks::destroy_block(level, p, true, kiln_blocks::flags::LIMIT);
                        kiln_blocks::set_block(level, p, want, kiln_blocks::flags::ALL);
                    });
                }
            }
        }
    }

    /// `NetherPortalBlock.getPortalDestination`: the exit portal (found near the scaled
    /// position, or built there) and where in it the player comes out, keeping its place in
    /// the entry portal. Returns the position and rotation.
    fn nether_destination(&mut self, conn: ConnId, from: DimId, to: DimId, entry: BlockPos) -> Option<([f64; 3], [f32; 2])> {
        let p = &self.players[&conn];
        let (pos, rot, spectator) = (p.pos, p.rot, p.game_mode == 3);
        let (width, height, _) = p.dimensions();
        self.nether_exit(from, to, entry, pos, rot, [width, height], spectator)
    }

    /// [`Sim::nether_destination`] for any entity: its position, rotation and size.
    #[allow(clippy::too_many_arguments)]
    fn nether_exit(
        &mut self,
        from: DimId,
        to: DimId,
        entry: BlockPos,
        pos: [f64; 3],
        rot: [f32; 2],
        [width, height]: [f32; 2],
        spectator: bool,
    ) -> Option<([f64; 3], [f32; 2])> {
        let scale = self.dims[from].kind.coordinate_scale / self.dims[to].kind.coordinate_scale;
        // `WorldBorder.clampToBounds`.
        let clamp = |v: f64| v.clamp(-BORDER, BORDER - 1.0);
        let exit = floor_pos([clamp(pos[0] * scale), pos[1], clamp(pos[2] * scale)]);
        let to_nether = to == NETHER_ID;
        let rect = match self.find_closest_portal(to, exit, to_nether) {
            Some(found) => {
                let state = self.block_loading(to, found);
                let axis = shape::portal_axis(state);
                largest_rectangle(found, axis, 21, Axis::Y, 21, &mut |q| self.block_in_level(to, q) == state)
            }
            None => {
                if spectator {
                    return None;
                }
                let entry_state = self.block_in_level(from, entry);
                let axis = if kiln_blocks::state::is(entry_state, block::NETHER_PORTAL) { shape::portal_axis(entry_state) } else { Axis::X };
                match self.create_portal(to, exit, axis) {
                    Some(r) => r,
                    None => {
                        warn!("Unable to create a portal, likely target out of worldborder");
                        return None;
                    }
                }
            }
        };
        // `getDimensionTransitionFromExit`.
        let entry_state = self.block_in_level(from, entry);
        let (axis, offset) = if kiln_blocks::state::is(entry_state, block::NETHER_PORTAL) {
            let axis = shape::portal_axis(entry_state);
            let entry_rect = largest_rectangle(entry, axis, 21, Axis::Y, 21, &mut |q| self.block_in_level(from, q) == entry_state);
            (axis, relative_position(entry_rect, axis, pos, width as f64, height as f64))
        } else {
            (Axis::X, [0.5, 0.0, 0.0])
        };
        // `createDimensionTransition`.
        let bottom_left = rect.min;
        let exit_state = self.block_in_level(to, bottom_left);
        let new_axis = if kiln_blocks::state::is(exit_state, block::NETHER_PORTAL) { shape::portal_axis(exit_state) } else { Axis::X };
        let rotation = if axis == new_axis { 0.0 } else { 90.0 };
        let (w, h) = (width as f64, height as f64);
        let x_off = w / 2.0 + (rect.size1 as f64 - w) * offset[0];
        let y_off = (rect.size2 as f64 - h) * offset[1];
        let z_off = 0.5 + offset[2];
        let x_axis = new_axis == Axis::X;
        let target = [
            bottom_left.x as f64 + if x_axis { x_off } else { z_off },
            bottom_left.y as f64 + y_off,
            bottom_left.z as f64 + if x_axis { z_off } else { x_off },
        ];
        // `PortalShape.findCollisionFreePosition`: a portal's interior is free, so Kiln keeps
        // the target (vanilla moves it only when blocks overlap the player's box there).
        Some((target, [rot[0] + rotation, rot[1]]))
    }

    /// A loaded block of `dim` (void air if not loaded).
    pub(crate) fn block_in_level(&self, dim: DimId, p: BlockPos) -> u16 {
        let d = &self.dims[dim];
        let c = ChunkPos::of_block(p.x, p.z);
        match d.regions.chunk(c).or_else(|| d.pending.get(&c)) {
            Some(chunk) => chunk.get((p.x & 15) as usize, p.y, (p.z & 15) as usize),
            None => block::VOID_AIR,
        }
    }

    /// `PortalForcer.findClosestPortalPosition`: the nearest portal block (then the lowest) in
    /// the square of radius 16 (into the nether) or 128 around `exit`.
    fn find_closest_portal(&mut self, dim: DimId, exit: BlockPos, to_nether: bool) -> Option<BlockPos> {
        let radius: i32 = if to_nether { 16 } else { 128 };
        let chunk_radius = radius.div_euclid(16) + 1;
        let center = ChunkPos::of_block(exit.x, exit.z);
        let portal = kiln_data::blocks_types::block_of(block::NETHER_PORTAL);
        let is_portal = |s: u16| (portal.first..=portal.last).contains(&s);
        let mut best: Option<(i64, i32, BlockPos)> = None;
        let mut consider = |p: BlockPos| {
            if (p.x - exit.x).abs() > radius || (p.z - exit.z).abs() > radius {
                return;
            }
            if p.x.abs() as f64 >= BORDER || p.z.abs() as f64 >= BORDER {
                return;
            }
            let (dx, dy, dz) = ((p.x - exit.x) as i64, (p.y - exit.y) as i64, (p.z - exit.z) as i64);
            let d = dx * dx + dy * dy + dz * dz;
            if best.is_none_or(|(bd, by, _)| (d, p.y) < (bd, by)) {
                best = Some((d, p.y, p));
            }
        };
        for cx in center.x - chunk_radius..=center.x + chunk_radius {
            for cz in center.z - chunk_radius..=center.z + chunk_radius {
                let pos = ChunkPos::new(cx, cz);
                let d = &mut self.dims[dim];
                let scan = |chunk: &kiln_world::chunk::Chunk, consider: &mut dyn FnMut(BlockPos)| {
                    for (si, section) in chunk.sections.iter().enumerate() {
                        let may_have = match &section.blocks {
                            kiln_world::section::BlockContainer::Single(s) => is_portal(*s),
                            kiln_world::section::BlockContainer::Nibble { palette, .. }
                            | kiln_world::section::BlockContainer::Byte { palette, .. } => palette.iter().any(|&s| is_portal(s)),
                            kiln_world::section::BlockContainer::Direct(_) => true,
                        };
                        if !may_have {
                            continue;
                        }
                        for i in 0..4096 {
                            if is_portal(section.blocks.get(i)) {
                                let (x, y, z) = (i & 15, i >> 8, (i >> 4) & 15);
                                consider(BlockPos::new(
                                    (cx << 4) + x as i32,
                                    chunk.min_y() + (si as i32) * 16 + y as i32,
                                    (cz << 4) + z as i32,
                                ));
                            }
                        }
                    }
                };
                if let Some(chunk) = d.regions.chunk(pos).or_else(|| d.pending.get(&pos)) {
                    scan(chunk, &mut consider);
                } else if let Some(mut chunk) = d.provider.load(pos) {
                    // A saved chunk nobody needs: read and let go.
                    scan(&chunk, &mut consider);
                    d.provider.unload(pos, &mut chunk);
                }
            }
        }
        best.map(|(_, _, p)| p)
    }

    /// `PortalForcer.canPortalReplaceBlock`.
    fn can_portal_replace(&mut self, dim: DimId, p: BlockPos) -> bool {
        let s = self.block_loading(dim, p);
        kiln_data::block_props::replaceable(s) && !kiln_data::blocks_types::has_fluid(s)
    }

    /// `PortalForcer.canHostFrame`.
    fn can_host_frame(&mut self, dim: DimId, origin: BlockPos, dir: Direction, offset: i32) -> bool {
        let cw = dir.clockwise();
        let [sx, _, sz] = dir.step();
        let [cx, _, cz] = cw.step();
        for width in -1..3 {
            for height in -1..4 {
                let p = origin.offset(sx * width + cx * offset, height, sz * width + cz * offset);
                if height < 0 && !kiln_data::block_logic::is_solid(self.block_loading(dim, p)) {
                    return false;
                }
                if height >= 0 && !self.can_portal_replace(dim, p) {
                    return false;
                }
            }
        }
        true
    }

    /// `PortalForcer.createPortal`: the best spot within 16 blocks (on solid ground with room
    /// for a portal, preferring room beside it), else a platform in the air; then an obsidian
    /// frame with a 2×3 portal.
    fn create_portal(&mut self, dim: DimId, origin: BlockPos, axis: Axis) -> Option<Rectangle> {
        let direction = axis_step(axis);
        self.load_area(dim, origin, 20);
        let (mut closest, mut closest_d) = (None, -1.0f64);
        let (mut closest_air, mut closest_air_d) = (None, -1.0f64);
        let kind = self.dims[dim].kind;
        let min_y = kind.min_y;
        let max_placeable = (kind.min_y + kind.height - 1).min(kind.min_y + kind.logical_height - 1);
        let dist = |a: BlockPos, b: BlockPos| {
            let (dx, dy, dz) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
            dx * dx + dy * dy + dz * dz
        };
        for column in spiral(origin, 16, Direction::East, Direction::South) {
            let height = max_placeable.min(self.motion_blocking_height(dim, column.x, column.z));
            let inside = |p: BlockPos| (p.x as f64) < BORDER && (p.x as f64) >= -BORDER && (p.z as f64) < BORDER && (p.z as f64) >= -BORDER;
            if !inside(column) || !inside(column.relative(direction)) {
                continue;
            }
            let mut y = height;
            while y >= min_y {
                let at = BlockPos::new(column.x, y, column.z);
                if self.can_portal_replace(dim, at) {
                    let first_empty = y;
                    while y > min_y && self.can_portal_replace(dim, BlockPos::new(column.x, y - 1, column.z)) {
                        y -= 1;
                    }
                    let delta = first_empty - y;
                    if y + 4 <= max_placeable && !(delta > 0 && delta < 3) {
                        let at = BlockPos::new(column.x, y, column.z);
                        if self.can_host_frame(dim, at, direction, 0) {
                            let d = dist(origin, at);
                            if self.can_host_frame(dim, at, direction, -1)
                                && self.can_host_frame(dim, at, direction, 1)
                                && (closest_d == -1.0 || closest_d > d)
                            {
                                closest_d = d;
                                closest = Some(at);
                            }
                            if closest_d == -1.0 && (closest_air_d == -1.0 || closest_air_d > d) {
                                closest_air_d = d;
                                closest_air = Some(at);
                            }
                        }
                    }
                }
                y -= 1;
            }
        }
        if closest.is_none() {
            closest = closest_air;
        }
        let [sx, _, sz] = direction.step();
        let closest = match closest {
            Some(c) => c,
            None => {
                let lo = (min_y + 1).max(70);
                let hi = max_placeable - 9;
                if hi < lo {
                    return None;
                }
                let c = BlockPos::new(origin.x - sx, origin.y.clamp(lo, hi), origin.z - sz);
                let cw = direction.clockwise();
                let [cx, _, cz] = cw.step();
                for bx in -1..2 {
                    for w in 0..2 {
                        for h in -1..3 {
                            let state = if h < 0 { block::OBSIDIAN } else { block::AIR };
                            let p = c.offset(w * sx + bx * cx, h, w * sz + bx * cz);
                            self.set_level_block(dim, p, state, kiln_blocks::flags::ALL);
                        }
                    }
                }
                c
            }
        };
        for w in -1..3 {
            for h in -1..4 {
                if w == -1 || w == 2 || h == -1 || h == 3 {
                    self.set_level_block(dim, closest.offset(w * sx, h, w * sz), block::OBSIDIAN, kiln_blocks::flags::ALL);
                }
            }
        }
        let portal = shape::portal_state(axis);
        for w in 0..2 {
            for h in 0..3 {
                self.set_level_block(dim, closest.offset(w * sx, h, w * sz), portal, kiln_blocks::flags::CLIENTS | kiln_blocks::flags::KNOWN_SHAPE);
            }
        }
        info!("built a nether portal in {} at {closest:?}", DIMENSIONS[dim].0);
        Some(Rectangle { min: closest, size1: 2, size2: 3 })
    }

    /// `EndGatewayBlock.getPortalDestination` / `TheEndGatewayBlockEntity.getPortalPosition`:
    /// the gateway's exit, found (and recorded) on first use in the End; `None` while the
    /// gateway cools down or when it leads nowhere.
    fn gateway_destination(&mut self, dim: DimId, entry: BlockPos) -> Option<[f64; 3]> {
        use kiln_proto::nbt::Tag;
        let chunk_pos = ChunkPos::of_block(entry.x, entry.z);
        let (lx, lz) = ((entry.x & 15) as usize, (entry.z & 15) as usize);
        let be = self.dims[dim].regions.chunk(chunk_pos)?.block_entity(lx, entry.y, lz)?.clone();
        let int = |k: &str| be.nbt.get(k).and_then(Tag::as_i64);
        let key = [entry.x, entry.y, entry.z];
        if self.dims[dim].gateway_cooldowns.get(&key).is_some_and(|&t| t > self.game_time) {
            return None;
        }
        let exit = match be.nbt.get("exit_portal") {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        };
        let exact = int("ExactTeleport").unwrap_or(0) != 0;
        let exit = match exit {
            Some(e) => e,
            None if dim == END_ID => {
                let e = self.gateway_teleport_pos(entry).offset(0, 10, 0);
                // `spawnGatewayPortal(level, exit, knownExit(entry, false))`: the way back.
                self.place_gateway(e, Some((entry, false)));
                self.record_gateway_exit(dim, entry, e);
                e
            }
            None => return None,
        };
        // `triggerCooldown` (the beam's block event is not sent).
        let until = self.game_time + GATEWAY_COOLDOWN as i64;
        let now = self.game_time;
        let cooldowns = &mut self.dims[dim].gateway_cooldowns;
        cooldowns.retain(|_, t| *t > now);
        cooldowns.insert(key, until);
        let target = if exact { exit } else { self.tallest_block(dim, exit.offset(0, 2, 0), 5, false).above() };
        Some([target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5])
    }

    fn record_gateway_exit(&mut self, dim: DimId, entry: BlockPos, exit: BlockPos) {
        use kiln_proto::nbt::Tag;
        let (lx, lz) = ((entry.x & 15) as usize, (entry.z & 15) as usize);
        let Some(chunk) = self.dims[dim].regions.chunk_mut(ChunkPos::of_block(entry.x, entry.z)) else { return };
        let Some(mut be) = chunk.block_entity(lx, entry.y, lz).cloned() else { return };
        if let Tag::Compound(f) = &mut be.nbt {
            f.retain(|(k, _)| k != "exit_portal");
            f.push(("exit_portal".into(), Tag::IntArray(vec![exit.x, exit.y, exit.z])));
        }
        chunk.set_block_entity(lx, entry.y, lz, be);
    }

    /// `TheEndGatewayBlockEntity.findTallestBlock`.
    fn tallest_block(&mut self, dim: DimId, around: BlockPos, dist: i32, allow_bedrock: bool) -> BlockPos {
        let d = self.dims[dim].provider.dimension;
        let max_y = d.min_y + d.height - 1;
        let mut tallest: Option<BlockPos> = None;
        for dx in -dist..=dist {
            for dz in -dist..=dist {
                if dx == 0 && dz == 0 && !allow_bedrock {
                    continue;
                }
                let mut y = max_y;
                while y > tallest.map_or(d.min_y, |t| t.y) {
                    let p = BlockPos::new(around.x + dx, y, around.z + dz);
                    let s = self.block_loading(dim, p);
                    if kiln_data::block_props::full_collision(s) && (allow_bedrock || !kiln_blocks::state::is(s, block::BEDROCK)) {
                        tallest = Some(p);
                        break;
                    }
                    y -= 1;
                }
            }
        }
        tallest.unwrap_or(around)
    }

    /// `TheEndGatewayBlockEntity.findOrCreateValidTeleportPos`: 1024 blocks out along the
    /// gateway's direction from the centre, stepping back past islands and forward past void,
    /// then the end stone nearest the origin in that chunk.
    fn gateway_teleport_pos(&mut self, entry: BlockPos) -> BlockPos {
        let len = ((entry.x as f64).powi(2) + (entry.z as f64).powi(2)).sqrt();
        let dir = if len < 1.0e-4 { [0.0, 0.0] } else { [entry.x as f64 / len, entry.z as f64 / len] };
        let mut exit = [dir[0] * 1024.0, dir[1] * 1024.0];
        let mut left = 16;
        while !self.chunk_empty(exit) && {
            left -= 1;
            left > 0
        } {
            exit = [exit[0] - dir[0] * 16.0, exit[1] - dir[1] * 16.0];
        }
        let mut left = 16;
        while self.chunk_empty(exit) && {
            left -= 1;
            left > 0
        } {
            exit = [exit[0] + dir[0] * 16.0, exit[1] + dir[1] * 16.0];
        }
        let chunk = ChunkPos::new((exit[0] / 16.0).floor() as i32, (exit[1] / 16.0).floor() as i32);
        let found = self.valid_spawn_in_chunk(chunk);
        let at = match found {
            Some(p) => p,
            None => {
                // No land: an `end_island` grows there (`RandomSource.create(pos.asLong())`).
                let p = BlockPos::new((exit[0] + 0.5).floor() as i32, 75, (exit[1] + 0.5).floor() as i32);
                let wp = kiln_worldgen::pos::BlockPos::new(p.x, p.y, p.z);
                self.load_area(END_ID, p, 8);
                for (q, state) in kiln_worldgen::end::end_island_blocks(wp, wp.as_long()) {
                    self.set_level_block(END_ID, BlockPos::new(q.x, q.y, q.z), state, kiln_blocks::flags::ALL);
                }
                p
            }
        };
        self.tallest_block(END_ID, at, 16, true)
    }

    /// `isChunkEmpty`: the chunk (generated if needed) has no blocks.
    fn chunk_empty(&mut self, at: [f64; 2]) -> bool {
        use kiln_world::spawn::LoadChunks;
        let c = ChunkPos::new((at[0] / 16.0).floor() as i32, (at[1] / 16.0).floor() as i32);
        let chunk = self.dims[END_ID].load_chunk(c);
        chunk.sections.iter().all(|s| s.is_empty())
    }

    /// `findValidSpawnInChunk`: end stone with two non-full blocks above, nearest the origin.
    fn valid_spawn_in_chunk(&mut self, c: ChunkPos) -> Option<BlockPos> {
        use kiln_world::spawn::LoadChunks;
        let chunk = self.dims[END_ID].load_chunk(c);
        let top = chunk.sections.iter().rposition(|s| !s.is_empty())? as i32 * 16 + chunk.min_y() + 15;
        let mut best: Option<(f64, BlockPos)> = None;
        for z in 0..16 {
            for y in 30..=top {
                for x in 0..16 {
                    let s = chunk.get(x, y, z);
                    if !kiln_blocks::state::is(s, block::END_STONE)
                        || kiln_data::block_props::full_collision(chunk.get(x, y + 1, z))
                        || kiln_data::block_props::full_collision(chunk.get(x, y + 2, z))
                    {
                        continue;
                    }
                    let p = BlockPos::new((c.x << 4) + x as i32, y, (c.z << 4) + z as i32);
                    let d = (p.x as f64 + 0.5).powi(2) + (p.y as f64 + 0.5).powi(2) + (p.z as f64 + 0.5).powi(2);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, p));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// `EndGatewayFeature` at `at` with its block entity (`exit`: a known exit and whether it
    /// is exact).
    pub(crate) fn place_gateway(&mut self, at: BlockPos, exit: Option<(BlockPos, bool)>) {
        let conv = |p: BlockPos| kiln_worldgen::pos::BlockPos::new(p.x, p.y, p.z);
        self.load_area(END_ID, at, 2);
        for (p, state) in kiln_worldgen::end::end_gateway_blocks(conv(at)) {
            self.set_level_block(END_ID, BlockPos::new(p.x, p.y, p.z), state, kiln_blocks::flags::ALL);
        }
        let tag = kiln_worldgen::end::end_gateway_entity(exit.map(|(p, exact)| (conv(p), exact)));
        let (lx, lz) = ((at.x & 15) as usize, (at.z & 15) as usize);
        if let (Some(be), Some(chunk)) =
            (kiln_world::block_entity::BlockEntity::from_saved(tag), self.dims[END_ID].regions.chunk_mut(ChunkPos::of_block(at.x, at.z)))
        {
            chunk.set_block_entity(lx, at.y, lz, be);
        }
    }

    /// `ServerPlayer.teleport` to another level (or within one): the client rebuilds its world
    /// from a Respawn packet keeping all data, gets the level's info again and its chunks
    /// stream anew; viewers in the old level forget the player.
    pub(crate) fn change_dimension(&mut self, conn: ConnId, dim: DimId, pos: [f64; 3], rot: [f32; 2]) {
        let now = self.game_time;
        let Some(p) = self.players.get(&conn) else { return };
        if p.dim == dim {
            let p = self.players.get_mut(&conn).unwrap();
            p.teleport(pos, rot, now);
            self.place_player(conn);
            return;
        }
        let from = p.dim;
        let info = packets::player::respawn(&self.spawn_info(dim, p), packets::player::respawn_keep::ALL);
        let difficulty = packets::change_difficulty(self.commands.difficulty as u8, false);
        let (spawn, spawn_rot) = (self.spawn, self.spawn_rot);
        let time = self.time_packet();
        let weather = self.level_info_packets(dim);
        let rules = self.rules.clone();
        self.untrack_everywhere(conn);
        self.post_effects_pending = true;
        let p = self.players.get_mut(&conn).unwrap();
        p.send(info);
        p.send(difficulty);
        // `ServerPlayer.teleport` to another level sends the post effects again.
        p.post_effects_dirty = true;
        self.sleep_status[p.dim].dirty = true;
        self.sleep_status[dim].dirty = true;
        // `enteredNetherPosition`: where the player left the overworld for the nether.
        if from == crate::OVERWORLD_ID && dim == crate::NETHER_ID {
            p.entered_nether = Some(p.pos);
        }
        p.dim = dim;
        p.using = None;
        p.digging = None;
        p.delayed_destroy = None;
        p.fall_distance = 0.0;
        p.vel = [0.0; 3];
        p.sent_chunks.clear();
        p.unacked_batches = 0;
        p.teleport(pos, rot, now);
        p.block_effects_from = pos;
        p.center = player_chunk(pos);
        p.send(packets::set_chunk_cache_center(p.center.x, p.center.z));
        // `PlayerList.sendLevelInfo`: world border (default), time, spawn position, weather.
        p.send(time);
        p.send(packets::set_default_spawn_position(crate::OVERWORLD, spawn, spawn_rot[0], spawn_rot[1]));
        for w in weather {
            p.send(w);
        }
        p.send(packets::game_event(packets::GAME_EVENT_START_WAITING_FOR_CHUNKS, 0.0));
        // `PlayerList.sendAllPlayerInfo` and `sendActivePlayerEffects`.
        p.send(packets::set_held_slot(p.inv.selected as i32));
        p.send_all_effects();
        p.sent_health = None;
        p.sync_health();
        p.sent_xp = None;
        p.sync_experience();
        p.self_meta_dirty = true;
        p.attributes_dirty = true;
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.open(env));
        info!("{} went from {} to {} at {pos:?}", p.name, DIMENSIONS[from].0, DIMENSIONS[dim].0);
        // `triggerDimensionChangeTriggers`: back from the nether, how far that took the player.
        if from == crate::NETHER_ID && dim == crate::OVERWORLD_ID
            && let Some(start) = p.entered_nether
        {
            p.distance_trigger("minecraft:nether_travel", start);
        }
        if dim != crate::NETHER_ID {
            p.entered_nether = None;
        }
        p.changed_dimension(DIMENSIONS[from].0, DIMENSIONS[dim].0);
        self.dims[dim].spawns.extend(spawns);
        self.place_player(conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spiral_visits_the_square_ring_by_ring() {
        let s = spiral(BlockPos::new(0, 0, 0), 1, Direction::East, Direction::South);
        assert_eq!(s[0], BlockPos::new(0, 0, 0));
        assert_eq!(s.len(), 9);
        let mut sorted = s.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 9, "{s:?}");
        assert_eq!(spiral(BlockPos::new(5, 64, 5), 16, Direction::East, Direction::South).len(), 33 * 33);
    }

    #[test]
    fn largest_rectangle_of_a_portal_interior() {
        // A 2×3 X-axis portal at (10..=11, 64..=66, 0).
        let inside = |p: BlockPos| (10..=11).contains(&p.x) && (64..=66).contains(&p.y) && p.z == 0;
        let r = largest_rectangle(BlockPos::new(11, 65, 0), Axis::X, 21, Axis::Y, 21, &mut |p| inside(p));
        assert_eq!(r, Rectangle { min: BlockPos::new(10, 64, 0), size1: 2, size2: 3 });
    }

    #[test]
    fn relative_position_in_a_portal() {
        let r = Rectangle { min: BlockPos::new(10, 64, 0), size1: 2, size2: 3 };
        let f = relative_position(r, Axis::X, [11.0, 64.0, 0.5], 0.6, 1.8);
        assert!((f[0] - 0.5).abs() < 1e-9 && f[1] == 0.0 && f[2] == 0.0, "{f:?}");
    }
}
