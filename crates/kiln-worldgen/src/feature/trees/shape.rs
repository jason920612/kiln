//! `StructureTemplate.updateShapeAtEdge` as trees use it: every face between a cell of the
//! tree and one outside gets `BlockState.updateShape` on both sides. Block classes whose
//! `updateShape` can change something next to a tree are mirrored here; the rest keep their
//! state (vanilla's scheduled ticks are not modelled).

use super::tree::Voxels;
use crate::block_facts::{Dir, Support, class_chain, is_face_sturdy};
use crate::blocks::{prop, same_block, state, with_prop};
use crate::pos::BlockPos;
use crate::region::Region;
use crate::vtags;

/// `StructureTemplate.updateShapeAtEdge(level, flags, shape, x, y, z)`.
pub fn update_shape_at_edge(r: &mut Region, flags: i32, shape: &Voxels, min: BlockPos) {
    let (xs, ys, zs) = shape.size;
    let full = |x: i32, y: i32, z: i32| shape.is_full(x, y, z);
    let mut faces: Vec<(Dir, i32, i32, i32)> = Vec::new();
    // AxisCycle.NONE: faces along z.
    for x in 0..xs {
        for y in 0..ys {
            scan(zs, |k| full(x, y, k), |k, d| faces.push((if d { Dir::South } else { Dir::North }, x, y, k)));
        }
    }
    // AxisCycle.FORWARD: faces along y, outer loops over z then x.
    for z in 0..zs {
        for x in 0..xs {
            scan(ys, |k| full(x, k, z), |k, d| faces.push((if d { Dir::Up } else { Dir::Down }, x, k, z)));
        }
    }
    // AxisCycle.BACKWARD: faces along x, outer loops over y then z.
    for y in 0..ys {
        for z in 0..zs {
            scan(xs, |k| full(k, y, z), |k, d| faces.push((if d { Dir::East } else { Dir::West }, k, y, z)));
        }
    }
    for (d, x, y, z) in faces {
        let p = min.offset(x, y, z);
        let n = p.relative(d);
        let s = r.get(p);
        let ns = r.get(n);
        let updated = update_shape(r, s, p, d, n, ns);
        if updated != s {
            r.set(p, updated, flags & !1);
        }
        let n_updated = update_shape(r, ns, n, d.opposite(), p, updated);
        if n_updated != ns {
            r.set(n, n_updated, flags & !1);
        }
    }
}

/// One row of `DiscreteVoxelShape.forAllAxisFaces`: reports `(k, positive)` at each boundary.
fn scan(len: i32, full: impl Fn(i32) -> bool, mut face: impl FnMut(i32, bool)) {
    let mut last = false;
    for k in 0..=len {
        let now = k != len && full(k);
        if !last && now {
            face(k, false);
        }
        if last && !now {
            face(k - 1, true);
        }
        last = now;
    }
}

/// `BlockState.updateShape(level, ticks, pos, direction, neighborPos, neighborState, random)`.
pub fn update_shape(r: &mut Region, s: u16, p: BlockPos, d: Dir, np: BlockPos, ns: u16) -> u16 {
    for class in class_chain(s).split('<') {
        if let Some(v) = class_update_shape(class, r, s, p, d, np, ns) {
            return v;
        }
    }
    s
}

/// The `updateShape` override of one class, if it decides (`None` defers to the superclass).
fn class_update_shape(class: &str, r: &mut Region, s: u16, p: BlockPos, d: Dir, _np: BlockPos, ns: u16) -> Option<u16> {
    let survives = |r: &mut Region| crate::survive::can_survive(s, r, p);
    match class {
        "VegetationBlock" | "CarpetBlock" | "SnowLayerBlock" => (!survives(r)).then_some(state::AIR),
        "DoublePlantBlock" => {
            let half = prop(s, "half");
            if !d.is_horizontal() && (half == Some("lower")) == (d == Dir::Up) && !(same_block(ns, s) && prop(ns, "half") != half) {
                return Some(state::AIR);
            }
            if half == Some("lower") && d == Dir::Down && !survives(r) {
                return Some(state::AIR);
            }
            None
        }
        "MangrovePropaguleBlock" => (d == Dir::Up && !survives(r)).then_some(state::AIR),
        "HangingMossBlock" => {
            let below = r.get(p.below());
            Some(with_prop(s, "tip", if same_block(below, s) { "false" } else { "true" }))
        }
        "SnowyBlock" => (d == Dir::Up).then(|| with_prop(s, "snowy", if vtags::is(ns, "snow") { "true" } else { "false" })),
        "CocoaBlock" => (prop(s, "facing") == Some(d.name()) && !survives(r)).then_some(state::AIR),
        "ShelfMushroomBlock" => (prop(s, "facing") == Some(d.opposite().name()) && !survives(r)).then_some(state::AIR),
        "VineBlock" => {
            if d == Dir::Down {
                return Some(s);
            }
            let updated = vine_updated_state(r, s, p);
            Some(if vine_has_faces(updated) { updated } else { state::AIR })
        }
        _ => None,
    }
}

