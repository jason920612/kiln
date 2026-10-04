//! `TrunkPlacer`s: the trunk's logs and where foliage attaches.

use super::foliage::Attachment;
use super::tree::{Ctx, Part, is_air_or_leaves, valid_tree_pos};
use super::{field, int_or};
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::with_prop;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{IntProvider, float, int};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::vtags;
use kiln_javamath::math::{floor, floor_f32};
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

#[derive(Debug)]
pub struct TrunkPlacer {
    base_height: i32,
    rand_a: i32,
    rand_b: i32,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Straight,
    Forking,
    Giant,
    MegaJungle,
    DarkOak,
    Fancy,
    Bending { min_height_for_leaves: i32, bend_length: IntProvider },
    UpwardsBranching { extra_branch_steps: IntProvider, place_branch_per_log_probability: f32, extra_branch_length: IntProvider, can_grow_through: Arc<BlockSet> },
    Cherry { branch_count: IntProvider, branch_horizontal_length: IntProvider, start: (i32, i32), second_start: (i32, i32), branch_end_offset_from_top: IntProvider },
    Poplar { trunk_height_above_branches: IntProvider, branch_amount: IntProvider },
}

/// `UniformInt.CODEC`: `{min_inclusive, max_inclusive}` (the type field is optional).
fn uniform(json: &Json) -> Result<(i32, i32), Error> {
    Ok((int(json, "min_inclusive")?, int(json, "max_inclusive")?))
}

/// `Direction.Plane.HORIZONTAL.getRandomDirection`.
pub fn random_horizontal(random: &mut WorldgenRandom) -> Dir {
    Dir::HORIZONTAL[random.next_int_bounded(4) as usize]
}

/// `RotatedPillarBlock.AXIS` set along a direction's axis (`trySetValue`).
pub fn along(s: u16, d: Dir) -> u16 {
    with_prop(s, "axis", axis_name(d))
}

fn axis_name(d: Dir) -> &'static str {
    match d {
        Dir::Down | Dir::Up => "y",
        Dir::North | Dir::South => "z",
        Dir::West | Dir::East => "x",
    }
}

impl TrunkPlacer {
    /// `TrunkPlacer.getBaseHeight`.
    pub fn base_height(&self) -> i32 {
        self.base_height
    }

