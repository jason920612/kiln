//! Incremental block and sky light propagation over loaded chunks.
//!
//! Increases spread breadth-first, each step losing max(1, dampening) of the entered block;
//! sky light at full strength goes straight down without loss through blocks that let
//! skylight through. Decreases clear every level that could have come from the removed
//! light, then re-spread from the brighter levels found at the edge of the cleared area.

use crate::chunk::LightLayer;
use crate::{Blocks, CellStore, ChunkPos};
use kiln_data::block_props::{face_full, light_dampening, light_emission, propagates_skylight_down, uses_shape_for_light_occlusion};
use std::collections::VecDeque;

/// Neighbor offsets indexed by protocol direction id: down, up, north, south, west, east.
const DIRS: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];
const DOWN: usize = 0;

type Queue = VecDeque<(i32, i32, i32, u8)>;

/// Light cannot cross between two blocks if either one's face on the shared side is solid.
fn occludes(from: u16, to: u16, dir: usize) -> bool {
    (uses_shape_for_light_occlusion(from) && face_full(from, dir as u8))
        || (uses_shape_for_light_occlusion(to) && face_full(to, (dir ^ 1) as u8))
}

/// Level arriving in `to` from a neighbor at `level` travelling in `dir`.
fn transmitted(layer: LightLayer, level: u8, to: u16, dir: usize) -> u8 {
    if layer == LightLayer::Sky && dir == DOWN && level == 15 && propagates_skylight_down(to) {
        15
    } else {
        level.saturating_sub(light_dampening(to).max(1))
    }
}

/// Whether a block state change can affect light.
pub fn affects_light(old: u16, new: u16) -> bool {
    light_emission(old) != light_emission(new)
        || light_dampening(old) != light_dampening(new)
        || propagates_skylight_down(old) != propagates_skylight_down(new)
        || uses_shape_for_light_occlusion(old)
        || uses_shape_for_light_occlusion(new)
}

/// Block state if its chunk is loaded and `y` is within the chunk's stored light. Positions
/// outside take no part in propagation (full sky light would otherwise keep travelling down
/// through void air forever).
fn state_at<S: CellStore + ?Sized>(w: &S, x: i32, y: i32, z: i32) -> Option<u16> {
    let c = w.chunk(ChunkPos::of_block(x, z))?;
    c.in_light_range(y).then(|| c.get((x & 15) as usize, y, (z & 15) as usize))
}

/// Light level at a position, if its chunk is loaded and light is stored there.
pub fn light_at<S: CellStore + ?Sized>(w: &S, layer: LightLayer, x: i32, y: i32, z: i32) -> Option<u8> {
    let c = w.chunk(ChunkPos::of_block(x, z))?;
    c.in_light_range(y).then(|| c.light(layer, (x & 15) as usize, y, (z & 15) as usize))
}

fn set_light_at<S: CellStore + ?Sized>(w: &mut S, layer: LightLayer, x: i32, y: i32, z: i32, v: u8) {
    if let Some(c) = w.chunk_mut(ChunkPos::of_block(x, z)) {
        c.set_light(layer, (x & 15) as usize, y, (z & 15) as usize, v);
    }
}

/// Re-lights around a block that changed from `old` to `new`.
pub fn update_light<S: CellStore + ?Sized>(w: &mut S, x: i32, y: i32, z: i32, old: u16, new: u16) {
    if !affects_light(old, new) {
        return;
    }
    for layer in [LightLayer::Block, LightLayer::Sky] {
        let mut relight = Queue::new();
        let current = light_at(w, layer, x, y, z).unwrap_or(0);
        if current > 0 {
            set_light_at(w, layer, x, y, z, 0);
            let mut removal = Queue::from([(x, y, z, current)]);
            decrease(w, layer, &mut removal, &mut relight);
        }
        if layer == LightLayer::Block && light_emission(new) > 0 {
            let e = light_emission(new);
            set_light_at(w, layer, x, y, z, e);
            relight.push_back((x, y, z, e));
        }
        // Neighbors may now shine into (or through) the changed block.
        for (dx, dy, dz) in DIRS {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if let Some(l) = light_at(w, layer, nx, ny, nz).filter(|&l| l > 0) {
                relight.push_back((nx, ny, nz, l));
            }
        }
        if layer == LightLayer::Sky && light_at(w, layer, x, y + 1, z).is_none_or(|l| l == 15) {
            relight.push_back((x, y + 1, z, 15));
        }
        increase(w, layer, &mut relight);
    }
}

