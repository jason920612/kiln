//! Riding stacks that move as a whole: `Entity.teleport` (the `/tp` command on an entity, the
//! stack of a portal trip) and `stopRiding` for the players that teleport on their own.
//!
//! A stack is an entity with everything that rides it, and what rides that, players included.
//! Teleported inside a level the stack keeps its ids and its links (a player's open menu on a
//! chest minecart keeps working), whichever regions the two places belong to: the entities move
//! from the list of one region to the list of the region owning the destination. Teleported to
//! another level (`teleportCrossDimension`) the entities come back as new ones, saved and
//! loaded again with their riders, and the players aboard change level and sit down again.

use crate::{DimId, Sim, entities};
use kiln_link::ConnId;

use kiln_proto::nbt::Tag;
use tracing::{info, warn};

/// An entity with its riders: what `Entity.teleport` moves together.
pub(crate) struct Stack {
    /// The entities (not players), the root first, then each one's riders depth first.
    pub entities: Vec<i32>,
    /// The players riding it somewhere: (connection, id of the entity it sits on).
    pub players: Vec<(ConnId, i32)>,
}

fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(fields) = tag {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => fields.push((key.to_owned(), value)),
        }
    }
}

impl Sim {
    /// The entity `id` of level `dim`.
    fn entity_in(&self, dim: DimId, id: i32) -> Option<&entities::Entity> {
        self.dims[dim].regions.iter().find_map(|r| {
            let list = &r.part().0.list;
            let i = list.binary_search_by_key(&id, |e| e.id).ok()?;
            Some(&list[i]).filter(|e| !e.removed)
        })
    }

    fn entity_in_mut(&mut self, dim: DimId, id: i32) -> Option<&mut entities::Entity> {
        self.dims[dim].regions.iter_mut().find_map(|r| {
            let list = &mut r.part_mut().0.list;
            let i = list.binary_search_by_key(&id, |e| e.id).ok()?;
            Some(&mut list[i]).filter(|e| !e.removed)
        })
    }

    /// Whether anything (a player included) rides entity `id` of level `dim`.
    pub(crate) fn entity_has_riders(&self, dim: DimId, id: i32) -> bool {
        self.entity_in(dim, id).and_then(|e| e.phys.as_deref()).is_some_and(|p| !p.passengers.is_empty())
    }

    /// `root` and everything that rides it, in level `dim`.

    pub(crate) fn stack_of(&self, dim: DimId, root: i32) -> Stack {
        let mut stack = Stack { entities: Vec::new(), players: Vec::new() };
        let mut todo = vec![root];
        while let Some(id) = todo.pop() {
            let Some(e) = self.entity_in(dim, id) else { continue };
            stack.entities.push(id);
            let riders = e.phys.as_deref().map(|p| p.passengers.clone()).unwrap_or_default();
            // Depth first, in seat order.
            for &r in riders.iter().rev() {
                match self.players.iter().find(|(_, p)| p.dim == dim && p.entity_id == r) {
                    Some((&conn, _)) => stack.players.push((conn, id)),
                    None => todo.push(r),
                }
            }
        }
        stack.players.sort_unstable();
        stack
    }

    /// `Entity.stopRiding` for the entity `id` of level `dim`: it leaves its vehicle.
    pub(crate) fn stop_riding_entity(&mut self, dim: DimId, id: i32) {
        let Some(e) = self.entity_in_mut(dim, id) else { return };
        let Some(vehicle) = e.phys.as_deref_mut().and_then(|p| p.vehicle.take()) else { return };
        if let Some(vp) = self.entity_in_mut(dim, vehicle).and_then(|v| v.phys.as_deref_mut()) {
            kiln_entity::ride::remove_passenger(vp, id);
        }
    }

    /// `Entity.stopRiding` for a player: it leaves what it rides where it is (the caller moves
    /// it). The mount's passenger list lets go of it, in whatever region the mount is.
    pub(crate) fn stop_riding_player(&mut self, conn: ConnId) {
        let Some(p) = self.players.get_mut(&conn) else { return };
        let (Some(vehicle), dim, pid) = (p.vehicle.take(), p.dim, p.entity_id) else { return };
        p.vehicle_type = None;
        p.dismount_on_teleport = false;
        if let Some(vp) = self.entity_in_mut(dim, vehicle).and_then(|v| v.phys.as_deref_mut()) {
            kiln_entity::ride::remove_passenger(vp, pid);
        }
    }

