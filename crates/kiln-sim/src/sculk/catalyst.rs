//! Sculk catalysts (`SculkCatalystBlockEntity`): a mob dying within 8 blocks makes the nearest
//! catalyst bloom and take the mob's experience as charge cursors, which its `SculkSpreader`
//! (the level spreader: growth cost 10, no growth within 4 blocks, decay rates 10 and 5) moves
//! through sculk, turning `#sculk_replaceable` blocks into sculk, spreading sculk veins and
//! growing sensors and shriekers.
//!
//! Vanilla draws the spreading from the level random; Kiln from a random seeded by the
//! catalyst's position and the game time, so it does not depend on how regions split the
//! world (an approximation, I class).

use super::{Kind, center};
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction, Level, flags};
use kiln_data::block_logic::Support;
use kiln_entity::math::Vec3;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;

/// `SculkSpreader.createLevelSpreader`.
const GROWTH_SPAWN_COST: i32 = 10;
const NO_GROWTH_RADIUS: i32 = 4;
const CHARGE_DECAY_RATE: i32 = 10;
const ADDITIONAL_DECAY_RATE: i32 = 5;
const REPLACEABLE: &str = "minecraft:sculk_replaceable";
/// `SculkSpreader.MAX_CURSORS`, `MAX_CHARGE`.
const MAX_CURSORS: usize = 32;
const MAX_CHARGE: i32 = 1000;

/// `SculkSpreader.ChargeCursor`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cursor {
    pub pos: BlockPos,
    pub charge: i32,
    pub update_delay: i32,
    pub decay_delay: i32,
    /// `facings`: `None` until the cursor stands on sculk (then its faces, in direction order).
    pub facings: Option<Vec<Direction>>,
}

impl Cursor {
    fn new(pos: BlockPos, charge: i32) -> Cursor {
        Cursor { pos, charge, update_delay: 0, decay_delay: 1, facings: None }
    }

    fn to_nbt(&self) -> Tag {
        let mut f = vec![
            ("pos".to_string(), Tag::IntArray(vec![self.pos.x, self.pos.y, self.pos.z])),
            ("charge".to_string(), Tag::Int(self.charge)),
            ("decay_delay".to_string(), Tag::Int(self.decay_delay)),
            ("update_delay".to_string(), Tag::Int(self.update_delay)),
        ];
        if let Some(faces) = &self.facings {
            f.push(("facings".into(), Tag::List(faces.iter().map(|d| Tag::String(d.name().into())).collect())));
        }
        Tag::Compound(f)
    }

    fn from_nbt(t: &Tag) -> Option<Cursor> {
        let pos = match t.get("pos")? {
            Tag::IntArray(v) if v.len() == 3 => BlockPos::new(v[0], v[1], v[2]),
            _ => return None,
        };
        let int = |k: &str, d: i64| t.get(k).and_then(Tag::as_i64).unwrap_or(d) as i32;
        let facings = t.get("facings").and_then(Tag::as_list).map(|l| {
            let mut v: Vec<Direction> = l.iter().filter_map(|x| x.unwrap_list_element().as_str()).filter_map(Direction::from_name).collect();
            v.sort();
            v.dedup();
            v
        });
        Some(Cursor { pos, charge: int("charge", 0).clamp(0, MAX_CHARGE), update_delay: int("update_delay", 0).max(0), decay_delay: int("decay_delay", 1).clamp(0, 1), facings })
    }
}

/// `SculkSpreader.load` (`cursors`, at most 32).
pub(crate) fn load_cursors(t: Option<&Tag>) -> Vec<Cursor> {
    t.and_then(Tag::as_list).map_or(Vec::new(), |l| l.iter().map(Tag::unwrap_list_element).filter_map(Cursor::from_nbt).take(MAX_CURSORS).collect())
}

pub(crate) fn save_cursors(cursors: &[Cursor]) -> Tag {
    Tag::List(cursors.iter().map(Cursor::to_nbt).collect())
}