    pub fn parse(json: &Json, l: &Loader) -> Result<TrunkPlacer, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        let ip = |k: &str| IntProvider::parse(field(json, k)?);
        let kind = match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "straight_trunk_placer" => Kind::Straight,
            "forking_trunk_placer" => Kind::Forking,
            "giant_trunk_placer" => Kind::Giant,
            "mega_jungle_trunk_placer" => Kind::MegaJungle,
            "dark_oak_trunk_placer" => Kind::DarkOak,
            "fancy_trunk_placer" => Kind::Fancy,
            "bending_trunk_placer" => {
                Kind::Bending { min_height_for_leaves: int_or(json, "min_height_for_leaves", 1), bend_length: ip("bend_length")? }
            }
            "upwards_branching_trunk_placer" => Kind::UpwardsBranching {
                extra_branch_steps: ip("extra_branch_steps")?,
                place_branch_per_log_probability: float(json, "place_branch_per_log_probability")?,
                extra_branch_length: ip("extra_branch_length")?,
                can_grow_through: l.blocks(field(json, "can_grow_through")?)?,
            },
            "cherry_trunk_placer" => {
                let start = uniform(field(json, "branch_start_offset_from_top")?)?;
                Kind::Cherry {
                    branch_count: ip("branch_count")?,
                    branch_horizontal_length: ip("branch_horizontal_length")?,
                    start,
                    second_start: (start.0, start.1 - 1),
                    branch_end_offset_from_top: ip("branch_end_offset_from_top")?,
                }
            }
            "poplar_trunk_placer" => {
                Kind::Poplar { trunk_height_above_branches: ip("trunk_height_above_branches")?, branch_amount: ip("branch_amount")? }
            }
            t => return Err(Error::Invalid(format!("unknown trunk placer {t}"))),
        };
        Ok(TrunkPlacer {
            base_height: int(json, "base_height")?,
            rand_a: int(json, "height_rand_a")?,
            rand_b: int(json, "height_rand_b")?,
            kind,
        })
    }

    /// `TrunkPlacer.getTreeHeight`.
    pub fn tree_height(&self, random: &mut WorldgenRandom) -> i32 {
        self.base_height + random.next_int_bounded(self.rand_a + 1) + random.next_int_bounded(self.rand_b + 1)
    }

    /// `TrunkPlacer.validTreePos` (overridden by the upwards branching placer).
    fn valid_tree_pos(&self, r: &mut Region, p: BlockPos) -> bool {
        if valid_tree_pos(r, p) {
            return true;
        }
        match &self.kind {
            Kind::UpwardsBranching { can_grow_through, .. } => can_grow_through.contains(r.get(p)),
            _ => false,
        }
    }

    /// `TrunkPlacer.isFree`.
    pub fn is_free(&self, r: &mut Region, p: BlockPos) -> bool {
        self.valid_tree_pos(r, p) || vtags::is(r.get(p), "logs")
    }

    /// `TrunkPlacer.placeLog` with a state modifier.
    fn place_log_with(&self, cx: &mut Ctx, p: BlockPos, modify: impl Fn(u16) -> u16) -> bool {
        if !self.valid_tree_pos(cx.r, p) {
            return false;
        }
        let s = cx.tree.trunk_provider.state(cx.r, cx.random, p);
        cx.put(Part::Trunk, p, modify(s));
        true
    }

    /// `TrunkPlacer.placeLog`.
    fn place_log(&self, cx: &mut Ctx, p: BlockPos) -> bool {
        self.place_log_with(cx, p, |s| s)
    }

    /// `TrunkPlacer.placeLogIfFree`.
    fn place_log_if_free(&self, cx: &mut Ctx, p: BlockPos) {
        if self.is_free(cx.r, p) {
            self.place_log(cx, p);
        }
    }

    /// `TrunkPlacer.placeBelowTrunkBlock`.
    fn place_below_trunk(cx: &mut Ctx, p: BlockPos) {
        if let Some(s) = cx.tree.below_trunk_provider.optional_state(cx.r, cx.random, p) {
            cx.put(Part::Trunk, p, s);
        }
    }

    /// `TrunkPlacer.placeTrunk`.
    pub fn place_trunk(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        match &self.kind {
            Kind::Straight => {
                Self::place_below_trunk(cx, origin.below());
                for i in 0..height {
                    self.place_log(cx, origin.above_n(i));
                }
                vec![Attachment::new(origin.above_n(height), 0, false)]
            }
            Kind::Forking => self.forking(cx, height, origin),
            Kind::Giant => self.giant(cx, height, origin),
            Kind::MegaJungle => self.mega_jungle(cx, height, origin),
            Kind::DarkOak => self.dark_oak(cx, height, origin),
            Kind::Fancy => self.fancy(cx, height, origin),
            Kind::Bending { min_height_for_leaves, bend_length } => self.bending(cx, height, origin, *min_height_for_leaves, bend_length),
            Kind::UpwardsBranching { extra_branch_steps, place_branch_per_log_probability, extra_branch_length, .. } => {
                self.upwards_branching(cx, height, origin, extra_branch_steps, *place_branch_per_log_probability, extra_branch_length)
            }
            Kind::Cherry { .. } => self.cherry(cx, height, origin),
            Kind::Poplar { trunk_height_above_branches, branch_amount } => {
                self.poplar(cx, height, origin, trunk_height_above_branches, branch_amount)
            }
        }
    }

    /// `ForkingTrunkPlacer.placeTrunk`.
    fn forking(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        Self::place_below_trunk(cx, origin.below());
        let mut out = Vec::new();
        let lean = random_horizontal(cx.random);
        let lean_height = height - cx.random.next_int_bounded(4) - 1;
        let mut lean_steps = 3 - cx.random.next_int_bounded(3);
        let (mut x, mut z) = (origin.x, origin.z);
        let mut top: Option<i32> = None;
        for i in 0..height {
            let y = origin.y + i;
            if i >= lean_height && lean_steps > 0 {
                let (dx, _, dz) = lean.offset();
                x += dx;
                z += dz;
                lean_steps -= 1;
            }
            if self.place_log(cx, BlockPos::new(x, y, z)) {
                top = Some(y + 1);
            }
        }
        if let Some(t) = top {
            out.push(Attachment::new(BlockPos::new(x, t, z), 1, false));
        }
        x = origin.x;
        z = origin.z;
        let branch = random_horizontal(cx.random);
        if branch != lean {
            let start = lean_height - cx.random.next_int_bounded(2) - 1;
            let mut steps = 1 + cx.random.next_int_bounded(3);
            top = None;
            let mut i = start;
            while i < height && steps > 0 {
                if i >= 1 {
                    let y = origin.y + i;
                    let (dx, _, dz) = branch.offset();
                    x += dx;
                    z += dz;
                    if self.place_log(cx, BlockPos::new(x, y, z)) {
                        top = Some(y + 1);
                    }
                }
                i += 1;
                steps -= 1;
            }
            if let Some(t) = top {
                out.push(Attachment::new(BlockPos::new(x, t, z), 0, false));
            }
        }
        out
    }

    /// `GiantTrunkPlacer.placeTrunk`.
    fn giant(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let below = origin.below();
        Self::place_below_trunk(cx, below);
        Self::place_below_trunk(cx, below.offset(1, 0, 0));
        Self::place_below_trunk(cx, below.offset(0, 0, 1));
        Self::place_below_trunk(cx, below.offset(1, 0, 1));
        for i in 0..height {
            self.place_log_if_free(cx, origin.offset(0, i, 0));
            if i < height - 1 {
                self.place_log_if_free(cx, origin.offset(1, i, 0));
                self.place_log_if_free(cx, origin.offset(1, i, 1));
                self.place_log_if_free(cx, origin.offset(0, i, 1));
            }
        }
        vec![Attachment::new(origin.above_n(height), 0, true)]
    }

    /// `MegaJungleTrunkPlacer.placeTrunk`.
    fn mega_jungle(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let mut out = self.giant(cx, height, origin);
        let mut branch_height = height - 2 - cx.random.next_int_bounded(4);
        while branch_height > height / 2 {
            let angle = cx.random.next_float() * 6.2831855;
            let (mut x, mut z) = (0, 0);
            for i in 0..5 {
                x = (1.5 + crate::carver::cos(angle as f64) * i as f32) as i32;
                z = (1.5 + crate::carver::sin(angle as f64) * i as f32) as i32;
                self.place_log(cx, origin.offset(x, branch_height - 3 + i / 2, z));
            }
            out.push(Attachment::new(origin.offset(x, branch_height, z), -2, false));
            branch_height -= 2 + cx.random.next_int_bounded(4);
        }
        out
    }

    /// `DarkOakTrunkPlacer.placeTrunk`.
    fn dark_oak(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let mut out = Vec::new();
        let below = origin.below();
        Self::place_below_trunk(cx, below);
        Self::place_below_trunk(cx, below.offset(1, 0, 0));
        Self::place_below_trunk(cx, below.offset(0, 0, 1));
        Self::place_below_trunk(cx, below.offset(1, 0, 1));
        let lean = random_horizontal(cx.random);
        let lean_height = height - cx.random.next_int_bounded(4);
        let mut lean_steps = 2 - cx.random.next_int_bounded(3);
        let (ox, oy, oz) = (origin.x, origin.y, origin.z);
        let (mut x, mut z) = (ox, oz);
        let top = oy + height - 1;
        for i in 0..height {
            if i >= lean_height && lean_steps > 0 {
                let (dx, _, dz) = lean.offset();
                x += dx;
                z += dz;
                lean_steps -= 1;
            }
            let p = BlockPos::new(x, oy + i, z);
            if is_air_or_leaves(cx.r, p) {
                self.place_log(cx, p);
                self.place_log(cx, p.offset(1, 0, 0));
                self.place_log(cx, p.offset(0, 0, 1));
                self.place_log(cx, p.offset(1, 0, 1));
            }
        }
        out.push(Attachment::new(BlockPos::new(x, top, z), 0, true));
        for dx in -1..=2 {
            for dz in -1..=2 {
                if (0..=1).contains(&dx) && (0..=1).contains(&dz) {
                    continue;
                }
                if cx.random.next_int_bounded(3) > 0 {
                    continue;
                }
                let n = cx.random.next_int_bounded(3) + 2;
                for j in 0..n {
                    self.place_log(cx, BlockPos::new(ox + dx, top - j - 1, oz + dz));
                }
                out.push(Attachment::new(BlockPos::new(ox + dx, top, oz + dz), 0, false));
            }
        }
        out
    }

    /// `FancyTrunkPlacer.placeTrunk`.
    fn fancy(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let h = height + 2;
        let trunk = floor(h as f64 * 0.618);
        Self::place_below_trunk(cx, origin.below());
        let clusters = 1.min(floor(1.382 + (1.0 * h as f64 / 13.0).powf(2.0)));
        let trunk_top = origin.y + trunk;
        let mut y = h - 5;
        let mut coords: Vec<(Attachment, i32)> = vec![(Attachment::new(origin.above_n(y), 0, false), trunk_top)];
        while y >= 0 {
            let shape = tree_shape(h, y);
            if shape >= 0.0 {
                for _ in 0..clusters {
                    let len = 1.0 * shape as f64 * (cx.random.next_float() as f64 + 0.328);
                    let angle = (cx.random.next_float() * 2.0) as f64 * std::f64::consts::PI;
                    let bx = len * angle.sin() + 0.5;
                    let bz = len * angle.cos() + 0.5;
                    let end = origin.offset(floor(bx), y - 1, floor(bz));
                    let up = end.above_n(5);
                    if self.make_limb(cx, end, up, false) {
                        let dx = origin.x - end.x;
                        let dz = origin.z - end.z;
                        let base_y = end.y as f64 - ((dx * dx + dz * dz) as f64).sqrt() * 0.381;
                        let base = if base_y > trunk_top as f64 { trunk_top } else { base_y as i32 };
                        let branch_base = BlockPos::new(origin.x, base, origin.z);
                        if self.make_limb(cx, branch_base, end, false) {
                            coords.push((Attachment::new(end, 0, false), base));
                        }
                    }
                }
            }
            y -= 1;
        }
        self.make_limb(cx, origin, origin.above_n(trunk), true);
        for (a, base) in &coords {
            let branch_base = BlockPos::new(origin.x, *base, origin.z);
            if branch_base != a.pos && trim_branches(h, base - origin.y) {
                self.make_limb(cx, branch_base, a.pos, true);
            }
        }
        coords.into_iter().filter(|(_, base)| trim_branches(h, base - origin.y)).map(|(a, _)| a).collect()
    }

    /// `FancyTrunkPlacer.makeLimb`: places logs along the segment, or (`place` false) checks
    /// that it is free.
    fn make_limb(&self, cx: &mut Ctx, from: BlockPos, to: BlockPos, place: bool) -> bool {
        if !place && from == to {
            return true;
        }
        let d = to.offset(-from.x, -from.y, -from.z);
        let steps = d.x.abs().max(d.y.abs()).max(d.z.abs());
        let fx = d.x as f32 / steps as f32;
        let fy = d.y as f32 / steps as f32;
        let fz = d.z as f32 / steps as f32;
        for i in 0..=steps {
            let p = from.offset(
                floor_f32(0.5 + i as f32 * fx),
                floor_f32(0.5 + i as f32 * fy),
                floor_f32(0.5 + i as f32 * fz),
            );
            if place {
                let axis = log_axis(from, p);
                self.place_log_with(cx, p, |s| with_prop(s, "axis", axis));
            } else if !self.is_free(cx.r, p) {
                return false;
            }
        }
        true
    }

    /// `BendingTrunkPlacer.placeTrunk`.
    fn bending(&self, cx: &mut Ctx, height: i32, origin: BlockPos, min_height_for_leaves: i32, bend_length: &IntProvider) -> Vec<Attachment> {
        let dir = random_horizontal(cx.random);
        let top = height - 1;
        let mut p = origin;
        Self::place_below_trunk(cx, p.below());
        let mut out = Vec::new();
        for i in 0..=top {
            if i + 1 >= top + cx.random.next_int_bounded(2) {
                p = p.relative(dir);
            }
            if valid_tree_pos(cx.r, p) {
                self.place_log(cx, p);
            }
            if i >= min_height_for_leaves {
                out.push(Attachment::new(p, 0, false));
            }
            p = p.above();
        }
        let bend = bend_length.sample(cx.random);
        for _ in 0..=bend {
            if valid_tree_pos(cx.r, p) {
                self.place_log(cx, p);
            }
            out.push(Attachment::new(p, 0, false));
            p = p.relative(dir);
        }
        out
    }

    /// `UpwardsBranchingTrunkPlacer.placeTrunk`.
    fn upwards_branching(
        &self,
        cx: &mut Ctx,
        height: i32,
        origin: BlockPos,
        extra_branch_steps: &IntProvider,
        probability: f32,
        extra_branch_length: &IntProvider,
    ) -> Vec<Attachment> {
        let mut out = Vec::new();
        for i in 0..height {
            let y = origin.y + i;
            let p = BlockPos::new(origin.x, y, origin.z);
            if self.place_log(cx, p) && i < height - 1 && cx.random.next_float() < probability {
                let dir = random_horizontal(cx.random);
                let len = extra_branch_length.sample(cx.random);
                let start = 0.max(len - extra_branch_length.sample(cx.random) - 1);
                let steps = extra_branch_steps.sample(cx.random);
                self.upwards_branch(cx, height, &mut out, p, y, dir, start, steps);
            }
            if i == height - 1 {
                out.push(Attachment::new(BlockPos::new(origin.x, y + 1, origin.z), 0, false));
            }
        }
        out
    }

    /// `UpwardsBranchingTrunkPlacer.placeBranch`.
    #[allow(clippy::too_many_arguments)]
    fn upwards_branch(&self, cx: &mut Ctx, height: i32, out: &mut Vec<Attachment>, p: BlockPos, y: i32, dir: Dir, start: i32, steps: i32) {
        let mut top = y + start;
        let (mut x, mut z) = (p.x, p.z);
        let mut steps = steps;
        let mut i = start;
        while i < height && steps > 0 {
            if i >= 1 {
                let by = y + i;
                let (dx, _, dz) = dir.offset();
                x += dx;
                z += dz;
                top = by;
                let q = BlockPos::new(x, by, z);
                if self.place_log(cx, q) {
                    top += 1;
                }
                out.push(Attachment::new(q, 0, false));
            }
            i += 1;
            steps -= 1;
        }
        if top - y > 1 {
            let q = BlockPos::new(x, top, z);
            out.push(Attachment::new(q, 0, false));
            out.push(Attachment::new(q.below_n(2), 0, false));
        }
    }

    /// `CherryTrunkPlacer.placeTrunk`.
    fn cherry(&self, cx: &mut Ctx, height: i32, origin: BlockPos) -> Vec<Attachment> {
        let Kind::Cherry { branch_count, start, second_start, .. } = &self.kind else { unreachable!() };
        Self::place_below_trunk(cx, origin.below());
        let first = 0.max(height - 1 + crate::providers::between_inclusive(cx.random, start.0, start.1));
        let mut second = 0.max(height - 1 + crate::providers::between_inclusive(cx.random, second_start.0, second_start.1));
        if second >= first {
            second += 1;
        }
        let count = branch_count.sample(cx.random);
        let three = count == 3;
        let two_or_more = count >= 2;
        let trunk = if three {
            height
        } else if two_or_more {
            first.max(second) + 1
        } else {
            first + 1
        };
        for i in 0..trunk {
            self.place_log(cx, origin.above_n(i));
        }
        let mut out = Vec::new();
        if three {
            out.push(Attachment::new(origin.above_n(trunk), 0, false));
        }
        let dir = random_horizontal(cx.random);
        out.push(self.cherry_branch(cx, height, origin, dir, first, first < trunk - 1));
        if two_or_more {
            out.push(self.cherry_branch(cx, height, origin, dir.opposite(), second, second < trunk - 1));
        }
        out
    }

    /// `CherryTrunkPlacer.generateBranch`.
    fn cherry_branch(&self, cx: &mut Ctx, height: i32, origin: BlockPos, dir: Dir, start: i32, short: bool) -> Attachment {
        let Kind::Cherry { branch_horizontal_length, branch_end_offset_from_top, .. } = &self.kind else { unreachable!() };
        let mut p = origin.above_n(start);
        let end_y = height - 1 + branch_end_offset_from_top.sample(cx.random);
        let extend = short || end_y < start;
        let len = branch_horizontal_length.sample(cx.random) + i32::from(extend);
        let end = origin.relative_n(dir, len).above_n(end_y);
        let sideways = |s: u16| along(s, dir);
        for _ in 0..if extend { 2 } else { 1 } {
            p = p.relative(dir);
            self.place_log_with(cx, p, sideways);
        }
        let vertical = if end.y > p.y { Dir::Up } else { Dir::Down };
        loop {
            let dist = p.dist_manhattan(end);
            if dist == 0 {
                break;
            }
            let chance = (end.y - p.y).abs() as f32 / dist as f32;
            let up = cx.random.next_float() < chance;
            p = p.relative(if up { vertical } else { dir });
            if up {
                self.place_log(cx, p);
            } else {
                self.place_log_with(cx, p, sideways);
            }
        }
        Attachment::new(end.above(), 0, false)
    }

    /// `PoplarTrunkPlacer.placeTrunk`.
    fn poplar(&self, cx: &mut Ctx, height: i32, origin: BlockPos, above: &IntProvider, amount: &IntProvider) -> Vec<Attachment> {
        Self::place_below_trunk(cx, origin.below());
        let branch_y = height - above.sample(cx.random);
        for i in 0..height {
            self.place_log(cx, origin.above_n(i));
            let dirs = shuffled_horizontal(cx.random);
            if branch_y - 1 == i {
                let n = amount.sample(cx.random);
                for d in dirs.iter().take(n.max(0) as usize) {
                    self.place_log_with(cx, origin.above_n(i).relative(*d), |s| along(s, *d));
                }
            }
        }
        vec![Attachment::new(origin.above_n(branch_y), 0, false)]
    }
}

