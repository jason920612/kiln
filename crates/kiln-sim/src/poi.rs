//! `PoiManager` queries over the chunks a region owns (points of interest live in their
//! chunks; see `kiln_world::poi`): records in range in vanilla's order, taking and releasing
//! tickets, and the distance to the nearest village section (`SectionTracker`).
//!
//! Regions are farther apart than any query reaches (64 blocks at most), so a region's own
//! chunks answer every query from inside it.

use kiln_world::poi::{self, ChunkPois, Occupancy, Record};
use kiln_world::{CellStore, ChunkPos};

fn pois<C: CellStore + ?Sized>(cells: &C, pos: ChunkPos) -> Option<&ChunkPois> {
    cells.cell(pos.cell())?.chunk(pos)?.pois.as_deref()
}

fn pois_mut<C: CellStore + ?Sized>(cells: &mut C, pos: ChunkPos) -> Option<&mut ChunkPois> {
    cells.cell_mut(pos.cell())?.chunk_mut(pos)?.pois.as_deref_mut()
}

/// `PoiManager.getInRange`: records of `kinds` within `radius` (squared distance to the block
/// positions) of `center` passing `occupancy`, chunk by chunk (x fastest), sections upward.
pub(crate) fn in_range<C: CellStore + ?Sized>(cells: &C, kinds: &dyn Fn(u8) -> bool, center: [i32; 3], radius: i32, occupancy: Occupancy) -> Vec<Record> {
    let chunk_radius = radius.div_euclid(16) + 1;
    let (ccx, ccz) = (center[0] >> 4, center[2] >> 4);
    let r2 = radius as i64 * radius as i64;
    let mut out = Vec::new();
    for cz in ccz - chunk_radius..=ccz + chunk_radius {
        for cx in ccx - chunk_radius..=ccx + chunk_radius {
            let Some(p) = pois(cells, ChunkPos::new(cx, cz)) else { continue };
            for s in p.sections.values() {
                for r in s.records(kinds, occupancy) {
                    let (dx, dy, dz) = ((r.pos[0] - center[0]) as i64, (r.pos[1] - center[1]) as i64, (r.pos[2] - center[2]) as i64);
                    if dx.abs() <= radius as i64 && dz.abs() <= radius as i64 && dx * dx + dy * dy + dz * dz <= r2 {
                        out.push(r.clone());
                    }
                }
            }
        }
    }
    out
}

/// `PoiManager.take`: the first record of `kinds` with space in range that `accept` takes gets
/// a ticket taken.
pub(crate) fn take<C: CellStore + ?Sized>(cells: &mut C, kinds: &dyn Fn(u8) -> bool, center: [i32; 3], radius: i32, accept: &dyn Fn(u8, [i32; 3]) -> bool) -> Option<[i32; 3]> {
    let found = in_range(cells, kinds, center, radius, Occupancy::HasSpace).into_iter().find(|r| accept(r.kind, r.pos))?;
    pois_mut(cells, ChunkPos::of_block(found.pos[0], found.pos[2]))?.acquire(found.pos).then_some(found.pos)
}

/// `PoiManager.release`.
pub(crate) fn release<C: CellStore + ?Sized>(cells: &mut C, pos: [i32; 3]) -> bool {
    pois_mut(cells, ChunkPos::of_block(pos[0], pos[2])).is_some_and(|p| p.release(pos))
}

/// `PoiManager.getType`.
pub(crate) fn type_at<C: CellStore + ?Sized>(cells: &C, pos: [i32; 3]) -> Option<u8> {
    pois(cells, ChunkPos::of_block(pos[0], pos[2]))?.get(pos).map(|r| r.kind)
}

/// `PoiManager.sectionsToVillage`: the distance in sections (moving to any of the 26
/// neighbours) to the nearest section holding an occupied village point of interest, 7 when
/// none is within 6. The distance tracker settles on exactly this.
pub(crate) fn sections_to_village<C: CellStore + ?Sized>(cells: &C, pos: [i32; 3]) -> i32 {
    let (sx, sy, sz) = (pos[0] >> 4, pos[1] >> 4, pos[2] >> 4);
    let mut best = 7;
    // Chunks in rings around the section's own, nearest first: a section in ring `r` is at least
    // `r` away, so once `best` has been reached the farther rings cannot improve on it.
    for r in 0i32..=6 {
        if r >= best {
            break;
        }
        for dz in -r..=r {
            let step = if dz.abs() == r { 1 } else { (2 * r).max(1) };
            let mut dx = -r;
            while dx <= r {
                if let Some(p) = pois(cells, ChunkPos::new(sx + dx, sz + dz)) {
                    for (&y, s) in p.sections.range(sy - 6..=sy + 6) {
                        let d = r.max((y - sy).abs());
                        if d < best && s.is_village_center() {
                            best = d;
                        }
                    }
                }
                dx += step;
            }
        }
    }
    best
}

