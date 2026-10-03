//! Players as entities: tab list entries, visibility to other players and movement/metadata
//! broadcasting (following vanilla's ChunkMap tracking and ServerEntity updates).

use crate::{Player, Sim};
use bytes::Bytes;
use kiln_data::entities::{data, pose, types::PLAYER};
use kiln_link::ConnId;
use kiln_proto::packets::entity::metadata::{HumanoidArm, shared_flags};
use kiln_proto::packets::entity::{self, DataValue, EntityData, MoveState, PlayerInfoActions, PlayerInfoEntry};
use kiln_proto::packets::ProfileProperty;
use kiln_world::ChunkPos;
use std::collections::HashMap;
use kiln_sched::{Ctx, Window};

/// Per-player windows of a crowd (a few microseconds per player).
const PLAYER_WINDOW: Window = Window::new();
/// Visibility: each player checks every mover (or everyone, when it moved).
const VISIBILITY_WINDOW: Window = Window::new();

/// A command chat message (`say`, `me`, `msg`) through its chat type.
pub(crate) fn chat_disguised(message: &kiln_command::ChatMessage) -> Bytes {
    let chat_type = kiln_data::synced_id("minecraft:chat_type", message.kind.id()).unwrap_or(0);
    let target = message.target.as_ref().map(|t| t.to_nbt());
    kiln_proto::packets::disguised_chat(&message.content.to_nbt(), chat_type, &message.sender.to_nbt(), target.as_ref())
}

/// A player's chat line through the `minecraft:chat` chat type ("<name> message").
pub(crate) fn chat_player(name: &str, content: &str) -> Bytes {
    let chat_type = kiln_data::synced_id("minecraft:chat_type", "minecraft:chat").unwrap_or(0);
    kiln_proto::packets::disguised_chat(&kiln_proto::nbt::text(content), chat_type, &kiln_proto::nbt::text(name), None)
}

/// Tab list game mode change.
pub(crate) fn game_mode_update(uuid: uuid::Uuid, mode: i32) -> Bytes {
    entity::player_info_update(PlayerInfoActions::UPDATE_GAME_MODE, &[PlayerInfoEntry::new(uuid, "", &[], mode)])
}

impl Player {
    pub(crate) fn move_state(&self) -> MoveState {
        MoveState { pos: self.pos, yaw: self.rot[0], pitch: self.rot[1], head_yaw: self.rot[0], on_ground: self.on_ground }
    }

    fn shared_flags(&self) -> i8 {
        let mut f = 0;
        if self.on_fire_flag {
            f |= shared_flags::ON_FIRE;
        }
        // `updateInvisibilityStatus` and `updateGlowingStatus`.
        if self.has_effect("minecraft:invisibility") {
            f |= shared_flags::INVISIBLE;
        }
        if self.has_effect("minecraft:glowing") {
            f |= shared_flags::GLOWING;
        }
        if self.sneaking {
            f |= shared_flags::CROUCHING;
        }
        if self.sprinting {
            f |= shared_flags::SPRINTING;
        }
        if self.fall_flying {
            f |= shared_flags::FALL_FLYING;
        }
        f as i8
    }

    fn pose(&self) -> i32 {
        if self.fall_flying {
            pose::FALL_FLYING
        } else if self.sleep.pos.is_some() {
            pose::SLEEPING
        } else if self.sneaking {
            pose::CROUCHING
        } else {
            pose::STANDING
        }
    }

