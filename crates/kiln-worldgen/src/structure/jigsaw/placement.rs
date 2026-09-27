//! `JigsawPlacement`: grows a jigsaw structure from its start piece, breadth first by
//! placement priority, fitting each new piece into the free space left by the others.

use super::JigsawStructure;
use super::piece::{Junction, PoolElementPiece};
use super::pool::{Pool, PoolElement, Projection, Pools};
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::structure::bbox::BoundingBox;
use crate::structure::piece::Piece;
use crate::structure::template::TemplateManager;
use crate::structure::transform::Rotation;
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

/// `PoolAliasLookup`: resolved pool aliases of one start.
pub struct Aliases(HashMap<String, String>);

impl Aliases {
    pub fn lookup<'s>(&'s self, pool: &'s str) -> &'s str {
        self.0.get(pool).map_or(pool, String::as_str)
    }
}

/// `PoolAliasBinding`.
#[derive(Debug)]
pub enum AliasBinding {
    Direct { alias: String, target: String },
    Random { alias: String, targets: Vec<(String, i32)> },
    RandomGroup(Vec<(Vec<AliasBinding>, i32)>),
}

/// `WeightedList.getRandomOrThrow`.
fn pick<'t, T>(list: &'t [(T, i32)], random: &mut impl RandomSource) -> &'t T {
    let total: i32 = list.iter().map(|e| e.1).sum();
    let mut i = random.next_int_bounded(total);
    for (v, w) in list {
        i -= w;
        if i < 0 {
            return v;
        }
    }
    unreachable!("weights sum to the total")
}

impl AliasBinding {
    fn resolve(&self, random: &mut impl RandomSource, out: &mut HashMap<String, String>) {
        match self {
            AliasBinding::Direct { alias, target } => {
                out.insert(alias.clone(), target.clone());
            }
            AliasBinding::Random { alias, targets } => {
                out.insert(alias.clone(), pick(targets, random).clone());
            }
            AliasBinding::RandomGroup(groups) => {
                for b in pick(groups, random) {
                    b.resolve(random, out);
                }
            }
        }
    }
}

/// `PoolAliasLookup.create(bindings, pos, seed)`.
pub fn aliases(bindings: &[AliasBinding], p: BlockPos, seed: i64) -> Aliases {
    let mut out = HashMap::new();
    if !bindings.is_empty() {
        use kiln_javamath::random::LegacyRandom;
        let fork = LegacyRandom::new(seed).next_long();
        let mut random = LegacyRandom::new(kiln_javamath::math::get_seed(p.x, p.y, p.z) ^ fork);
        for b in bindings {
            b.resolve(&mut random, &mut out);
        }
    }
    Aliases(out)
}

/// The free space a piece may grow into (a `VoxelShape` built from integer boxes): a box
/// minus the boxes placed in it.
struct Free {
    base: Option<BoundingBox>,
    taken: Vec<BoundingBox>,
}

impl Free {
    fn of(b: BoundingBox) -> Free {
        Free { base: Some(b), taken: Vec::new() }
    }

    /// `!joinIsNotEmpty(free, box.deflate(0.25), ONLY_SECOND)`: every block of `b` is free.
    fn fits(&self, b: &BoundingBox) -> bool {
        let Some(base) = self.base else { return false };
        b.min_x >= base.min_x
            && b.min_y >= base.min_y
            && b.min_z >= base.min_z
            && b.max_x <= base.max_x
            && b.max_y <= base.max_y
            && b.max_z <= base.max_z
            && !self.taken.iter().any(|t| t.intersects(b))
    }
}

/// A piece waiting for its children (`JigsawPlacement.PieceState`).
struct PieceState {
    piece: usize,
    free: usize,
    depth: i32,
}

/// `SequencedPriorityIterator`: highest priority first, first in first out within one.
#[derive(Default)]
struct Queue(BTreeMap<i32, VecDeque<PieceState>>);

impl Queue {
    fn add(&mut self, s: PieceState, priority: i32) {
        self.0.entry(priority).or_default().push_back(s);
    }

    fn next(&mut self) -> Option<PieceState> {
        loop {
            let mut e = self.0.last_entry()?;
            if let Some(s) = e.get_mut().pop_front() {
                return Some(s);
            }
            e.remove();
        }
    }
}

struct Placer<'a> {
    pools: &'a Pools,
    tm: &'a TemplateManager,
    aliases: &'a Aliases,
    max_depth: i32,
    use_expansion_hack: bool,
    liquid: crate::structure::template::LiquidSettings,
    pieces: Vec<PoolElementPiece>,
    frees: Vec<Free>,
    placing: Queue,
}

