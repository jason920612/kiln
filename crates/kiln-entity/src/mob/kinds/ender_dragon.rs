//! Ender dragon (`EnderDragon`): a `Mob` without goals whose `aiStep` flies toward the target
//! of its current phase (`EnderDragonPhaseManager`: holding pattern, strafing a player, the
//! landing approach, landing, sitting scanning / attacking / flaming, taking off, charging a
//! player, dying, hovering) over the fight's 24 path nodes (`findPath`), heals from the nearest
//! end crystal, knocks back and bites what its wings and head touch, breaks blocks it flies
//! through and dies in a 200-tick animation that pays out experience and tells the fight.
//!
//! The eight parts (`EnderDragonPart`: head, neck, body, three tail pieces, two wings) are not
//! entities here: their positions live in the dragon's state, the client makes them with ids
//! right after the dragon's (the simulation keeps those ids free). A hit on a part other than
//! the head or neck deals a quarter (plus up to one).

use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, DragonFightEvent, DragonFightView, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, MobExt, state, state_mut};
use crate::mob::goals::{self, Living};
use crate::mob::{self, DamageSource, MobData, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct EnderDragon;

pub static KIND: EnderDragon = EnderDragon;

static INFO: Info = Info {
    fire_immune: true,
    monster_base: false,
    // `EnderDragon extends Mob`.
    extends_monster: false,
    sounds: Some("ender_dragon"),
    ..Info::monster("minecraft:ender_dragon", &[(MaxHealth, 200.0), (CameraDistance, 16.0)])
};

/// `EnderDragonPhase`, by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    HoldingPattern,
    StrafePlayer,
    LandingApproach,
    Landing,
    Takeoff,
    SittingFlaming,
    SittingScanning,
    SittingAttacking,
    ChargingPlayer,
    Dying,
    Hover,
}

const PHASES: [Phase; 11] = [
    Phase::HoldingPattern,
    Phase::StrafePlayer,
    Phase::LandingApproach,
    Phase::Landing,
    Phase::Takeoff,
    Phase::SittingFlaming,
    Phase::SittingScanning,
    Phase::SittingAttacking,
    Phase::ChargingPlayer,
    Phase::Dying,
    Phase::Hover,
];

impl Phase {
    /// `EnderDragonPhase.getById`: unknown ids are the holding pattern.
    pub fn by_id(id: i32) -> Phase {
        PHASES.get(id as usize).copied().filter(|_| id >= 0).unwrap_or(Phase::HoldingPattern)
    }

    pub fn id(self) -> i32 {
        PHASES.iter().position(|&p| p == self).unwrap() as i32
    }

    /// The vanilla name (`EnderDragonPhase.name`).
    pub fn name(self) -> &'static str {
        match self {
            Phase::HoldingPattern => "HoldingPattern",
            Phase::StrafePlayer => "StrafePlayer",
            Phase::LandingApproach => "LandingApproach",
            Phase::Landing => "Landing",
            Phase::Takeoff => "Takeoff",
            Phase::SittingFlaming => "SittingFlaming",
            Phase::SittingScanning => "SittingScanning",
            Phase::SittingAttacking => "SittingAttacking",
            Phase::ChargingPlayer => "ChargingPlayer",
            Phase::Dying => "Dying",
            Phase::Hover => "Hover",
        }
    }

    /// `isSitting`: the three sitting phases and hovering.
    pub fn is_sitting(self) -> bool {
        matches!(self, Phase::SittingFlaming | Phase::SittingScanning | Phase::SittingAttacking | Phase::Hover)
    }

    /// `getFlySpeed`.
    fn fly_speed(self) -> f32 {
        match self {
            Phase::ChargingPlayer | Phase::Dying => 3.0,
            Phase::Hover => 1.0,
            Phase::Landing => 1.5,
            _ => 0.6,
        }
    }
}

/// `subEntities`: name, width, height.
pub const PARTS: [(&str, f32, f32); 8] = [
    ("head", 1.0, 1.0),
    ("neck", 3.0, 3.0),
    ("body", 5.0, 3.0),
    ("tail", 2.0, 2.0),
    ("tail", 2.0, 2.0),
    ("tail", 2.0, 2.0),
    ("wing", 4.0, 2.0),
    ("wing", 4.0, 2.0),
];
pub const HEAD: usize = 0;
pub const NECK: usize = 1;
pub const BODY: usize = 2;
const TAIL1: usize = 3;
const WING1: usize = 6;
const WING2: usize = 7;

/// A path over the node graph (`Path` of `Node`s; only the positions matter).
#[derive(Clone, Debug, Default)]
struct DPath {
    nodes: Vec<[i32; 3]>,
    next: usize,
}

impl DPath {
    fn advance(&mut self) {
        self.next += 1;
    }
    fn is_done(&self) -> bool {
        self.next >= self.nodes.len()
    }
    fn next_pos(&self) -> [i32; 3] {
        self.nodes[self.next]
    }
}

/// `DragonFlightHistory`: the last 64 (y, yaw) samples.
#[derive(Clone, Debug)]
struct FlightHistory {
    samples: [(f64, f32); 64],
    head: i32,
}

impl FlightHistory {
    fn record(&mut self, y: f64, y_rot: f32) {
        if self.head < 0 {
            self.samples = [(y, y_rot); 64];
        }
        self.head += 1;
        if self.head == 64 {
            self.head = 0;
        }
        self.samples[self.head as usize] = (y, y_rot);
    }

    fn get(&self, delay: i32) -> (f64, f32) {
        self.samples[((self.head - delay) & 63) as usize]
    }
}

/// Each phase instance's fields (`EnderDragonPhaseManager` keeps one instance per phase, so
/// what `begin` does not reset carries over: the holding direction, the flame count).
#[derive(Clone, Debug, Default)]
struct PhaseData {
    holding_path: Option<DPath>,
    holding_target: Option<Vec3>,
    holding_clockwise: bool,
    strafe_charge: i32,
    strafe_path: Option<DPath>,
    strafe_target: Option<Vec3>,
    strafe_attack: Option<i32>,
    strafe_clockwise: bool,
    approach_path: Option<DPath>,
    approach_target: Option<Vec3>,
    landing_target: Option<Vec3>,
    takeoff_first: bool,
    takeoff_path: Option<DPath>,
    takeoff_target: Option<Vec3>,
    flame_ticks: i32,
    flame_count: i32,
    /// A flame (breath cloud) was made this sitting and is still to be discarded.
    flame: bool,
    scanning_time: i32,
    attacking_ticks: i32,
    charge_target: Option<Vec3>,
    charge_time: i32,
    dying_target: Option<Vec3>,
    dying_time: i32,
    hover_target: Option<Vec3>,
}

#[derive(Clone, Debug)]
pub struct DragonState {
    pub phase: Phase,
    ph: PhaseData,
    flight: FlightHistory,
    /// Where the parts are (`EnderDragonPart` positions, in [`PARTS`] order).
    pub parts: [Vec3; 8],
    pub flap_time: f32,
    pub o_flap_time: f32,
    pub in_wall: bool,
    pub death_time: i32,
    pub y_rot_a: f32,
    /// `nearestCrystal`: the end crystal healing the dragon.
    pub nearest_crystal: Option<i32>,
    pub fight_origin: BlockPos,
    pub sitting_damage: f32,
    /// The fight's dragon (`dragonFight` set).
    pub in_fight: bool,
    /// The 24 path nodes, made on first use.
    nodes: Option<[[i32; 3]; 24]>,
    /// The part a projectile's hit landed on (for its damage).
    aimed: Option<usize>,
}

impl DragonState {
    fn new() -> DragonState {
        // `EnderDragonPhaseManager`'s constructor: `setPhase(HOVERING)`.
        DragonState {
            phase: Phase::Hover,
            ph: PhaseData::default(),
            flight: FlightHistory { samples: [(0.0, 0.0); 64], head: -1 },
            parts: [Vec3::ZERO; 8],
            flap_time: 0.0,
            o_flap_time: 0.0,
            in_wall: false,
            death_time: 0,
            y_rot_a: 0.0,
            nearest_crystal: None,
            fight_origin: BlockPos::new(0, 0, 0),
            sitting_damage: 0.0,
            in_fight: false,
            nodes: None,
            aimed: None,
        }
    }

    /// A part's bounding box (`EntityDimensions.scalable(w, h)` at its position).
    pub fn part_box(&self, i: usize) -> Aabb {
        let (_, w, h) = PARTS[i];
        let p = self.parts[i];
        let hw = (w / 2.0) as f64;
        Aabb::new(p.x - hw, p.y, p.z - hw, p.x + hw, p.y + h as f64, p.z + hw)
    }

