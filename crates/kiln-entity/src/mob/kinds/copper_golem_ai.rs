//! `TransportItemsBetweenContainers` as the copper golem's `CopperGolemAi` configures it: speed
//! 1.0, copper chests as the source and (trapped) chests as the destination, 32 blocks around
//! and 8 up and down; it walks to a chest, stands at it 60 ticks (opening it on the first, a
//! sound on the ninth, closing it on the sixtieth) and then takes up to 16 items from a copper
//! chest, or puts what it holds into a chest that has the same item or nothing.

use super::chest_access;
use super::copper_golem::{self, st_mut};
use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::Event;
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::mob::brain::memory::{GlobalPos, Tracker};
use crate::mob::brain::persist::OVERWORLD;
use crate::mob::brain::{Behavior, Cx, Mem, Status, Val, util};
use crate::mob::path;
use crate::mob::{self, MAINHAND};
use kiln_item::ItemStack;

/// `TransportItemsBetweenContainers.TransportItemState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TravelState {
    Travelling,
    Queuing,
    Interacting,
}

/// `TransportItemsBetweenContainers.ContainerInteractionState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interaction {
    PickupItem,
    PickupNoItem,
    PlaceItem,
    PlaceNoItem,
}

/// `TransportItemTarget`: the chest block entity at `pos` (`serial` tells it from a new one in
/// the same place), its state when found, and the halves of its container.
#[derive(Clone, Debug)]
struct Target {
    pos: BlockPos,
    state: u16,
    serial: u64,
    halves: Vec<BlockPos>,
}

#[derive(Clone, Debug)]
pub struct TransportItemsBetweenContainers {
    target: Option<Target>,
    state: TravelState,
    interaction: Option<Interaction>,
    ticks_since_reaching_target: i32,
}

/// `speedModifier`, `horizontalSearchDistance`, `verticalSearchDistance`.
const SPEED: f32 = 1.0;
const HORIZONTAL: i32 = 32;
const VERTICAL: i32 = 8;

impl TransportItemsBetweenContainers {
    pub fn new() -> TransportItemsBetweenContainers {
        TransportItemsBetweenContainers { target: None, state: TravelState::Travelling, interaction: None, ticks_since_reaching_target: 0 }
    }

    /// `isPickingUpItems`: nothing in the main hand.
    fn picking_up(cx: &Cx) -> bool {
        cx.m.equipment[MAINHAND].is_empty()
    }

    /// `isWantedBlock`: a copper chest to pick up from, a chest or trapped chest to put down in.
    fn is_wanted_block(cx: &Cx, state: u16) -> bool {
        if Self::picking_up(cx) {
            super::wolf::block_in_tag(state, "minecraft:copper_chests")
        } else {
            matches!(crate::blocks::block_name(state), "minecraft:chest" | "minecraft:trapped_chest")
        }
    }

    /// `TransportItemTarget.tryCreatePossibleTarget(pos, level)`.
    fn try_create(cx: &Cx, pos: BlockPos) -> Option<Target> {
        let serial = cx.level.block_entity_serial(pos)?;
        let state = cx.level.block(pos);
        let halves = if chest_access::is_chest_block(state) { chest_access::combine(&*cx.level, pos, state)? } else { return None };
        Some(Target { pos, state, serial, halves })
    }

    /// `getConnectedTargets`.
    fn connected(cx: &Cx, t: &Target) -> Vec<Target> {
        if chest_access::double_half(t.state).is_none() {
            return vec![t.clone()];
        }
        match Self::try_create(cx, chest_access::connected_pos(t.pos, t.state)) {
            Some(o) => vec![t.clone(), o],
            None => vec![t.clone()],
        }
    }

    /// `ChestBlockEntity.getEntitiesWithContainerOpen` for the chest at `pos`: the level's, and the
    /// golem itself (its data is out of its entity while it thinks).
    fn users(cx: &Cx, pos: BlockPos) -> Vec<i32> {
        let mut v = cx.level.container_users(pos);
        if super::copper_golem::has_container_open(cx.m, &*cx.level, pos) && !v.contains(&cx.e.id) {
            v.push(cx.e.id);
        }
        v
    }