    /// Entity data a new viewer needs (fields that differ from their defaults).
    fn spawn_metadata(&self) -> EntityData {
        let mut d = EntityData::new();
        d.set(data::living_entity::HEALTH, &DataValue::Float(20.0));
        d.set(data::avatar::PLAYER_MODE_CUSTOMISATION, &DataValue::Byte(self.client.skin_parts as i8));
        if self.client.main_hand == 0 {
            d.set(data::avatar::PLAYER_MAIN_HAND, &DataValue::HumanoidArm(HumanoidArm::Left));
        }
        if self.shoulders.iter().any(Option::is_some) {
            self.shoulder_data(&mut d);
        }
        if self.shared_flags() != 0 || self.sleep.pos.is_some() {
            d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(self.shared_flags()));
            d.set(data::entity::POSE, &DataValue::Pose(self.pose()));
        }
        if self.sleep.pos.is_some() {
            d.set(data::living_entity::SLEEPING_POS, &DataValue::OptionalBlockPos(self.sleep.pos));
        }
        if self.living_flags() != 0 {
            d.set(data::living_entity::LIVING_ENTITY_FLAGS, &DataValue::Byte(self.living_flags()));
        }
        if self.air != crate::hazards::MAX_AIR {
            d.set(data::entity::AIR_SUPPLY, &DataValue::Int(self.air));
        }
        if !self.effects.is_empty() {
            self.effect_data(&mut d);
        }
        d
    }

    /// `DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID`.
    fn effect_data(&self, d: &mut EntityData) {
        d.set(data::living_entity::EFFECT_PARTICLES, &DataValue::Particles(self.effect_particles()));
        d.set(data::living_entity::EFFECT_AMBIENCE, &DataValue::Boolean(!self.effects.is_empty() && self.effects_ambient()));
    }

    fn info_entry<'a>(&'a self, props: &'a [ProfileProperty<'a>]) -> PlayerInfoEntry<'a> {
        PlayerInfoEntry::new(self.uuid, &self.name, props, self.game_mode as i32)
    }

    fn profile_properties(&self) -> Vec<ProfileProperty<'_>> {
        self.properties
            .iter()
            .map(|p| ProfileProperty { name: &p.name, value: &p.value, signature: p.signature.as_deref() })
            .collect()
    }

    fn spawn_packets(&self) -> Vec<Bytes> {
        let mut out = vec![
            entity::bundle_delimiter(),
            self.tracker.spawn(self.uuid, PLAYER.id, [0.0; 3], 0),
            entity::set_entity_data(self.entity_id, &self.spawn_metadata()),
        ];
        // `ServerEntity.sendPairingData`: the equipment that is not empty.
        let worn: Vec<(u8, &kiln_item::ItemStack)> =
            (0..SHOWN_SLOTS).map(|i| (i as u8, self.inv.equipped(crate::combat::SLOTS[i]))).filter(|(_, s)| !s.is_empty()).collect();
        if !worn.is_empty() {
            out.push(set_equipment(self.entity_id, &worn));
        }
        out.push(entity::bundle_delimiter());
        out
    }

    /// `LivingEntity.detectEquipmentUpdates`: the slots whose stack changed since the last
    /// broadcast, as a Set Equipment packet for viewers.
    fn equipment_changes(&mut self) -> Option<Bytes> {
        let mut changed = Vec::new();
        for i in 0..SHOWN_SLOTS {
            let now = self.inv.equipped(crate::combat::SLOTS[i]);
            if !kiln_inventory::stack::matches(&self.equipment_sent[i], now) {
                self.equipment_sent[i] = now.clone();
                changed.push(i);
            }
        }
        if changed.is_empty() {
            return None;
        }
        let slots: Vec<(u8, &kiln_item::ItemStack)> = changed.iter().map(|&i| (i as u8, &self.equipment_sent[i])).collect();
        Some(set_equipment(self.entity_id, &slots))
    }
}

impl Sim {
    /// Tab list: the joiner to everyone, and everyone to the joiner.
    pub(crate) fn announce_join(&mut self, conn: ConnId) {
        let Some(joiner) = self.players.get(&conn) else { return };
        let props = joiner.profile_properties();
        let to_all = entity::player_info_update(PlayerInfoActions::INITIALIZE, &[joiner.info_entry(&props)]);
        // In join order (connection ids grow), not hash order.
        let mut everyone: Vec<&Player> = self.players.values().collect();
        everyone.sort_by_key(|p| p.conn);
        let all_props: Vec<Vec<ProfileProperty>> = everyone.iter().map(|p| p.profile_properties()).collect();
        let entries: Vec<PlayerInfoEntry> = everyone.iter().zip(&all_props).map(|(p, props)| p.info_entry(props)).collect();
        let to_joiner = entity::player_info_update(PlayerInfoActions::INITIALIZE, &entries);
        drop(entries);
        self.broadcast(to_all);
        if let Some(p) = self.players.get_mut(&conn) {
            p.send(to_joiner);
        }
    }

