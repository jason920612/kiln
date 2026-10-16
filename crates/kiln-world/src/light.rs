//! Incremental block and sky light propagation over loaded chunks.
//!
//! Increases spread breadth-first, each step losing max(1, dampening) of the entered block;
//! sky light at full strength goes straight down without loss through blocks that let
//! skylight through. Decreases clear every level that could have come from the removed
//! light, then re-spread from the brighter levels found at the edge of the cleared area.

use crate::chunk::LightLayer;
use crate::chunk::Chunk;
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

/// Light level at a position, if its chunk is loaded and light is stored there.
pub fn light_at<S: CellStore + ?Sized>(w: &S, layer: LightLayer, x: i32, y: i32, z: i32) -> Option<u8> {
    let c = w.chunk(ChunkPos::of_block(x, z))?;
    c.in_light_range(y).then(|| c.light(layer, (x & 15) as usize, y, (z & 15) as usize))
}

/// What light propagation reads and writes: blocks and light of loaded chunks.
trait Access {
    fn chunk_at(&self, pos: ChunkPos) -> Option<&Chunk>;
    fn chunk_at_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk>;

    /// Block state if its chunk is loaded and `y` is within the chunk's stored light. Positions
    /// outside take no part in propagation (full sky light would otherwise keep travelling down
    /// through void air forever).
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let c = self.chunk_at(ChunkPos::of_block(x, z))?;
        c.in_light_range(y).then(|| c.get((x & 15) as usize, y, (z & 15) as usize))
    }

    /// [`light_at`].
    fn light(&self, layer: LightLayer, x: i32, y: i32, z: i32) -> Option<u8> {
        let c = self.chunk_at(ChunkPos::of_block(x, z))?;
        c.in_light_range(y).then(|| c.light(layer, (x & 15) as usize, y, (z & 15) as usize))
    }

    /// Block state and light together (one chunk lookup).
    fn state_and_light(&self, layer: LightLayer, x: i32, y: i32, z: i32) -> Option<(u16, u8)> {
        let c = self.chunk_at(ChunkPos::of_block(x, z))?;
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        c.in_light_range(y).then(|| (c.get(lx, y, lz), c.light(layer, lx, y, lz)))
    }

    fn set(&mut self, layer: LightLayer, x: i32, y: i32, z: i32, v: u8) {
        if let Some(c) = self.chunk_at_mut(ChunkPos::of_block(x, z)) {
            c.set_light(layer, (x & 15) as usize, y, (z & 15) as usize, v);
        }
    }
}

/// The chunks around a light change (3×3 around a new chunk, 5×5 around a changed block),
/// taken out of their cells for the time it takes, so each step reads its chunk without a cell
/// lookup. Light never leaves them: a level fades by at least one a block sideways, so from a
/// new chunk, or from the border of a neighbour shining into it, it travels at most 15 blocks;
/// a block change clears at most 15 blocks around it and light spreads back from at most one
/// block beyond, fading within 15 more.
struct Window {
    /// Chunk coordinates of the north-west corner chunk.
    x0: i32,
    z0: i32,
    /// Chunks on a side (3 or 5).
    side: i32,
    /// By `dz * side + dx`.
    chunks: [Option<Box<Chunk>>; 25],
}

impl Window {
    fn take<S: CellStore + ?Sized>(w: &mut S, center: ChunkPos, side: i32) -> Window {
        let (x0, z0) = (center.x - side / 2, center.z - side / 2);
        let chunks = std::array::from_fn(|i| {
            let i = i as i32;
            if i >= side * side {
                return None;
            }
            let p = ChunkPos::new(x0 + i % side, z0 + i / side);
            w.cell_mut(p.cell())?.remove(p)
        });
        Window { x0, z0, side, chunks }
    }

    /// Puts the chunks back where they were.
    fn put_back<S: CellStore + ?Sized>(self, w: &mut S) {
        for (i, c) in self.chunks.into_iter().enumerate() {
            if let Some(c) = c {
                let p = ChunkPos::new(self.x0 + i as i32 % self.side, self.z0 + i as i32 / self.side);
                w.cell_mut(p.cell()).expect("the cell a chunk was taken from").put_back(p, c);
            }
        }
    }