    /// `isAnotherMobInteractingWithTarget`: a chest of the target has something open.
    fn another_mob_interacting(cx: &Cx, t: &Target) -> bool {
        Self::connected(cx, t).iter().any(|c| !Self::users(cx, c.pos).is_empty())
    }

    /// `getCenterPos`.
    fn center_pos(cx: &Cx) -> Vec3 {
        let bb = cx.e.bounding_box();
        cx.e.position().add(0.0, bb.y_size() / 2.0, 0.0)
    }

    /// `hasFinishedPath`.
    fn has_finished_path(cx: &Cx) -> bool {
        cx.m.nav.path.as_ref().is_some_and(|p| p.is_done())
    }

    /// `getInteractionRange`.
    fn interaction_range(cx: &Cx) -> f64 {
        if Self::has_finished_path(cx) { 1.0 } else { 0.5 }
    }

    /// `isWithinTargetDistance`: the mob's box, centred on `pos`, meets the chest's collision
    /// box grown by `range` sideways and half a block up and down.
    fn within_distance(cx: &Cx, range: f64, t: &Target, pos: Vec3) -> bool {
        let bb = cx.e.bounding_box();
        let (xs, ys, zs) = (bb.x_size(), bb.y_size(), bb.z_size());
        let mine = Aabb::new(pos.x - xs / 2.0, pos.y - ys / 2.0, pos.z - zs / 2.0, pos.x + xs / 2.0, pos.y + ys / 2.0, pos.z + zs / 2.0);
        let (shape, _) = crate::collision::collision_shape(t.state, t.pos, &crate::collision::CollisionContext::EMPTY);
        let boxes = shape.boxes();
        if boxes.is_empty() {
            return false;
        }
        let bounds = boxes.iter().fold(None::<Aabb>, |acc, b| {
            Some(match acc {
                None => *b,
                Some(a) => Aabb::new(a.min_x.min(b.min_x), a.min_y.min(b.min_y), a.min_z.min(b.min_z), a.max_x.max(b.max_x), a.max_y.max(b.max_y), a.max_z.max(b.max_z)),
            })
        });
        let Some(bounds) = bounds else { return false };
        bounds.inflate(range, 0.5, range).offset(t.pos.x as f64, t.pos.y as f64, t.pos.z as f64).intersects(&mine)
    }

    /// `canSeeAnyTargetSide`: a ray from `pos` to the middle of some side of the chest block
    /// meets the chest first.
    fn can_see_any_side(cx: &Cx, t: &Target, pos: Vec3) -> bool {
        let center = Vec3::new(t.pos.x as f64 + 0.5, t.pos.y as f64 + 0.5, t.pos.z as f64 + 0.5);
        let ctx = cx.e.collision_context();
        Direction::ALL.iter().any(|d| {
            let (sx, sy, sz) = d.step();
            let to = center.add(0.5 * sx as f64, 0.5 * sy as f64, 0.5 * sz as f64);
            let hit = crate::clip::traverse_blocks(pos, to, |p| {
                let s = cx.level.block(p);
                let (shape, _) = crate::collision::collision_shape(s, p, &ctx);
                crate::clip::shape_clips(&shape, pos, to, p).then_some(p)
            });
            hit == Some(t.pos)
        })
    }

    /// `getPositionToReachTargetFrom`.
    fn position_to_reach_from(cx: &Cx, path: &Option<path::Path>) -> Vec3 {
        let base = match path.as_ref().and_then(|p| p.end_node()) {
            Some(n) => Vec3::new(n.x as f64 + 0.5, n.y as f64, n.z as f64 + 0.5),
            None => cx.e.position(),
        };
        base.add(0.0, cx.e.bounding_box().y_size() / 2.0, 0.0)
    }

