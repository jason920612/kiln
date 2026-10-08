//! The beardifier (`Beardifier`): the `minecraft:beardifier` density term that adapts terrain
//! around structure pieces with a terrain adaptation, and around jigsaw junctions.

use super::bbox::BoundingBox;
use super::jigsaw::piece::Junction;
use super::jigsaw::pool::Projection;
use super::{ChunkStarts, Structures, TerrainAdjustment};
use crate::volume::Volume;
use std::sync::{Arc, OnceLock};

/// `Beardifier.Rigid`: a piece's box, its structure's adaptation and ground level delta.
#[derive(Clone, Copy, Debug)]
struct Rigid {
    bbox: BoundingBox,
    adjustment: TerrainAdjustment,
    ground_level_delta: i32,
}

/// `Beardifier` for one chunk.
#[derive(Debug)]
pub struct Beardifier {
    pieces: Vec<Rigid>,
    junctions: Vec<Junction>,
    affected: BoundingBox,
}

/// `BEARD_KERNEL`: `exp(-(x² + (y + 0.5)² + z²) / 16)` over a 24³ cube, index
/// `z * 576 + x * 24 + y`.
fn kernel() -> &'static [f32] {
    static K: OnceLock<Vec<f32>> = OnceLock::new();
    K.get_or_init(|| {
        let mut k = vec![0f32; 24 * 24 * 24];
        for z in 0..24 {
            for x in 0..24 {
                for y in 0..24 {
                    let (dx, dy, dz) = ((x - 12) as f64, (y - 12) as f64 + 0.5, (z - 12) as f64);
                    let d = dx * dx + dy * dy + dz * dz;
                    k[(z * 576 + x * 24 + y) as usize] = kiln_javamath::pow::pow(std::f64::consts::E, -d / 16.0) as f32;
                }
            }
        }
        k
    })
}

/// `Mth.fastInvSqrt(double)`.
fn fast_inv_sqrt(x: f64) -> f64 {
    let half = 0.5 * x;
    let i = 6_910_469_410_427_058_090i64.wrapping_sub(x.to_bits() as i64 >> 1);
    let y = f64::from_bits(i as u64);
    y * (1.5 - half * y * y)
}

/// `getBuryContribution`.
fn bury(x: f32, y: f32, z: f32) -> f32 {
    let d = x * x + y * y + z * z;
    if d >= 36.0 { 0.0 } else { 1.0 - d.sqrt() / 6.0 }
}

/// `getBeardContribution(dx, dy, dz, yForDensity)`.
fn beard(dx: i32, dy: i32, dz: i32, y_density: i32) -> f32 {
    let (kx, ky, kz) = (dx + 12, dy + 12, dz + 12);
    if !(0..24).contains(&kx) || !(0..24).contains(&ky) || !(0..24).contains(&kz) {
        return 0.0;
    }
    let y = y_density as f32 + 0.5;
    let d = dx as f32 * dx as f32 + y * y + dz as f32 * dz as f32;
    let v = -y * fast_inv_sqrt((d / 2.0) as f64) as f32 / 2.0;
    v * kernel()[(kz * 576 + kx * 24 + ky) as usize]
}

impl Beardifier {
    /// `Beardifier.forStructuresInChunk`: the pieces of adapted structures within 12 blocks
    /// of the chunk, and the junctions of their jigsaw pieces near it. `None` is `EMPTY`.
    pub fn for_chunk(structures: &Structures, starts: &ChunkStarts, cx: i32, cz: i32) -> Option<Arc<Beardifier>> {
        let (x0, z0) = (cx << 4, cz << 4);
        let mut pieces = Vec::new();
        let mut junctions = Vec::new();
        let mut affected: Option<BoundingBox> = None;
        let mut include = |b: &BoundingBox| match &mut affected {
            Some(a) => a.encapsulate(b),
            None => affected = Some(*b),
        };
        for start in starts.all() {
            let adjustment = structures.structures[start.structure].terrain_adaptation;
            if adjustment == TerrainAdjustment::None {
                continue;
            }
            for p in &start.pieces {
                let bbox = p.base().bbox;
                if !bbox.intersects_xz(x0 - 12, z0 - 12, x0 + 15 + 12, z0 + 15 + 12) {
                    continue;
                }
                match p.as_pool_element() {
                    Some(pe) => {
                        if pe.element.projection == Projection::Rigid {
                            pieces.push(Rigid { bbox, adjustment, ground_level_delta: pe.ground_level_delta });
                            include(&bbox);
                        }
                        for j in &pe.junctions {
                            let (x, z) = (j.source_x, j.source_z);
                            if x > x0 - 12 && z > z0 - 12 && x < x0 + 15 + 12 && z < z0 + 15 + 12 {
                                junctions.push(*j);
                                include(&BoundingBox::new(x, j.source_ground_y, z, x, j.source_ground_y, z));
                            }
                        }
                    }
                    None => {
                        pieces.push(Rigid { bbox, adjustment, ground_level_delta: 0 });
                        include(&bbox);
                    }
                }
            }
        }
        let affected = affected?.inflated(24, 24, 24);
        Some(Arc::new(Beardifier { pieces, junctions, affected }))
    }