    /// Removes a player that has left from everyone's view and tab list.
    pub(crate) fn announce_leave(&mut self, gone: &Player, conn: ConnId) {
        let remove = entity::remove_entities(&[gone.entity_id]);
        for v in &gone.seen_by {
            if let Some(p) = self.players.get_mut(v) {
                p.send(remove.clone());
            }
        }
        for p in self.players.values_mut() {
            if let Ok(i) = p.seen_by.binary_search(&conn) {
                p.seen_by.remove(i);
            }
        }
        self.broadcast(entity::player_info_remove(&[gone.uuid]));
    }

    /// Ends every pairing of `conn` with other players (it respawned): viewers forget it and
    /// tracking starts over.
    pub(crate) fn untrack_everywhere(&mut self, conn: ConnId) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        let (id, viewers) = (p.entity_id, std::mem::take(&mut p.seen_by));
        p.section = None;
        let despawn = entity::remove_entities(&[id]);
        for v in viewers {
            if let Some(q) = self.players.get_mut(&v) {
                q.send(despawn.clone());
                q.section = None;
            }
        }
    }

    /// Entities seen by players that are in another region now (they were teleported or the
    /// regions split): tracking only runs within a region, so the viewer is told the entity is
    /// gone, as it would be in one region when the player moves out of range.
    pub(crate) fn drop_cross_region_viewers(&mut self) {
        let home: HashMap<ConnId, (crate::DimId, kiln_region::RegionId)> = self.players.iter().map(|(&c, p)| (c, (p.dim, p.region))).collect();
        let mut forget: Vec<(ConnId, i32)> = Vec::new();
        for (dim, d) in self.dims.iter_mut().enumerate() {
            for r in d.regions.iter_mut() {
                let rid = r.id();
                for e in r.part_mut().0.list.iter_mut().filter(|e| !e.seen_by.is_empty()) {
                    let id = e.id;
                    e.seen_by.retain(|v| match home.get(v) {
                        Some(&(d2, r2)) if d2 == dim && r2 == rid => true,
                        Some(_) => {
                            forget.push((*v, id));
                            false
                        }
                        None => false,
                    });
                }
            }
        }
        forget.sort_unstable();
        for (viewer, id) in forget {
            if let Some(v) = self.players.get_mut(&viewer) {
                v.send(entity::remove_entities(&[id]));
            }
        }
    }

    /// Ends pairings between players now in different regions (one was teleported away):

    /// tracking only runs within a region, so the viewer forgets the entity.
    pub(crate) fn drop_cross_region_pairs(&mut self) {
        let region: HashMap<ConnId, ((crate::DimId, kiln_region::RegionId), i32)> =
            self.players.iter().map(|(&c, p)| (c, ((p.dim, p.region), p.entity_id))).collect();
        let mut forget: Vec<(ConnId, i32)> = Vec::new();
        for (&conn, p) in self.players.iter_mut() {
            let (mine, id) = region[&conn];
            p.seen_by.retain(|v| {
                let same = region.get(v).is_some_and(|&(r, _)| r == mine);
                if !same {
                    forget.push((*v, id));
                }
                same
            });
        }
        forget.sort_unstable();
        for (viewer, id) in forget {
            if let Some(v) = self.players.get_mut(&viewer) {
                v.send(entity::remove_entities(&[id]));
            }
        }
    }

}

