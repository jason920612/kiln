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
use std::collections::HashSet;

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
            p.seen_by.remove(&conn);
        }
        self.broadcast(entity::player_info_remove(&[gone.uuid]));
    }

    /// Recomputes who sees whom, then streams movement and metadata changes to viewers.
    pub(crate) fn update_tracking(&mut self) {
        let range_cap = PLAYER.tracking_range as f64 * 16.0;
        let ids: Vec<ConnId> = self.players.keys().copied().collect();

        // Which viewers should see each player (vanilla: within the entity's tracking range
        // and the viewer's view distance, and in a chunk the viewer has loaded).
        let mut changes: Vec<(ConnId, Vec<ConnId>, Vec<ConnId>)> = Vec::new();
        for &t in &ids {
            let target = &self.players[&t];
            let chunk = ChunkPos::of_block(target.pos[0].floor() as i32, target.pos[2].floor() as i32);
            let mut want = HashSet::new();
            for &v in &ids {
                if v == t {
                    continue;
                }
                let viewer = &self.players[&v];
                let r = range_cap.min(viewer.view_distance as f64 * 16.0);
                let (dx, dz) = (viewer.pos[0] - target.pos[0], viewer.pos[2] - target.pos[2]);
                if dx * dx + dz * dz <= r * r && viewer.sent_chunks.contains(&chunk) {
                    want.insert(v);
                }
            }
            let added: Vec<ConnId> = want.difference(&target.seen_by).copied().collect();
            let removed: Vec<ConnId> = target.seen_by.difference(&want).copied().collect();
            if !added.is_empty() || !removed.is_empty() {
                changes.push((t, added, removed));
            }
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
            for v in added {
                target.seen_by.insert(v);
            }
            for v in removed {
                target.seen_by.remove(&v);
            }
        }

        // Movement and metadata: encoded once per player, shared by all its viewers.
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
