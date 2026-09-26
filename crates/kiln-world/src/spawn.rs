//! Where players without a saved position appear: vanilla 26.3 `PlayerSpawnFinder`.

use crate::{ChunkPos, World};
use kiln_data::block_props::{Aabb, collision, motion_blocking};
use kiln_data::blocks_types::{has_fluid, is_air};

/// Candidates vanilla tries at most.
const MAX_CANDIDATES: i64 = 1024;
/// Player bounding box: 0.6 wide, 1.8 high, standing on the bottom center of the block.
const PLAYER_MIN: f32 = 0.2;
const PLAYER_MAX: f32 = 0.8;
const PLAYER_HEIGHT: f32 = 1.8;

impl World {
    /// A spawn point around `center` (the world spawn). Vanilla visits the columns of the
    /// (2r+1)² square around it (at most 1024) with a stride coprime to their count, starting
    /// at index `offset` (drawn at random by vanilla), and takes the first column with a
    /// surface to stand on where the player touches neither blocks nor liquid. Without one,
    /// the player goes to the nearest free space at `center`.
    ///
    /// `radius` is the `respawn_radius` game rule; vanilla also shrinks it to the distance to
    /// the world border (Kiln has no border yet) and uses radius 0 in adventure-mode worlds
    /// by skipping the search, which [`World::free_spawn_at`] covers.
    pub fn find_spawn(&mut self, center: [i32; 3], radius: i32, offset: u32) -> [f64; 3] {
        let r = radius.max(0) as i64;
        let side = 2 * r + 1;
        let count = MAX_CANDIDATES.min(side * side);
        let coprime = if count <= 16 { count - 1 } else { 17 };
        for i in 0..count {
            let idx = (offset as i64 + coprime * i) % count;
            let x = center[0] + (idx % side - r) as i32;
            let z = center[2] + (idx / side - r) as i32;
            if let Some(p) = self.respawn_pos(x, z).filter(|&p| self.fits_player(p)) {
                return bottom_center(p);
            }
        }
        self.free_spawn_at(center)
    }

    /// Vanilla `fixupSpawnHeight`: up from `pos` until the player fits, then down onto
    /// whatever is below.
    pub fn free_spawn_at(&mut self, pos: [i32; 3]) -> [f64; 3] {
        let (min_y, max_y) = (self.dimension.min_y, self.dimension.min_y + self.dimension.height - 1);
        let mut p = pos;
        while !self.fits_player(p) && p[1] < max_y {
            p[1] += 1;
        }
        p[1] -= 1;
        while self.fits_player(p) && p[1] > min_y {
            p[1] -= 1;
        }
        p[1] += 1;
        bottom_center(p)
    }

    /// Vanilla `getLevelRespawnPos` for a dimension without a ceiling: the block above the
    /// highest full-topped block under the MOTION_BLOCKING surface, unless the column is
    /// covered by liquid.
    fn respawn_pos(&mut self, x: i32, z: i32) -> Option<[i32; 3]> {
        let min_y = self.dimension.min_y;
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let chunk = self.chunk_mut(ChunkPos::of_block(x, z));
        let motion = chunk.column_height(lx, lz, motion_blocking);
        if motion < min_y {
            return None;
        }
        let surface = chunk.column_height(lx, lz, |s| !is_air(s));
        let floor = chunk.column_height(lx, lz, |s| motion_blocking(s) && !has_fluid(s));
        if surface <= motion && surface > floor {
            return None;
        }
        for y in (min_y..=motion + 1).rev() {
            let state = chunk.get(lx, y, lz);
            if has_fluid(state) {
                return None;
            }
            if top_face_full(collision(state)) {
                return Some([x, y + 1, z]);
            }
        }
        None
    }

    /// Vanilla `noCollisionNoLiquid`: a player standing at `pos` touches no collision box and
    /// no liquid.
    fn fits_player(&mut self, pos: [i32; 3]) -> bool {
        let [x, y, z] = pos;
        let chunk = self.chunk_mut(ChunkPos::of_block(x, z));
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        // The box spans cells y and y + 1; a shape taller than a block (fences, walls) can
        // reach up from y - 1.
        for dy in -1..=1 {
            let state = chunk.get(lx, y + dy, lz);
            if dy >= 0 && has_fluid(state) {
                return false;
            }
            let dy = dy as f32;
            let hit = collision(state).iter().any(|b| {
                b[0] < PLAYER_MAX
                    && b[3] > PLAYER_MIN
                    && b[1] + dy < PLAYER_HEIGHT
                    && b[4] + dy > 0.0
                    && b[2] < PLAYER_MAX
                    && b[5] > PLAYER_MIN
            });
            if hit {
                return false;
            }
        }
        true
    }
}