impl<'a> Placer<'a> {
    fn pool(&self, name: &str) -> Option<&'a Pool> {
        self.pools.id(self.aliases.lookup(name)).map(|i| &self.pools.pools[i])
    }

    /// `Placer.tryPlacingChildren`.
    fn try_placing_children(&mut self, ctx: &mut GenCtx, index: usize, outer: usize, depth: i32) {
        let (element, position, rotation, bbox) = {
            let p = &self.pieces[index];
            (p.element.clone(), p.position, p.rotation, p.base.bbox)
        };
        let rigid = element.projection == Projection::Rigid;
        let mut interior: Option<usize> = None;
        let min_y = bbox.min_y;
        let jigsaws = element.shuffled_jigsaws(self.tm, position, rotation, &mut ctx.random);
        'jigsaw: for j in &jigsaws {
            let front = j.front();
            let jpos = j.pos;
            let target = jpos.relative(front);
            let rel_y = jpos.y - min_y;
            let mut surface = i32::MIN;
            let Some(pool) = self.pool(&j.pool) else { continue };
            if pool.templates.is_empty() && pool.name != "minecraft:empty" {
                continue;
            }
            let fallback = &self.pools.pools[pool.fallback];
            if fallback.templates.is_empty() && fallback.name != "minecraft:empty" {
                continue;
            }
            let free = if bbox.is_inside(target) {
                *interior.get_or_insert_with(|| {
                    self.frees.push(Free::of(bbox));
                    self.frees.len() - 1
                })
            } else {
                outer
            };
            let mut candidates: Vec<Arc<PoolElement>> = Vec::new();
            if depth != self.max_depth {
                candidates.extend(pool.shuffled(&mut ctx.random));
            }
            candidates.extend(fallback.shuffled(&mut ctx.random));
            let priority = j.placement_priority;
            for next in candidates {
                if next.is_empty() {
                    continue 'jigsaw;
                }
                let mut rotations = Rotation::ALL.to_vec();
                super::pool::shuffle(&mut rotations, &mut ctx.random);
                for rot in rotations {
                    let jigsaws2 = next.shuffled_jigsaws(self.tm, BlockPos::new(0, 0, 0), rot, &mut ctx.random);
                    let box0 = next.bounding_box(self.tm, BlockPos::new(0, 0, 0), rot);
                    let expansion = if !self.use_expansion_hack || box0.y_span() > 16 {
                        0
                    } else {
                        jigsaws2
                            .iter()
                            .map(|k| {
                                if !box0.is_inside(k.pos.relative(k.front())) {
                                    return 0;
                                }
                                match self.pool(&k.pool) {
                                    None => 0,
                                    Some(p) => p.max_size(self.tm).max(self.pools.pools[p.fallback].max_size(self.tm)),
                                }
                            })
                            .max()
                            .unwrap_or(0)
                    };
                    for k in &jigsaws2 {
                        if !j.can_attach(k) {
                            continue;
                        }
                        let offset = BlockPos::new(target.x - k.pos.x, target.y - k.pos.y, target.z - k.pos.z);
                        let box43 = next.bounding_box(self.tm, offset, rot);
                        let rigid2 = next.projection == Projection::Rigid;
                        let ky = k.pos.y;
                        let delta = rel_y - ky + front.offset().1;
                        let y = if rigid && rigid2 {
                            min_y + delta
                        } else {
                            if surface == i32::MIN {
                                surface = ctx.first_free_height(jpos.x, jpos.z, Heightmap::WorldSurfaceWg);
                            }
                            surface - ky
                        };
                        let dy = y - box43.min_y;
                        let mut placed = box43.moved(0, dy, 0);
                        let at = offset.offset(0, dy, 0);
                        if expansion > 0 {
                            let h = (expansion + 1).max(placed.max_y - placed.min_y);
                            placed.encapsulate_pos(BlockPos::new(placed.min_x, placed.min_y + h, placed.min_z));
                        }
                        if !self.frees[free].fits(&placed) {
                            continue;
                        }
                        self.frees[free].taken.push(placed);
                        let gld = self.pieces[index].ground_level_delta;
                        let gld2 = if rigid2 { gld - delta } else { next.ground_level_delta() };
                        let mut child = PoolElementPiece::new(next.clone(), at, gld2, rot, placed, self.liquid);
                        let ground = if rigid {
                            min_y + rel_y
                        } else if rigid2 {
                            y + ky
                        } else {
                            if surface == i32::MIN {
                                surface = ctx.first_free_height(jpos.x, jpos.z, Heightmap::WorldSurfaceWg);
                            }
                            surface + delta / 2
                        };
                        self.pieces[index].junctions.push(Junction {
                            source_x: target.x,
                            source_ground_y: ground - rel_y + gld,
                            source_z: target.z,
                            delta_y: delta,
                            dest_projection: next.projection,
                        });
                        child.junctions.push(Junction {
                            source_x: jpos.x,
                            source_ground_y: ground - ky + gld2,
                            source_z: jpos.z,
                            delta_y: -delta,
                            dest_projection: element.projection,
                        });
                        self.pieces.push(child);
                        if depth < self.max_depth {
                            self.placing.add(PieceState { piece: self.pieces.len() - 1, free, depth: depth + 1 }, priority);
                        }
                        continue 'jigsaw;
                    }
                }
            }
        }
    }
}