    /// `Entity.teleport` of the entity `id` of level `from` (not a player) to `pos` in level `to`:
    /// it gets off what it rides, and its riders go with it.
    pub(crate) fn teleport_entity(&mut self, from: DimId, id: i32, to: DimId, pos: [f64; 3], rot: Option<[f32; 2]>) {
        if self.entity_in(from, id).is_none() {
            return;
        }
        self.stop_riding_entity(from, id);
        if from != to {
            self.stack_changes_level(from, id, to, pos, rot, None);
            return;
        }
        // The destination's chunk (and its region) first.
        use kiln_world::spawn::LoadChunks;
        let chunk = entities::chunk_of(pos);

        self.dims[to].load_chunk(chunk);
        self.apply_topology();
        self.update_membership(true);
        let stack = self.stack_of(from, id);
        let now = self.game_time;
        // Out of whatever regions they are in...
        let mut taken: Vec<entities::Entity> = Vec::with_capacity(stack.entities.len());
        for r in self.dims[from].regions.iter_mut() {
            let list = &mut r.part_mut().0.list;
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(list).into_iter().partition(|e| stack.entities.contains(&e.id));
            *list = rest;
            taken.extend(mine);
        }
        // ...set down at the destination, riders in their seats...
        let order = |taken: &[entities::Entity], id: i32| taken.iter().position(|e| e.id == id);
        if let Some(i) = order(&taken, id) {
            taken[i].relocate(pos, rot);
        }
        for &vehicle in &stack.entities {
            let Some(vi) = order(&taken, vehicle) else { continue };
            let Some(vp) = taken[vi].phys.clone() else { continue };
            for (k, &rider) in vp.passengers.iter().enumerate() {
                if let Some(ri) = order(&taken, rider) {
                    // `positionRider`, then it stands there like the rest (still, on its seat).
                    let Some(rp) = taken[ri].phys.as_deref_mut() else { continue };
                    kiln_entity::ride::position_rider(rp, &vp);
                    let at = rp.position();
                    taken[ri].relocate([at.x, at.y, at.z], None);
                } else if let Some((_, p)) = self.players.iter_mut().find(|(_, p)| p.entity_id == rider && p.dim == from) {
                    let seat = kiln_entity::ride::rider_position(&vp, k, "minecraft:player", 1.0);
                    let rot = p.rot;
                    p.teleport([seat.x, seat.y, seat.z], rot, now);
                    p.fall_distance = 0.0;
                    p.vel = [0.0; 3];
                }
            }
        }
        // ...and into the region that owns it, in id order, with their viewers asked to look again.
        taken.sort_unstable_by_key(|e| e.id);
        let mut gone = Vec::new();
        let region = self.dims[to].regions.at_mut(chunk.cell());
        let Some(region) = region else {
            warn!("teleported entity {id}: nowhere to put it at {pos:?}");
            return;
        };
        let home = taken.iter().find(|e| e.id == id).map(|e| e.cell);
        for mut e in taken {
            gone.push((e.id, e.forget_viewers()));
            // The cell an entity is routed by is one of the region's.
            if !region.cells().contains(e.cell)
                && let Some(c) = home.filter(|c| region.cells().contains(*c))
            {
                e.cell = c;
            }
            let list = &mut region.part_mut().0.list;
            let at = list.partition_point(|x| x.id < e.id);
            list.insert(at, e);
        }
        self.forget_entities(gone);
        // Players that rode along are in the region of their seat.
        self.update_membership(true);
    }

