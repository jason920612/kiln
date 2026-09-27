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
        if self.sneaking {
            f |= shared_flags::CROUCHING;
        }
        if self.sprinting {
            f |= shared_flags::SPRINTING;
        }
        f as i8
    }

    fn pose(&self) -> i32 {
        if self.sneaking { pose::CROUCHING } else { pose::STANDING }
    }

    /// Entity data a new viewer needs (fields that differ from their defaults).
    fn spawn_metadata(&self) -> EntityData {
        let mut d = EntityData::new();
        d.set(data::living_entity::HEALTH, &DataValue::Float(20.0));
        d.set(data::avatar::PLAYER_MODE_CUSTOMISATION, &DataValue::Byte(self.client.skin_parts as i8));
        if self.client.main_hand == 0 {
            d.set(data::avatar::PLAYER_MAIN_HAND, &DataValue::HumanoidArm(HumanoidArm::Left));
        }
        if self.shared_flags() != 0 {
            d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(self.shared_flags()));
            d.set(data::entity::POSE, &DataValue::Pose(self.pose()));
        }
        if self.living_flags() != 0 {
            d.set(data::living_entity::LIVING_ENTITY_FLAGS, &DataValue::Byte(self.living_flags()));
        }
        d
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

    fn spawn_packets(&self) -> [Bytes; 4] {
        [
            entity::bundle_delimiter(),
            self.tracker.spawn(self.uuid, PLAYER.id, [0.0; 3], 0),
            entity::set_entity_data(self.entity_id, &self.spawn_metadata()),
            entity::bundle_delimiter(),
        ]
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

    /// Ends pairings between players now in different regions (one was teleported away):
    /// tracking only runs within a region, so the viewer forgets the entity.
    pub(crate) fn drop_cross_region_pairs(&mut self) {
        let region: HashMap<ConnId, (kiln_region::RegionId, i32)> =
            self.players.iter().map(|(&c, p)| (c, (p.region, p.entity_id))).collect();
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
pub(crate) fn update_visibility(players: &mut [&mut Player]) -> Vec<ConnId> {
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
    let mut changes: Vec<(usize, Vec<ConnId>, Vec<ConnId>)> = Vec::new();
    let mut want: Vec<ConnId> = Vec::with_capacity(snaps.len());
    for (ti, t) in snaps.iter().enumerate() {
        let seen = &players[ti].seen_by;
        let (added, removed) = if t.moved {
            want.clear();
            want.extend(snaps.iter().filter(|v| sees(v, t)).map(|v| v.conn));
            if seen[..] == want[..] {
                continue;
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
                continue;
            }
            (added, removed)
        };
        changes.push((ti, added, removed));
    }
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
pub(crate) fn broadcast_movement(players: &mut [&mut Player]) {
    for ti in 0..players.len() {
        let target = &mut players[ti];
        let state = target.move_state();
        let mut packets = target.tracker.tick(&state);
        if target.meta_dirty {
            target.meta_dirty = false;
            let mut d = EntityData::new();
            d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(target.shared_flags()));
            d.set(data::entity::POSE, &DataValue::Pose(target.pose()));
            d.set(data::living_entity::LIVING_ENTITY_FLAGS, &DataValue::Byte(target.living_flags()));
            packets.push(entity::set_entity_data(target.entity_id, &d));
        }
        if let Some(damage_type) = target.damaged.take() {
            packets.push(entity::damage_event(target.entity_id, damage_type, None, None, None));
        }
        if std::mem::take(&mut target.died) {
            // `EntityEvent.DEATH`: the death animation and sound.
            packets.push(entity::entity_event(target.entity_id, 3));
        }
        if std::mem::take(&mut target.swung) {
            packets.push(entity::swing_animation(
                target.entity_id,
                false,
                entity::swing::WHACK,
                entity::swing::DEFAULT_DURATION,
            ));
        }
        if packets.is_empty() || target.seen_by.is_empty() {
            continue;
        }
        let viewers = target.seen_by.clone();
        for v in viewers {
            if let Ok(i) = players.binary_search_by_key(&v, |p| p.conn) {
                for pkt in &packets {
                    players[i].send(pkt.clone());
                }
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
