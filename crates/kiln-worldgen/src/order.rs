//! Kiln's canonical decoration order.
//!
//! Vanilla decorates a chunk (FEATURES, write radius 1) whenever its neighbours have terrain,
//! so where features cross chunk borders the result depends on which neighbour happened to be
//! decorated first (MC-55596). Kiln fixes the order: two chunks whose 3×3 write windows overlap
//! (Chebyshev distance ≤ 2) are decorated in order of [`rank`], a function of `(x mod 3,
//! z mod 3)`; chunks of equal rank are at least 3 apart and do not interact. The result equals
//! vanilla decorating every chunk in `(rank, x, z)` order, whatever order threads get to them.
//!
//! The rank permutation minimises how far the "decorated before" closure of a chunk reaches
//! (at most 7 chunks; 31 chunks on average) among all 9! orders of the residue classes.

use std::collections::HashSet;

/// Decoration rank by `(x mod 3) * 3 + (z mod 3)`.
pub const RANK: [u8; 9] = [0, 1, 2, 7, 6, 3, 8, 5, 4];

/// A chunk's decoration rank: within distance 2, lower ranks are decorated first.
#[inline]
pub fn rank(x: i32, z: i32) -> u8 {
    RANK[(x.rem_euclid(3) * 3 + z.rem_euclid(3)) as usize]
}

/// The chunks within distance 2 of `(x, z)` that must be decorated before it.
pub fn predecessors(x: i32, z: i32) -> impl Iterator<Item = (i32, i32)> {
    let r = rank(x, z);
    (-2..=2).flat_map(move |dx| (-2..=2).map(move |dz| (x + dx, z + dz))).filter(move |&(a, b)| rank(a, b) < r)
}

/// Every chunk to decorate so that each target is final (all chunks within distance 1 of it
/// decorated), in canonical `(rank, x, z)` order.
pub fn decoration_order(targets: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut set: HashSet<(i32, i32)> = HashSet::new();
    let mut stack = Vec::new();
    for &(x, z) in targets {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if set.insert((x + dx, z + dz)) {
                    stack.push((x + dx, z + dz));
                }
            }
        }
    }
    while let Some((x, z)) = stack.pop() {
        for p in predecessors(x, z) {
            if set.insert(p) {
                stack.push(p);
            }
        }
    }
    let mut out: Vec<(i32, i32)> = set.into_iter().collect();
    out.sort_by_key(|&(x, z)| (rank(x, z), x, z));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interacting_chunks_have_distinct_ranks() {
        for x in 0..3 {
            for z in 0..3 {
                for dx in -2..=2i32 {
                    for dz in -2..=2i32 {
                        if (dx, dz) != (0, 0) {
                            assert_ne!(rank(x, z), rank(x + dx, z + dz), "{x},{z} vs +{dx},{dz}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn closure_reach_is_bounded() {
        for &(x, z) in &[(0, 0), (1, 0), (2, 2), (-1, 5)] {
            let order = decoration_order(&[(x, z)]);
            let reach = order.iter().map(|&(a, b)| (a - x).abs().max((b - z).abs())).max().unwrap();
            assert!(reach <= 8, "reach {reach}");
        }
    }
}