/// Recomputes who sees whom among one region's players (sorted by connection), with
/// vanilla's tracking triggers (`ChunkMap.tick`): a player whose section changed has its
/// viewers re-evaluated against everyone, and every player re-evaluates its pairing with
/// the viewers whose section changed. Nothing else changes who sees whom. Players of other
/// regions are too far away to track.
pub(crate) fn update_visibility(players: &mut [&mut Player], ctx: &Ctx<'_>) -> Vec<ConnId> {
    struct Snap {
        conn: ConnId,
        x: f64,
        z: f64,
        chunk: ChunkPos,
        /// Viewer's tracking radius in blocks and its view distance in chunks.
        range: f64,
        view: i32,
        moved: bool,
    }
    let range_cap = PLAYER.tracking_range as f64 * 16.0;
    let snaps: Vec<Snap> = players
        .iter_mut()
        .map(|p| {
            let block = p.pos.map(|c| c.floor() as i32);
            let section = block.map(|c| c >> 4);
            let moved = p.section != Some(section);
            p.section = Some(section);
            Snap {
                conn: p.conn,
                x: p.pos[0],
                z: p.pos[2],
                chunk: ChunkPos::of_block(block[0], block[2]),
                range: range_cap.min(p.view_distance as f64 * 16.0),
                view: p.view_distance,
                moved,
            }
        })
        .collect();
    let movers: Vec<&Snap> = snaps.iter().filter(|s| s.moved).collect();
    let mover_conns: Vec<ConnId> = movers.iter().map(|s| s.conn).collect();
    if movers.is_empty() {
        return mover_conns;
    }

    // Vanilla `updatePlayer`: within the entity's tracking range and the viewer's view
    // distance, and in a chunk inside the viewer's chunk view.
    let sees = |v: &Snap, t: &Snap| {
        let (dx, dz) = (v.x - t.x, v.z - t.z);
        v.conn != t.conn
            && dx * dx + dz * dz <= v.range * v.range
            && (t.chunk.x - v.chunk.x).abs() <= v.view
            && (t.chunk.z - v.chunk.z).abs() <= v.view
    };
    // Each player's viewer changes depend only on the snapshots and its own viewer list, so
    // the players split into windows; the changes apply below in connection order.
    let seen: Vec<&[ConnId]> = players.iter().map(|p| p.seen_by.as_slice()).collect();
    let targets: Vec<usize> = (0..snaps.len()).collect();
    let diffs = ctx.map_indexed_with(VISIBILITY_WINDOW, &targets, |_, &ti| {
        let (t, seen) = (&snaps[ti], seen[ti]);
        let (added, removed) = if t.moved {
            let want: Vec<ConnId> = snaps.iter().filter(|v| sees(v, t)).map(|v| v.conn).collect();
            if seen[..] == want[..] {
                return None;
            }
            sorted_diff(&want, seen)
        } else {
            let (mut added, mut removed) = (Vec::new(), Vec::new());
            for v in &movers {
                match (sees(v, t), seen.binary_search(&v.conn).is_ok()) {
                    (true, false) => added.push(v.conn),
                    (false, true) => removed.push(v.conn),
                    _ => {}
                }
            }
            if added.is_empty() && removed.is_empty() {
                return None;
            }
            (added, removed)
        };
        Some((ti, added, removed))
    });
    drop(seen);
    let changes: Vec<(usize, Vec<ConnId>, Vec<ConnId>)> = diffs.into_iter().flatten().collect();
    let index = |conn: ConnId| snaps.binary_search_by_key(&conn, |s| s.conn).ok();
    for (ti, added, removed) in changes {
        let (spawn, id) = (players[ti].spawn_packets(), players[ti].entity_id);
        let despawn = entity::remove_entities(&[id]);
        for &v in &added {
            if let Some(i) = index(v) {
                for pkt in &spawn {
                    players[i].send(pkt.clone());
                }
            }
        }
        for &v in &removed {
            if let Some(i) = index(v) {
                players[i].send(despawn.clone());
            }
        }
        let target = &mut players[ti];
        target.seen_by.retain(|v| removed.binary_search(v).is_err());
        target.seen_by.extend(added);
        target.seen_by.sort_unstable();
    }
    mover_conns
}