    /// `EnderDragonPhaseManager.setPhase`: `end` of the old phase, `begin` of the new one.
    fn set_phase(&mut self, e: &Entity, level: Option<&mut dyn EntityLevel>, target: Phase) {
        if self.phase == target {
            return;
        }
        if self.phase == Phase::SittingFlaming && std::mem::take(&mut self.ph.flame) {
            // `DragonSittingFlamingPhase.end`: the flame goes.
            if let Some(level) = level {
                discard_flame(e, level);
            }
        }
        self.phase = target;
        let p = &mut self.ph;
        match target {
            Phase::HoldingPattern => {
                p.holding_path = None;
                p.holding_target = None;
            }
            Phase::StrafePlayer => {
                p.strafe_charge = 0;
                p.strafe_target = None;
                p.strafe_path = None;
                p.strafe_attack = None;
            }
            Phase::LandingApproach => {
                p.approach_path = None;
                p.approach_target = None;
            }
            Phase::Landing => p.landing_target = None,
            Phase::Takeoff => {
                p.takeoff_first = true;
                p.takeoff_path = None;
                p.takeoff_target = None;
            }
            Phase::SittingFlaming => {
                p.flame_ticks = 0;
                p.flame_count += 1;
            }
            Phase::SittingScanning => p.scanning_time = 0,
            Phase::SittingAttacking => p.attacking_ticks = 0,
            Phase::ChargingPlayer => {
                p.charge_target = None;
                p.charge_time = 0;
            }
            Phase::Dying => {
                p.dying_target = None;
                p.dying_time = 0;
            }
            Phase::Hover => p.hover_target = None,
        }
    }

    /// `getFlyTargetLocation` of `phase`'s instance.
    fn fly_target(&self, phase: Phase) -> Option<Vec3> {
        let p = &self.ph;
        match phase {
            Phase::HoldingPattern => p.holding_target,
            Phase::StrafePlayer => p.strafe_target,
            Phase::LandingApproach => p.approach_target,
            Phase::Landing => p.landing_target,
            Phase::Takeoff => p.takeoff_target,
            Phase::ChargingPlayer => p.charge_target,
            Phase::Dying => p.dying_target,
            Phase::Hover => p.hover_target,
            Phase::SittingFlaming | Phase::SittingScanning | Phase::SittingAttacking => None,
        }
    }
}

/// The flame of a sitting dragon's `SittingFlaming` phase: the breath cloud it owns near its
/// head, discarded when the phase ends.
fn discard_flame(e: &Entity, level: &mut dyn EntityLevel) {
    let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
    for id in level.entities_in(&area, EntityFilter::Any, e.id) {
        if let Some(c) = level.entity_mut(id)
            && c.type_name == "minecraft:area_effect_cloud"
            && breath_cloud_owner(c) == Some(e.id)
        {
            c.discard();
        }
    }
}

/// The owner of a dragon breath cloud (the flame of a sitting dragon).
fn breath_cloud_owner(c: &Entity) -> Option<i32> {
    crate::ext_entity::get::<crate::ext_entity::area_effect_cloud::AreaEffectCloud>(c).and_then(|c| c.owner)
}

/// `new AreaEffectCloud(level, x, y, z)` of dragon's breath: the `dragon_breath` particle
/// (power 1), `owner` (id and UUID), `radius`, `duration`, `radius_per_tick`,
/// `potionDurationScale` 0.25 and `instant_damage` (`effect_duration`, `amplifier`), added to
/// the level.
#[allow(clippy::too_many_arguments)]
pub fn spawn_breath_cloud(
    level: &mut dyn EntityLevel,
    owner: Option<(i32, u128)>,
    pos: Vec3,
    radius: f32,
    duration: i32,
    radius_per_tick: f32,
    effect_duration: i32,
    amplifier: i32,
) {
    use crate::ext_entity::area_effect_cloud::{self as aec, AreaEffectCloud};
    let particle = kiln_data::builtin_id("minecraft:particle_type", "minecraft:dragon_breath")
        .map(|kind| kiln_proto::packets::entity::metadata::Particle { kind, options: 1.0f32.to_be_bytes().to_vec() });
    let harm = crate::effect::Effect::simple(crate::effect::ids::instant_damage(), effect_duration, amplifier);
    let cloud = AreaEffectCloud {
        owner: owner.map(|o| o.0),
        owner_uuid: owner.map(|o| o.1).filter(|&u| u != 0),
        radius,
        duration,
        radius_per_tick,
        potion_duration_scale: 0.25,
        custom_particle: particle,
        potion: kiln_item::component::PotionContents { custom_effects: vec![harm.to_item()], ..Default::default() },
        ..AreaEffectCloud::default()
    };
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    level.add_entity(aec::new(id, 0, pos, cloud, seed));
}

fn dragon(m: &MobData) -> &DragonState {
    state::<DragonState>(m).expect("ender dragon state")
}

fn dragon_mut(m: &mut MobData) -> &mut DragonState {
    state_mut::<DragonState>(m).expect("ender dragon state")
}

/// The dragon's state, if `e` is an ender dragon.
pub fn state_of(e: &Entity) -> Option<&DragonState> {
    mob::data(e).and_then(state::<DragonState>)
}

/// The fight as this dragon sees it (`dragonFight`: set when the level's fight is its own).
fn fight(s: &DragonState, level: &dyn EntityLevel) -> Option<DragonFightView> {
    if s.in_fight { level.dragon_fight() } else { None }
}

// ---------------------------------------------------------------------- the node graph

/// `findClosestNode()`: makes the nodes first (radius 60 at the heightmap + 5, radius 40 at
/// + 15, radius 20 at + 5; never below 73).
fn find_closest_node(e: &Entity, s: &mut DragonState, level: &dyn EntityLevel) -> usize {
    if s.nodes.is_none() {
        let mut nodes = [[0; 3]; 24];
        for (i, n) in nodes.iter_mut().enumerate() {
            let mut y_adjust = 5;
            let (x, z);
            let tau = -std::f32::consts::PI;
            if i < 12 {
                let a = 2.0f32 * (tau + (std::f64::consts::PI / 12.0) as f32 * i as f32);
                x = kiln_javamath::math::floor_f32(60.0f32 * mth::cos(a as f64));
                z = kiln_javamath::math::floor_f32(60.0f32 * mth::sin(a as f64));
            } else if i < 20 {
                let k = (i - 12) as f32;
                let a = 2.0f32 * (tau + (std::f64::consts::PI / 8.0) as f32 * k);
                x = kiln_javamath::math::floor_f32(40.0f32 * mth::cos(a as f64));
                z = kiln_javamath::math::floor_f32(40.0f32 * mth::sin(a as f64));
                y_adjust += 10;
            } else {
                let k = (i - 20) as f32;
                let a = 2.0f32 * (tau + (std::f64::consts::PI / 4.0) as f32 * k);
                x = kiln_javamath::math::floor_f32(20.0f32 * mth::cos(a as f64));
                z = kiln_javamath::math::floor_f32(20.0f32 * mth::sin(a as f64));
            }
            let y = 73.max(level.heightmap(x, z, true) + y_adjust);
            *n = [x, y, z];
        }
        s.nodes = Some(nodes);
    }
    let alive = fight(s, level).map_or(0, |f| f.alive_crystals);
    closest_node(s, alive, e.x(), e.y(), e.z())
}

/// `findClosestNode(x, y, z)`: the first nodes (the outer ring) only while crystals stand.
fn closest_node(s: &DragonState, alive_crystals: i32, x: f64, y: f64, z: f64) -> usize {
    let Some(nodes) = &s.nodes else { return 0 };
    let cur = [crate::math::floor(x), crate::math::floor(y), crate::math::floor(z)];
    let start = if alive_crystals == 0 { 12 } else { 0 };
    let mut best = 10000.0f32;
    let mut index = 0;
    for (i, n) in nodes.iter().enumerate().skip(start) {
        let d = node_dist_sqr(*n, cur);
        if d < best {
            best = d;
            index = i;
        }
    }
    index
}

/// `Node.distanceToSqr` (floats of the integer differences).
fn node_dist_sqr(a: [i32; 3], b: [i32; 3]) -> f32 {
    let (dx, dy, dz) = ((b[0] - a[0]) as f32, (b[1] - a[1]) as f32, (b[2] - a[2]) as f32);
    dx * dx + dy * dy + dz * dz
}

/// `Node.distanceTo`.
fn node_dist(a: [i32; 3], b: [i32; 3]) -> f32 {
    mth::sqrt_f(node_dist_sqr(a, b))
}

/// `nodeAdjacency`: which nodes each node links to.
const ADJACENCY: [i32; 24] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057, 3166214, 2138120, 6373424, 4358208, 12910976,
    9044480, 9706496, 15216640, 13688832, 11763712, 8257536,
];