/// Lights a chunk just installed next to its loaded neighbours (a generated chunk, whose
/// sky light [`crate::chunk::Chunk::new`] derived from its column tops): block light from
/// its emitters; sky light down through translucent tops (water, leaves) and sideways into
/// shaded columns; and light from the neighbours' borders spreading in, and back out.
/// Vanilla does this in the `INITIALIZE_LIGHT` and `LIGHT` statuses (`LightEngine`).
pub fn light_new_chunk<S: CellStore + ?Sized>(w: &mut S, pos: ChunkPos) {
    let Some(c) = w.chunk(pos) else { return };
    let (min_y, max_y) = (c.min_y(), c.min_y() + c.height() - 1);
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let mut block = Queue::new();
    let mut sky = Queue::new();
    for y in min_y..=max_y {
        for z in 0..16 {
            for x in 0..16 {
                let e = light_emission(c.get(x, y, z));
                if e > 0 {
                    block.push_back((bx + x as i32, y, bz + z as i32, e));
                }
            }
        }
    }
    // Column tops (y above the highest non-air block) of the chunk and the ring around it.
    let top = |w: &S, x: i32, z: i32| {
        w.chunk(ChunkPos::of_block(x, z))
            .map(|c| c.column_height((x & 15) as usize, (z & 15) as usize, |s| !kiln_data::blocks_types::is_air(s)))
    };
    let mut tops = [[None; 18]; 18];
    for (i, row) in tops.iter_mut().enumerate() {
        for (j, t) in row.iter_mut().enumerate() {
            *t = top(w, bx + i as i32 - 1, bz + j as i32 - 1);
        }
    }
    for &(x, y, z, e) in &block {
        if e > light_at(w, LightLayer::Block, x, y, z).unwrap_or(15) {
            set_light_at(w, LightLayer::Block, x, y, z, e);
        }
    }
    let chunk_top = tops[1..17].iter().flat_map(|row| row[1..17].iter().flatten()).copied().max().unwrap_or(min_y);
    for i in 1..17 {
        for j in 1..17 {
            let (x, z) = (bx + i as i32 - 1, bz + j as i32 - 1);
            let Some(own) = tops[i][j] else { continue };
            // Full sky light next to lower columns spreads sideways.
            let highest = [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)].iter().filter_map(|&(a, b)| tops[a][b]).max().unwrap_or(own);
            for y in own..highest.min(max_y + 1) {
                sky.push_back((x, y, z, 15));
            }
            // Straight down through the top blocks while they let light through.
            let mut level = 15;
            let mut above = kiln_data::blocks::default_state::AIR;
            for y in (min_y..own).rev() {
                let Some(to) = state_at(w, x, y, z) else { break };
                if occludes(above, to, DOWN) {
                    break;
                }
                level = transmitted(LightLayer::Sky, level, to, DOWN);
                if level == 0 {
                    break;
                }
                if level > light_at(w, LightLayer::Sky, x, y, z).unwrap_or(15) {
                    set_light_at(w, LightLayer::Sky, x, y, z, level);
                    sky.push_back((x, y, z, level));
                }
                above = to;
            }
        }
    }
    // The neighbours' border columns shine in.
    for k in 0..16 {
        for (x, z) in [(bx - 1, bz + k), (bx + 16, bz + k), (bx + k, bz - 1), (bx + k, bz + 16)] {
            if w.chunk(ChunkPos::of_block(x, z)).is_none() {
                continue;
            }
            for y in min_y..=max_y {
                if let Some(l) = light_at(w, LightLayer::Block, x, y, z).filter(|&l| l > 1) {
                    block.push_back((x, y, z, l));
                }
                // Above every column of the chunk, its sky light is full already.
                if y < chunk_top
                    && let Some(l) = light_at(w, LightLayer::Sky, x, y, z).filter(|&l| l > 1)
                {
                    sky.push_back((x, y, z, l));
                }
            }
        }
    }
    increase(w, LightLayer::Block, &mut block);
    increase(w, LightLayer::Sky, &mut sky);
}