/// Streams movement and metadata changes of one region's players (sorted by connection):
/// encoded once per player, shared by its viewers.
///
/// Two windows: each player encodes its own changes (touching only itself), then each run
/// of consecutive viewers collects what the players it sees encoded, in connection order.
/// Every outbox ends up exactly as a serial loop over the players would have left it: the
/// packets of lower connections first, the player's own packets at its turn.
pub(crate) fn broadcast_movement(players: &mut [&mut Player], ctx: &Ctx<'_>) {
    let encoded = ctx.map_mut_with(PLAYER_WINDOW, players, |_, t| encode_movement(t));
    let mut runs: Vec<(usize, &mut [&mut Player])> = Vec::new();
    let mut start = 0;
    for run in players.chunks_mut(VIEWER_RUN) {
        let n = run.len();
        runs.push((start, run));
        start += n;
    }
    let encoded = &encoded[..];
    ctx.map_mut_with(Window::new(), &mut runs, |_, (lo, run)| deliver_movement(*lo, run, encoded));
}

/// Players per viewer run in [`broadcast_movement`]'s delivery window.
const VIEWER_RUN: usize = 32;

/// One player's movement changes: for its viewers, for itself, and who its viewers are.
struct Encoded {
    to_viewers: Vec<Bytes>,
    to_self: Vec<Bytes>,
    viewers: Vec<ConnId>,
}

fn encode_movement(target: &mut Player) -> Encoded {
    let mut to_self = Vec::new();
    let state = target.move_state();
    let mut packets = target.tracker.tick(&state);
    packets.extend(target.equipment_changes());
    // `updateDataBeforeSync`: effect particles, ambience and the flags effects set.
    let effects_dirty = std::mem::take(&mut target.effects_dirty);
    if effects_dirty {
        target.meta_dirty = true;
        target.self_meta_dirty = true;
    }
    // Lying down or getting up: the pose and the bed position go to the player as well.
    let sleep_dirty = std::mem::take(&mut target.sleep.meta_dirty);
    if sleep_dirty {
        let mut d = EntityData::new();
        d.set(data::entity::POSE, &DataValue::Pose(target.pose()));
        d.set(data::living_entity::SLEEPING_POS, &DataValue::OptionalBlockPos(target.sleep.pos));
        to_self.push(entity::set_entity_data(target.entity_id, &d));
    }
    if std::mem::take(&mut target.woke_up) {
        // `ClientboundAnimatePacket.WAKE_UP`, to the player and its viewers.
        let pkt = entity::animate(target.entity_id, 0);
        to_self.push(pkt.clone());
        packets.push(pkt);
    }
    if target.meta_dirty {
        target.meta_dirty = false;
        let mut d = EntityData::new();
        d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(target.shared_flags()));
        d.set(data::entity::POSE, &DataValue::Pose(target.pose()));
        d.set(data::living_entity::LIVING_ENTITY_FLAGS, &DataValue::Byte(target.living_flags()));
        if sleep_dirty {
            d.set(data::living_entity::SLEEPING_POS, &DataValue::OptionalBlockPos(target.sleep.pos));
        }
        if effects_dirty {
            target.effect_data(&mut d);
        }
        if std::mem::take(&mut target.shoulder_dirty) {
            target.shoulder_data(&mut d);
        }
        packets.push(entity::set_entity_data(target.entity_id, &d));
    }
    // What the player's own client needs of its entity data: burning, invisibility and
    // effect particles, and the air supply (`ServerEntity.sendDirtyEntityData` sends to the
    // player too).
    let air_changed = target.air != target.air_sent;
    if std::mem::take(&mut target.self_meta_dirty) || air_changed {
        let mut d = EntityData::new();
        d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(target.shared_flags()));
        if air_changed {
            target.air_sent = target.air;
            d.set(data::entity::AIR_SUPPLY, &DataValue::Int(target.air));
        }
        if effects_dirty {
            target.effect_data(&mut d);
        }
        to_self.push(entity::set_entity_data(target.entity_id, &d));
    }
    if std::mem::take(&mut target.attributes_dirty) {
        let pkt = target.effect_attributes_packet();
        to_self.push(pkt.clone());
        packets.push(pkt);
    }
    packets.append(&mut target.pending_sounds);
    if let Some((damage_type, cause, direct)) = target.damaged.take() {
        packets.push(entity::damage_event(target.entity_id, damage_type, cause, direct, None));
    }
    for event in std::mem::take(&mut target.entity_events) {
        packets.push(entity::entity_event(target.entity_id, event));
    }
    // `ServerEntity.sendChanges` for a hit that was not answered by the attack itself
    // (swept players): the velocity goes to viewers and the player.
    // A `push` (riptide): only the players that see it get the motion.
    if std::mem::take(&mut target.push_sync) {
        packets.push(entity::set_entity_motion(target.entity_id, target.vel));
    }
    if std::mem::take(&mut target.sync_velocity) {
        let motion = entity::set_entity_motion(target.entity_id, target.vel);
        to_self.push(motion.clone());
        packets.push(motion);
    }
    if std::mem::take(&mut target.died) {
        // `EntityEvent.DEATH`: the death animation and sound.
        packets.push(entity::entity_event(target.entity_id, 3));
    }
    if std::mem::take(&mut target.swung) {
        packets.push(entity::swing_animation(target.entity_id, false, target.swing_kind, target.swing_wire_duration));
    }
    let viewers = if packets.is_empty() { Vec::new() } else { target.seen_by.clone() };
    Encoded { to_viewers: packets, to_self, viewers }
}