/// `findPath`: A* over the 24 nodes with vanilla's `BinaryHeap`; `final_node` is appended to a
/// found path (or to the path to the closest node reached).
fn find_path(s: &DragonState, alive_crystals: i32, start: usize, end: usize, final_node: Option<[i32; 3]>) -> Option<DPath> {
    let nodes = s.nodes.as_ref()?;
    #[derive(Clone, Copy)]
    struct N {
        closed: bool,
        f: f32,
        g: f32,
        h: f32,
        came_from: Option<usize>,
        heap: i32,
    }
    let mut n = [N { closed: false, f: 0.0, g: 0.0, h: 0.0, came_from: None, heap: -1 }; 24];
    let mut heap: Vec<usize> = Vec::new();
    // `BinaryHeap.upHeap` / `downHeap` on node indices.
    fn up(heap: &mut [usize], n: &mut [N; 24], mut idx: usize) {
        let node = heap[idx];
        let cost = n[node].f;
        while idx > 0 {
            let parent = (idx - 1) >> 1;
            let pn = heap[parent];
            if !(cost < n[pn].f) {
                break;
            }
            heap[idx] = pn;
            n[pn].heap = idx as i32;
            idx = parent;
        }
        heap[idx] = node;
        n[node].heap = idx as i32;
    }
    fn down(heap: &mut [usize], n: &mut [N; 24], mut idx: usize) {
        let node = heap[idx];
        let cost = n[node].f;
        loop {
            let left = 1 + (idx << 1);
            let right = left + 1;
            if left >= heap.len() {
                break;
            }
            let ln = heap[left];
            let lc = n[ln].f;
            let (rn, rc) = if right >= heap.len() { (usize::MAX, f32::INFINITY) } else { (heap[right], n[heap[right]].f) };
            if lc < rc {
                if !(lc < cost) {
                    break;
                }
                heap[idx] = ln;
                n[ln].heap = idx as i32;
                idx = left;
            } else {
                if !(rc < cost) {
                    break;
                }
                heap[idx] = rn;
                n[rn].heap = idx as i32;
                idx = right;
            }
        }
        heap[idx] = node;
        n[node].heap = idx as i32;
    }
    let to = end;
    n[start].g = 0.0;
    n[start].h = node_dist(nodes[start], nodes[to]);
    n[start].f = n[start].h;
    heap.push(start);
    n[start].heap = 0;
    let mut closest = start;
    let min_index = if alive_crystals == 0 { 12 } else { 0 };
    while !heap.is_empty() {
        // `pop`.
        let open = heap[0];
        let last = heap.pop().unwrap();
        if !heap.is_empty() {
            heap[0] = last;
            down(&mut heap, &mut n, 0);
        }
        n[open].heap = -1;
        if nodes[open] == nodes[to] {
            return Some(reconstruct(nodes, &n.map(|x| x.came_from), start, to, final_node));
        }
        if node_dist(nodes[open], nodes[to]) < node_dist(nodes[closest], nodes[to]) {
            closest = open;
        }
        n[open].closed = true;
        for i in min_index..24 {
            if ADJACENCY[open] & (1 << i) <= 0 || n[i].closed {
                continue;
            }
            let g = n[open].g + node_dist(nodes[open], nodes[i]);
            if n[i].heap < 0 || g < n[i].g {
                n[i].came_from = Some(open);
                n[i].g = g;
                n[i].h = node_dist(nodes[i], nodes[to]);
                if n[i].heap >= 0 {
                    // `changeCost`.
                    let old = n[i].f;
                    let new = n[i].g + n[i].h;
                    n[i].f = new;
                    let at = n[i].heap as usize;
                    if new < old { up(&mut heap, &mut n, at) } else { down(&mut heap, &mut n, at) }
                } else {
                    n[i].f = n[i].g + n[i].h;
                    heap.push(i);
                    let at = heap.len() - 1;
                    up(&mut heap, &mut n, at);
                }
            }
        }
    }
    if closest == start {
        return None;
    }
    Some(reconstruct(nodes, &n.map(|x| x.came_from), start, closest, final_node))
}

/// `reconstructPath` from `to` back through `cameFrom` (the final node, when given, after `to`).
fn reconstruct(nodes: &[[i32; 3]; 24], came_from: &[Option<usize>; 24], _from: usize, to: usize, final_node: Option<[i32; 3]>) -> DPath {
    let mut out = Vec::new();
    if let Some(f) = final_node {
        out.push(f);
    }
    let mut at = Some(to);
    while let Some(i) = at {
        out.push(nodes[i]);
        at = came_from[i];
    }
    out.reverse();
    DPath { nodes: out, next: 0 }
}

// ---------------------------------------------------------------------- helpers

/// `getHeightmapPos(MOTION_BLOCKING[_NO_LEAVES], getPodiumLocation(fightOrigin))`.
fn egg(s: &DragonState, level: &dyn EntityLevel, no_leaves: bool) -> BlockPos {
    let o = s.fight_origin;
    BlockPos::new(o.x, level.heightmap(o.x, o.z, no_leaves), o.z)
}

fn bottom_center(p: BlockPos) -> Vec3 {
    Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5)
}

/// `BlockPos.distToCenterSqr(position)`.
fn dist_to_center_sqr(p: BlockPos, v: Vec3) -> f64 {
    let (dx, dy, dz) = (p.x as f64 + 0.5 - v.x, p.y as f64 + 0.5 - v.y, p.z as f64 + 0.5 - v.z);
    dx * dx + dy * dy + dz * dz
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// `Entity.calculateViewVector(xRot, yRot)`.
fn view_vector(x_rot: f32, y_rot: f32) -> Vec3 {
    crate::ext_entity::fireball::view_vector(x_rot, y_rot)
}

/// `getHeadLookVector(1)`: the look while landing or taking off tilts toward the podium,
/// while sitting 45 degrees up; the view vector (pitch, head yaw) otherwise.
fn head_look_vector(e: &Entity, m: &MobData, s: &DragonState, level: &dyn EntityLevel) -> Vec3 {
    let x_rot = if matches!(s.phase, Phase::Landing | Phase::Takeoff) {
        let egg = egg(s, level, true);
        let dist = kiln_javamath::math::max((dist_to_center_sqr(egg, e.position()).sqrt() as f32) / 4.0, 1.0);
        let y_offset = 6.0 / dist;
        -y_offset * 1.5 * 5.0
    } else if s.phase.is_sitting() {
        -45.0
    } else {
        e.x_rot
    };
    view_vector(x_rot, m.y_head_rot)
}

/// `canBeSeenAsEnemy` of a target (`EnderDragon.canAttack`).
fn seen_as_enemy(t: &Living) -> bool {
    !t.invulnerable && !t.creative && !t.spectator && t.alive
}

/// `TargetingConditions.forCombat()` (with `range` when positive, the line of sight through
/// the dragon's `Sensing` when `los`) for player `p`.
fn combat_ok(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, p: &Living, range: f64, los: bool) -> bool {
    if p.id == e.id || p.spectator || !p.alive || !seen_as_enemy(p) {
        return false;
    }
    if range > 0.0 {
        let mut vis = 1.0;
        if p.sneaking {
            vis *= 0.8;
        }
        if p.invisible {
            vis *= 0.7 * p.armor_cover.max(0.1) as f64;
        }
        let d = (range * mth::clamp_d(vis, 0.0, 10.0)).max(2.0);
        if e.position().distance_to_sqr(p.pos) > d * d {
            return false;
        }
    }
    !los || mob::has_line_of_sight_cached(e, m, level, p)
}

/// `level.getNearestPlayer(conditions, dragon, x, y, z)`.
fn nearest_player(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, at: Vec3, range: f64, los: bool, filter: impl Fn(&Living) -> bool) -> Option<Living> {
    let mut best: Option<(f64, Living)> = None;
    for p in level.players() {
        let t = goals::living_player(p);
        if !filter(&t) || !combat_ok(e, m, level, &t, range, los) {
            continue;
        }
        let d = t.pos.distance_to_sqr(at);
        if best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, t));
        }
    }
    best.map(|(_, t)| t)
}

/// `LivingEntity.hasLineOfSight` (not cached).
fn has_line_of_sight(e: &Entity, t: &Living, level: &dyn EntityLevel) -> bool {
    let from = Vec3::new(e.x(), e.eye_y(), e.z());
    let to = Vec3::new(t.pos.x, t.eye_y, t.pos.z);
    to.distance_to_sqr(from).sqrt() <= 128.0 && !mob::clip_blocks(level, from, to)
}

/// A random height above a path node (`navigateToNextPathNode`).
fn above_node(e: &mut Entity, node: [i32; 3]) -> Vec3 {
    let mut y;
    loop {
        // `current.getY() + nextFloat() * 20.0F`: float arithmetic.
        y = (node[1] as f32 + e.random.next_float() * 20.0) as f64;
        if y >= node[1] as f64 {
            break;
        }
    }
    Vec3::new(node[0] as f64, y, node[2] as f64)
}

