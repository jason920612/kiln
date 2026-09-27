//! `FoliagePlacer`s: leaves around each foliage attachment of the trunk.

use super::field;
use super::tree::{Ctx, Part, valid_tree_pos};
use super::trunk::along;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{has_prop, prop, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{IntProvider, float, int};
use crate::random::WorldgenRandom;
use kiln_javamath::math::floor_f32;
use kiln_javamath::random::RandomSource;

/// `FoliagePlacer.FoliageAttachment`.
#[derive(Clone, Copy, Debug)]
pub struct Attachment {
    pub pos: BlockPos,
    radius_offset: i32,
    height_offset: i32,
    double_trunk: bool,
}

impl Attachment {
    /// `new FoliageAttachment(pos, radiusOffset, doubleTrunk)`.
    pub fn new(pos: BlockPos, radius_offset: i32, double_trunk: bool) -> Attachment {
        Attachment { pos, radius_offset, height_offset: 0, double_trunk }
    }
}

#[derive(Debug)]
pub struct FoliagePlacer {
    radius: IntProvider,
    offset: IntProvider,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Blob { height: i32 },
    Fancy { height: i32 },
    Bush { height: i32 },
    MegaJungle { height: i32 },
    Acacia,
    DarkOak,
    RandomSpread { foliage_height: IntProvider, attempts: i32 },
    Pine { height: IntProvider },
    Spruce { trunk_height: IntProvider },
    MegaPine { crown_height: IntProvider },
    Cherry { height: IntProvider, wide_bottom_hole: f32, corner_hole: f32, hanging: f32, hanging_extension: f32 },
    Poplar { height: IntProvider, side_hole_chance: f32 },
}

impl FoliagePlacer {
    pub fn parse(json: &Json) -> Result<FoliagePlacer, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        let ip = |k: &str| IntProvider::parse(field(json, k)?);
        let kind = match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "blob_foliage_placer" => Kind::Blob { height: int(json, "height")? },
            "fancy_foliage_placer" => Kind::Fancy { height: int(json, "height")? },
            "bush_foliage_placer" => Kind::Bush { height: int(json, "height")? },
            "jungle_foliage_placer" => Kind::MegaJungle { height: int(json, "height")? },
            "acacia_foliage_placer" => Kind::Acacia,
            "dark_oak_foliage_placer" => Kind::DarkOak,
            "random_spread_foliage_placer" => {
                Kind::RandomSpread { foliage_height: ip("foliage_height")?, attempts: int(json, "leaf_placement_attempts")? }
            }
            "pine_foliage_placer" => Kind::Pine { height: ip("height")? },
            "spruce_foliage_placer" => Kind::Spruce { trunk_height: ip("trunk_height")? },
            "mega_pine_foliage_placer" => Kind::MegaPine { crown_height: ip("crown_height")? },
            "cherry_foliage_placer" => Kind::Cherry {
                height: ip("height")?,
                wide_bottom_hole: float(json, "wide_bottom_layer_hole_chance")?,
                corner_hole: float(json, "corner_hole_chance")?,
                hanging: float(json, "hanging_leaves_chance")?,
                hanging_extension: float(json, "hanging_leaves_extension_chance")?,
            },
            "poplar_foliage_placer" => Kind::Poplar { height: ip("height")?, side_hole_chance: float(json, "side_hole_chance")? },
            t => return Err(Error::Invalid(format!("unknown foliage placer {t}"))),
        };
        Ok(FoliagePlacer { radius: ip("radius")?, offset: ip("offset")?, kind })
    }

    /// `FoliagePlacer.foliageHeight`.
    pub fn foliage_height(&self, random: &mut WorldgenRandom, tree_height: i32) -> i32 {
        match &self.kind {
            Kind::Blob { height } | Kind::Fancy { height } | Kind::Bush { height } | Kind::MegaJungle { height } => *height,
            Kind::Acacia => 0,
            Kind::DarkOak => 4,
            Kind::RandomSpread { foliage_height, .. } => foliage_height.sample(random),
            Kind::Pine { height } | Kind::Cherry { height, .. } | Kind::Poplar { height, .. } => height.sample(random),
            Kind::Spruce { trunk_height } => 4.max(tree_height - trunk_height.sample(random)),
            Kind::MegaPine { crown_height } => crown_height.sample(random),
        }
    }

    /// `FoliagePlacer.foliageRadius`.
    pub fn foliage_radius(&self, random: &mut WorldgenRandom, trunk_height: i32) -> i32 {
        let r = self.radius.sample(random);
        match self.kind {
            Kind::Pine { .. } => r + random.next_int_bounded((trunk_height + 1).max(1)),
            _ => r,
        }
    }

    /// `FoliagePlacer.createFoliage` (samples the offset, then places).
    pub fn create_foliage(&self, cx: &mut Ctx, a: &Attachment, height: i32, radius: i32) {
        let offset = self.offset.sample(cx.random);
        let pos = a.pos;
        let dt = a.double_trunk;
        match &self.kind {
            Kind::Blob { .. } => {
                let h = height + a.height_offset;
                for y in (offset - h..=offset).rev() {
                    let rr = 0.max(radius + a.radius_offset - 1 - y / 2);
                    self.place_leaves_row(cx, pos, rr, y, dt);
                }
            }
            Kind::Fancy { .. } => {
                for y in (offset - height..=offset).rev() {
                    let rr = radius + if y == offset || y == offset - height { 0 } else { 1 };
                    self.place_leaves_row(cx, pos, rr, y, dt);
                }
            }
            Kind::Bush { .. } => {
                let h = height + a.height_offset;
                for y in (offset - h..=offset).rev() {
                    let rr = radius + a.radius_offset - 1 - y;
                    self.place_leaves_row(cx, pos, rr, y, dt);
                }
            }
            Kind::MegaJungle { .. } => {
                let h = if dt { height } else { 1 + cx.random.next_int_bounded(2) } + a.height_offset;
                for y in (offset - h..=offset).rev() {
                    let rr = radius + a.radius_offset + 1 - y;
                    self.place_leaves_row(cx, pos, rr, y, dt);
                }
            }
            Kind::Acacia => {
                let p = pos.above_n(offset);
                let h = height + a.height_offset;
                self.place_leaves_row(cx, p, radius + a.radius_offset, -1 - h, dt);
                self.place_leaves_row(cx, p, radius - 1, -h, dt);
                self.place_leaves_row(cx, p, radius + a.radius_offset - 1, 0, dt);
            }
            Kind::DarkOak => {
                let p = pos.above_n(offset);
                if dt {
                    self.place_leaves_row(cx, p, radius + 2, -1, dt);
                    self.place_leaves_row(cx, p, radius + 3, 0, dt);
                    self.place_leaves_row(cx, p, radius + 2, 1, dt);
                    if cx.random.next_bool() {
                        self.place_leaves_row(cx, p, radius, 2, dt);
                    }
                } else {
                    self.place_leaves_row(cx, p, radius + 2, -1, dt);
                    self.place_leaves_row(cx, p, radius + 1, 0, dt);
                }
            }
            Kind::RandomSpread { attempts, .. } => {
                for _ in 0..*attempts {
                    let dx = cx.random.next_int_bounded(radius) - cx.random.next_int_bounded(radius);
                    let dy = cx.random.next_int_bounded(height) - cx.random.next_int_bounded(height);
                    let dz = cx.random.next_int_bounded(radius) - cx.random.next_int_bounded(radius);
                    try_place_leaf(cx, pos.offset(dx, dy, dz));
                }
            }
            Kind::Pine { .. } => {
                let mut rr = 0;
                let h = height + a.height_offset;
                for y in (offset - h..=offset).rev() {
                    self.place_leaves_row(cx, pos, rr, y, dt);
                    if rr >= 1 && y == offset - h + 1 {
                        rr -= 1;
                    } else if rr < radius + a.radius_offset {
                        rr += 1;
                    }
                }
            }
            Kind::Spruce { .. } => {
                let mut rr = cx.random.next_int_bounded(2);
                let mut max = 1;
                let mut min = 0;
                let h = height + a.height_offset;
                for y in (-h..=offset).rev() {
                    self.place_leaves_row(cx, pos, rr, y, dt);
                    if rr >= max {
                        rr = min;
                        min = 1;
                        max = (max + 1).min(radius + a.radius_offset);
                    } else {
                        rr += 1;
                    }
                }
            }
            Kind::MegaPine { .. } => {
                let mut prev = 0;
                let h = height + a.height_offset;
                for y in pos.y - h + offset..=pos.y + offset {
                    let dy = pos.y - y;
                    let base = radius + a.radius_offset + floor_f32(dy as f32 / h as f32 * 3.5);
                    let rr = if dy > 0 && base == prev && y & 1 == 0 { base + 1 } else { base };
                    self.place_leaves_row(cx, BlockPos::new(pos.x, y, pos.z), rr, 0, dt);
                    prev = base;
                }
            }
            Kind::Cherry { hanging, hanging_extension, .. } => {
                let p = pos.above_n(offset);
                let rr = radius + a.radius_offset - 1;
                let h = height + a.height_offset;
                self.place_leaves_row(cx, p, rr - 2, h - 3, dt);
                self.place_leaves_row(cx, p, rr - 1, h - 4, dt);
                for y in (0..=h - 5).rev() {
                    self.place_leaves_row(cx, p, rr, y, dt);
                }
                self.place_leaves_row_hanging(cx, p, rr, -1, dt, *hanging, *hanging_extension);
                self.place_leaves_row_hanging(cx, p, rr - 1, -2, dt, *hanging, *hanging_extension);
            }
            Kind::Poplar { .. } => self.poplar(cx, a, height, radius, offset),
        }
    }

    /// `FoliagePlacer.shouldSkipLocation`.
    fn should_skip(&self, random: &mut WorldgenRandom, dx: i32, y: i32, dz: i32, radius: i32, double_trunk: bool) -> bool {
        match &self.kind {
            Kind::Blob { .. } => dx == radius && dz == radius && (random.next_int_bounded(2) == 0 || y == 0),
            Kind::Bush { .. } => dx == radius && dz == radius && random.next_int_bounded(2) == 0,
            Kind::Fancy { .. } => {
                let sq = |v: f32| v * v;
                sq(dx as f32 + 0.5) + sq(dz as f32 + 0.5) > (radius * radius) as f32
            }
            Kind::MegaJungle { .. } | Kind::MegaPine { .. } => {
                if dx + dz >= 7 {
                    return true;
                }
                dx * dx + dz * dz > radius * radius
            }
            Kind::Acacia => {
                if y == 0 {
                    (dx > 1 || dz > 1) && dx != 0 && dz != 0
                } else {
                    dx == radius && dz == radius && radius > 0
                }
            }
            Kind::DarkOak => {
                if y == -1 && !double_trunk {
                    dx == radius && dz == radius
                } else if y == 1 {
                    dx + dz > radius * 2 - 2
                } else {
                    false
                }
            }
            Kind::RandomSpread { .. } => false,
            Kind::Pine { .. } | Kind::Spruce { .. } => dx == radius && dz == radius && radius > 0,
            Kind::Cherry { wide_bottom_hole, corner_hole, .. } => {
                if y == -1 && (dx == radius || dz == radius) && random.next_float() < *wide_bottom_hole {
                    return true;
                }
                let corner = dx == radius && dz == radius;
                if radius > 2 {
                    corner || (dx + dz > radius * 2 - 2 && random.next_float() < *corner_hole)
                } else {
                    corner && random.next_float() < *corner_hole
                }
            }
            Kind::Poplar { .. } => unreachable!("poplar foliage uses its own rows"),
        }
    }

    /// `FoliagePlacer.shouldSkipLocationSigned`.
    fn should_skip_signed(&self, random: &mut WorldgenRandom, dx: i32, y: i32, dz: i32, radius: i32, double_trunk: bool) -> bool {
        if let Kind::DarkOak = self.kind
            && y == 0
            && double_trunk
            && (dx == -radius || dx >= radius)
            && (dz == -radius || dz >= radius)
        {
            return true;
        }
        let (ax, az) = if double_trunk {
            (dx.abs().min((dx - 1).abs()), dz.abs().min((dz - 1).abs()))
        } else {
            (dx.abs(), dz.abs())
        };
        self.should_skip(random, ax, y, az, radius, double_trunk)
    }

    /// `FoliagePlacer.placeLeavesRow`.
    fn place_leaves_row(&self, cx: &mut Ctx, pos: BlockPos, radius: i32, y: i32, double_trunk: bool) {
        let extra = i32::from(double_trunk);
        for dx in -radius..=radius + extra {
            for dz in -radius..=radius + extra {
                if self.should_skip_signed(cx.random, dx, y, dz, radius, double_trunk) {
                    continue;
                }
                try_place_leaf(cx, pos.offset(dx, y, dz));
            }
        }
    }

    /// `FoliagePlacer.placeLeavesRowWithHangingLeavesBelow`.
    #[allow(clippy::too_many_arguments)]
    fn place_leaves_row_hanging(&self, cx: &mut Ctx, pos: BlockPos, radius: i32, y: i32, double_trunk: bool, chance: f32, extension: f32) {
        self.place_leaves_row(cx, pos, radius, y, double_trunk);
        let extra = i32::from(double_trunk);
        let below = pos.below();
        for d in Dir::HORIZONTAL {
            let cw = d.clockwise();
            let positive = matches!(cw, Dir::South | Dir::East);
            let side = if positive { radius + extra } else { radius };
            let mut p = pos.offset(0, y - 1, 0).relative_n(cw, side).relative_n(d, -radius);
            for _ in -radius..radius + extra {
                let set = cx.leaves.contains(p.above());
                if set && try_place_extension(cx, chance, below, p) {
                    try_place_extension(cx, extension, below, p.below());
                }
                p = p.relative(d);
            }
        }
    }

    /// `PoplarFoliagePlacer.createFoliage`.
    fn poplar(&self, cx: &mut Ctx, a: &Attachment, height: i32, radius: i32, offset: i32) {
        let dt = a.double_trunk;
        let p = a.pos.above_n(offset);
        let rr = radius + a.radius_offset - 1;
        let flip = cx.random.next_bool();
        let h = height + a.height_offset;
        self.poplar_row(cx, p, rr - 2, h - 1, dt, h, flip);
        self.poplar_row(cx, p, rr - 1, h - 2, dt, h, flip);
        self.poplar_row(cx, p, rr - 1, h - 3, dt, h, flip);
        for y in (1..=h - 4).rev() {
            self.poplar_row(cx, p, rr, y, dt, h, flip);
        }
        self.poplar_logs(cx, p, rr, h - 4, dt, h, flip);
        self.poplar_row(cx, p, rr - 1, 0, dt, h, flip);
        self.poplar_row(cx, p, (rr - 2).clamp(1, 2), -1, dt, h, flip);
    }

    /// `PoplarFoliagePlacer.placeLeavesRow`.
    #[allow(clippy::too_many_arguments)]
    fn poplar_row(&self, cx: &mut Ctx, pos: BlockPos, radius: i32, y: i32, double_trunk: bool, height: i32, flip: bool) {
        let extra = i32::from(double_trunk);
        for dx in -radius..=radius + extra {
            for dz in -radius..=radius + extra {
                if self.poplar_skip(cx.random, dx, y, dz, radius, height, flip) {
                    continue;
                }
                try_place_leaf(cx, pos.offset(dx, y, dz));
            }
        }
    }

    /// `PoplarFoliagePlacer.shouldSkipLocation`.
    #[allow(clippy::too_many_arguments)]
    fn poplar_skip(&self, random: &mut WorldgenRandom, dx: i32, y: i32, dz: i32, radius: i32, height: i32, flip: bool) -> bool {
        let Kind::Poplar { side_hole_chance, .. } = self.kind else { unreachable!() };
        let partial = partial_rhombus_row(height, y);
        let corner = rhombus_corner_cut(dx, dz, radius, partial, flip);
        let (ax, az) = (dx.abs(), dz.abs());
        if partial && (ax == radius || az == radius) {
            return true;
        }
        let hole = random.next_float() <= side_hole_chance;
        !within_rhombus(radius, ax, az, corner, i32::from(hole))
    }

    /// `PoplarFoliagePlacer.replaceLeavesWithLog`.
    #[allow(clippy::too_many_arguments)]
    fn poplar_logs(&self, cx: &mut Ctx, pos: BlockPos, radius: i32, y: i32, double_trunk: bool, height: i32, flip: bool) {
        let extra = i32::from(double_trunk);
        for dx in -radius..=radius + extra {
            for dz in -radius..=radius + extra {
                let (ax, az) = (dx.abs(), dz.abs());
                let corner = rhombus_corner_cut(dx, dz, radius, partial_rhombus_row(height, y), flip);
                if !within_rhombus(radius, ax, az, corner, 2) {
                    continue;
                }
                if (az == 0 && radius - ax >= 4) || (ax == 0 && radius - az >= 4) {
                    let d = if az == 0 { Dir::East } else { Dir::South };
                    let p = pos.offset(dx, y, dz);
                    let here = cx.r.get(p);
                    let leaves = cx.tree.foliage_provider.state(cx.r, cx.random, p);
                    if here == leaves {
                        let s = cx.tree.trunk_provider.state(cx.r, cx.random, p);
                        cx.put(Part::Foliage, p, along(s, d));
                    }
                }
            }
        }
    }
}