    /// `hasValidTravellingPath`.
    fn has_valid_travelling_path(&self, cx: &mut Cx, t: &Target) -> bool {
        let path = match cx.m.nav.path.clone() {
            Some(p) => Some(p),
            None => path::create_path(cx.e, cx.m, &*cx.level, t.pos, 0),
        };
        let pos = Self::position_to_reach_from(cx, &path);
        let within = Self::within_distance(cx, Self::interaction_range(cx), t, pos);
        let no_path_and_not_within = path.is_none() && !within;
        if std::env::var_os("KILN_CG_DEBUG").is_some() {
            eprintln!(
                "cg tick {} target {:?} path {:?} pos {:?} within {} range {} see {}",
                cx.time,
                t.pos,
                path.as_ref().map(|p| (p.next, p.nodes.iter().map(|n| (n.x, n.y, n.z)).collect::<Vec<_>>())),
                pos,
                within,
                Self::interaction_range(cx),
                within && Self::can_see_any_side(cx, t, pos)
            );
        }
        no_path_and_not_within || (within && Self::can_see_any_side(cx, t, pos))
    }

    /// `hasValidTarget`.
    fn has_valid_target(&mut self, cx: &mut Cx) -> bool {
        let Some(t) = self.target.clone() else { return false };
        let valid = Self::is_wanted_block(cx, t.state) && cx.level.block_entity_serial(t.pos) == Some(t.serial);
        if valid && !chest_access::blocked_at(&*cx.level, t.pos) {
            if self.state != TravelState::Travelling {
                return true;
            }
            if self.has_valid_travelling_path(cx, &t) {
                return true;
            }
            self.mark_unreachable(cx, t.pos);
        }
        false
    }

    fn visited(cx: &Cx) -> Vec<GlobalPos> {
        cx.b.mem.positions(Mem::VisitedBlockPositions).to_vec()
    }

    fn unreachable(cx: &Cx) -> Vec<GlobalPos> {
        cx.b.mem.positions(Mem::UnreachableTransportBlockPositions).to_vec()
    }

    /// `setVisitedBlockPos`.
    fn set_visited(&mut self, cx: &mut Cx, pos: BlockPos) {
        let mut v = Self::visited(cx);
        let g = GlobalPos::new(OVERWORLD, pos);
        if !v.contains(&g) {
            v.push(g);
        }
        if v.len() > 10 {
            self.enter_cooldown(cx);
        } else {
            cx.b.mem.set_expiring(Mem::VisitedBlockPositions, Val::Positions(v), 6000);
        }
    }

    /// `markVisitedBlockPosAsUnreachable`.
    fn mark_unreachable(&mut self, cx: &mut Cx, pos: BlockPos) {
        let g = GlobalPos::new(OVERWORLD, pos);
        let mut v = Self::visited(cx);
        v.retain(|p| *p != g);
        let mut u = Self::unreachable(cx);
        if !u.contains(&g) {
            u.push(g);
        }
        if u.len() > 50 {
            self.enter_cooldown(cx);
        } else {
            cx.b.mem.set_expiring(Mem::VisitedBlockPositions, Val::Positions(v), 6000);
            cx.b.mem.set_expiring(Mem::UnreachableTransportBlockPositions, Val::Positions(u), 6000);
        }
    }

    /// `isTargetValidToPick`.
    fn target_valid_to_pick(cx: &Cx, pos: BlockPos, area: &Aabb, visited: &[GlobalPos], unreachable: &[GlobalPos]) -> Option<Target> {
        let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
        // `AABB.contains(x, y, z)`: closed below, open above.
        if !(x >= area.min_x && x < area.max_x && y >= area.min_y && y < area.max_y && z >= area.min_z && z < area.max_z) {
            return None;
        }
        let t = Self::try_create(cx, pos)?;
        let seen = |c: &Target| {
            let g = GlobalPos::new(OVERWORLD, c.pos);
            unreachable.contains(&g) || visited.contains(&g)
        };
        let wanted = Self::is_wanted_block(cx, t.state);
        let already = Self::connected(cx, &t).iter().any(seen);
        let locked = cx.level.container_locked(t.pos);
        (wanted && !already && !locked).then_some(t)
    }