/// The holding pattern's next node index from `cur` (one step around the ring, sometimes
/// across and turning around).
fn ring_step(e: &mut Entity, cur: usize, clockwise: &mut bool, outer: bool) -> usize {
    let mut target = cur as i32;
    if e.random.next_int_bounded(8) == 0 {
        *clockwise = !*clockwise;
        target = cur as i32 + 6;
    }
    if *clockwise {
        target += 1;
    } else {
        target -= 1;
    }
    wrap_node(target, outer)
}

fn wrap_node(mut target: i32, outer: bool) -> usize {
    if outer {
        target %= 12;
        if target < 0 {
            target += 12;
        }
    } else {
        target -= 12;
        target &= 7;
        target += 12;
    }
    target as usize
}

// ---------------------------------------------------------------------- phases

/// The current phase's `doServerTick`.
fn phase_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let phase = dragon(m).phase;
    match phase {
        Phase::HoldingPattern => holding_tick(e, m, level),
        Phase::StrafePlayer => strafe_tick(e, m, level),
        Phase::LandingApproach => approach_tick(e, m, level),
        Phase::Landing => landing_tick(e, m, level),
        Phase::Takeoff => takeoff_tick(e, m, level),
        Phase::SittingFlaming => flaming_tick(e, m, level),
        Phase::SittingScanning => scanning_tick(e, m, level),
        Phase::SittingAttacking => {
            let s = dragon_mut(m);
            let t = s.ph.attacking_ticks;
            s.ph.attacking_ticks += 1;
            if t >= 40 {
                s.set_phase(e, Some(level), Phase::SittingFlaming);
            }
        }
        Phase::ChargingPlayer => charge_tick(e, m, level),
        Phase::Dying => dying_tick(e, m, level),
        Phase::Hover => {
            let s = dragon_mut(m);
            if s.ph.hover_target.is_none() {
                s.ph.hover_target = Some(e.position());
            }
        }
    }
}

/// Whether the fly target is reached (or lost): `distToTarget < 100 || > 22500`, or a
/// collision (never, with no physics).
fn target_reached(e: &Entity, target: Option<Vec3>) -> bool {
    let d = target.map_or(0.0, |t| t.distance_to_sqr(e.position()));
    d < 100.0 || d > 22500.0 || e.horizontal_collision || e.vertical_collision
}

/// `DragonHoldingPatternPhase.doServerTick`.
fn holding_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !target_reached(e, dragon(m).ph.holding_target) {
        return;
    }
    // `findNewTarget`.
    let f = fight(dragon(m), level);
    if dragon(m).ph.holding_path.as_ref().is_some_and(DPath::is_done) {
        let egg = egg(dragon(m), level, true);
        let crystals = f.map_or(0, |f| f.alive_crystals);
        if e.random.next_int_bounded(crystals + 3) == 0 {
            dragon_mut(m).set_phase(e, Some(level), Phase::LandingApproach);
            return;
        }
        let p = nearest_player(e, m, level, Vec3::new(egg.x as f64, egg.y as f64, egg.z as f64), 0.0, false, |_| true);
        let dist = p.as_ref().map_or(64.0, |p| dist_to_center_sqr(egg, p.pos) / 512.0);
        if let Some(p) = p
            && (e.random.next_int_bounded((dist + 2.0) as i32) == 0 || e.random.next_int_bounded(crystals + 2) == 0)
        {
            strafe_player(e, m, level, &p);
            return;
        }
    }
    if dragon(m).ph.holding_path.as_ref().is_none_or(DPath::is_done) {
        let cur = find_closest_node(e, dragon_mut(m), level);
        let mut clockwise = dragon(m).ph.holding_clockwise;
        let target = ring_step(e, cur, &mut clockwise, f.is_some_and(|f| f.alive_crystals >= 0));
        let s = dragon_mut(m);
        s.ph.holding_clockwise = clockwise;
        s.ph.holding_path = find_path(s, f.map_or(0, |f| f.alive_crystals), cur, target, None);
        if let Some(p) = s.ph.holding_path.as_mut() {
            p.advance();
        }
    }
    // `navigateToNextPathNode`.
    let next = dragon(m).ph.holding_path.as_ref().filter(|p| !p.is_done()).map(DPath::next_pos);
    if let Some(node) = next {
        dragon_mut(m).ph.holding_path.as_mut().unwrap().advance();
        let t = above_node(e, node);
        dragon_mut(m).ph.holding_target = Some(t);
    }
}

/// `strafePlayer`: into the strafe phase against `p` (`setTarget`).
fn strafe_player(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, p: &Living) {
    dragon_mut(m).set_phase(e, Some(level), Phase::StrafePlayer);
    let s = dragon_mut(m);
    s.ph.strafe_attack = Some(p.id);
    let alive = fight(s, level).map_or(0, |f| f.alive_crystals);
    let cur = find_closest_node(e, s, level);
    let target = closest_node(s, alive, p.pos.x, p.pos.y, p.pos.z);
    let (fx, fz) = (crate::math::floor(p.pos.x), crate::math::floor(p.pos.z));
    let (xd, zd) = (fx as f64 - e.x(), fz as f64 - e.z());
    let sd = (xd * xd + zd * zd).sqrt();
    let ho = crate::math::jmin(0.4000000059604645 + sd / 80.0 - 1.0, 10.0);
    let fy = crate::math::floor(p.pos.y + ho);
    s.ph.strafe_path = find_path(s, alive, cur, target, Some([fx, fy, fz]));
    if let Some(p) = s.ph.strafe_path.as_mut() {
        p.advance();
        strafe_navigate(e, m);
    }
}

fn strafe_navigate(e: &mut Entity, m: &mut MobData) {
    let next = dragon(m).ph.strafe_path.as_ref().filter(|p| !p.is_done()).map(DPath::next_pos);
    if let Some(node) = next {
        dragon_mut(m).ph.strafe_path.as_mut().unwrap().advance();
        let t = above_node(e, node);
        dragon_mut(m).ph.strafe_target = Some(t);
    }
}

/// `DragonStrafePlayerPhase.doServerTick`.
fn strafe_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(t) = dragon(m).ph.strafe_attack.and_then(|id| goals::living(level, id)) else {
        dragon_mut(m).set_phase(e, Some(level), Phase::HoldingPattern);
        return;
    };
    if dragon(m).ph.strafe_path.as_ref().is_some_and(DPath::is_done) {
        let (xd, zd) = (t.pos.x - e.x(), t.pos.z - e.z());
        let dist = (xd * xd + zd * zd).sqrt();
        let ho = crate::math::jmin(0.4000000059604645 + dist / 80.0 - 1.0, 10.0);
        dragon_mut(m).ph.strafe_target = Some(Vec3::new(t.pos.x, t.pos.y + ho, t.pos.z));
    }
    let d = dragon(m).ph.strafe_target.map_or(0.0, |v| v.distance_to_sqr(e.position()));
    if d < 100.0 || d > 22500.0 {
        // `findNewTarget`.
        if dragon(m).ph.strafe_path.as_ref().is_none_or(DPath::is_done) {
            let s = dragon_mut(m);
            let alive = fight(s, level).map_or(0, |f| f.alive_crystals);
            let cur = find_closest_node(e, s, level);
            let mut clockwise = s.ph.strafe_clockwise;
            let target = ring_step(e, cur, &mut clockwise, alive > 0);
            let s = dragon_mut(m);
            s.ph.strafe_clockwise = clockwise;
            s.ph.strafe_path = find_path(s, alive, cur, target, None);
            if let Some(p) = s.ph.strafe_path.as_mut() {
                p.advance();
            }
        }
        strafe_navigate(e, m);
    }
    if t.pos.distance_to_sqr(e.position()) < 4096.0 {
        if has_line_of_sight(e, &t, level) {
            dragon_mut(m).ph.strafe_charge += 1;
            let aim = Vec3::new(t.pos.x - e.x(), 0.0, t.pos.z - e.z()).normalize();
            let r = (e.y_rot * 0.017453292) as f64;
            let dir = Vec3::new(mth::sin(r) as f64, 0.0, -mth::cos(r) as f64).normalize();
            let dt = dot(dir, aim) as f32;
            let angle = ((dt as f64).acos() * 180.0 / std::f32::consts::PI as f64) as f32 + 0.5;
            if dragon(m).ph.strafe_charge >= 5 && (0.0..10.0).contains(&angle) {
                let view = view_vector(e.x_rot, m.y_head_rot);
                let head = dragon(m).parts[HEAD];
                let sx = head.x - view.x;
                let sy = head.y + 0.5 + 0.5;
                let sz = head.z - view.z;
                let th = (t.bb.max_y - t.bb.min_y) as f32 as f64;
                let dir = Vec3::new(t.pos.x - sx, t.pos.y + th * 0.5 - sy, t.pos.z - sz);
                if !e.silent {
                    level.emit(Event::LevelEvent { event: 1017, pos: e.block_position(), data: 0 });
                }
                let id = level.next_entity_id();
                let seed = level.fresh_seed();
                let mut fb = crate::ext_entity::dragon_fireball::new(id, e, dir.normalize(), seed);
                fb.set_pos(Vec3::new(sx, sy, sz));
                fb.y_rot = 0.0;
                fb.x_rot = 0.0;
                fb.set_old_pos_and_rot();
                level.add_entity(fb);
                let s = dragon_mut(m);
                s.ph.strafe_charge = 0;
                if let Some(p) = s.ph.strafe_path.as_mut() {
                    p.next = p.next.max(p.nodes.len());
                }
                s.set_phase(e, Some(level), Phase::HoldingPattern);
            }
        } else if dragon(m).ph.strafe_charge > 0 {
            dragon_mut(m).ph.strafe_charge -= 1;
        }
    } else if dragon(m).ph.strafe_charge > 0 {
        dragon_mut(m).ph.strafe_charge -= 1;
    }
}