const VINE_FACES: [(Dir, &str); 5] = [(Dir::Up, "up"), (Dir::North, "north"), (Dir::East, "east"), (Dir::South, "south"), (Dir::West, "west")];

fn vine_has_faces(s: u16) -> bool {
    VINE_FACES.iter().any(|(_, f)| prop(s, f) == Some("true"))
}

/// `MultifaceBlock.canAttachTo(level, direction, pos, state)`.
pub fn can_attach_to(d: Dir, s: u16) -> bool {
    let face = d.opposite();
    is_face_sturdy(s, face, Support::Full) || collision_face_full(s, face)
}

/// `Block.isFaceFull(state.getCollisionShape(level, pos), face)`.
fn collision_face_full(s: u16, face: Dir) -> bool {
    let boxes = kiln_data::block_props::collision(s);
    let (axis, positive) = match face {
        Dir::Down => (1, false),
        Dir::Up => (1, true),
        Dir::North => (2, false),
        Dir::South => (2, true),
        Dir::West => (0, false),
        Dir::East => (0, true),
    };
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let touching: Vec<&[f32; 6]> =
        boxes.iter().filter(|b| if positive { b[axis + 3] >= 1.0 - 1e-7 } else { b[axis] <= 1e-7 }).collect();
    if touching.is_empty() {
        return false;
    }
    let mut us: Vec<f32> = vec![0.0, 1.0];
    let mut vs: Vec<f32> = vec![0.0, 1.0];
    for b in &touching {
        us.extend([b[u].clamp(0.0, 1.0), b[u + 3].clamp(0.0, 1.0)]);
        vs.extend([b[v].clamp(0.0, 1.0), b[v + 3].clamp(0.0, 1.0)]);
    }
    us.sort_by(f32::total_cmp);
    vs.sort_by(f32::total_cmp);
    for iu in us.windows(2).filter(|w| w[1] > w[0]) {
        for iv in vs.windows(2).filter(|w| w[1] > w[0]) {
            let (cu, cv) = ((iu[0] + iu[1]) / 2.0, (iv[0] + iv[1]) / 2.0);
            if !touching.iter().any(|b| b[u] <= cu && cu <= b[u + 3] && b[v] <= cv && cv <= b[v + 3]) {
                return false;
            }
        }
    }
    true
}

/// `VineBlock.isAcceptableNeighbour`.
fn acceptable_neighbour(r: &mut Region, p: BlockPos, d: Dir) -> bool {
    can_attach_to(d, r.get(p))
}

/// `VineBlock.canSupportAtFace`.
fn vine_supported_at(r: &mut Region, p: BlockPos, d: Dir) -> bool {
    if d == Dir::Down {
        return false;
    }
    if acceptable_neighbour(r, p.relative(d), d) {
        return true;
    }
    if d == Dir::Up {
        return false;
    }
    let face = VINE_FACES.iter().find(|(f, _)| *f == d).map(|(_, n)| *n).unwrap_or("up");
    let above = r.get(p.above());
    same_block(above, state::VINE) && prop(above, face) == Some("true")
}

/// `VineBlock.getUpdatedState`.
pub fn vine_updated_state(r: &mut Region, s: u16, p: BlockPos) -> u16 {
    let mut s = s;
    if prop(s, "up") == Some("true") {
        let ok = acceptable_neighbour(r, p.above(), Dir::Down);
        s = with_prop(s, "up", if ok { "true" } else { "false" });
    }
    let mut above: Option<u16> = None;
    for d in Dir::HORIZONTAL {
        let face = d.name();
        if prop(s, face) != Some("true") {
            continue;
        }
        let mut ok = vine_supported_at(r, p, d);
        if !ok {
            let a = *above.get_or_insert_with(|| r.get(p.above()));
            ok = same_block(a, state::VINE) && prop(a, face) == Some("true");
        }
        s = with_prop(s, face, if ok { "true" } else { "false" });
    }
    s
}