    fn index(&self, pos: ChunkPos) -> Option<usize> {
        let (dx, dz) = (pos.x - self.x0, pos.z - self.z0);
        let inside = (0..self.side).contains(&dx) && (0..self.side).contains(&dz);
        debug_assert!(inside, "light left the window around {:?} at {pos:?}", (self.x0 + self.side / 2, self.z0 + self.side / 2));
        inside.then(|| (dz * self.side + dx) as usize)
    }
}

impl Access for Window {
    fn chunk_at(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks[self.index(pos)?].as_deref()
    }
    fn chunk_at_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        let i = self.index(pos)?;
        self.chunks[i].as_deref_mut()
    }
}

/// [`Window`] over any rectangle of chunks: the changed blocks' chunks and two around them.
struct Area {
    x0: i32,
    z0: i32,
    width: i32,
    depth: i32,
    /// By `dz * width + dx`.
    chunks: Vec<Option<Box<Chunk>>>,
}

impl Area {
    fn take<S: CellStore + ?Sized>(w: &mut S, min: ChunkPos, max: ChunkPos) -> Area {
        let (width, depth) = (max.x - min.x + 1, max.z - min.z + 1);
        let chunks = (0..width * depth)
            .map(|i| {
                let p = ChunkPos::new(min.x + i % width, min.z + i / width);
                w.cell_mut(p.cell())?.remove(p)
            })
            .collect();
        Area { x0: min.x, z0: min.z, width, depth, chunks }
    }

    fn put_back<S: CellStore + ?Sized>(self, w: &mut S) {
        for (i, c) in self.chunks.into_iter().enumerate() {
            if let Some(c) = c {
                let p = ChunkPos::new(self.x0 + i as i32 % self.width, self.z0 + i as i32 / self.width);
                w.cell_mut(p.cell()).expect("the cell a chunk was taken from").put_back(p, c);
            }
        }
    }

    fn index(&self, pos: ChunkPos) -> Option<usize> {
        let (dx, dz) = (pos.x - self.x0, pos.z - self.z0);
        let inside = (0..self.width).contains(&dx) && (0..self.depth).contains(&dz);
        debug_assert!(inside, "light left the area from {:?} at {pos:?}", (self.x0, self.z0));
        inside.then(|| (dz * self.width + dx) as usize)
    }
}

impl Access for Area {
    fn chunk_at(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks[self.index(pos)?].as_deref()
    }
    fn chunk_at_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        let i = self.index(pos)?;
        self.chunks[i].as_deref_mut()
    }
}

/// Chunks on a side of the groups [`update_light_many`] relights at once (two more on each side
/// are taken with them).
const GROUP_CHUNKS: i32 = 8;

/// [`update_light`] for blocks that changed since their light was last right (each changed while
/// its light waited; vanilla's light engine likewise takes the checks a tick queued together):
/// the light every block would have after updating them one by one (each level is the brightest
/// some source reaches, whatever order they spread in), but cleared and spread once.
pub fn update_light_many<S: CellStore + ?Sized>(w: &mut S, changed: &[[i32; 3]]) {
    let mut changed = changed.to_vec();
    // By group of chunks, so far-apart changes do not take the chunks between them.
    let group = |p: &[i32; 3]| ((p[0] >> 4).div_euclid(GROUP_CHUNKS), (p[2] >> 4).div_euclid(GROUP_CHUNKS));
    changed.sort_unstable_by_key(|p| (group(p), *p));
    changed.dedup();
    for positions in changed.chunk_by(|a, b| group(a) == group(b)) {
        let (mut min, mut max) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
        for p in positions {
            let (cx, cz) = (p[0] >> 4, p[2] >> 4);
            min = (min.0.min(cx), min.1.min(cz));
            max = (max.0.max(cx), max.1.max(cz));
        }
        // Light reaches at most two chunks beyond a change (see [`Window`]).
        let mut area = Area::take(w, ChunkPos::new(min.0 - 2, min.1 - 2), ChunkPos::new(max.0 + 2, max.1 + 2));
        update_many_in(&mut area, positions);
        area.put_back(w);
    }
}