/// `DragonLandingApproachPhase.doServerTick`.
fn approach_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !target_reached(e, dragon(m).ph.approach_target) {
        return;
    }
    if dragon(m).ph.approach_path.as_ref().is_none_or(DPath::is_done) {
        let alive = fight(dragon(m), level).map_or(0, |f| f.alive_crystals);
        let cur = find_closest_node(e, dragon_mut(m), level);
        let egg = egg(dragon(m), level, true);
        let p = nearest_player(e, m, level, Vec3::new(egg.x as f64, egg.y as f64, egg.z as f64), 0.0, false, |_| true);
        let s = dragon_mut(m);
        let target = match p {
            Some(p) => {
                let aim = Vec3::new(p.pos.x, 0.0, p.pos.z).normalize();
                closest_node(s, alive, -aim.x * 40.0, 105.0, -aim.z * 40.0)
            }
            None => closest_node(s, alive, 40.0, egg.y as f64, 0.0),
        };
        s.ph.approach_path = find_path(s, alive, cur, target, Some([egg.x, egg.y, egg.z]));
        if let Some(p) = s.ph.approach_path.as_mut() {
            p.advance();
        }
    }
    let next = dragon(m).ph.approach_path.as_ref().filter(|p| !p.is_done()).map(DPath::next_pos);
    if let Some(node) = next {
        dragon_mut(m).ph.approach_path.as_mut().unwrap().advance();
        let t = above_node(e, node);
        dragon_mut(m).ph.approach_target = Some(t);
    }
    if dragon(m).ph.approach_path.as_ref().is_some_and(DPath::is_done) {
        dragon_mut(m).set_phase(e, Some(level), Phase::Landing);
    }
}

/// `DragonLandingPhase.doServerTick`.
fn landing_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if dragon(m).ph.landing_target.is_none() {
        let t = bottom_center(egg(dragon(m), level, true));
        dragon_mut(m).ph.landing_target = Some(t);
    }
    if dragon(m).ph.landing_target.unwrap().distance_to_sqr(e.position()) < 1.0 {
        let s = dragon_mut(m);
        s.ph.flame_count = 0;
        s.set_phase(e, Some(level), Phase::SittingScanning);
    }
}

/// `DragonTakeoffPhase.doServerTick`.
fn takeoff_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !dragon(m).ph.takeoff_first && dragon(m).ph.takeoff_path.is_some() {
        let egg = egg(dragon(m), level, true);
        if dist_to_center_sqr(egg, e.position()) >= 100.0 {
            dragon_mut(m).set_phase(e, Some(level), Phase::HoldingPattern);
        }
        return;
    }
    dragon_mut(m).ph.takeoff_first = false;
    // `findNewTarget`.
    let alive = fight(dragon(m), level).map_or(0, |f| f.alive_crystals);
    let cur = find_closest_node(e, dragon_mut(m), level);
    let look = head_look_vector(e, m, dragon(m), level);
    let s = dragon_mut(m);
    let target = wrap_node(closest_node(s, alive, -look.x * 40.0, 105.0, -look.z * 40.0) as i32, alive > 0);
    s.ph.takeoff_path = find_path(s, alive, cur, target, None);
    // `navigateToNextPathNode`.
    let Some(p) = s.ph.takeoff_path.as_mut() else { return };
    p.advance();
    if p.is_done() {
        return;
    }
    let node = p.next_pos();
    p.advance();
    let t = above_node(e, node);
    dragon_mut(m).ph.takeoff_target = Some(t);
}

/// `DragonSittingFlamingPhase.doServerTick`: after 10 ticks the flame (a dragon breath cloud
/// on the ground ahead of the head); after 200, scanning again or, the fourth time, off.
fn flaming_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = dragon_mut(m);
    s.ph.flame_ticks += 1;
    if s.ph.flame_ticks >= 200 {
        let next = if s.ph.flame_count >= 4 { Phase::Takeoff } else { Phase::SittingScanning };
        s.set_phase(e, Some(level), next);
    } else if s.ph.flame_ticks == 10 {
        let head = s.parts[HEAD];
        let look = Vec3::new(head.x - e.x(), 0.0, head.z - e.z()).normalize();
        let x = head.x + look.x * 5.0 / 2.0;
        let z = head.z + look.z * 5.0 / 2.0;
        let initial_y = head.y + 0.5;
        let mut y = initial_y;
        let mut pos = BlockPos::containing(x, initial_y, z);
        while crate::physics::is_air(level.block(pos)) {
            y -= 1.0;
            if y < 0.0 {
                y = initial_y;
                break;
            }
            pos = BlockPos::containing(x, y, z);
        }
        let y = crate::math::floor(y) as f64 + 1.0;
        s.ph.flame = true;
        spawn_breath_cloud(level, Some((e.id, e.uuid)), Vec3::new(x, y, z), 5.0, 200, 0.0, 0, 0);
    }
}

/// `DragonSittingScanningPhase.doServerTick`.
fn scanning_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    dragon_mut(m).ph.scanning_time += 1;
    let dy = e.y();
    let target = nearest_player(e, m, level, e.position(), 20.0, true, |t| (t.pos.y - dy).abs() <= 10.0);
    if let Some(t) = target {
        if dragon(m).ph.scanning_time > 25 {
            dragon_mut(m).set_phase(e, Some(level), Phase::SittingAttacking);
            return;
        }
        let aim = Vec3::new(t.pos.x - e.x(), 0.0, t.pos.z - e.z()).normalize();
        let r = (e.y_rot * 0.017453292) as f64;
        let dir = Vec3::new(mth::sin(r) as f64, 0.0, -mth::cos(r) as f64).normalize();
        let dt = dot(dir, aim) as f32;
        let angle = ((dt as f64).acos() * 180.0 / std::f32::consts::PI as f64) as f32 + 0.5;
        if angle < 0.0 || angle > 10.0 {
            let head = dragon(m).parts[HEAD];
            let (xa, za) = (t.pos.x - head.x, t.pos.z - head.z);
            let delta = mth::clamp_d(
                mth::wrap_degrees_d(180.0 - mth::atan2(xa, za) * 180.0 / std::f32::consts::PI as f64 - e.y_rot as f64),
                -100.0,
                100.0,
            );
            let s = dragon_mut(m);
            s.y_rot_a *= 0.8;
            let mut dist = (xa * xa + za * za).sqrt() as f32 + 1.0;
            let rot_speed = dist;
            if dist > 40.0 {
                dist = 40.0;
            }
            s.y_rot_a += delta as f32 * (0.7 / dist / rot_speed);
            e.y_rot += s.y_rot_a;
        }
    } else if dragon(m).ph.scanning_time >= 100 {
        let target = nearest_player(e, m, level, e.position(), 150.0, true, |_| true);
        let s = dragon_mut(m);
        s.set_phase(e, Some(level), Phase::Takeoff);
        if let Some(t) = target {
            s.set_phase(e, Some(level), Phase::ChargingPlayer);
            s.ph.charge_target = Some(t.pos);
        }
    }
}

/// `DragonChargePlayerPhase.doServerTick`.
fn charge_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = dragon_mut(m);
    let Some(target) = s.ph.charge_target else {
        s.set_phase(e, Some(level), Phase::HoldingPattern);
        return;
    };
    if s.ph.charge_time > 0 {
        let t = s.ph.charge_time;
        s.ph.charge_time += 1;
        if t >= 10 {
            s.set_phase(e, Some(level), Phase::HoldingPattern);
            return;
        }
    }
    let d = target.distance_to_sqr(e.position());
    if d < 100.0 || d > 22500.0 || e.horizontal_collision || e.vertical_collision {
        s.ph.charge_time += 1;
    }
}