/// `PoplarFoliagePlacer.shouldRowBePartialRhombusShape`.
fn partial_rhombus_row(height: i32, y: i32) -> bool {
    height - 1 == y || height - 2 == y
}

/// `PoplarFoliagePlacer.getCornerBlocksToCutForRhombusShape`.
fn rhombus_corner_cut(dx: i32, dz: i32, radius: i32, partial: bool, flip: bool) -> i32 {
    let corner = if flip {
        (dx > 0 && dz > 0) || (dz < 0 && dx < 0)
    } else {
        (dx > 0 && dz < 0) || (dz > 0 && dx < 0)
    };
    if corner {
        radius - 1
    } else if partial {
        radius + 1
    } else {
        radius
    }
}

/// `PoplarFoliagePlacer.isWithinRhombusShape`.
fn within_rhombus(radius: i32, ax: i32, az: i32, cut: i32, extra: i32) -> bool {
    ax + az <= radius * 2 - (cut + extra)
}

/// `FoliagePlacer.tryPlaceExtension`.
fn try_place_extension(cx: &mut Ctx, chance: f32, origin: BlockPos, p: BlockPos) -> bool {
    if p.dist_manhattan(origin) >= 7 {
        return false;
    }
    if cx.random.next_float() > chance {
        return false;
    }
    try_place_leaf(cx, p)
}

/// `FoliagePlacer.tryPlaceLeaf`.
pub fn try_place_leaf(cx: &mut Ctx, p: BlockPos) -> bool {
    let here = cx.r.get(p);
    let persistent = prop(here, "persistent") == Some("true");
    if persistent || !valid_tree_pos(cx.r, p) {
        return false;
    }
    let mut s = cx.tree.foliage_provider.state(cx.r, cx.random, p);
    if has_prop(s, "waterlogged") {
        let water = cx.r.fluid(p).is_water_source();
        s = with_prop(s, "waterlogged", if water { "true" } else { "false" });
    }
    cx.put(Part::Foliage, p, s);
    true
}