fn update_many_in<A: Access + ?Sized>(w: &mut A, changed: &[[i32; 3]]) {
    for layer in [LightLayer::Block, LightLayer::Sky] {
        let mut removal = Queue::new();
        let mut relight = Queue::new();
        for &[x, y, z] in changed {
            let current = w.light(layer, x, y, z).unwrap_or(0);
            if current > 0 {
                w.set(layer, x, y, z, 0);
                removal.push_back((x, y, z, current));
            }
        }
        decrease(w, layer, &mut removal, &mut relight);
        for &[x, y, z] in changed {
            let Some(new) = w.state(x, y, z) else { continue };
            if layer == LightLayer::Block && light_emission(new) > 0 {
                let e = light_emission(new);
                if e > w.light(layer, x, y, z).unwrap_or(0) {
                    w.set(layer, x, y, z, e);
                }
                relight.push_back((x, y, z, e));
            }
            seed_around(w, layer, x, y, z, &mut relight);
        }
        increase(w, layer, &mut relight);
    }
}

/// The neighbours that may now shine into (or through) a changed block.
fn seed_around<A: Access + ?Sized>(w: &A, layer: LightLayer, x: i32, y: i32, z: i32, relight: &mut Queue) {
    for (dx, dy, dz) in DIRS {
        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
        if let Some(l) = w.light(layer, nx, ny, nz).filter(|&l| l > 0) {
            relight.push_back((nx, ny, nz, l));
        }
    }
    if layer == LightLayer::Sky && w.light(layer, x, y + 1, z).is_none_or(|l| l == 15) {
        relight.push_back((x, y + 1, z, 15));
    }
}

/// Re-lights around a block that changed from `old` to `new`.
pub fn update_light<S: CellStore + ?Sized>(w: &mut S, x: i32, y: i32, z: i32, old: u16, new: u16) {
    if !affects_light(old, new) {
        return;
    }
    let mut window = Window::take(w, ChunkPos::of_block(x, z), 5);
    update_light_in(&mut window, x, y, z, new);
    window.put_back(w);
}