/// Hands the viewers `run` (players `lo..lo + run.len()`) what every player they see encoded,
/// in connection order, and each viewer its own packets at its turn.
fn deliver_movement(lo: usize, run: &mut [&mut Player], encoded: &[Encoded]) {
    let (Some(first), Some(last)) = (run.first().map(|p| p.conn), run.last().map(|p| p.conn)) else { return };
    for (ti, e) in encoded.iter().enumerate() {
        if (lo..lo + run.len()).contains(&ti) {
            run[ti - lo].outbox.extend(e.to_self.iter().cloned());
        }
        if e.to_viewers.is_empty() {
            continue;
        }
        let from = e.viewers.partition_point(|&v| v < first);
        let to = e.viewers.partition_point(|&v| v <= last);
        let mut j = 0;
        for &v in &e.viewers[from..to] {
            while j < run.len() && run[j].conn < v {
                j += 1;
            }
            if j < run.len() && run[j].conn == v {
                run[j].outbox.extend(e.to_viewers.iter().cloned());
            }
        }
    }
}

/// (in `want` but not `have`, in `have` but not `want`) for two sorted lists.
fn sorted_diff(want: &[ConnId], have: &[ConnId]) -> (Vec<ConnId>, Vec<ConnId>) {
    let (mut added, mut removed) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < want.len() || j < have.len() {
        match (want.get(i), have.get(j)) {
            (Some(a), Some(b)) if a == b => {
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a < b => {
                added.push(*a);
                i += 1;
            }
            (Some(_), Some(b)) => {
                removed.push(*b);
                j += 1;
            }
            (Some(a), None) => {
                added.push(*a);
                i += 1;
            }
            (None, Some(b)) => {
                removed.push(*b);
                j += 1;
            }
            (None, None) => unreachable!(),
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::sorted_diff;

    #[test]
    fn diff_of_sorted_viewer_lists() {
        assert_eq!(sorted_diff(&[1, 3, 5, 7], &[2, 3, 7, 9]), (vec![1, 5], vec![2, 9]));
        assert_eq!(sorted_diff(&[], &[4]), (vec![], vec![4]));
        assert_eq!(sorted_diff(&[4], &[4]), (vec![], vec![]));
    }
}

/// Main hand, off hand and the four armor slots (players have no body or saddle slot).
const SHOWN_SLOTS: usize = 6;

/// `ClientboundSetEquipmentPacket`: (slot ordinal, stack) pairs, each slot byte flagged 0x80
/// when another follows.
pub(crate) fn set_equipment(entity_id: i32, slots: &[(u8, &kiln_item::ItemStack)]) -> Bytes {
    use bytes::BufMut;
    use kiln_proto::WriteExt;
    let mut b = bytes::BytesMut::new();
    b.put_varint(kiln_data::packets::play::clientbound::SET_EQUIPMENT);
    b.put_varint(entity_id);
    for (i, (slot, stack)) in slots.iter().enumerate() {
        b.put_u8(if i + 1 < slots.len() { slot | 0x80 } else { *slot });
        stack.write_optional(&mut b);
    }
    b.freeze()
}