/// `Direction.allShuffled` filtered to the horizontal directions.
fn shuffled_horizontal(random: &mut WorldgenRandom) -> Vec<Dir> {
    let mut all = Dir::ALL.to_vec();
    shuffle(&mut all, random);
    all.retain(|d| d.is_horizontal());
    all
}

/// `Util.shuffle`.
pub fn shuffle<T>(list: &mut [T], random: &mut WorldgenRandom) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
}

/// `FancyTrunkPlacer.treeShape`.
fn tree_shape(height: i32, y: i32) -> f32 {
    if (y as f32) < height as f32 * 0.3 {
        return -1.0;
    }
    let radius = height as f32 / 2.0;
    let adjacent = radius - y as f32;
    let mut distance = ((radius * radius - adjacent * adjacent) as f64).sqrt() as f32;
    if adjacent == 0.0 {
        distance = radius;
    } else if adjacent.abs() >= radius {
        return 0.0;
    }
    distance * 0.5
}

/// `FancyTrunkPlacer.trimBranches`.
fn trim_branches(height: i32, local_y: i32) -> bool {
    local_y as f64 >= height as f64 * 0.2
}

/// `FancyTrunkPlacer.getLogAxis`.
fn log_axis(from: BlockPos, to: BlockPos) -> &'static str {
    let dx = (to.x - from.x).abs();
    let dz = (to.z - from.z).abs();
    let m = dx.max(dz);
    if m > 0 {
        if dx == m { "x" } else { "z" }
    } else {
        "y"
    }
}
