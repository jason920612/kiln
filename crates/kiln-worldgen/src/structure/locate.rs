//! `ChunkGenerator.findNearestMapStructure`: the nearest structure start of some structures,
//! for `/locate structure` and the eye of ender.
//!
//! Random-spread placements are searched in square rings of grid cells around the origin chunk
//! (the first ring with a hit decides, the closest hit of that ring wins); concentric rings
//! (strongholds) check every ring position, nearest first. A candidate chunk is real when the
//! chunk's generated starts contain the structure (vanilla gets there through
//! `StructureCheck.checkStart` and the chunk's `STRUCTURE_STARTS`).

use super::{Start, Structures};
use super::placement::Placement;
use crate::generator::Generator;
use std::sync::Arc;

/// A located structure start: the block position `getLocatePos` gives and the structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub pos: [i32; 3],
    pub structure: usize,
}

/// `Vec3i.distSqr` as a double.
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> f64 {
    let d = |i: usize| (a[i] as f64) - (b[i] as f64);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

impl Structures {
    /// `getPlacementsForStructure` over `wanted`, grouped per structure set (the placement
    /// object of vanilla's map key) in first-seen order: (set, wanted structures of it).
    fn placements_of(&self, wanted: &[usize]) -> Vec<(usize, Vec<usize>)> {
        let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
        for &st in wanted {
            for &set in &self.possible {
                if !self.sets[set].structures.iter().any(|&(s, _)| s == st) {
                    continue;
                }
                match out.iter_mut().find(|(s, _)| *s == set) {
                    Some((_, list)) => {
                        if !list.contains(&st) {
                            list.push(st);
                        }
                    }
                    None => out.push((set, vec![st])),
                }
            }
        }
        out
    }

    /// `StructurePlacement.getLocatePos`.
    fn locate_pos(&self, set: usize, chunk: (i32, i32)) -> [i32; 3] {
        let o = self.sets[set].placement.common().locate_offset;
        [(chunk.0 << 4) + o.0, o.1, (chunk.1 << 4) + o.2]
    }

    /// `ChunkGenerator.findNearestMapStructure(level, structures, origin, radius, false)` for
    /// the structures `wanted` (indices), where `starts(x, z)` gives a chunk's generated starts.
    pub fn find_nearest(
        &self,
        generator: &Generator,
        starts: &mut dyn FnMut(i32, i32) -> Arc<Vec<Start>>,
        wanted: &[usize],
        origin: [i32; 3],
        radius: i32,
    ) -> Option<Found> {
        let placements = self.placements_of(wanted);
        if placements.is_empty() {
            return None;
        }
        let mut closest: Option<Found> = None;
        let mut closest_distance = f64::MAX;
        let mut spreads: Vec<&(usize, Vec<usize>)> = Vec::new();
        for entry in &placements {
            match &self.sets[entry.0].placement {
                Placement::ConcentricRings { .. } => {
                    if let Some(found) = self.nearest_in_rings(generator, starts, entry, origin) {
                        let d = dist_sqr(origin, found.pos);
                        if d < closest_distance {
                            closest_distance = d;
                            closest = Some(found);
                        }
                    }
                }
                Placement::RandomSpread { .. } => spreads.push(entry),
            }
        }
        if !spreads.is_empty() {
            let (cx, cz) = (origin[0] >> 4, origin[2] >> 4);
            for ring in 0..=radius {
                let mut hit = false;
                for entry in &spreads {
                    if let Some(found) = self.nearest_in_spread_ring(starts, entry, cx, cz, ring) {
                        hit = true;
                        let d = dist_sqr(origin, found.pos);
                        if d < closest_distance {
                            closest_distance = d;
                            closest = Some(found);
                        }
                    }
                }
                if hit {
                    return closest;
                }
            }
        }
        closest
    }

    /// `getStructureGeneratingAt`: the first of the entry's structures that has a start in
    /// `chunk`.
    fn generating_at(&self, starts: &mut dyn FnMut(i32, i32) -> Arc<Vec<Start>>, entry: &(usize, Vec<usize>), chunk: (i32, i32)) -> Option<Found> {
        let (set, structures) = entry;
        let all = starts(chunk.0, chunk.1);
        for &st in structures {
            if all.iter().any(|s| s.structure == st) {
                return Some(Found { pos: self.locate_pos(*set, chunk), structure: st });
            }
        }
        None
    }

    /// The ring-placement overload of `getNearestGeneratedStructure`.
    fn nearest_in_rings(
        &self,
        generator: &Generator,
        starts: &mut dyn FnMut(i32, i32) -> Arc<Vec<Start>>,
        entry: &(usize, Vec<usize>),
        origin: [i32; 3],
    ) -> Option<Found> {
        let positions = self.ring_positions(generator, entry.0)?;
        let mut closest: Option<Found> = None;
        let mut closest_distance = f64::MAX;
        for &(cx, cz) in positions {
            let d = dist_sqr([(cx << 4) + 8, 32, (cz << 4) + 8], origin);
            if closest.is_none() || d < closest_distance {
                if let Some(found) = self.generating_at(starts, entry, (cx, cz)) {
                    closest = Some(found);
                    closest_distance = d;
                }
            }
        }
        closest
    }

    /// The random-spread overload: the cells on the square ring `ring` around the origin
    /// chunk, in vanilla's scan order (x, then z); the first with a start wins.
    fn nearest_in_spread_ring(
        &self,
        starts: &mut dyn FnMut(i32, i32) -> Arc<Vec<Start>>,
        entry: &(usize, Vec<usize>),
        section_x: i32,
        section_z: i32,
        ring: i32,
    ) -> Option<Found> {
        let placement = &self.sets[entry.0].placement;
        let Placement::RandomSpread { spacing, .. } = placement else { return None };
        for x in -ring..=ring {
            let edge_x = x == -ring || x == ring;
            for z in -ring..=ring {
                let edge_z = z == -ring || z == ring;
                if !edge_x && !edge_z {
                    continue;
                }
                let sx = section_x.wrapping_add(spacing.wrapping_mul(x));
                let sz = section_z.wrapping_add(spacing.wrapping_mul(z));
                let Some(chunk) = placement.potential_chunk(self.seed, sx, sz) else { continue };
                if let Some(found) = self.generating_at(starts, entry, chunk) {
                    return Some(found);
                }
            }
        }
        None
    }
}