/// `DragonDeathPhase.doServerTick`: toward the podium's top, dead (health 0) once near it.
fn dying_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = dragon_mut(m);
    s.ph.dying_time += 1;
    if s.ph.dying_target.is_none() {
        s.ph.dying_target = Some(bottom_center(egg(s, level, false)));
    }
    let d = s.ph.dying_target.unwrap().distance_to_sqr(e.position());
    let far = !(d < 100.0) && !(d > 22500.0) && !e.horizontal_collision && !e.vertical_collision;
    m.set_health(if far { 1.0 } else { 0.0 });
}

// ---------------------------------------------------------------------- aiStep

/// Which blocks the dragon flies through (`DRAGON_TRANSPARENT`) or cannot break
/// (`DRAGON_IMMUNE`), per state: bit 0 transparent, bit 1 immune.
fn dragon_block_flags(s: u16) -> u8 {
    static FLAGS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FLAGS.get_or_init(|| {
        let mut out = vec![0u8; kiln_data::blocks::STATE_COUNT as usize];
        let tags = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == "minecraft:block").map_or(&[][..], |(_, t)| *t);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for (bit, tag) in [(1u8, "minecraft:dragon_transparent"), (2, "minecraft:dragon_immune")] {
            let ids = tags.iter().find(|(t, _)| *t == tag).map_or(&[][..], |(_, ids)| *ids);
            for &id in ids {
                if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                    for f in &mut out[info.first as usize..=info.last as usize] {
                        *f |= bit;
                    }
                }
            }
        }
        out
    })[s as usize]
}

/// `checkWalls`: breaks what the box overlaps (unless immune, or without mob griefing, when it
/// only reports hitting a wall).
fn check_walls(e: &mut Entity, level: &mut dyn EntityLevel, bb: Aabb) -> bool {
    let (x0, y0, z0) = (crate::math::floor(bb.min_x), crate::math::floor(bb.min_y), crate::math::floor(bb.min_z));
    let (x1, y1, z1) = (crate::math::floor(bb.max_x), crate::math::floor(bb.max_y), crate::math::floor(bb.max_z));
    let (mut hit_wall, mut destroyed) = (false, false);
    let griefing = level.mob_griefing();
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let p = BlockPos::new(x, y, z);
                let s = level.block(p);
                if kiln_data::blocks_types::is_air(s) || dragon_block_flags(s) & 1 != 0 {
                    continue;
                }
                if griefing && dragon_block_flags(s) & 2 == 0 {
                    // `removeBlock`: air (a fluid source stays: it is its own fluid's block;
                    // Kiln does not leave the water of waterlogged blocks).
                    if crate::physics::fluid_state(s).is_empty() {
                        destroyed = level.set_block(p, 0, 3) || destroyed;
                    }
                } else {
                    hit_wall = true;
                }
            }
        }
    }
    if destroyed {
        let x = x0 + e.random.next_int_bounded(x1 - x0 + 1);
        let y = y0 + e.random.next_int_bounded(y1 - y0 + 1);
        let z = z0 + e.random.next_int_bounded(z1 - z0 + 1);
        level.emit(Event::LevelEvent { event: 2008, pos: BlockPos::new(x, y, z), data: 0 });
    }
    hit_wall
}

/// `checkCrystals`: the nearest crystal heals one point every 10 ticks; one tick in ten the
/// dragon looks for the nearest within 32 blocks.
fn check_crystals(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    if let Some(c) = dragon(m).nearest_crystal {
        if level.entity(c).is_none_or(|c| c.is_removed()) {
            dragon_mut(m).nearest_crystal = None;
        } else if e.tick_count % 10 == 0 && m.health < m.max_health() {
            let h = m.health + 1.0;
            m.set_health(h);
        }
    }
    if e.random.next_int_bounded(10) == 0 {
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&e.bounding_box().inflate(32.0, 32.0, 32.0), EntityFilter::Any, e.id) {
            let Some(c) = level.entity(id) else { continue };
            if c.type_name != "minecraft:end_crystal" {
                continue;
            }
            let d = c.position().distance_to_sqr(e.position());
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        dragon_mut(m).nearest_crystal = best.map(|(_, id)| id);
    }
}

/// `knockBack`: the living entities around a wing (not creative or spectator players) are
/// thrown away from the body and, while the dragon flies, hit for 5.
fn knock_back(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, area: Aabb) {
    let body = dragon(m).part_box(BODY);
    let (xm, zm) = ((body.min_x + body.max_x) / 2.0, (body.min_z + body.max_z) / 2.0);
    let sitting = dragon(m).phase.is_sitting();
    for id in affected(e, level, &area) {
        let Some(t) = goals::living(level, id) else { continue };
        let (xd, zd) = (t.pos.x - xm, t.pos.z - zm);
        let dd = crate::math::jmax(xd * xd + zd * zd, 0.1);
        if let Some(o) = level.entity_mut(id) {
            o.delta = o.delta.add(xd / dd * 4.0, 0.20000000298023224, zd / dd * 4.0);
            o.needs_sync = true;
        }
        // `getLastHurtByMobTimestamp() < tickCount - 2`.
        let recent = match level.player(id) {
            Some(p) => p.last_hurt_by_mob_time >= p.tick_count - 2,
            None => level.entity(id).and_then(|o| mob::data(o).map(|om| om.last_hurt_by_mob_timestamp >= o.tick_count - 2)).unwrap_or(false),
        };
        if !sitting && !recent {
            hurt_target(e, level, &t, 5.0);
        }
    }
}

/// `getEntities(dragon, area, NO_CREATIVE_OR_SPECTATOR)`, living ones.
fn affected(e: &Entity, level: &dyn EntityLevel, area: &Aabb) -> Vec<i32> {
    level
        .entities_in(area, EntityFilter::Living, e.id)
        .into_iter()
        .filter(|&id| match level.player(id) {
            Some(p) => !p.creative && !p.spectator,
            None => level.entity(id).is_some_and(|o| mob::data(o).is_some()),
        })
        .collect()
}

/// `target.hurtServer(mobAttack(dragon), amount)`.
fn hurt_target(e: &Entity, level: &mut dyn EntityLevel, t: &Living, amount: f32) {
    let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    mob::hurt_living(level, t, source, amount);
}

/// `EnderDragon.aiStep`.
fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    // `processFlappingMovement`.
    {
        let s = dragon(m);
        let flap = mth::cos((s.flap_time * 6.2831855) as f64);
        let old = mth::cos((s.o_flap_time * 6.2831855) as f64);
        if old <= -0.3 && flap >= -0.3 {
            level.emit(Event::GameEvent { event: "minecraft:flap", pos: e.position(), entity: Some(e.id) });
        }
    }
    if !dragon(m).in_fight && level.dragon_fight().is_some_and(|f| f.dragon == Some(e.uuid)) {
        dragon_mut(m).in_fight = true;
    }
    let s = dragon_mut(m);
    s.o_flap_time = s.flap_time;
    if m.is_dead_or_dying() {
        // The explosion particles' offsets.
        for _ in 0..3 {
            e.random.next_float();
        }
        return;
    }
    check_crystals(e, m, level);
    let v = e.delta;
    let mut flap_speed = 0.2f32 / (v.horizontal_distance() as f32 * 10.0 + 1.0);
    flap_speed *= 2.0f64.powf(v.y) as f32;
    let s = dragon_mut(m);
    if s.phase.is_sitting() {
        s.flap_time += 0.1;
    } else if s.in_wall {
        s.flap_time += flap_speed * 0.5;
    } else {
        s.flap_time += flap_speed;
    }
    e.y_rot = mth::wrap_degrees(e.y_rot);
    if m.no_ai {
        dragon_mut(m).flap_time = 0.5;
        return;
    }
    let (y, yaw) = (e.y(), e.y_rot);
    dragon_mut(m).flight.record(y, yaw);
    // The phase ticks (again if it changed); the flight follows the instance that ticked last,
    // even if it changed the phase once more.
    let mut phase = dragon(m).phase;
    phase_tick(e, m, level);
    if dragon(m).phase != phase {
        phase = dragon(m).phase;
        phase_tick(e, m, level);
    }
    let s = dragon(m);
    if let Some(target) = s.fly_target(phase) {
        let xdd = target.x - e.x();
        let mut ydd = target.y - e.y();
        let zdd = target.z - e.z();
        let dist = xdd * xdd + ydd * ydd + zdd * zdd;
        let max = phase.fly_speed();
        let horizontal = (xdd * xdd + zdd * zdd).sqrt();
        if horizontal > 0.0 {
            ydd = mth::clamp_d(ydd / horizontal, -max as f64, max as f64);
        }
        e.delta = e.delta.add(0.0, ydd * 0.01, 0.0);
        e.y_rot = mth::wrap_degrees(e.y_rot);
        let aim = (target - e.position()).normalize();
        let r = (e.y_rot * 0.017453292) as f64;
        let dir = Vec3::new(mth::sin(r) as f64, e.delta.y, -mth::cos(r) as f64).normalize();
        let d = kiln_javamath::math::max((dot(dir, aim) as f32 + 0.5) / 1.5, 0.0);
        if xdd.abs() > 9.999999747378752e-6 || zdd.abs() > 9.999999747378752e-6 {
            let yaw_d = mth::clamp(mth::wrap_degrees(180.0 - mth::atan2(xdd, zdd) as f32 * (180.0f32 / std::f32::consts::PI) - e.y_rot), -50.0, 50.0);
            // `getTurnSpeed`.
            let rot_speed = e.delta.horizontal_distance() as f32 + 1.0;
            let lim = kiln_javamath::math::min(rot_speed, 40.0);
            let turn = if phase == Phase::Landing { lim / rot_speed } else { 0.7 / lim / rot_speed };
            let s = dragon_mut(m);
            s.y_rot_a *= 0.8;
            s.y_rot_a += yaw_d * turn;
            e.y_rot += s.y_rot_a * 0.1;
        }
        let span = (2.0 / (dist + 1.0)) as f32;
        mob::move_relative(e, 0.06 * (d * span + (1.0 - span)), Vec3::new(0.0, 0.0, -1.0));
        let step = if dragon(m).in_wall { e.delta.scale(0.800000011920929) } else { e.delta };
        e.do_move(level, MoverType::SelfMove, step);
        let actual = e.delta.normalize();
        let slide = 0.8 + 0.15 * (dot(actual, dir) + 1.0) / 2.0;
        e.delta = e.delta.multiply(slide, 0.9100000262260437, slide);
    }
    e.apply_effects_from_blocks(level);
    m.y_body_rot = e.y_rot;
    tick_parts(e, m, level);
}