/// `JigsawPlacement.addPieces(context, startPool, startJigsawName, maxDepth, pos,
/// useExpansionHack, projectStartToHeightmap, maxDistance, aliases, padding, liquid)`.
pub fn add_pieces<'k>(s: &'k JigsawStructure, ctx: &mut GenCtx, pos: BlockPos) -> Option<Stub<'k>> {
    let aliases = aliases(&s.aliases, pos, ctx.seed);
    let structures = ctx.structures;
    let tm = &structures.templates;
    let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
    let start = s.pools.id(aliases.lookup(&s.start_pool)).or_else(|| s.pools.id(&s.start_pool))?;
    let element = s.pools.pools[start].random_template(&mut ctx.random)?.clone();
    if element.is_empty() {
        return None;
    }
    let anchor = match &s.start_jigsaw_name {
        Some(name) => {
            let jigsaws = element.shuffled_jigsaws(tm, pos, rotation, &mut ctx.random);
            jigsaws.iter().find(|j| j.name.as_deref() == Some(name.as_str()))?.pos
        }
        None => pos,
    };
    let offset = BlockPos::new(anchor.x - pos.x, anchor.y - pos.y, anchor.z - pos.z);
    let at = BlockPos::new(pos.x - offset.x, pos.y - offset.y, pos.z - offset.z);
    let bbox = element.bounding_box(tm, at, rotation);
    let mut piece = PoolElementPiece::new(element.clone(), at, element.ground_level_delta(), rotation, bbox, s.liquid);
    let (cx, cz) = ((bbox.max_x + bbox.min_x) / 2, (bbox.max_z + bbox.min_z) / 2);
    let y = match s.project_start_to_heightmap {
        None => at.y,
        Some(map) => {
            let (lo, hi) = (ctx.min_y(), ctx.max_y());
            if !ctx.could_exist_in_column(cx, cz, lo, hi) {
                return None;
            }
            pos.y + ctx.first_free_height(cx, cz, map)
        }
    };
    let ground = bbox.min_y + piece.ground_level_delta;
    piece.shift_by(0, y - ground, 0);
    let b = piece.base.bbox;
    if let Some((bottom, top)) = s.padding
        && (b.min_y < ctx.min_y() + bottom || b.max_y > ctx.max_y() - top)
    {
        return None;
    }
    let start_y = y + offset.y;
    Some(Stub {
        pos: BlockPos::new(cx, start_y, cz),
        build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
            let tm = ctx.structures.templates.clone();
            let mut placer = Placer {
                pools: &s.pools,
                tm: &tm,
                aliases: &aliases,
                max_depth: s.max_depth,
                use_expansion_hack: s.use_expansion_hack,
                liquid: s.liquid,
                pieces: vec![piece],
                frees: Vec::new(),
                placing: Queue::default(),
            };
            if s.max_depth > 0 {
                let (h, v) = s.max_distance;
                let (bottom, top) = s.padding.unwrap_or((0, 0));
                let lo = (start_y - v).max(ctx.min_y() + bottom);
                let hi = (start_y + v + 1).min(ctx.max_y() + 1 - top);
                let base = (lo != hi).then(|| BoundingBox::new(cx - h, lo.min(hi), cz - h, cx + h, lo.max(hi) - 1, cz + h));
                placer.frees.push(Free { base, taken: vec![b] });
                placer.try_placing_children(ctx, 0, 0, 0);
                while let Some(state) = placer.placing.next() {
                    placer.try_placing_children(ctx, state.piece, state.free, state.depth);
                }
            }
            out.extend(placer.pieces.into_iter().map(|p| Box::new(p) as Box<dyn Piece>));
        }),
    })
}