fn bottom_center(p: [i32; 3]) -> [f64; 3] {
    [p[0] as f64 + 0.5, p[1] as f64, p[2] as f64 + 0.5]
}

/// Whether the boxes cover the whole top face of the block (vanilla `Block.isFaceFull` of the
/// collision shape towards UP).
fn top_face_full(boxes: &[Aabb]) -> bool {
    const EPS: f32 = 1e-7;
    let top: Vec<&Aabb> = boxes.iter().filter(|b| b[1] < 1.0 && b[4] >= 1.0 - EPS).collect();
    if top.is_empty() {
        return false;
    }
    // Union of rectangles: every cell of the grid spanned by their edges must be covered.
    let mut xs: Vec<f32> = top.iter().flat_map(|b| [b[0], b[3]]).chain([0.0, 1.0]).collect();
    let mut zs: Vec<f32> = top.iter().flat_map(|b| [b[2], b[5]]).chain([0.0, 1.0]).collect();
    for v in [&mut xs, &mut zs] {
        v.retain(|c| (0.0..=1.0).contains(c));
        v.sort_by(f32::total_cmp);
        v.dedup();
    }
    xs.windows(2).all(|wx| {
        let cx = (wx[0] + wx[1]) / 2.0;
        zs.windows(2).all(|wz| {
            let cz = (wz[0] + wz[1]) / 2.0;
            top.iter().any(|b| b[0] <= cx && cx <= b[3] && b[2] <= cz && cz <= b[5])
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OVERWORLD;
    use kiln_data::blocks::default_state as block;

    #[test]
    fn top_faces() {
        let full: Aabb = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(top_face_full(&[full]));
        assert!(!top_face_full(&[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]));
        assert!(top_face_full(&[[0.0, 0.0, 0.0, 0.5, 1.0, 1.0], [0.5, 0.0, 0.0, 1.0, 1.0, 1.0]]));
        assert!(!top_face_full(&[[0.0, 0.0, 0.0, 1.0, 1.0, 0.5]]));
        assert!(!top_face_full(&[]));
        assert!(!top_face_full(collision(block::OAK_FENCE)));
        assert!(top_face_full(collision(block::GLASS)));
    }

    #[test]
    fn flat_world_spawns_on_the_grass() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        let surface = w.flat_surface_y();
        for offset in [0, 7, 440] {
            let p = w.find_spawn([8, 0, 8], 10, offset);
            assert_eq!(p[1], surface, "offset {offset}");
            assert!((p[0] - 8.0).abs() <= 10.5 && (p[2] - 8.0).abs() <= 10.5);
        }
        // Radius 0: exactly the spawn column.
        assert_eq!(w.find_spawn([3, 0, -4], 0, 123), [3.5, surface, -3.5]);
    }

    #[test]
    fn skips_water_and_blocked_columns() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        let top = OVERWORLD.min_y + 3; // the grass layer
        w.set_block(0, top, 0, block::WATER);
        assert_eq!(w.respawn_pos(0, 0), None);
        w.set_block(1, top + 1, 0, block::OAK_LEAVES);
        assert_eq!(w.respawn_pos(1, 0), Some([1, top + 2, 0]));
        // Radius 1: 9 columns, stride 8; offset 4 is the center (water), then (-1, 0).
        assert_eq!(w.find_spawn([0, 0, 0], 1, 4), [-0.5, (top + 1) as f64, 0.5]);
        // A covered column: the player would stand in stone.
        w.set_block(5, top + 2, 5, block::STONE);
        assert!(!w.fits_player([5, top + 1, 5]));
        assert!(w.fits_player([6, top + 1, 5]));
        // Tall collision below reaches into the player's box.
        w.set_block(7, top + 1, 7, block::OAK_FENCE);
        assert!(!w.fits_player([7, top + 2, 7]));
    }

    #[test]
    fn free_spawn_moves_up_then_down() {
        let mut w = World::flat(OVERWORLD, 0, 67);
        let surface = w.flat_surface_y();
        assert_eq!(w.free_spawn_at([2, 100, 2])[1], surface);
        assert_eq!(w.free_spawn_at([2, -64, 2])[1], surface);
    }
}
