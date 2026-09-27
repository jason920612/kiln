//! Shape queries on collision boxes that `block_facts` does not precompute.

use crate::block_facts::{Dir, Support, is_face_sturdy};

/// `Block.isFaceFull(state.getCollisionShape(level, pos), dir)`: the collision boxes touching
/// the block's `dir` face cover it completely.
pub fn collision_face_full(state: u16, dir: Dir) -> bool {
    const EPS: f32 = 1.0e-7;
    let (axis, positive) = match dir {
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

/// `MultifaceBlock.canAttachTo(level, dir, pos, state)`: a block attached on its `dir` side to
/// `state` (support or collision face full).
pub fn can_attach_to(state: u16, dir: Dir) -> bool {
    is_face_sturdy(state, dir.opposite(), Support::Full) || collision_face_full(state, dir.opposite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn faces() {
        assert!(collision_face_full(d::STONE, Dir::North));
        assert!(!collision_face_full(d::AIR, Dir::Up));
        assert!(collision_face_full(d::STONE_SLAB, Dir::Down));
        assert!(!collision_face_full(d::STONE_SLAB, Dir::Up));
        assert!(!collision_face_full(d::STONE_SLAB, Dir::North));
    }
}