/// The parts follow the body and the flight history; the wings and head act on what they
/// touch; blocks in the head, neck and body break; the fight hears from its dragon.
fn tick_parts(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = dragon(m);
    let h5 = s.flight.get(5);
    let tilt = (h5.0 - s.flight.get(10).0) as f32 * 10.0 * 0.017453292;
    let (cc_tilt, ss_tilt) = (mth::cos(tilt as f64), mth::sin(tilt as f64));
    let rot1 = e.y_rot * 0.017453292;
    let (ss1, cc1) = (mth::sin(rot1 as f64), mth::cos(rot1 as f64));
    let p = e.position();
    let at = |x: f64, y: f64, z: f64| Vec3::new(p.x + x, p.y + y, p.z + z);
    let s = dragon_mut(m);
    s.parts[BODY] = at((ss1 * 0.5) as f64, 0.0, (-cc1 * 0.5) as f64);
    s.parts[WING1] = at((cc1 * 4.5) as f64, 2.0, (ss1 * 4.5) as f64);
    s.parts[WING2] = at((cc1 * -4.5) as f64, 2.0, (ss1 * -4.5) as f64);
    if m.hurt_time <= 0 {
        let w1 = dragon(m).part_box(WING1).inflate(4.0, 2.0, 4.0).offset(0.0, -2.0, 0.0);
        knock_back(e, m, level, w1);
        let w2 = dragon(m).part_box(WING2).inflate(4.0, 2.0, 4.0).offset(0.0, -2.0, 0.0);
        knock_back(e, m, level, w2);
        for part in [HEAD, NECK] {
            let area = dragon(m).part_box(part).inflate(1.0, 1.0, 1.0);
            for id in affected(e, level, &area) {
                if let Some(t) = goals::living(level, id) {
                    hurt_target(e, level, &t, 10.0);
                }
            }
        }
    }
    let s = dragon(m);
    let r2 = e.y_rot * 0.017453292 - s.y_rot_a * 0.01;
    let (ss2, cc2) = (mth::sin(r2 as f64), mth::cos(r2 as f64));
    let y_offset = if s.phase.is_sitting() { -1.0 } else { (s.flight.get(5).0 - s.flight.get(0).0) as f32 };
    let p1 = s.flight.get(5);
    let mut tails = [Vec3::ZERO; 3];
    for (i, tail) in tails.iter_mut().enumerate() {
        let p0 = s.flight.get(12 + i as i32 * 2);
        let rot = e.y_rot * 0.017453292 + mth::wrap_degrees_d((p0.1 - p1.1) as f64) as f32 * 0.017453292;
        let (ss, cc) = (mth::sin(rot as f64), mth::cos(rot as f64));
        let dd = (i + 1) as f32 * 2.0;
        *tail = at(
            (-(ss1 * 1.5 + ss * dd) * cc_tilt) as f64,
            p0.0 - p1.0 - ((dd + 1.5) * ss_tilt) as f64 + 1.5,
            ((cc1 * 1.5 + cc * dd) * cc_tilt) as f64,
        );
    }
    let s = dragon_mut(m);
    s.parts[HEAD] = at((ss2 * 6.5 * cc_tilt) as f64, (y_offset + ss_tilt * 6.5) as f64, (-cc2 * 6.5 * cc_tilt) as f64);
    s.parts[NECK] = at((ss2 * 5.5 * cc_tilt) as f64, (y_offset + ss_tilt * 5.5) as f64, (-cc2 * 5.5 * cc_tilt) as f64);
    s.parts[TAIL1..TAIL1 + 3].copy_from_slice(&tails);
    let (head, neck, body) = (s.part_box(HEAD), s.part_box(NECK), s.part_box(BODY));
    let wall = check_walls(e, level, head) | check_walls(e, level, neck) | check_walls(e, level, body);
    dragon_mut(m).in_wall = wall;
    update_fight(e, m, level);
}

/// `EnderDragonFight.updateDragon`.
fn update_fight(e: &Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if dragon(m).in_fight {
        level.emit(Event::DragonFight(DragonFightEvent::Update { dragon: e.id, uuid: e.uuid, pos: e.position(), health: m.health, max_health: m.max_health() }));
    }
}

/// `tickDeath`: rises for 200 ticks paying out experience (12000 the first time the fight's
/// dragon dies, else 500), then tells the fight and is removed.
fn tick_death(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    update_fight(e, m, level);
    if m.dead && dragon(m).death_time == 0 && dragon(m).phase != Phase::Dying {
        // `kill`: removed at once (`/kill`), the fight told.
        e.removed = Some(crate::entity::RemovalReason::Killed);
        if dragon(m).in_fight {
            level.emit(Event::DragonFight(DragonFightEvent::Killed { dragon: e.id, uuid: e.uuid }));
        }
        return;
    }
    let s = dragon_mut(m);
    s.death_time += 1;
    let t = s.death_time;
    if (180..=200).contains(&t) {
        for _ in 0..3 {
            e.random.next_float();
        }
    }
    let first = dragon(m).in_fight && level.dragon_fight().is_some_and(|f| !f.previously_killed);
    let xp = if first { 12000 } else { 500 };
    if t > 150 && t % 5 == 0 && level.mob_drops() {
        mob::award_experience(level, e.position(), kiln_javamath::math::floor_f32(xp as f32 * 0.08));
    }
    if t == 1 && !e.silent && dragon(m).in_fight {
        level.emit(Event::DragonFight(DragonFightEvent::DeathRoar { pos: e.block_position() }));
    } else if t == 1 && !e.silent {
        level.emit(Event::LevelEvent { event: 1028, pos: e.block_position(), data: 0 });
    }
    let rise = Vec3::new(0.0, 0.10000000149011612, 0.0);
    e.do_move(level, MoverType::SelfMove, rise);
    let s = dragon_mut(m);
    for p in &mut s.parts {
        *p = *p + rise;
    }
    if t >= 200 {
        if level.mob_drops() {
            mob::award_experience(level, e.position(), kiln_javamath::math::floor_f32(xp as f32 * 0.2));
        }
        if dragon(m).in_fight {
            level.emit(Event::DragonFight(DragonFightEvent::Killed { dragon: e.id, uuid: e.uuid }));
        }
        e.removed = Some(crate::entity::RemovalReason::Killed);
        level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: Some(e.id) });
    }
}

// ---------------------------------------------------------------------- damage

