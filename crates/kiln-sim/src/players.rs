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
        let all_props: Vec<Vec<ProfileProperty>> = self.players.values().map(|p| p.profile_properties()).collect();
        let entries: Vec<PlayerInfoEntry> =
            self.players.values().zip(&all_props).map(|(p, props)| p.info_entry(props)).collect();
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

    /// Recomputes who sees whom.
    pub(crate) fn update_visibility(&mut self) {
        struct Snap {
            conn: ConnId,
            x: f64,
            z: f64,
            chunk: ChunkPos,
            /// Viewer's tracking radius in blocks and its view distance in chunks.
            range: f64,
            view: i32,
        }
        let range_cap = PLAYER.tracking_range as f64 * 16.0;
        // Sorted by connection so the wanted viewer lists come out sorted, like `seen_by`.
        let mut snaps: Vec<Snap> = self
            .players
            .iter()
            .map(|(&conn, p)| Snap {
                conn,
                x: p.pos[0],
                z: p.pos[2],
                chunk: ChunkPos::of_block(p.pos[0].floor() as i32, p.pos[2].floor() as i32),
                range: range_cap.min(p.view_distance as f64 * 16.0),
                view: p.view_distance,
            })
            .collect();
        snaps.sort_unstable_by_key(|s| s.conn);

        // Which viewers should see each player (vanilla: within the entity's tracking range
        // and the viewer's view distance, and in a chunk inside the viewer's chunk view).
        let mut changes: Vec<(ConnId, Vec<ConnId>, Vec<ConnId>)> = Vec::new();
        let mut want: Vec<ConnId> = Vec::with_capacity(snaps.len());
        for t in &snaps {
            want.clear();
            for v in &snaps {
                if v.conn == t.conn {
                    continue;
                }
                let (dx, dz) = (v.x - t.x, v.z - t.z);
                if dx * dx + dz * dz <= v.range * v.range
                    && (t.chunk.x - v.chunk.x).abs() <= v.view
                    && (t.chunk.z - v.chunk.z).abs() <= v.view
                {
                    want.push(v.conn);
                }
            }
            let seen = &self.players[&t.conn].seen_by;
            if seen[..] == want[..] {
                continue;
            }
            let (added, removed) = sorted_diff(&want, seen);
            changes.push((t.conn, added, removed));
        }
        for (t, added, removed) in changes {
            let (spawn, id) = {
                let target = &self.players[&t];
                (target.spawn_packets(), target.entity_id)
            };
            let despawn = entity::remove_entities(&[id]);
            for v in &added {
                if let Some(p) = self.players.get_mut(v) {
                    for pkt in &spawn {
                        p.send(pkt.clone());
                    }
                }
            }
            for v in &removed {
                if let Some(p) = self.players.get_mut(v) {
                    p.send(despawn.clone());
                }
            }
            let target = self.players.get_mut(&t).unwrap();
            target.seen_by.retain(|v| removed.binary_search(v).is_err());
            target.seen_by.extend(added);
            target.seen_by.sort_unstable();
        }
    }

    /// Streams movement and metadata changes: encoded once per player, shared by its viewers.
    pub(crate) fn broadcast_movement(&mut self) {
        let ids: Vec<ConnId> = self.players.keys().copied().collect();
        for &t in &ids {
            let (packets, viewers) = {
                let target = self.players.get_mut(&t).unwrap();
                let state = target.move_state();
                let mut packets = target.tracker.tick(&state);
                if target.meta_dirty {
                    target.meta_dirty = false;
                    let mut d = EntityData::new();
                    d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(target.shared_flags()));
                    d.set(data::entity::POSE, &DataValue::Pose(target.pose()));
                    packets.push(entity::set_entity_data(target.entity_id, &d));
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
                (packets, target.seen_by.iter().copied().collect::<Vec<_>>())
            };
            for v in viewers {
                if let Some(p) = self.players.get_mut(&v) {
                    for pkt in &packets {
                        p.send(pkt.clone());
                    }
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