fn update_light_in<A: Access + ?Sized>(w: &mut A, x: i32, y: i32, z: i32, new: u16) {
    for layer in [LightLayer::Block, LightLayer::Sky] {
        let mut relight = Queue::new();
        let current = w.light(layer, x, y, z).unwrap_or(0);
        if current > 0 {
            w.set(layer, x, y, z, 0);
            let mut removal = Queue::from([(x, y, z, current)]);
            decrease(w, layer, &mut removal, &mut relight);
        }
        if layer == LightLayer::Block && light_emission(new) > 0 {
            let e = light_emission(new);
            w.set(layer, x, y, z, e);
            relight.push_back((x, y, z, e));
        }
        // Neighbors may now shine into (or through) the changed block.
        for (dx, dy, dz) in DIRS {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if let Some(l) = w.light(layer, nx, ny, nz).filter(|&l| l > 0) {
                relight.push_back((nx, ny, nz, l));
            }
        }
        if layer == LightLayer::Sky && w.light(layer, x, y + 1, z).is_none_or(|l| l == 15) {
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
    if w.chunk(pos).is_none() {
        return;
    }
    let mut window = Window::take(w, pos, 3);
    let center = window.chunks[4].as_deref_mut().expect("the chunk being lit");
    if center.lit_alone() {
        center.set_lit_alone(false);
        light_border_in(&mut window, pos);
    } else {
        light_new_in(&mut window, pos);
    }
    window.put_back(w);
}

/// The part of [`light_new_chunk`] a generated chunk can have before it has neighbours, done
/// where it is made (a generation thread, off the tick): its light as if none of its neighbours
/// were loaded. [`light_new_chunk`] then adds only the light that crosses its borders (from
/// taller columns and lit blocks next door, and its own going out), and the result is the same
/// as lighting it all at once: light only ever increases here, and every level is the
/// brightest that some source reaches, whatever order the sources spread in.
pub fn light_alone(chunk: Chunk, pos: ChunkPos) -> Chunk {
    let mut window = Window { x0: pos.x - 1, z0: pos.z - 1, side: 3, chunks: Default::default() };
    window.chunks[4] = Some(Box::new(chunk));
    light_new_in(&mut window, pos);
    let mut chunk = *window.chunks[4].take().expect("the chunk being lit");
    chunk.set_lit_alone(true);
    chunk
}

fn light_new_in(w: &mut Window, pos: ChunkPos) {
    let Some(c) = w.chunk_at(pos) else { return };
    let (min_y, max_y) = (c.min_y(), c.min_y() + c.height() - 1);
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let mut block = Queue::new();
    let mut sky = Queue::new();
    // Emitters, bottom up (sections whose palette has none are skipped whole).
    for (si, section) in c.sections.iter().enumerate() {
        if !section.blocks.maybe_has(|s| light_emission(s) > 0) {
            continue;
        }
        let y0 = min_y + 16 * si as i32;
        for ly in 0..16 {
            if y0 + ly > max_y {
                break;
            }
            for z in 0..16 {
                for x in 0..16 {
                    let e = light_emission(section.get(x, ly as usize, z));
                    if e > 0 {
                        block.push_back((bx + x as i32, y0 + ly, bz + z as i32, e));
                    }
                }
            }
        }
    }
    let tops = ring_tops(w, pos);
    for &(x, y, z, e) in &block {
        if e > w.light(LightLayer::Block, x, y, z).unwrap_or(15) {
            w.set(LightLayer::Block, x, y, z, e);
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
                let Some((to, current)) = w.state_and_light(LightLayer::Sky, x, y, z) else { break };
                if occludes(above, to, DOWN) {
                    break;
                }
                level = transmitted(LightLayer::Sky, level, to, DOWN);
                if level == 0 {
                    break;
                }
                if level > current {
                    w.set(LightLayer::Sky, x, y, z, level);
                    sky.push_back((x, y, z, level));
                }
                above = to;
            }
        }
    }
    neighbour_seeds(w, pos, chunk_top, &mut block, &mut sky);
    increase(w, LightLayer::Block, &mut block);
    increase(w, LightLayer::Sky, &mut sky);
}

/// Column tops (y above the highest non-air block) of the chunk and the ring around it, by
/// x then z from the ring's north-west corner; `None` where no chunk is loaded.
fn ring_tops(w: &Window, pos: ChunkPos) -> [[Option<i32>; 18]; 18] {
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let mut tops = [[None; 18]; 18];
    // The 3×3 chunks the ring touches, each looked up once.
    let around: [[Option<&Chunk>; 3]; 3] =
        std::array::from_fn(|dx| std::array::from_fn(|dz| w.chunk_at(ChunkPos::new(pos.x + dx as i32 - 1, pos.z + dz as i32 - 1))));
    for (i, row) in tops.iter_mut().enumerate() {
        for (j, t) in row.iter_mut().enumerate() {
            let (x, z) = (bx + i as i32 - 1, bz + j as i32 - 1);
            let chunk = around[((x >> 4) - pos.x + 1) as usize][((z >> 4) - pos.z + 1) as usize];
            *t = chunk.map(|c| c.surface_y((x & 15) as usize, (z & 15) as usize));
        }
    }
    tops
}

/// The neighbours' border columns shine in: their lit cells next to the chunk (sky light only
/// below `chunk_top`: above every column of the chunk, its sky light is full already). Sections
/// with no light above 1 are skipped whole.
fn neighbour_seeds(w: &Window, pos: ChunkPos, chunk_top: i32, block: &mut Queue, sky: &mut Queue) {
    let Some(c) = w.chunk_at(pos) else { return };
    let (min_y, max_y) = (c.min_y(), c.min_y() + c.height() - 1);
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let sides = [ChunkPos::new(pos.x - 1, pos.z), ChunkPos::new(pos.x + 1, pos.z), ChunkPos::new(pos.x, pos.z - 1), ChunkPos::new(pos.x, pos.z + 1)];
    let neighbours = sides.map(|p| w.chunk_at(p));
    for k in 0..16 {
        for (side, (x, z)) in [(bx - 1, bz + k), (bx + 16, bz + k), (bx + k, bz - 1), (bx + k, bz + 16)].into_iter().enumerate() {
            let Some(n) = neighbours[side] else { continue };
            let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
            for (layer, queue, top) in [(LightLayer::Block, &mut *block, max_y + 1), (LightLayer::Sky, &mut *sky, chunk_top.min(max_y + 1))] {
                let mut y = min_y;
                while y < top {
                    if !n.in_light_range(y) {
                        y += 1;
                        continue;
                    }
                    if n.uniform_light(layer, y) == Some(0) {
                        y = (y - min_y + 16) / 16 * 16 + min_y;
                        continue;
                    }
                    let l = n.light(layer, lx, y, lz);
                    if l > 1 {
                        queue.push_back((x, y, z, l));
                    }
                    y += 1;
                }
            }
        }
    }
}

/// [`light_new_in`] for a chunk lit on its own already ([`light_alone`]): the sky light that
/// comes sideways from taller columns across its borders, the neighbours' light shining in,
/// and its own light going out where it is brighter than what is next door.
fn light_border_in(w: &mut Window, pos: ChunkPos) {
    let Some(c) = w.chunk_at(pos) else { return };
    let (min_y, max_y) = (c.min_y(), c.min_y() + c.height() - 1);
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let mut block = Queue::new();
    let mut sky = Queue::new();
    let tops = ring_tops(w, pos);
    let chunk_top = tops[1..17].iter().flat_map(|row| row[1..17].iter().flatten()).copied().max().unwrap_or(min_y);
    // Full sky light next to columns across the border that are taller than every column
    // next to it inside (those were done alone).
    let inside = |a: usize, b: usize| (1..17).contains(&a) && (1..17).contains(&b);
    for i in 1..17 {
        for j in 1..17 {
            if !(i == 1 || i == 16 || j == 1 || j == 16) {
                continue;
            }
            let Some(own) = tops[i][j] else { continue };
            let adjacent = [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)];
            let highest = adjacent.iter().filter_map(|&(a, b)| tops[a][b]).max().unwrap_or(own);
            let alone = adjacent.iter().filter(|&&(a, b)| inside(a, b)).filter_map(|&(a, b)| tops[a][b]).max().unwrap_or(own);
            let (x, z) = (bx + i as i32 - 1, bz + j as i32 - 1);
            for y in own.max(alone)..highest.min(max_y + 1) {
                sky.push_back((x, y, z, 15));
            }
        }
    }
    neighbour_seeds(w, pos, chunk_top, &mut block, &mut sky);
    // The chunk's border cells whose light would brighten the cell across the border.
    let sides = [(ChunkPos::new(pos.x - 1, pos.z), 4usize), (ChunkPos::new(pos.x + 1, pos.z), 5), (ChunkPos::new(pos.x, pos.z - 1), 2), (ChunkPos::new(pos.x, pos.z + 1), 3)];
    let (lo, hi) = (min_y - 16, max_y + 16);
    for (np, dir) in sides {
        let Some(n) = w.chunk_at(np) else { continue };
        let (dx, _, dz) = DIRS[dir];
        for k in 0..16 {
            let (x, z) = match dir {
                4 => (bx, bz + k),
                5 => (bx + 15, bz + k),
                2 => (bx + k, bz),
                _ => (bx + k, bz + 15),
            };
            let (ox, oz) = (x + dx, z + dz);
            let (lx, lz, nx, nz) = ((x & 15) as usize, (z & 15) as usize, (ox & 15) as usize, (oz & 15) as usize);
            for (layer, queue) in [(LightLayer::Block, &mut block), (LightLayer::Sky, &mut sky)] {
                let mut y = lo;
                while y <= hi {
                    if !c.in_light_range(y) || !n.in_light_range(y) {
                        y += 1;
                        continue;
                    }
                    // Nothing to give (all 0 here) or nothing to take (all 15 there): the
                    // section's rows go at once.
                    let mine = c.uniform_light(layer, y);
                    if mine == Some(0) || n.uniform_light(layer, y) == Some(15) {
                        y = (y - min_y + 16).div_euclid(16) * 16 + min_y;
                        continue;
                    }
                    // Sideways a level loses at least one: only a level two above the cell across can
                    // brighten it (the blocks are read only then).
                    let l = c.light(layer, lx, y, lz);
                    let there = n.light(layer, nx, y, nz);
                    if l > there + 1 {
                        let (from, to) = (c.get(lx, y, lz), n.get(nx, y, nz));
                        if !occludes(from, to, dir) && transmitted(layer, l, to, dir) > there {
                            queue.push_back((x, y, z, l));
                        }
                    }
                    y += 1;
                }
            }
        }
    }
    increase(w, LightLayer::Block, &mut block);
    increase(w, LightLayer::Sky, &mut sky);
}

