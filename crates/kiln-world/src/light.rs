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