/// The catalyst that hears a death at `pos` first (`BY_DISTANCE`: the nearest within 8
/// blocks), if any.
pub(crate) fn nearest(level: &RegionLevel, pos: Vec3) -> Option<BlockPos> {
    if level.blocks.sculk.map.is_empty() {
        return None;
    }
    let mut best: Option<(f64, BlockPos)> = None;
    for p in super::listeners_near(level, pos, kiln_entity::vibration::DEFAULT_RADIUS) {
        if level.blocks.sculk.map.get(&p).is_some_and(|b| b.kind == Kind::Catalyst) && super::within(p, pos, Kind::Catalyst.radius()) {
            let d = pos.distance_to_sqr(center(p));
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// `CatalystListener.handleGameEvent` for a death at `pos`: the charge becomes cursors half a
/// block above it, and the catalyst blooms.
pub(crate) fn feed(level: &mut RegionLevel, at: BlockPos, pos: Vec3, charge: i32) {
    if let Some(be) = level.blocks.sculk.map.get_mut(&at) {
        let start = super::containing(Vec3::new(pos.x, pos.y + 0.5, pos.z));
        let mut left = charge;
        while left > 0 {
            let c = left.min(MAX_CHARGE);
            if be.cursors.len() < MAX_CURSORS {
                be.cursors.push(Cursor::new(start, c));
            }
            left -= c;
        }
        be.dirty = true;
    }
    // `bloom`.
    let s = level.block(at);
    kiln_blocks::set_block_and_update(level, at, kiln_blocks::state::set_bool(s, "bloom", true));
    kiln_blocks::schedule_block_tick(level, at, kiln_blocks::BlockId::of(s), 8, kiln_blocks::TickPriority::Normal);
    if let Some(kind) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:sculk_soul") {
        let p = [at.x as f64 + 0.5, at.y as f64 + 1.15, at.z as f64 + 0.5];
        let pkt = kiln_proto::packets::world_fx::level_particles(&kiln_proto::packets::world_fx::LevelParticles {
            particle: kiln_proto::packets::world_fx::Particle { kind, options: kiln_proto::packets::world_fx::ParticleOptions::None },
            override_limiter: false,
            always_show: false,
            pos: p,
            offset: [0.2, 0.0, 0.2],
            max_speed: [0.0; 3],
            count: 2,
            randomization: kiln_proto::packets::world_fx::ParticleRandomization::Default,
        });
        level.out.packets.push((p, 32.0, pkt));
    }
    let pitch = 0.6 + level.random().next_float() * 0.4;
    level.effect(kiln_blocks::Effect::Sound { pos: at, sound: "minecraft:block.sculk_catalyst.bloom", volume: 2.0, pitch });
}

/// `SculkCatalystBlockEntity.serverTick`: `updateCursors(level, pos, random, true)`.
pub(crate) fn tick(level: &mut RegionLevel, origin: BlockPos) {
    let Some(be) = level.blocks.sculk.map.get_mut(&origin) else { return };
    if be.cursors.is_empty() {
        return;
    }
    let mut cursors = std::mem::take(&mut be.cursors);
    be.dirty = true;
    let mut rng = crate::container::pos_random(level, origin, 0x5343_554c_4b);
    let mut processed: Vec<Cursor> = Vec::new();
    // Merge targets by position (index into `processed`), and the charge at each position.
    let mut mergeable: BTreeMap<BlockPos, usize> = BTreeMap::new();
    let mut charges: BTreeMap<BlockPos, i32> = BTreeMap::new();
    for mut c in cursors.drain(..) {
        let d = c.pos;
        if (d.x - origin.x).abs().max((d.y - origin.y).abs()).max((d.z - origin.z).abs()) > 1024 {
            continue;
        }
        update(&mut c, level, origin, &mut rng);
        if c.charge <= 0 {
            level.effect(kiln_blocks::Effect::LevelEvent { id: 3006, pos: c.pos, data: 0 });
            continue;
        }
        *charges.entry(c.pos).or_insert(0) += c.charge;
        match mergeable.get(&c.pos).copied() {
            None => {
                mergeable.insert(c.pos, processed.len());
                processed.push(c);
            }
            Some(i) if c.charge + processed[i].charge <= MAX_CHARGE => {
                // `mergeWith`.
                processed[i].charge += c.charge;
                processed[i].update_delay = processed[i].update_delay.min(c.update_delay);
            }
            Some(i) => {
                let smaller = c.charge < processed[i].charge;
                let pos = c.pos;
                processed.push(c);
                if smaller {
                    mergeable.insert(pos, processed.len() - 1);
                }
            }
        }
    }
    for (pos, charge) in charges {
        let faces = mergeable.get(&pos).and_then(|&i| processed[i].facings.clone());
        if charge > 0
            && let Some(faces) = faces
        {
            let n = (kiln_javamath::strict::log1p(charge as f64) / 2.3f32 as f64) as i32 + 1;
            let mask = faces.iter().fold(0, |m, d| m | 1 << (*d as i32));
            level.effect(kiln_blocks::Effect::LevelEvent { id: 3006, pos, data: (n << 6) + mask });
        }
    }
    if let Some(be) = level.blocks.sculk.map.get_mut(&origin) {
        be.cursors = processed;
    }
}

/// Which `SculkBehaviour` a block has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Sculk,
    Vein,
    Default,
}

fn behaviour(s: u16) -> Behaviour {
    if kiln_blocks::state::is(s, kiln_data::blocks::default_state::SCULK) {
        Behaviour::Sculk
    } else if is_vein(s) {
        Behaviour::Vein
    } else {
        Behaviour::Default
    }
}

fn is_vein(s: u16) -> bool {
    kiln_blocks::state::is(s, kiln_data::blocks::default_state::SCULK_VEIN)
}

fn has_face(s: u16, d: Direction) -> bool {
    kiln_blocks::state::get(s, d.name()) == Some("true")
}

/// `ChargeCursor.update` (level spreader, veins spread).
fn update(c: &mut Cursor, level: &mut RegionLevel, origin: BlockPos, rng: &mut LegacyRandom) {
    if c.charge <= 0 {
        return;
    }
    if c.update_delay > 0 {
        c.update_delay -= 1;
        return;
    }
    let mut state = level.block(c.pos);
    let mut b = behaviour(state);
    if attempt_spread_vein(b, level, c.pos, state, c.facings.as_deref()) {
        if b != Behaviour::Sculk {
            state = level.block(c.pos);
            b = behaviour(state);
        }
        level.effect(kiln_blocks::Effect::Sound { pos: c.pos, sound: "minecraft:block.sculk.spread", volume: 1.0, pitch: 1.0 });
    }
    c.charge = attempt_use_charge(c, b, level, origin, rng);
    if c.charge <= 0 {
        on_discharged(b, level, state, c.pos);
        return;
    }
    if let Some(next) = valid_movement_pos(level, c.pos, rng) {
        on_discharged(b, level, state, c.pos);
        c.pos = next;
        state = level.block(next);
    }
    if behaviour(state) != Behaviour::Default {
        c.facings = Some(Direction::ALL.into_iter().filter(|&d| has_face(state, d)).collect());
    }
    c.decay_delay = if b == Behaviour::Default { (c.decay_delay - 1).max(0) } else { 1 };
    c.update_delay = 1;
}

/// `SculkBehaviour.attemptUseCharge`.
fn attempt_use_charge(c: &Cursor, b: Behaviour, level: &mut RegionLevel, origin: BlockPos, rng: &mut LegacyRandom) -> i32 {
    let charge = c.charge;
    match b {
        Behaviour::Default => {
            if c.decay_delay > 0 {
                charge
            } else {
                0
            }
        }
        Behaviour::Vein => {
            if attempt_place_sculk(level, c.pos, rng) {
                return charge - 1;
            }
            if rng.next_int_bounded(CHARGE_DECAY_RATE) == 0 { kiln_javamath::math::floor_f32(charge as f32 * 0.5) } else { charge }
        }
        Behaviour::Sculk => {
            if charge == 0 || rng.next_int_bounded(CHARGE_DECAY_RATE) != 0 {
                return charge;
            }
            let p = c.pos;
            let d2 = dist_sqr(p, origin);
            let near = d2 < (NO_GROWTH_RADIUS * NO_GROWTH_RADIUS) as f64;
            if near || !can_place_growth(level, p) {
                if rng.next_int_bounded(ADDITIONAL_DECAY_RATE) != 0 {
                    return charge;
                }
                return charge - if near { 1 } else { decay_penalty(d2, charge) };
            }
            if rng.next_int_bounded(GROWTH_SPAWN_COST) < charge {
                let above = p.above();
                let (s, sound) = if rng.next_int_bounded(11) == 0 {
                    (kiln_data::blocks::default_state::SCULK_SHRIEKER, "minecraft:block.sculk_shrieker.place")
                } else {
                    (kiln_data::blocks::default_state::SCULK_SENSOR, "minecraft:block.sculk_sensor.place")
                };
                let wet = kiln_data::block_logic::fluid(level.block(above)).amount > 0;
                let s = if wet { kiln_blocks::state::set_bool(s, "waterlogged", true) } else { s };
                kiln_blocks::set_block_and_update(level, above, s);
                level.effect(kiln_blocks::Effect::Sound { pos: p, sound, volume: 1.0, pitch: 1.0 });
            }
            0.max(charge - GROWTH_SPAWN_COST)
        }
    }
}

fn dist_sqr(a: BlockPos, b: BlockPos) -> f64 {
    let (x, y, z) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
    x * x + y * y + z * z
}

/// `SculkBlock.getDecayPenalty`.
fn decay_penalty(d2: f64, charge: i32) -> i32 {
    let d = d2.sqrt() as f32 - NO_GROWTH_RADIUS as f32;
    let f = d * d;
    let i = (24 - NO_GROWTH_RADIUS) * (24 - NO_GROWTH_RADIUS);
    let g = 1.0f32.min(f / i as f32);
    1.max((charge as f32 * g * 0.5) as i32)
}

/// `SculkBlock.canPlaceGrowth`: open above, at most two growth inhibitors around.
fn can_place_growth(level: &RegionLevel, p: BlockPos) -> bool {
    let above = level.block(p.above());
    let water = kiln_blocks::state::is(above, kiln_data::blocks::default_state::WATER);
    if !(kiln_data::blocks_types::is_air(above) || water) {
        return false;
    }
    let mut inhibitors = 0;
    for y in p.y..=p.y + 2 {
        for z in p.z - 4..=p.z + 4 {
            for x in p.x - 4..=p.x + 4 {
                if kiln_blocks::tags::is(level.block(BlockPos::new(x, y, z)), "minecraft:sculk_growth_inhibitors") {
                    inhibitors += 1;
                    if inhibitors > 2 {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// `SculkBehaviour.attemptSpreadVein`.
fn attempt_spread_vein(b: Behaviour, level: &mut RegionLevel, p: BlockPos, state: u16, facings: Option<&[Direction]>) -> bool {
    if b == Behaviour::Default {
        match facings {
            None => {
                let s = level.block(p);
                return spread_all(level, s, p, true) > 0;
            }
            Some(faces) if !faces.is_empty() => {
                let water = kiln_data::block_logic::fluid(state).kind == kiln_data::block_logic::FluidKind::Water;
                return (kiln_data::blocks_types::is_air(state) || water) && regrow(level, p, state, faces);
            }
            Some(_) => {}
        }
    }
    spread_all(level, state, p, false) > 0
}

/// `SculkVeinBlock.regrow`.
fn regrow(level: &mut RegionLevel, p: BlockPos, state: u16, faces: &[Direction]) -> bool {
    let mut vein = kiln_data::blocks::default_state::SCULK_VEIN;
    let mut any = false;
    for &d in faces {
        if can_attach_to(level.block(p.relative(d)), d) {
            vein = kiln_blocks::state::set_bool(vein, d.name(), true);
            any = true;
        }
    }
    if !any {
        return false;
    }
    if kiln_data::block_logic::fluid(state).amount > 0 {
        vein = kiln_blocks::state::set_bool(vein, "waterlogged", true);
    }
    kiln_blocks::set_block_and_update(level, p, vein);
    true
}

/// `SculkVeinBlock.onDischarged`.
fn on_discharged(b: Behaviour, level: &mut RegionLevel, state: u16, p: BlockPos) {
    if b != Behaviour::Vein || !is_vein(state) {
        return;
    }
    let mut s = state;
    for d in Direction::ALL {
        if has_face(s, d) && kiln_blocks::state::is(level.block(p.relative(d)), kiln_data::blocks::default_state::SCULK) {
            s = kiln_blocks::state::set_bool(s, d.name(), false);
        }
    }
    if !Direction::ALL.into_iter().any(|d| has_face(s, d)) {
        let wet = kiln_data::block_logic::fluid(level.block(p)).amount > 0;
        s = if wet { kiln_data::blocks::default_state::WATER } else { kiln_data::blocks::default_state::AIR };
    }
    kiln_blocks::set_block_and_update(level, p, s);
}

/// `SculkVeinBlock.attemptPlaceSculk`: a `#sculk_replaceable` block a vein face rests on
/// becomes sculk.
fn attempt_place_sculk(level: &mut RegionLevel, p: BlockPos, rng: &mut LegacyRandom) -> bool {
    let state = level.block(p);
    for d in all_shuffled(rng) {
        if !has_face(state, d) {
            continue;
        }
        let q = p.relative(d);
        if !kiln_blocks::tags::is(level.block(q), REPLACEABLE) {
            continue;
        }
        let sculk = kiln_data::blocks::default_state::SCULK;
        kiln_blocks::set_block_and_update(level, q, sculk);
        level.effect(kiln_blocks::Effect::Sound { pos: q, sound: "minecraft:block.sculk.spread", volume: 1.0, pitch: 1.0 });
        spread_all(level, sculk, q, false);
        let back = d.opposite();
        for e in Direction::ALL {
            if e == back {
                continue;
            }
            let n = q.relative(e);
            let ns = level.block(n);
            if is_vein(ns) {
                on_discharged(Behaviour::Vein, level, ns, n);
            }
        }
        return true;
    }
    false
}

/// `Util.shuffle`.
fn shuffle<T>(list: &mut [T], rng: &mut LegacyRandom) {
    for i in (2..=list.len()).rev() {
        let j = rng.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
}

/// `Direction.allShuffled`.
fn all_shuffled(rng: &mut LegacyRandom) -> [Direction; 6] {
    let mut d = Direction::ALL;
    shuffle(&mut d, rng);
    d
}

/// `ChargeCursor.NON_CORNER_NEIGHBOURS`, in `betweenClosed` order.
fn non_corner_neighbours() -> Vec<(i32, i32, i32)> {
    let mut v = Vec::with_capacity(18);
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                if (x == 0 || y == 0 || z == 0) && (x, y, z) != (0, 0, 0) {
                    v.push((x, y, z));
                }
            }
        }
    }
    v
}

/// `ChargeCursor.getValidMovementPos` (the level spreader may go anywhere).
fn valid_movement_pos(level: &RegionLevel, p: BlockPos, rng: &mut LegacyRandom) -> Option<BlockPos> {
    let mut offsets = non_corner_neighbours();
    shuffle(&mut offsets, rng);
    let mut result = p;
    for (dx, dy, dz) in offsets {
        let m = p.offset(dx, dy, dz);
        let s = level.block(m);
        if behaviour(s) != Behaviour::Default && movement_unobstructed(level, p, m) {
            result = m;
            if has_substrate_access(level, s, m) {
                break;
            }
        }
    }
    (result != p).then_some(result)
}

/// `ChargeCursor.isMovementUnobstructed`.
fn movement_unobstructed(level: &RegionLevel, from: BlockPos, to: BlockPos) -> bool {
    let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
    if dx.abs() + dy.abs() + dz.abs() == 1 {
        return true;
    }
    let x = if dx < 0 { Direction::West } else { Direction::East };
    let y = if dy < 0 { Direction::Down } else { Direction::Up };
    let z = if dz < 0 { Direction::North } else { Direction::South };
    let open = |d: Direction| !kiln_blocks::behaviour::sturdy(level.block(from.relative(d)), d.opposite(), Support::Full);
    if dx == 0 {
        open(y) || open(z)
    } else if dy == 0 {
        open(x) || open(z)
    } else {
        open(x) || open(y)
    }
}

/// `SculkVeinBlock.hasSubstrateAccess`.
fn has_substrate_access(level: &RegionLevel, s: u16, p: BlockPos) -> bool {
    is_vein(s) && Direction::ALL.into_iter().any(|d| has_face(s, d) && kiln_blocks::tags::is(level.block(p.relative(d)), REPLACEABLE))
}

/// `Block.isFaceFull(state.getCollisionShape(), dir)`.
fn collision_face_full(state: u16, dir: Direction) -> bool {
    const EPS: f32 = 1.0e-7;
    let (axis, positive) = match dir {
        Direction::Down => (1, false),
        Direction::Up => (1, true),
        Direction::North => (2, false),
        Direction::South => (2, true),
        Direction::West => (0, false),
        Direction::East => (0, true),
    };
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let rects: Vec<[f32; 4]> = kiln_data::block_props::collision(state)
        .iter()
        .filter(|b| if positive { b[axis + 3] > 1.0 - EPS } else { b[axis] < EPS })
        .map(|b| [b[u], b[v], b[u + 3], b[v + 3]])
        .collect();
    if rects.is_empty() {
        return false;
    }
    let mut us: Vec<f32> = rects.iter().flat_map(|r| [r[0], r[2]]).chain([0.0, 1.0]).collect();
    let mut vs: Vec<f32> = rects.iter().flat_map(|r| [r[1], r[3]]).chain([0.0, 1.0]).collect();
    for c in [&mut us, &mut vs] {
        c.retain(|x| (0.0..=1.0).contains(x));
        c.sort_by(f32::total_cmp);
        c.dedup();
    }
    us.windows(2).all(|a| {
        vs.windows(2).all(|b| {
            let (cu, cv) = ((a[0] + a[1]) / 2.0, (b[0] + b[1]) / 2.0);
            rects.iter().any(|r| r[0] <= cu && cu <= r[2] && r[1] <= cv && cv <= r[3])
        })
    })
}

/// `MultifaceBlock.canAttachTo`: the neighbour `n` on side `dir` holds a face.
fn can_attach_to(n: u16, dir: Direction) -> bool {
    kiln_blocks::behaviour::sturdy(n, dir.opposite(), Support::Full) || collision_face_full(n, dir.opposite())
}

/// A sculk vein's `MultifaceSpreader` (`SculkVeinSpreaderConfig`): `same_space` spreads only
/// within the block (`getSameSpaceSpreader`).
fn spread_all(level: &mut RegionLevel, s: u16, p: BlockPos, same_space: bool) -> i64 {
    let mut n = 0;
    // `isOtherBlockValidAsSource`: anything but a vein spreads from every face.
    let other = !is_vein(s);
    for from in Direction::ALL {
        if !(other || has_face(s, from)) {
            continue;
        }
        for to in Direction::ALL {
            if let Some((q, face)) = spread_target(level, s, p, from, to, same_space)
                && spread_to_face(level, q, face)
            {
                n += 1;
            }
        }
    }
    n
}

/// `getSpreadFromFaceTowardDirection` with `canSpreadInto`.
fn spread_target(level: &RegionLevel, s: u16, p: BlockPos, from: Direction, to: Direction, same_space: bool) -> Option<(BlockPos, Direction)> {
    if to.axis() == from.axis() {
        return None;
    }
    if is_vein(s) && (!has_face(s, from) || has_face(s, to)) {
        return None;
    }
    let candidates = [(p, to), (p.relative(to), from), (p.relative(to).relative(from), to.opposite())];
    let n = if same_space { 1 } else { 3 };
    candidates.into_iter().take(n).find(|&(q, face)| can_spread_into(level, p, q, face))
}

/// `canSpreadInto`: the vein config's `stateCanBeReplaced` and `isValidStateForPlacement`.
fn can_spread_into(level: &RegionLevel, from: BlockPos, q: BlockPos, face: Direction) -> bool {
    let s = level.block(q);
    let against = level.block(q.relative(face));
    use kiln_data::blocks::default_state as d;
    if [d::SCULK, d::SCULK_CATALYST, d::MOVING_PISTON].iter().any(|&b| kiln_blocks::state::is(against, b)) {
        return false;
    }
    let (dx, dy, dz) = (q.x - from.x, q.y - from.y, q.z - from.z);
    if dx.abs() + dy.abs() + dz.abs() == 2 {
        let n = from.relative(face.opposite());
        if kiln_blocks::behaviour::sturdy(level.block(n), face, Support::Full) {
            return false;
        }
    }
    let fluid = kiln_data::block_logic::fluid(s);
    if fluid.amount > 0 && fluid.kind != kiln_data::block_logic::FluidKind::Water {
        return false;
    }
    if kiln_blocks::tags::is(s, "minecraft:fire") {
        return false;
    }
    let replaceable = kiln_data::block_props::replaceable(s)
        || kiln_data::blocks_types::is_air(s)
        || is_vein(s)
        || (kiln_blocks::state::is(s, d::WATER) && fluid.source);
    if !replaceable {
        return false;
    }
    // `isValidStateForPlacement`.
    if is_vein(s) && has_face(s, face) {
        return false;
    }
    can_attach_to(against, face)
}

/// `spreadToFace` (`placeBlock` with flags 2).
fn spread_to_face(level: &mut RegionLevel, q: BlockPos, face: Direction) -> bool {
    let s = level.block(q);
    let base = if is_vein(s) {
        s
    } else if kiln_data::block_logic::fluid(s).source && kiln_data::block_logic::fluid(s).kind == kiln_data::block_logic::FluidKind::Water {
        kiln_blocks::state::set_bool(kiln_data::blocks::default_state::SCULK_VEIN, "waterlogged", true)
    } else {
        kiln_data::blocks::default_state::SCULK_VEIN
    };
    kiln_blocks::set_block(level, q, kiln_blocks::state::set_bool(base, face.name(), true), flags::CLIENTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip() {
        let c = Cursor { pos: BlockPos::new(1, -2, 3), charge: 17, update_delay: 1, decay_delay: 0, facings: Some(vec![Direction::Down, Direction::North]) };
        let back = load_cursors(Some(&save_cursors(std::slice::from_ref(&c))));
        assert_eq!(back, vec![c]);
        // (0 - 4)^2 / 20^2 of half the charge.
        assert_eq!(decay_penalty(0.0, 100), 2);
        assert_eq!(non_corner_neighbours().len(), 18);
    }
}