fn increase<A: Access + ?Sized>(w: &mut A, layer: LightLayer, queue: &mut Queue) {
    while let Some((x, y, z, level)) = queue.pop_front() {
        if level <= 1 {
            continue;
        }
        let from = match w.state_and_light(layer, x, y, z) {
            // A level cleared again after it was queued (another removal found it came from
            // there) is no longer there to spread: `LightEngine.propagateIncreases` skips it too.
            Some((_, stored)) if stored < level => continue,
            Some((s, _)) => s,
            None => kiln_data::blocks::default_state::AIR,
        };
        for (dir, (dx, dy, dz)) in DIRS.iter().enumerate() {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let Some((to, current)) = w.state_and_light(layer, nx, ny, nz) else { continue };
            if occludes(from, to, dir) {
                continue;
            }
            let v = transmitted(layer, level, to, dir);
            if v > current {
                w.set(layer, nx, ny, nz, v);
                queue.push_back((nx, ny, nz, v));
            }
        }
    }
}

fn decrease<A: Access + ?Sized>(w: &mut A, layer: LightLayer, removal: &mut Queue, relight: &mut Queue) {
    while let Some((x, y, z, level)) = removal.pop_front() {
        for (dir, (dx, dy, dz)) in DIRS.iter().enumerate() {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let Some((to, cur)) = w.state_and_light(layer, nx, ny, nz) else { continue };
            if cur == 0 {
                continue;
            }
            // Full sky light directly below full sky light came from above.
            let dependent_sky = layer == LightLayer::Sky && dir == DOWN && level == 15 && cur == 15;
            if cur < level || dependent_sky {
                w.set(layer, nx, ny, nz, 0);
                removal.push_back((nx, ny, nz, cur));
                if layer == LightLayer::Block && light_emission(to) > 0 {
                    let e = light_emission(to);
                    w.set(layer, nx, ny, nz, e);
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

    /// Random terrain-like chunks: stone up to a column height with caves, water, leaves
    /// above, glass, and light sources.
    fn random_chunk(seed: &mut u64, sections: usize, min_y: i32) -> crate::chunk::Chunk {
        let mut next = |n: u64| {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (*seed >> 33) % n
        };
        let height = sections as i32 * 16;
        let mut c = crate::chunk::Chunk::new((0..sections).map(|_| crate::section::Section::filled(b::AIR, 0)).collect(), min_y);
        let base = 8 + next(20) as i32;
        for x in 0..16 {
            for z in 0..16 {
                let h = (base + next(12) as i32 - 6).clamp(1, height - 12);
                for y in 0..h {
                    let s = match next(40) {
                        0..=3 => b::AIR,
                        4 => b::WATER,
                        5 => b::GLASS,
                        6 if next(4) == 0 => b::GLOWSTONE,
                        _ => b::STONE,
                    };
                    c.set(x, min_y + y, z, s);
                }
                match next(10) {
                    0..=2 => {
                        for y in h + 3..h + 3 + next(4) as i32 {
                            c.set(x, min_y + y, z, b::OAK_LEAVES);
                        }
                    }
                    3 => {
                        c.set(x, min_y + h, z, b::TORCH);
                    }
                    4 => {
                        c.set(x, min_y + h, z, b::WATER);
                    }
                    _ => {}
                }
            }
        }
        // Rebuilt so its sky light comes from its column tops, as a generated chunk's does.
        crate::chunk::Chunk::new(c.sections.clone(), min_y)
    }

    #[test]
    fn a_chunk_lit_alone_then_at_its_borders_gets_the_same_light() {
        use crate::ChunkPos;
        let (sections, min_y) = (4, 0);
        for round in 0..6u64 {
            let mut seed = 0x9e37 + round;
            let mut order: Vec<ChunkPos> = (-2..=2).flat_map(|x| (-2..=2).map(move |z| ChunkPos::new(x, z))).collect();
            for i in (1..order.len()).rev() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                order.swap(i, (seed >> 33) as usize % (i + 1));
            }
            let mut whole = World::with_source(OVERWORLD, Box::new(NoChunks), crate::Terrain::Void, 0, 67);
            let mut split = World::with_source(OVERWORLD, Box::new(NoChunks), crate::Terrain::Void, 0, 67);
            for &pos in &order {
                let chunk = random_chunk(&mut seed, sections, min_y);
                let alone = super::light_alone(crate::chunk::Chunk::new(chunk.sections.clone(), min_y), pos);
                whole.cells.entry(pos.cell()).or_default().insert(pos, chunk);
                split.cells.entry(pos.cell()).or_default().insert(pos, alone);
                super::light_new_chunk(&mut whole, pos);
                super::light_new_chunk(&mut split, pos);
                for layer in [Block, Sky] {
                    for x in -32..48 {
                        for z in -32..48 {
                            for y in min_y - 16..min_y + 16 * sections as i32 + 16 {
                                assert_eq!(whole.light_at(layer, x, y, z), split.light_at(layer, x, y, z), "round {round}, {layer:?} at {x},{y},{z} after {pos:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Light propagation over the whole world, without a window (the reference).
    struct Whole<'a>(&'a mut World);

    impl super::Access for Whole<'_> {
        fn chunk_at(&self, pos: crate::ChunkPos) -> Option<&crate::chunk::Chunk> {
            self.0.chunk(pos)
        }
        fn chunk_at_mut(&mut self, pos: crate::ChunkPos) -> Option<&mut crate::chunk::Chunk> {
            Blocks::chunk_mut(self.0, pos)
        }
    }

    #[test]
    fn block_changes_relight_within_the_window_as_over_the_whole_world() {
        use crate::ChunkPos;
        let (sections, min_y) = (4, 0);
        let mut seed = 0x51u64;
        let mut whole = World::with_source(OVERWORLD, Box::new(NoChunks), crate::Terrain::Void, 0, 67);
        let mut windowed = World::with_source(OVERWORLD, Box::new(NoChunks), crate::Terrain::Void, 0, 67);
        for x in -3..=3 {
            for z in -3..=3 {
                let pos = ChunkPos::new(x, z);
                let chunk = random_chunk(&mut seed, sections, min_y);
                let copy = crate::chunk::Chunk::new(chunk.sections.clone(), min_y);
                whole.cells.entry(pos.cell()).or_default().insert(pos, chunk);
                windowed.cells.entry(pos.cell()).or_default().insert(pos, copy);
                super::light_new_chunk(&mut whole, pos);
                super::light_new_chunk(&mut windowed, pos);
            }
        }
        let states = [b::AIR, b::STONE, b::GLOWSTONE, b::TORCH, b::WATER, b::OAK_LEAVES, b::GLASS];
        for step in 0..120 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let (x, y, z) = (((seed >> 33) % 48) as i32 - 24, ((seed >> 20) % 50) as i32, ((seed >> 40) % 48) as i32 - 24);
            let new = states[((seed >> 8) % states.len() as u64) as usize];
            let pos = ChunkPos::of_block(x, z);
            let old = whole.chunk(pos).unwrap().get((x & 15) as usize, y, (z & 15) as usize);
            Blocks::chunk_mut(&mut whole, pos).unwrap().set((x & 15) as usize, y, (z & 15) as usize, new);
            Blocks::chunk_mut(&mut windowed, pos).unwrap().set((x & 15) as usize, y, (z & 15) as usize, new);
            if super::affects_light(old, new) {
                super::update_light_in(&mut Whole(&mut whole), x, y, z, new);
            }
            super::update_light(&mut windowed, x, y, z, old, new);
            for layer in [Block, Sky] {
                for cx in x - 34..x + 34 {
                    for cz in z - 34..z + 34 {
                        for cy in min_y - 16..min_y + 16 * sections as i32 + 16 {
                            assert_eq!(whole.light_at(layer, cx, cy, cz), windowed.light_at(layer, cx, cy, cz), "step {step}, {layer:?} at {cx},{cy},{cz}");
                        }
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