/// `EnderDragon.hurt(part, source, damage)`: nothing while dying; the phase's `onHurt` (a
/// sitting dragon shrugs off arrows); a quarter (plus up to one) for parts other than the
/// head and neck; only players and `always_hurts_ender_dragons` damage count. A sitting dragon
/// that lost a quarter of its health takes off.
pub fn hurt_part(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, part: usize, source: DamageSource, damage: f32) -> bool {
    let phase = dragon(m).phase;
    if phase == Phase::Dying {
        return false;
    }
    let mut damage = damage;
    // `AbstractDragonSittingPhase.onHurt`: arrows and wind charges (they catch fire) do nothing.
    if matches!(phase, Phase::SittingFlaming | Phase::SittingScanning | Phase::SittingAttacking)
        && matches!(source.kind, DamageKind::Arrow | DamageKind::Trident)
    {
        if let Some(d) = source.direct {
            level.ignite(d, 1.0);
        }
        damage = 0.0;
    }
    if part != HEAD && part != NECK {
        damage = damage / 4.0 + kiln_javamath::math::min(damage, 1.0);
    }
    if damage < 0.01 {
        return false;
    }
    let by_player = source.attacker_is_player || source.attacker.is_some_and(|a| level.player(a).is_some());
    if by_player || source.kind.is_tag("minecraft:always_hurts_ender_dragons") {
        let before = m.health;
        mob::hurt_base(e, m, level, source, damage);
        if dragon(m).phase.is_sitting() {
            let lost = before - m.health;
            let max = m.max_health();
            let s = dragon_mut(m);
            s.sitting_damage += lost;
            if s.sitting_damage > 0.25 * max {
                s.sitting_damage = 0.0;
                s.set_phase(e, Some(level), Phase::Takeoff);
            }
        }
    }
    true
}

/// Damage to dragon `e` (from outside its tick) on part `part`, as a player's attack on the
/// part's entity would deal it.
pub fn hurt_entity_part(e: &mut Entity, level: &mut dyn EntityLevel, part: usize, source: DamageSource, damage: f32) -> bool {
    let Some(m) = mob::data_mut(e) else { return false };
    if state::<DragonState>(m).is_none() {
        return false;
    }
    dragon_mut(m).aimed = Some(part);
    mob::hurt_entity(e, level, source, damage)
}

/// The part of dragon `e` a projectile at `at` hit (the one whose box is nearest), for its
/// damage.
pub fn aim_at(e: &mut Entity, at: Vec3) {
    let Some(s) = mob::data_mut(e).and_then(state_mut::<DragonState>) else { return };
    let mut best = (f64::MAX, BODY);
    for i in 0..PARTS.len() {
        let b = s.part_box(i);
        let d = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
        let dist = d(at.x, b.min_x, b.max_x).powi(2) + d(at.y, b.min_y, b.max_y).powi(2) + d(at.z, b.min_z, b.max_z).powi(2);
        if dist < best.0 {
            best = (dist, i);
        }
    }
    s.aimed = Some(best.1);
}

/// Where a projectile's segment `from..to` meets one of the dragon's parts (their boxes
/// inflated by `margin`), nearest first: vanilla's projectiles hit the parts (the dragon itself
/// is not pickable).
pub fn clip_parts(e: &Entity, margin: f64, from: Vec3, to: Vec3) -> Option<Vec3> {
    let s = state_of(e)?;
    let mut best: Option<(f64, Vec3)> = None;
    for i in 0..PARTS.len() {
        if let Some(p) = s.part_box(i).inflate(margin, margin, margin).clip(from, to) {
            let d = from.distance_to_sqr(p);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// `EnderDragon.onCrystalDestroyed`: the healing crystal's explosion hits the head for 10;
/// the holding pattern turns on the player responsible (else the nearest player within 64 of
/// the crystal).
pub fn on_crystal_destroyed(e: &mut Entity, level: &mut dyn EntityLevel, crystal: i32, crystal_pos: Vec3, attacker: Option<i32>) {
    let Some(m) = mob::data(e) else { return };
    let Some(s) = state::<DragonState>(m) else { return };
    let player = match attacker.and_then(|a| level.player(a)) {
        Some(p) => Some(goals::living_player(&p)),
        None => {
            // `CRYSTAL_DESTROY_TARGETING` without a targeter: combat conditions only.
            let at = Vec3::new(crate::math::floor(crystal_pos.x) as f64, crate::math::floor(crystal_pos.y) as f64, crate::math::floor(crystal_pos.z) as f64);
            let mut best: Option<(f64, Living)> = None;
            if level.difficulty() != 0 {
                for p in level.players() {
                    let t = goals::living_player(p);
                    if !seen_as_enemy(&t) {
                        continue;
                    }
                    let d = t.pos.distance_to_sqr(at);
                    if best.as_ref().is_none_or(|(b, _)| d < *b) {
                        best = Some((d, t));
                    }
                }
            }
            best.map(|(_, t)| t)
        }
    };
    if s.nearest_crystal == Some(crystal) {
        let source = DamageSource { kind: DamageKind::Explosion, attacker: player.as_ref().map(|p| p.id), direct: Some(crystal), pos: Some(crystal_pos), attacker_is_player: player.is_some() };
        hurt_entity_part(e, level, HEAD, source, 10.0);
    }
    let Some(m) = mob::data(e) else { return };
    if dragon(m).phase == Phase::HoldingPattern
        && let Some(p) = player
        && seen_as_enemy(&p)
    {
        let mut m = std::mem::replace(&mut e.kind, crate::entity::EntityKind::MobTicking { gravity: 0.0 });
        if let crate::entity::EntityKind::Mob(md) = &mut m {
            strafe_player(e, md, level, &p);
        }
        e.kind = m;
    }
}

// ---------------------------------------------------------------------- the type

impl Kind for EnderDragon {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(DragonState::new()))
    }
    fn register_goals(&self, _m: &mut MobData) {}
    fn replaces_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        ai_step(e, m, level);
        true
    }
    fn tick_death(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        tick_death(e, m, level);
        true
    }
    /// `handleKillingBlow`: a flying dragon keeps one health point and starts dying.
    fn handle_killing_blow(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !dragon(m).phase.is_sitting() {
            m.set_health(1.0);
            dragon_mut(m).set_phase(e, Some(level), Phase::Dying);
        }
        true
    }
    fn knockback_immune(&self, m: &MobData) -> bool {
        dragon(m).phase.is_sitting()
    }
    fn despawns(&self) -> bool {
        false
    }
    /// `addEffect` is overridden to refuse every effect.
    fn can_be_affected(&self, _m: &MobData, _effect: &crate::effect::Effect, _base: bool) -> bool {
        false
    }
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let part = dragon_mut(m).aimed.take().unwrap_or(BODY);
        Some(hurt_part(e, m, level, part, *source, amount))
    }
    fn checks_fall_damage(&self) -> bool {
        false
    }
    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(0)
    }
    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let phase = r.num("DragonPhase").map(|v| v as i32);
        let s = dragon_mut(m);
        if let Some(id) = phase {
            s.set_phase(e, None, Phase::by_id(id));
        }
        s.death_time = r.int_or("DragonDeathTime", 0);
        s.sitting_damage = r.float_or("sitting_damage_received", 0.0);
    }
    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = dragon(m);
        o.put("DragonPhase", Tag::Int(s.phase.id()));
        o.put("DragonDeathTime", Tag::Int(s.death_time));
        o.put("sitting_damage_received", Tag::Float(s.sitting_damage));
    }
    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::ender_dragon::PHASE, &DataValue::Int(dragon(m).phase.id()));
    }
    fn max_spawn_cluster(&self) -> i32 {
        1
    }
}

/// A new ender dragon of the fight (`createNewDragon`): the holding pattern, facing `yaw`,
/// at `pos` (the fight's origin, 128 up).
pub fn new_for_fight(id: i32, uuid: u128, seed: i64, pos: Vec3, yaw: f32, origin: BlockPos) -> Entity {
    let mut e = mob::new(mob::MobKind::EnderDragon, id, uuid, seed);
    e.no_physics = true;
    e.set_pos(pos);
    e.y_rot = yaw;
    e.x_rot = 0.0;
    e.set_old_pos_and_rot();
    if let Some(m) = mob::data_mut(&mut e) {
        m.y_head_rot = yaw;
        m.y_body_rot = yaw;
        let s = dragon_mut(m);
        s.fight_origin = origin;
        s.in_fight = true;
        let e2 = Entity::new("minecraft:marker", 0, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0);
        s.set_phase(&e2, None, Phase::HoldingPattern);
    }
    e
}

/// Sets the phase of dragon `e` (`getPhaseManager().setPhase`), e.g. from a command or test.
pub fn set_phase(e: &mut Entity, level: Option<&mut dyn EntityLevel>, phase: Phase) {
    let mut kind = std::mem::replace(&mut e.kind, crate::entity::EntityKind::MobTicking { gravity: 0.0 });
    if let crate::entity::EntityKind::Mob(m) = &mut kind
        && let Some(s) = state_mut::<DragonState>(m)
    {
        s.set_phase(e, level, phase);
    }
    e.kind = kind;
}
