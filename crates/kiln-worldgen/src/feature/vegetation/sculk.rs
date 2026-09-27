//! `SculkPatchFeature`: charge cursors of a world generation `SculkSpreader` turning blocks
//! into sculk, with veins, sensors and shriekers (`SculkSpreader`, `SculkSpreader.ChargeCursor`,
//! and the `SculkBehaviour` of sculk, sculk veins and other blocks).

use super::multiface::{Spreader, all_shuffled, has_face, shuffle};
use super::shape::can_attach_to;
use crate::Error;
use crate::block_facts::{Dir, FluidKind, Support, fluid, is_face_sturdy};
use crate::blocks::{has_prop, is_air, is_block, is_water, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::int;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::vtags;
use kiln_javamath::random::RandomSource;

/// `SculkSpreader.createWorldGenSpreader` constants.
const GROWTH_SPAWN_COST: i32 = 50;
const NO_GROWTH_RADIUS: i32 = 1;
const CHARGE_DECAY_RATE: i32 = 5;
const ADDITIONAL_DECAY_RATE: i32 = 10;
const REPLACEABLE: &str = "sculk_replaceable_world_gen";

#[derive(Debug)]
pub struct SculkPatch {
    charge_count: i32,
    amount_per_charge: i32,
    spread_attempts: i32,
    growth_rounds: i32,
    spread_rounds: i32,
}

/// Which `SculkBehaviour` a block has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Sculk,
    Vein,
    Default,
}

fn behaviour(s: u16) -> Behaviour {
    if is_block(s, "minecraft:sculk") {
        Behaviour::Sculk
    } else if is_block(s, "minecraft:sculk_vein") {
        Behaviour::Vein
    } else {
        Behaviour::Default
    }
}

/// `SculkSpreader.ChargeCursor`.
#[derive(Clone, Debug)]
struct Cursor {
    pos: BlockPos,
    charge: i32,
    update_delay: i32,
    decay_delay: i32,
    /// `facings`: `None` until the cursor sits on a sculk block (then the faces of it, as
    /// `MultifaceBlock.availableFaces`, in direction order).
    facings: Option<Vec<Dir>>,
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

impl SculkPatch {
    pub fn parse(json: &Json) -> Result<Self, Error> {
        Ok(Self {
            charge_count: int(json, "charge_count")?,
            amount_per_charge: int(json, "amount_per_charge")?,
            spread_attempts: int(json, "spread_attempts")?,
            growth_rounds: int(json, "growth_rounds")?,
            spread_rounds: int(json, "spread_rounds")?,
        })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        if !can_spread_from(r, origin) {
            return false;
        }
        let mut cursors: Vec<Cursor> = Vec::new();
        for round in 0..self.spread_rounds + self.growth_rounds {
            for _ in 0..self.charge_count {
                let mut amount = self.amount_per_charge;
                while amount > 0 {
                    let c = amount.min(1000);
                    if cursors.len() < 32 {
                        cursors.push(Cursor { pos: origin, charge: c, update_delay: 0, decay_delay: 1, facings: None });
                    }
                    amount -= c;
                }
            }
            let spread_veins = round < self.spread_rounds;
            for _ in 0..self.spread_attempts {
                update_cursors(&mut cursors, r, origin, random, spread_veins);
            }
            cursors.clear();
        }
        true
    }
}

/// `SculkPatchFeature.canSpreadFrom`.
fn can_spread_from(r: &mut Region, p: BlockPos) -> bool {
    let s = r.get(p);
    if behaviour(s) != Behaviour::Default {
        return true;
    }
    if is_air(s) || (is_water(s) && fluid(s).source) {
        return Dir::ALL.into_iter().any(|d| kiln_data::block_props::full_collision(r.get(p.relative(d))));
    }
    false
}

/// `SculkSpreader.updateCursors` (world generation: cursors never merge).
fn update_cursors(cursors: &mut Vec<Cursor>, r: &mut Region, origin: BlockPos, random: &mut WorldgenRandom, spread_veins: bool) {
    if cursors.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(cursors.len());
    for mut c in cursors.drain(..) {
        let d = c.pos;
        if (d.x - origin.x).abs().max((d.y - origin.y).abs()).max((d.z - origin.z).abs()) > 1024 {
            continue;
        }
        c.update(r, origin, random, spread_veins);
        if c.charge > 0 {
            kept.push(c);
        }
    }
    *cursors = kept;
}

impl Cursor {
    /// `ChargeCursor.update`.
    fn update(&mut self, r: &mut Region, origin: BlockPos, random: &mut WorldgenRandom, spread_veins: bool) {
        if self.charge <= 0 {
            return;
        }
        if self.update_delay > 0 {
            self.update_delay -= 1;
            return;
        }
        let mut state = r.get(self.pos);
        let mut b = behaviour(state);
        if spread_veins && attempt_spread_vein(b, r, self.pos, state, self.facings.as_deref()) && b != Behaviour::Sculk {
            state = r.get(self.pos);
            b = behaviour(state);
        }
        self.charge = self.attempt_use_charge(b, r, origin, random, spread_veins);
        if self.charge <= 0 {
            on_discharged(b, r, state, self.pos, random);
            return;
        }
        match valid_movement_pos(r, self.pos, random, origin) {
            Some(next) => {
                on_discharged(b, r, state, self.pos, random);
                self.pos = next;
                state = r.get(next);
            }
            None => {
                on_discharged(b, r, state, self.pos, random);
                self.charge = 0;
                return;
            }
        }
        if behaviour(state) != Behaviour::Default {
            self.facings = Some(Dir::ALL.into_iter().filter(|&d| has_face(state, d)).collect());
        }
        self.decay_delay = if b == Behaviour::Default { (self.decay_delay - 1).max(0) } else { 1 };
        self.update_delay = 1;
    }

