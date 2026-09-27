//! The initial world spawn (`MinecraftServer.setInitialSpawn`): a climate search for the
//! origin chunk (`NoiseSpawnFinder`), then a spiral over the 11×11 chunks around it for the
//! first standable column (`PlayerSpawnFinder.getSpawnPosInChunk`).

use crate::biome::quantize;
use crate::block_facts::{collision_top_full, fluid};
use crate::generator::{GenScratch, Generator};
use crate::pos::BlockPos;
use crate::proto::{Heightmap, ProtoChunk};

/// `ChunkGenerator.getSpawnHeight`: the fallback spawn y when no chunk has a standable column
/// (`NoiseBasedChunkGenerator` does not override it).
pub const FALLBACK_SPAWN_Y: i32 = 64;

impl Generator {
    /// `NoiseBasedChunkGenerator.getOrigin`: the chunk containing the best `spawn_target`
    /// fit (`NoiseSpawnFinder.findSpawnPosition`), or chunk (0, 0) without a spawn target.
    pub fn spawn_origin(&self, gs: &mut GenScratch) -> (i32, i32) {
        if self.spawn_target.is_empty() {
            return (0, 0);
        }
        let mut best = self.spawn_fitness(gs, 0, 0);
        // `radialSearch(2048, 512)` then `radialSearch(512, 32)`, each around the best
        // position when it starts.
        for (max, step) in [(2048.0f32, 512.0f32), (512.0, 32.0)] {
            let center = best.0;
            let mut angle = 0.0f32;
            let mut r = step;
            while r <= max {
                let x = center.0 + ((angle as f64).sin() * r as f64) as i32;
                let z = center.1 + ((angle as f64).cos() * r as f64) as i32;
                let candidate = self.spawn_fitness(gs, x, z);
                if candidate.1 < best.1 {
                    best = candidate;
                }
                angle += step / r;
                if angle as f64 > std::f64::consts::TAU {
                    angle = 0.0;
                    r += step;
                }
            }
        }
        (best.0.0 >> 4, best.0.1 >> 4)
    }

    /// `NoiseSpawnFinder.getSpawnPositionAndFitness`: the climate distance to the nearest
    /// target point at the quart column (y = 0), weighted over the squared distance from
    /// the origin.
    fn spawn_fitness(&self, gs: &mut GenScratch, x: i32, z: i32) -> ((i32, i32), i64) {
        let (qx, qz) = ((x >> 2) << 2, (z >> 2) << 2);
        let s = gs.point_context();
        let mut fit = i64::MAX;
        for point in &self.spawn_target {
            let sum = point
                .iter()
                .map(|(f, p)| {
                    let d = p.distance(quantize(f.point(s, qx, 0, qz)));
                    d.wrapping_mul(d)
                })
                .fold(0i64, i64::wrapping_add);
            fit = fit.min(sum);
        }
        let origin = (x as i64).wrapping_mul(x as i64).wrapping_add((z as i64).wrapping_mul(z as i64));
        ((x, z), fit.wrapping_mul(2048 * 2048).wrapping_add(origin))
    }
}

/// `MinecraftServer.setInitialSpawn`: from the origin chunk, spiral over the 11×11 chunks
/// around it (vanilla's order) and take the first chunk with a standable column. `column`
/// answers `PlayerSpawnFinder.getSpawnPosInChunk` for a chunk, which vanilla asks of FULL
/// chunks (see [`spawn_pos_in_chunk`]). Falls back to the origin's centre at
/// [`FALLBACK_SPAWN_Y`].
pub fn initial_spawn(origin: (i32, i32), mut column: impl FnMut(i32, i32) -> Option<BlockPos>) -> BlockPos {
    let (mut x, mut z, mut dx, mut dz) = (0i32, 0i32, 0i32, -1i32);
    for _ in 0..11 * 11 {
        if (-5..=5).contains(&x)
            && (-5..=5).contains(&z)
            && let Some(p) = column(origin.0 + x, origin.1 + z)
        {
            return p;
        }
        if x == z || (x < 0 && x == -z) || (x > 0 && x == 1 - z) {
            (dx, dz) = (-dz, dx);
        }
        x += dx;
        z += dz;
    }
    BlockPos::new(origin.0 * 16 + 8, FALLBACK_SPAWN_Y, origin.1 * 16 + 8)
}

/// `PlayerSpawnFinder.getSpawnPosInChunk` over a finished chunk: the first column (x, then
/// z ascending) with a standable block (see [`respawn_pos`]).
pub fn spawn_pos_in_chunk(chunk: &ProtoChunk) -> Option<BlockPos> {
    let (bx, bz) = (chunk.x * 16, chunk.z * 16);
    for x in 0..16 {
        for z in 0..16 {
            let height = |map| chunk.height(map, x, z);
            let block = |y| chunk.get(x, y, z);
            if let Some(y) = respawn_y(chunk.min_y, height, block) {
                return Some(BlockPos::new(bx + x as i32, y, bz + z as i32));
            }
        }
    }
    None
}

/// `PlayerSpawnFinder.getLevelRespawnPos` for a dimension without ceiling: the y above the
/// highest collision-top-full block at or below the `MOTION_BLOCKING` top, unless the
/// column is under water (`WORLD_SURFACE` at most `MOTION_BLOCKING` and above
/// `OCEAN_FLOOR`) or a fluid comes first. `height(map)` is the column's heightmap value
/// (the y above its top block), `block(y)` the state at y.
pub fn respawn_y(min_y: i32, height: impl Fn(Heightmap) -> i32, block: impl Fn(i32) -> u16) -> Option<i32> {
    let top = height(Heightmap::MotionBlocking);
    if top < min_y {
        return None;
    }
    let surface = height(Heightmap::WorldSurface);
    if surface <= top && surface > height(Heightmap::OceanFloor) {
        return None;
    }
    for y in (min_y..=top + 1).rev() {
        let state = block(y);
        if !fluid(state).is_empty() {
            return None;
        }
        if collision_top_full(state) {
            return Some(y + 1);
        }
    }
    None
}