    /// `getTransportTarget`: the nearest chest block entity of the loaded chunks around that fits.
    fn get_transport_target(&self, cx: &Cx) -> Option<Target> {
        let b = cx.e.block_position();
        let area = Aabb::new(b.x as f64, b.y as f64, b.z as f64, (b.x + 1) as f64, (b.y + 1) as f64, (b.z + 1) as f64).inflate(HORIZONTAL as f64, VERTICAL as f64, HORIZONTAL as f64);
        let visited = Self::visited(cx);
        let unreachable = Self::unreachable(cx);
        let radius = HORIZONTAL.div_euclid(16) + 1;
        let (cx0, cz0) = (b.x >> 4, b.z >> 4);
        let here = cx.e.position();
        let mut best: Option<Target> = None;
        let mut best_d = f32::MAX as f64;
        // `ChunkPos.rangeClosed`: x runs fastest.
        for cz in (cz0 - radius)..=(cz0 + radius) {
            for cxx in (cx0 - radius)..=(cx0 + radius) {
                let Some(list) = cx.level.chest_block_entities(cxx, cz) else { continue };
                for pos in list {
                    // `BlockPos.distToCenterSqr`.
                    let d = {
                        let (dx, dy, dz) = (pos.x as f64 + 0.5 - here.x, pos.y as f64 + 0.5 - here.y, pos.z as f64 + 0.5 - here.z);
                        dx * dx + dy * dy + dz * dz
                    };
                    if d < best_d
                        && let Some(t) = Self::target_valid_to_pick(cx, pos, &area, &visited, &unreachable)
                    {
                        best = Some(t);
                        best_d = d;
                    }
                }
            }
        }
        best
    }

    /// `updateInvalidTarget`: whether the target was replaced (or lost).
    fn update_invalid_target(&mut self, cx: &mut Cx) -> bool {
        if self.has_valid_target(cx) {
            return false;
        }
        self.stop_targeting(cx);
        match self.get_transport_target(cx) {
            Some(t) => {
                let pos = t.pos;
                self.target = Some(t);
                self.on_start_travelling(cx);
                self.set_visited(cx, pos);
            }
            None => self.enter_cooldown(cx),
        }
        true
    }

    /// `stopTargetingCurrentTarget`.
    fn stop_targeting(&mut self, cx: &mut Cx) {
        self.ticks_since_reaching_target = 0;
        self.target = None;
        cx.m.nav.stop();
        cx.b.mem.erase(Mem::WalkTarget);
    }

    /// `clearMemoriesAfterMatchingTargetFound`.
    fn clear_after_found(&mut self, cx: &mut Cx) {
        self.stop_targeting(cx);
        cx.b.mem.erase(Mem::VisitedBlockPositions);
        cx.b.mem.erase(Mem::UnreachableTransportBlockPositions);
    }

    /// `enterCooldownAfterNoMatchingTargetFound`.
    fn enter_cooldown(&mut self, cx: &mut Cx) {
        self.stop_targeting(cx);
        cx.b.mem.set(Mem::TransportItemsCooldownTicks, Val::Int(140));
        cx.b.mem.erase(Mem::VisitedBlockPositions);
        cx.b.mem.erase(Mem::UnreachableTransportBlockPositions);
    }

    /// `onStartTravelling`: the golem lets go of the chest and stands idle.
    fn on_start_travelling(&mut self, cx: &mut Cx) {
        let s = st_mut(cx.m);
        s.opened_chest = None;
        s.state = copper_golem::IDLE;
        self.state = TravelState::Travelling;
        self.interaction = None;
        self.ticks_since_reaching_target = 0;
    }

    /// `stopInPlace`.
    fn stop_in_place(cx: &mut Cx) {
        cx.m.nav.stop();
        cx.m.xxa = 0.0;
        cx.m.yya = 0.0;
        cx.m.speed = 0.0;
        cx.m.zza = 0.0;
        cx.e.delta = Vec3::new(0.0, cx.e.delta.y, 0.0);
    }

    /// `walkTowardsTarget`.
    fn walk_towards_target(&self, cx: &mut Cx) {
        if let Some(t) = &self.target {
            util::set_walk_and_look(cx, Tracker::block(t.pos), SPEED, 0);
        }
    }