    /// `sampleValue`.
    pub fn sample(&self, x: i32, y: i32, z: i32) -> f32 {
        if !self.affected.is_inside(crate::pos::BlockPos::new(x, y, z)) {
            return 0.0;
        }
        self.sample_unchecked(x, y, z)
    }

    /// `sampleVolume`.
    pub fn fill(&self, vol: &Volume, out: &mut [f32]) {
        out.fill(0.0);
        let a = &self.affected;
        let max = [vol.max_block(0), vol.max_block(1), vol.max_block(2)];
        let hits = a.max_x >= vol.min[0]
            && a.min_x <= max[0]
            && a.max_y >= vol.min[1]
            && a.min_y <= max[1]
            && a.max_z >= vol.min[2]
            && a.min_z <= max[2];
        if !hits {
            return;
        }
        let lo = |axis: usize, m: i32| (m - vol.min[axis]).max(0).div_euclid(vol.step[axis]);
        let hi = |axis: usize, m: i32| (vol.size[axis] - 1).min((m - vol.min[axis]).div_euclid(vol.step[axis]));
        let (x0, y0, z0) = (lo(0, a.min_x), lo(1, a.min_y), lo(2, a.min_z));
        let (x1, y1, z1) = (hi(0, a.max_x), hi(1, a.max_y), hi(2, a.max_z));
        for zi in z0..=z1 {
            let z = vol.block_z(zi);
            for xi in x0..=x1 {
                let x = vol.block_x(xi);
                for yi in y0..=y1 {
                    out[vol.index(xi, yi, zi)] = self.sample_unchecked(x, vol.block_y(yi), z);
                }
            }
        }
    }

    /// `sampleValueUnchecked`.
    fn sample_unchecked(&self, x: i32, y: i32, z: i32) -> f32 {
        let mut density = 0f32;
        for r in &self.pieces {
            let b = &r.bbox;
            let dx = 0.max((b.min_x - x).max(x - b.max_x));
            let dz = 0.max((b.min_z - z).max(z - b.max_z));
            let ground = b.min_y + r.ground_level_delta;
            let dy_ground = y - ground;
            let dy = match r.adjustment {
                TerrainAdjustment::None => 0,
                TerrainAdjustment::Bury | TerrainAdjustment::BeardThin => dy_ground,
                TerrainAdjustment::BeardBox => 0.max((ground - y).max(y - b.max_y)),
                TerrainAdjustment::Encapsulate => 0.max((b.min_y - y).max(y - b.max_y)),
            };
            density += match r.adjustment {
                TerrainAdjustment::None => 0.0,
                TerrainAdjustment::Bury => bury(dx as f32, dy as f32 / 2.0, dz as f32),
                TerrainAdjustment::BeardThin | TerrainAdjustment::BeardBox => beard(dx, dy, dz, dy_ground) * 0.8,
                TerrainAdjustment::Encapsulate => bury(dx as f32 / 2.0, dy as f32 / 2.0, dz as f32 / 2.0) * 0.8,
            };
        }
        for j in &self.junctions {
            let (dx, dy, dz) = (x - j.source_x, y - j.source_ground_y, z - j.source_z);
            density += beard(dx, dy, dz, dy) * 0.4;
        }
        density
    }
}