fn increase<S: CellStore + ?Sized>(w: &mut S, layer: LightLayer, queue: &mut Queue) {
    while let Some((x, y, z, level)) = queue.pop_front() {
        if level <= 1 {
            continue;
        }
        let from = state_at(w, x, y, z).unwrap_or(kiln_data::blocks::default_state::AIR);
        for (dir, (dx, dy, dz)) in DIRS.iter().enumerate() {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let Some(to) = state_at(w, nx, ny, nz) else { continue };
            if occludes(from, to, dir) {
                continue;
            }
            let v = transmitted(layer, level, to, dir);
            if v > light_at(w, layer, nx, ny, nz).unwrap_or(15) {
                set_light_at(w, layer, nx, ny, nz, v);
                queue.push_back((nx, ny, nz, v));
            }
        }
    }
}

fn decrease<S: CellStore + ?Sized>(w: &mut S, layer: LightLayer, removal: &mut Queue, relight: &mut Queue) {
    while let Some((x, y, z, level)) = removal.pop_front() {
        for (dir, (dx, dy, dz)) in DIRS.iter().enumerate() {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let Some(to) = state_at(w, nx, ny, nz) else { continue };
            let cur = light_at(w, layer, nx, ny, nz).unwrap_or(0);
            if cur == 0 {
                continue;
            }
            // Full sky light directly below full sky light came from above.
            let dependent_sky = layer == LightLayer::Sky && dir == DOWN && level == 15 && cur == 15;
            if cur < level || dependent_sky {
                set_light_at(w, layer, nx, ny, nz, 0);
                removal.push_back((nx, ny, nz, cur));
                if layer == LightLayer::Block && light_emission(to) > 0 {
                    let e = light_emission(to);
                    set_light_at(w, layer, nx, ny, nz, e);
                    relight.push_back((nx, ny, nz, e));
                }
            } else {
                relight.push_back((nx, ny, nz, cur));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::chunk::LightLayer::{Block, Sky};
    use crate::{Blocks, OVERWORLD, World};
    use kiln_data::blocks::default_state as b;

    fn world() -> World {
        let mut w = World::flat(OVERWORLD, 0, 67);
        for cx in -2..=2 {
            for cz in -2..=2 {
                w.load_chunk(crate::ChunkPos::new(cx, cz));
            }
        }
        w
    }

    #[test]
    fn torch_lights_and_unlights_surroundings() {
        let mut w = world();
        let y = -60; // first air layer above the flat ground
        w.set_block(0, y, 0, b::TORCH);
        assert_eq!(w.light_at(Block, 0, y, 0), Some(14));
        assert_eq!(w.light_at(Block, 1, y, 0), Some(13));
        assert_eq!(w.light_at(Block, 5, y, 3), Some(6)); // manhattan distance 8
        assert_eq!(w.light_at(Block, -14, y, 0), Some(0));
        assert_eq!(w.light_at(Block, 0, y - 1, 0), Some(0), "grass is opaque");
        w.set_block(0, y, 0, b::AIR);
        for x in -15..=15 {
            assert_eq!(w.light_at(Block, x, y, 0), Some(0), "at x={x}");
        }
    }

    #[test]
    fn roof_shades_the_ground_and_sky_returns_when_removed() {
        let mut w = world();
        let y = -60;
        assert_eq!(w.light_at(Sky, 0, y, 0), Some(15));
        // A 9x9 stone roof two blocks up leaves the middle dark except for side light.
        for x in -4..=4 {
            for z in -4..=4 {
                w.set_block(x, y + 2, z, b::STONE);
            }
        }
        assert_eq!(w.light_at(Sky, 0, y, 0), Some(10)); // 15 at the edge, minus 5 steps
        assert_eq!(w.light_at(Sky, 0, y + 1, 0), Some(10));
        assert_eq!(w.light_at(Sky, 0, y + 3, 0), Some(15));
        for x in -4..=4 {
            for z in -4..=4 {
                w.set_block(x, y + 2, z, b::AIR);
            }
        }
        assert_eq!(w.light_at(Sky, 0, y, 0), Some(15));
        assert_eq!(w.light_at(Sky, 0, y + 2, 0), Some(15));
    }

    #[test]
    fn new_chunk_lighting_matches_incremental_edits() {
        let y = -60;
        let edits = |set: &mut dyn FnMut(i32, i32, i32, u16)| {
            set(3, y, 3, b::TORCH);
            set(15, y, 8, b::GLOWSTONE);
            for x in 0..=12 {
                for z in 0..=12 {
                    set(x, y + 3, z, b::STONE);
                }
            }
            set(6, y + 3, 6, b::WATER);
            set(6, y + 4, 6, b::WATER);
        };
        // Reference: the incremental engine, block by block.
        let mut reference = world();
        edits(&mut |x, y, z, s| {
            reference.set_block(x, y, z, s);
        });
        // Raw edits to a fresh chunk (sky light derived from its tops), then one pass.
        let mut w = world();
        let pos = crate::ChunkPos::new(0, 0);
        let mut sections = Vec::new();
        {
            let c = w.chunk_mut(pos).unwrap();
            edits(&mut |x, y, z, s| {
                c.set(x as usize, y, z as usize, s);
            });
            sections.extend(c.sections.iter().cloned());
        }
        let fresh = crate::chunk::Chunk::new(sections, -64);
        *w.chunk_mut(pos).unwrap() = fresh;
        super::light_new_chunk(&mut w, pos);
        for layer in [Block, Sky] {
            for x in -8..24 {
                for z in -8..24 {
                    for yy in y - 2..y + 8 {
                        assert_eq!(w.light_at(layer, x, yy, z), reference.light_at(layer, x, yy, z), "{layer:?} at {x},{yy},{z}");
                    }
                }
            }
        }
    }

    #[test]
    fn edits_in_void_terminate_at_the_bottom_of_stored_light() {
        let mut w = World::with_source(OVERWORLD, Box::new(NoChunks), crate::Terrain::Void, 0, 67);
        w.set_block(0, 100, 0, b::STONE);
        assert_eq!(w.light_at(Sky, 0, 99, 0), Some(14));
        assert_eq!(w.light_at(Sky, 0, -80, 0), Some(14));
        w.set_block(0, 100, 0, b::AIR);
        assert_eq!(w.light_at(Sky, 0, -80, 0), Some(15));
    }

    struct NoChunks;
    impl crate::ChunkSource for NoChunks {
        fn load(&mut self, _: crate::ChunkPos, _: crate::Dimension) -> Option<crate::chunk::Chunk> {
            None
        }
    }

    #[test]
    fn glass_lets_full_sky_light_down() {
        let mut w = world();
        let y = -60;
        for x in -4..=4 {
            for z in -4..=4 {
                w.set_block(x, y + 2, z, b::GLASS);
            }
        }
        assert_eq!(w.light_at(Sky, 0, y, 0), Some(15));
    }
}