    /// `SculkBehaviour.attemptUseCharge`.
    fn attempt_use_charge(&self, b: Behaviour, r: &mut Region, origin: BlockPos, random: &mut WorldgenRandom, spread_veins: bool) -> i32 {
        let charge = self.charge;
        match b {
            Behaviour::Default => {
                if self.decay_delay > 0 {
                    charge
                } else {
                    0
                }
            }
            Behaviour::Vein => {
                if spread_veins && attempt_place_sculk(r, self.pos, random) {
                    return charge - 1;
                }
                if random.next_int_bounded(CHARGE_DECAY_RATE) == 0 { kiln_javamath::math::floor_f32(charge as f32 * 0.5) } else { charge }
            }
            Behaviour::Sculk => {
                if charge == 0 || random.next_int_bounded(CHARGE_DECAY_RATE) != 0 {
                    return charge;
                }
                let p = self.pos;
                let near = p.dist_sqr(origin) < (NO_GROWTH_RADIUS * NO_GROWTH_RADIUS) as f64;
                if near || !can_place_growth(r, p) {
                    if random.next_int_bounded(ADDITIONAL_DECAY_RATE) != 0 {
                        return charge;
                    }
                    return charge - if near { 1 } else { decay_penalty(p, origin, charge) };
                }
                if random.next_int_bounded(GROWTH_SPAWN_COST) < charge {
                    let above = p.above();
                    let s = random_growth_state(r, above, random);
                    r.set(above, s, 3);
                }
                0.max(charge - GROWTH_SPAWN_COST)
            }
        }
    }
}

/// `SculkBlock.getDecayPenalty`.
fn decay_penalty(p: BlockPos, origin: BlockPos, charge: i32) -> i32 {
    let d = p.dist_sqr(origin).sqrt() as f32 - NO_GROWTH_RADIUS as f32;
    let f = d * d;
    let i = (24 - NO_GROWTH_RADIUS) * (24 - NO_GROWTH_RADIUS);
    let g = 1.0f32.min(f / i as f32);
    1.max((charge as f32 * g * 0.5) as i32)
}

/// `SculkBlock.getRandomGrowthState`.
fn random_growth_state(r: &mut Region, p: BlockPos, random: &mut WorldgenRandom) -> u16 {
    let s = if random.next_int_bounded(11) == 0 {
        with_prop(crate::blocks::state::SCULK_SHRIEKER, "can_summon", "true")
    } else {
        crate::blocks::state::SCULK_SENSOR
    };
    if has_prop(s, "waterlogged") && !r.fluid(p).is_empty() { with_prop(s, "waterlogged", "true") } else { s }
}

/// `SculkBlock.canPlaceGrowth`.
fn can_place_growth(r: &mut Region, p: BlockPos) -> bool {
    let above = r.get(p.above());
    if !(is_air(above) || (is_water(above) && fluid(above).kind == FluidKind::Water)) {
        return false;
    }
    let mut inhibitors = 0;
    for y in p.y..=p.y + 2 {
        if r.is_outside_build_height(y) {
            continue;
        }
        for z in p.z - 4..=p.z + 4 {
            for x in p.x - 4..=p.x + 4 {
                if vtags::is(r.get(BlockPos::new(x, y, z)), "sculk_growth_inhibitors") {
                    inhibitors += 1;
                }
            }
        }
    }
    inhibitors <= 2
}

/// `SculkBehaviour.attemptSpreadVein`.
fn attempt_spread_vein(b: Behaviour, r: &mut Region, p: BlockPos, state: u16, facings: Option<&[Dir]>) -> bool {
    if b == Behaviour::Default {
        match facings {
            None => {
                let s = r.get(p);
                return Spreader::sculk_same_space().spread_all(r, s, p, true) > 0;
            }
            Some(faces) if !faces.is_empty() => {
                return (is_air(state) || fluid(state).kind == FluidKind::Water) && regrow(r, p, state, faces);
            }
            Some(_) => {}
        }
    }
    Spreader::of(crate::blocks::state::SCULK_VEIN).spread_all(r, state, p, true) > 0
}

/// `SculkVeinBlock.regrow`.
fn regrow(r: &mut Region, p: BlockPos, state: u16, faces: &[Dir]) -> bool {
    let mut vein = crate::blocks::state::SCULK_VEIN;
    let mut any = false;
    for &d in faces {
        if can_attach_to(r.get(p.relative(d)), d) {
            vein = with_prop(vein, d.name(), "true");
            any = true;
        }
    }
    if !any {
        return false;
    }
    if !fluid(state).is_empty() {
        vein = with_prop(vein, "waterlogged", "true");
    }
    r.set(p, vein, 3);
    true
}

/// `SculkBehaviour.onDischarged` (only sculk veins do something).
fn on_discharged(b: Behaviour, r: &mut Region, state: u16, p: BlockPos, _random: &mut WorldgenRandom) {
    if b != Behaviour::Vein || !is_block(state, "minecraft:sculk_vein") {
        return;
    }
    let mut s = state;
    for d in Dir::ALL {
        if has_face(s, d) && is_block(r.get(p.relative(d)), "minecraft:sculk") {
            s = with_prop(s, d.name(), "false");
        }
    }
    if !Dir::ALL.into_iter().any(|d| has_face(s, d)) {
        s = if r.fluid(p).is_empty() { crate::blocks::state::AIR } else { crate::blocks::state::WATER };
    }
    r.set(p, s, 3);
}

/// `SculkVeinBlock.attemptPlaceSculk`.
fn attempt_place_sculk(r: &mut Region, p: BlockPos, random: &mut WorldgenRandom) -> bool {
    let state = r.get(p);
    for d in all_shuffled(random) {
        if !has_face(state, d) {
            continue;
        }
        let q = p.relative(d);
        if !vtags::is(r.get(q), REPLACEABLE) {
            continue;
        }
        let sculk = crate::blocks::state::SCULK;
        r.set(q, sculk, 3);
        Spreader::of(crate::blocks::state::SCULK_VEIN).spread_all(r, sculk, q, true);
        let back = d.opposite();
        for e in Dir::ALL {
            if e == back {
                continue;
            }
            let n = q.relative(e);
            let ns = r.get(n);
            if is_block(ns, "minecraft:sculk_vein") {
                on_discharged(Behaviour::Vein, r, ns, n, random);
            }
        }
        return true;
    }
    false
}

/// `ChargeCursor.getValidMovementPos`.
fn valid_movement_pos(r: &mut Region, p: BlockPos, random: &mut WorldgenRandom, origin: BlockPos) -> Option<BlockPos> {
    let mut offsets = non_corner_neighbours();
    shuffle(&mut offsets, random);
    let mut result = p;
    for (dx, dy, dz) in offsets {
        let m = p.offset(dx, dy, dz);
        let (ox, oz) = (origin.x - m.x, origin.z - m.z);
        if ox * ox + oz * oz > 144 {
            continue;
        }
        let s = r.get(m);
        if behaviour(s) != Behaviour::Default && movement_unobstructed(r, p, m) {
            result = m;
            if has_substrate_access(r, s, m) {
                break;
            }
        }
    }
    (result != p).then_some(result)
}

/// `ChargeCursor.isMovementUnobstructed`.
fn movement_unobstructed(r: &mut Region, from: BlockPos, to: BlockPos) -> bool {
    if from.dist_manhattan(to) == 1 {
        return true;
    }
    let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
    let x = if dx < 0 { Dir::West } else { Dir::East };
    let y = if dy < 0 { Dir::Down } else { Dir::Up };
    let z = if dz < 0 { Dir::North } else { Dir::South };
    let mut open = |d: Dir| !is_face_sturdy(r.get(from.relative(d)), d.opposite(), Support::Full);
    if dx == 0 {
        open(y) || open(z)
    } else if dy == 0 {
        open(x) || open(z)
    } else {
        open(x) || open(y)
    }
}

/// `SculkVeinBlock.hasSubstrateAccess`.
fn has_substrate_access(r: &mut Region, s: u16, p: BlockPos) -> bool {
    is_block(s, "minecraft:sculk_vein")
        && Dir::ALL.into_iter().any(|d| has_face(s, d) && vtags::is(r.get(p.relative(d)), "sculk_replaceable"))
}