/// The sections within `radius` (a cube) of section `at` that hold an occupied village point of
/// interest: what [`sections_to_village`] looks for.
pub(crate) fn village_centers<C: CellStore + ?Sized>(cells: &C, at: [i32; 3], radius: i32) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for cz in at[2] - radius..=at[2] + radius {
        for cx in at[0] - radius..=at[0] + radius {
            let Some(p) = pois(cells, ChunkPos::new(cx, cz)) else { continue };
            for (&y, s) in p.sections.range(at[1] - radius..=at[1] + radius) {
                if s.is_village_center() {
                    out.push((cx, y, cz));
                }
            }
        }
    }
    out
}

/// A `minecraft:point_of_interest_type` entry or `#tag` as a predicate on type indices.
pub(crate) fn kinds_of(names: &[&str]) -> impl Fn(u8) -> bool + use<> {
    let mut want = [false; poi::TYPES.len()];
    for n in names {
        match *n {
            "#minecraft:village" => (0..poi::TYPES.len()).filter(|&i| poi::is_village(i as u8)).for_each(|i| want[i] = true),
            "#minecraft:acquirable_job_site" => (0..poi::TYPES.len()).filter(|&i| poi::is_job_site(i as u8)).for_each(|i| want[i] = true),
            "#minecraft:bee_home" => {
                want[15] = true;
                want[16] = true;
            }
            name => {
                if let Some(i) = poi::type_index(name) {
                    want[i as usize] = true;
                }
            }
        }
    }
    move |k: u8| want.get(k as usize).copied().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_world::{Cell, CellPos};
    use std::collections::HashMap;

    type Cells = HashMap<CellPos, Box<Cell>>;
    use kiln_world::chunk::Chunk;
    use kiln_world::section::Section;

    fn level_with(beds: &[[i32; 3]], claim: bool) -> Cells {
        let mut cells: Cells = HashMap::new();
        for cz in -3..=3 {
            for cx in -3..=3 {
                let pos = ChunkPos::new(cx, cz);
                let mut chunk = Chunk::new((0..24).map(|_| Section::filled(0, 0)).collect(), -64);
                chunk.init_pois(cx, cz, None);
                cells.entry(pos.cell()).or_default().insert(pos, chunk);
            }
        }
        let bed = kiln_data::blocks_types::block_by_name("minecraft:red_bed").unwrap();
        let head = (bed.first..=bed.last).find(|&s| kiln_world::poi::type_of(s).is_some()).unwrap();
        for b in beds {
            let pos = ChunkPos::of_block(b[0], b[2]);
            let c = cells.get_mut(&pos.cell()).unwrap().chunk_mut(pos).unwrap();
            c.set((b[0] & 15) as usize, b[1], (b[2] & 15) as usize, head);
            if claim {
                c.pois.as_mut().unwrap().acquire(*b);
            }
        }
        cells
    }

    #[test]
    fn beds_make_a_village_once_claimed() {
        let cells = level_with(&[[5, 70, 5]], false);
        assert_eq!(in_range(&cells, &kinds_of(&["minecraft:home"]), [0, 70, 0], 16, Occupancy::Any).len(), 1);
        assert_eq!(sections_to_village(&cells, [5, 70, 5]), 7);
        let mut cells = level_with(&[[5, 70, 5]], true);
        assert_eq!(sections_to_village(&cells, [5, 70, 5]), 0);
        assert_eq!(sections_to_village(&cells, [20, 70, 5]), 1);
        assert_eq!(sections_to_village(&cells, [40, 110, 5]), 2);
        assert!(release(&mut cells, [5, 70, 5]));
        assert_eq!(sections_to_village(&cells, [5, 70, 5]), 7);
        let got = take(&mut cells, &kinds_of(&["#minecraft:village"]), [0, 70, 0], 48, &|_, _| true);
        assert_eq!(got, Some([5, 70, 5]));
        assert_eq!(take(&mut cells, &kinds_of(&["#minecraft:village"]), [0, 70, 0], 48, &|_, _| true), None);
    }
}