    /// `startOnReachedTargetInteraction`: which interaction it will be, by the hand and the chest.
    fn start_interaction(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        let items = chest_access::items(&*cx.level, &t.halves);
        self.interaction = Some(if Self::picking_up(cx) {
            if items.iter().all(ItemStack::is_empty) { Interaction::PickupNoItem } else { Interaction::PickupItem }
        } else {
            let held = &cx.m.equipment[MAINHAND];
            if items.iter().all(ItemStack::is_empty) || items.iter().any(|s| s.is_same_item(held)) { Interaction::PlaceItem } else { Interaction::PlaceNoItem }
        });
        self.state = TravelState::Interacting;
    }

    /// `onTargetInteraction`: look at the chest, stand still, and the golem's own part by tick.
    fn on_target_interaction(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(t.pos)));
        Self::stop_in_place(cx);
        let Some(which) = self.interaction else { return };
        // `CopperGolemAi.onReachedTargetInteraction(state, sound)`.
        let (state, sound) = match which {
            Interaction::PickupItem => (copper_golem::GETTING_ITEM, "minecraft:entity.copper_golem.no_item_get"),
            Interaction::PickupNoItem => (copper_golem::GETTING_NO_ITEM, "minecraft:entity.copper_golem.no_item_no_get"),
            Interaction::PlaceItem => (copper_golem::DROPPING_ITEM, "minecraft:entity.copper_golem.item_drop"),
            Interaction::PlaceNoItem => (copper_golem::DROPPING_NO_ITEM, "minecraft:entity.copper_golem.item_no_drop"),
        };
        let ticks = self.ticks_since_reaching_target;
        let id = cx.e.id;
        if ticks == 1 {
            // `container.startOpen(golem)`: every half; the chest counts it as a user.
            for &h in &t.halves {
                cx.level.container_start_open(h, id, super::copper_golem::CONTAINER_INTERACTION_RANGE);
            }
            let s = st_mut(cx.m);
            s.opened_chest = Some(t.pos);
            s.state = state;
        }
        if ticks == 9 && !cx.e.silent {
            let pos = cx.e.position();
            cx.level.emit(Event::Sound { pos, sound, source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        if ticks == 60 {
            // `container.getEntitiesWithContainerOpen().contains(golem)`: only a single chest answers
            // (a `CompoundContainer` always says none, and so is never closed here).
            if t.halves.len() == 1 && Self::users(cx, t.pos).contains(&id) {
                cx.level.container_stop_open(t.pos, id);
            }
            st_mut(cx.m).opened_chest = None;
        }
    }

    /// `pickUpItems`: up to 16 of the first stack, into the hand.
    fn pick_up_items(&mut self, cx: &mut Cx, t: &Target) {
        let mut items = chest_access::items(&*cx.level, &t.halves);
        let mut picked = ItemStack::empty();
        for s in items.iter_mut() {
            if !s.is_empty() {
                let n = s.count().min(16);
                picked = s.split(n);
                break;
            }
        }
        cx.m.equipment[MAINHAND] = picked;
        cx.m.drop_chances[MAINHAND] = 2.0;
        chest_access::set_items(&mut *cx.level, &t.halves, items);
        self.clear_after_found(cx);
    }

    /// `addItemsToContainer` and `putDownItem`.
    fn put_down_item(&mut self, cx: &mut Cx, t: &Target) {
        let mut items = chest_access::items(&*cx.level, &t.halves);
        let mut hand = std::mem::replace(&mut cx.m.equipment[MAINHAND], ItemStack::empty());
        let mut rest = None;
        for slot in items.iter_mut() {
            if slot.is_empty() {
                *slot = std::mem::replace(&mut hand, ItemStack::empty());
                rest = Some(ItemStack::empty());
                break;
            }
            if slot.is_same_item_same_components(&hand) && slot.count() < slot.max_stack_size() {
                // (vanilla shrinks the hand by the room, not by what moved: a small hand is gone)
                let room = slot.max_stack_size() - slot.count();
                let moved = room.min(hand.count());
                slot.set_count(slot.count() + moved);
                hand.set_count(hand.count() - room);
                if hand.is_empty() {
                    rest = Some(ItemStack::empty());
                    break;
                }
            }
        }
        let left = rest.unwrap_or(hand);
        chest_access::set_items(&mut *cx.level, &t.halves, items);
        let done = left.is_empty();
        cx.m.equipment[MAINHAND] = left;
        if done {
            self.clear_after_found(cx);
        } else {
            self.stop_targeting(cx);
        }
    }

    /// `doReachedTargetInteraction` at the end of the standing: pick up or put down by what the
    /// golem and the chest are (the interaction was fixed on arrival).
    fn finish_interaction(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        // The same choice as the arrival's, made again on the chest as it is now.
        if Self::picking_up(cx) {
            let items = chest_access::items(&*cx.level, &t.halves);
            if items.iter().any(|s| !s.is_empty()) {
                self.pick_up_items(cx, &t);
            } else {
                self.stop_targeting(cx);
            }
        } else {
            let items = chest_access::items(&*cx.level, &t.halves);
            let held = cx.m.equipment[MAINHAND].clone();
            if items.iter().all(ItemStack::is_empty) || items.iter().any(|s| s.is_same_item(&held)) {
                self.put_down_item(cx, &t);
            } else {
                self.stop_targeting(cx);
            }
        }
    }

    fn on_queuing(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        if !Self::another_mob_interacting(cx, &t) {
            // `resumeTravelling`.
            self.state = TravelState::Travelling;
            self.walk_towards_target(cx);
        }
    }

    fn on_travel(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        let center = Self::center_pos(cx);
        if std::env::var_os("KILN_CG_DEBUG").is_some() {
            eprintln!(
                "cg travel tick {} pos {:?} w3 {} others {} w_range {} (range {}) path {:?}",
                cx.time,
                cx.e.position(),
                Self::within_distance(cx, 3.0, &t, center),
                Self::another_mob_interacting(cx, &t),
                Self::within_distance(cx, Self::interaction_range(cx), &t, Self::center_pos(cx)),
                Self::interaction_range(cx),
                cx.m.nav.path.as_ref().map(|p| p.next)
            );
        }
        if Self::within_distance(cx, 3.0, &t, center) && Self::another_mob_interacting(cx, &t) {
            // `startQueuing`.
            Self::stop_in_place(cx);
            self.state = TravelState::Queuing;
        } else if Self::within_distance(cx, Self::interaction_range(cx), &t, Self::center_pos(cx)) {
            self.start_interaction(cx);
        } else {
            self.walk_towards_target(cx);
        }
    }

    fn on_reached(&mut self, cx: &mut Cx) {
        let Some(t) = self.target.clone() else { return };
        if !Self::within_distance(cx, 2.0, &t, Self::center_pos(cx)) {
            self.on_start_travelling(cx);
            return;
        }
        self.ticks_since_reaching_target += 1;
        self.on_target_interaction(cx);
        if self.ticks_since_reaching_target >= 60 {
            self.finish_interaction(cx);
            self.on_start_travelling(cx);
        }
    }
}

impl Default for TransportItemsBetweenContainers {
    fn default() -> Self {
        Self::new()
    }
}

impl Behavior for TransportItemsBetweenContainers {
    fn name(&self) -> &'static str {
        "TransportItemsBetweenContainers"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::VisitedBlockPositions, Status::Registered),
            (Mem::UnreachableTransportBlockPositions, Status::Registered),
            (Mem::TransportItemsCooldownTicks, Status::ValueAbsent),
            (Mem::IsPanicking, Status::ValueAbsent),
        ]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        !crate::leash::is_leashed(cx.e)
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.m.nav.can_path_below_surface = true;
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        !cx.b.mem.has(Mem::TransportItemsCooldownTicks) && !cx.b.mem.has(Mem::IsPanicking) && !crate::leash::is_leashed(cx.e)
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn tick(&mut self, cx: &mut Cx) {
        let replaced = self.update_invalid_target(cx);
        if self.target.is_none() {
            self.stop(cx);
            return;
        }
        if replaced {
            return;
        }
        if self.state == TravelState::Queuing {
            self.on_queuing(cx);
        }
        if self.state == TravelState::Travelling {
            self.on_travel(cx);
        }
        if self.state == TravelState::Interacting {
            self.on_reached(cx);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        self.on_start_travelling(cx);
        cx.m.nav.can_path_below_surface = false;
    }
    behavior_boilerplate!();
}

#[allow(dead_code)]
fn unused(_: &Entity, _: mob::MobKind) {}