    /// `teleportCrossDimension` for the stack rooted at `root`: it leaves level `from` and
    /// arrives in `to` at `pos` as new entities (ids, but not UUIDs, are new), the players aboard
    /// changing level and sitting down again. `cooldown`: portal trips; the arrivals do not use
    /// a portal until it is over.
    pub(crate) fn stack_changes_level(&mut self, from: DimId, root: i32, to: DimId, pos: [f64; 3], rot: Option<[f32; 2]>, cooldown: Option<i64>) {
        let stack = self.stack_of(from, root);
        let Some(mut tag) = self.entity_with_passengers(from, root) else { return };
        put(&mut tag, "Pos", Tag::List(pos.iter().map(|&c| Tag::Double(c)).collect()));
        if let Some([yaw, pitch]) = rot {
            put(&mut tag, "Rotation", Tag::List(vec![Tag::Float(yaw), Tag::Float(pitch)]));
        }
        let root_uuid = self.entity_in(from, root).map_or(0, |e| e.uuid.as_u128());
        let spawn = match entities::Spawn::from_saved(&tag, entities::seed_for_uuid(root_uuid), false) {
            Ok(s) => s,
            Err(e) => {
                warn!("the stack of entity {root} cannot change level: {e:?}");
                return;
            }
        };
        let uuids: Vec<u128> = stack.entities.iter().filter_map(|&id| self.entity_in(from, id)).map(|e| e.uuid.as_u128()).collect();
        let sitting: Vec<(ConnId, u128)> =
            stack.players.iter().filter_map(|&(c, v)| self.entity_in(from, v).map(|e| (c, e.uuid.as_u128()))).collect();
        // The old entities go (their viewers are told).
        let mut gone = Vec::new();
        for r in self.dims[from].regions.iter_mut() {
            let list = &mut r.part_mut().0.list;
            list.retain_mut(|e| {
                if stack.entities.contains(&e.id) {
                    gone.push((e.id, std::mem::take(&mut e.seen_by)));
                    false
                } else {
                    true
                }
            });
        }
        self.forget_entities(gone);
        self.load_area(to, kiln_blocks::BlockPos::new(pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32), 1);
        if let Some(until) = cooldown {
            for u in uuids {
                self.dims[to].portal_cooldowns.insert(u, until);
            }
        }
        self.dims[to].spawns.push(spawn);
        info!("a stack of {} entities went from {} to {} at {pos:?}", stack.entities.len(), crate::DIMENSIONS[from].0, crate::DIMENSIONS[to].0);
        // The riders get off here and are sat down on what arrives.
        for (conn, attach) in sitting {
            let Some(p) = self.players.get_mut(&conn) else { continue };
            p.vehicle = None;
            p.vehicle_type = None;
            p.dismount_on_teleport = false;
            let rot = rot.unwrap_or(p.rot);
            self.change_dimension(conn, to, pos, rot);
            if let Some(p) = self.players.get_mut(&conn) {
                p.returning_vehicle = Some(crate::persist::ReturningVehicle::spawned(attach, tag.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{Client, join};
    use crate::{Sim, SimConfig};
    use kiln_link::ToSim;

    /// A chorus fruit or an ender pearl teleports a rider (`Entity.teleport` stops the riding):
    /// it stays where it went, the mount lets go of it.
    #[test]
    fn a_rider_that_teleports_itself_gets_off() {
        let mut sim = Sim::new(SimConfig::new(2, 4, None));
        let (msg, stats) = join(1, "Rider", 2);
        let cmds = ["gamemode creative Rider", "gamerule minecraft:spawn_mobs false"];
        assert!(sim.step(std::iter::once(msg).chain(cmds.iter().map(|c| ToSim::Console((*c).into())))));
        let mut client = Client::new(1, stats);
        let mut settle = |sim: &mut Sim, n: usize| {
            for _ in 0..n {
                let mut inbox = Vec::new();
                client.tick(None, &mut inbox);
                assert!(sim.step(inbox));
            }
        };
        settle(&mut sim, 5);
        let at = sim.players[&1].pos;
        assert!(sim.step([ToSim::Console(format!("summon minecraft:oak_boat {} {} {}", at[0] + 1.0, at[1], at[2]))]));
        settle(&mut sim, 2);
        assert!(sim.step([ToSim::Console("ride Rider mount @e[type=minecraft:oak_boat,limit=1]".into())]));
        settle(&mut sim, 3);
        let boat = sim.entity_ids_of("minecraft:oak_boat")[0];
        assert_eq!(sim.vehicle_of(1), Some(boat));
        // The region's player tick moved it by itself, as chorus fruit does.
        let to = [at[0] + 9.0, at[1], at[2] + 9.0];
        {
            let p = sim.players.get_mut(&1).unwrap();
            p.dismount_on_teleport = true;
            p.teleport(to, [0.0, 0.0], 100);
        }
        settle(&mut sim, 3);
        assert_eq!(sim.vehicle_of(1), None, "got off");
        let riders = sim.riding().into_iter().find(|r| r.0 == boat).unwrap().2;
        assert!(riders.is_empty(), "the boat let go of it: {riders:?}");
        let now = sim.players[&1].pos;
        assert!((now[0] - to[0]).abs() < 0.5 && (now[2] - to[2]).abs() < 0.5, "it stayed where it went: {now:?}");
    }
}

