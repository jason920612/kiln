//! Structure templates (`StructureTemplate`) and their placement (`placeInWorld` with
//! `StructurePlaceSettings`), loaded on demand by the [`TemplateManager`]
//! (`StructureTemplateManager`) from the vanilla jar or an extracted `data/` tree.

use super::bbox::BoundingBox;
use super::processor::{Processor, put};
use super::transform::{Mirror, Rotation, mirror, rotate};
use crate::block_facts::{Dir, FluidKind, block_class, fluid, is_instance};
use crate::blocks::{has_prop, prop, state, with_prop};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_data::blocks_types::block_of;
use kiln_javamath::math::get_seed;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// `StructureTemplate.StructureBlockInfo`: a block of a template (template-local position) or
/// of a placement (world position).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub pos: BlockPos,
    pub state: u16,
    pub nbt: Option<Arc<Tag>>,
}

/// `StructureTemplate.JigsawBlockInfo`.
#[derive(Clone, Debug)]
pub struct JigsawInfo {
    pub pos: BlockPos,
    pub state: u16,
    /// `JointType.ROLLABLE` (else `ALIGNED`).
    pub rollable: bool,
    /// `None` for the synthetic jigsaw of a feature element.
    pub name: Option<String>,
    pub pool: String,
    pub target: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl JigsawInfo {
    /// `JigsawBlock.getFrontFacing`.
    pub fn front(&self) -> Dir {
        orientation(self.state).0
    }

    /// `JigsawBlock.getTopFacing`.
    pub fn top(&self) -> Dir {
        orientation(self.state).1
    }

    /// `JigsawBlock.canAttach(self, other)`.
    pub fn can_attach(&self, other: &JigsawInfo) -> bool {
        self.front() == other.front().opposite()
            && (self.rollable || self.top() == other.top())
            && other.name.as_ref().is_none_or(|n| *n == self.target)
    }
}

/// The `orientation` (`FrontAndTop`) of a jigsaw state.
pub fn orientation(s: u16) -> (Dir, Dir) {
    let v = prop(s, "orientation").unwrap_or("north_up");
    let (front, top) = v.split_once('_').unwrap_or(("north", "up"));
    (Dir::by_name(front).unwrap_or(Dir::North), Dir::by_name(top).unwrap_or(Dir::Up))
}

/// `StructureTemplate.Palette`.
#[derive(Debug, Default)]
pub struct Palette {
    pub blocks: Vec<BlockInfo>,
    pub jigsaws: Vec<JigsawInfo>,
    /// Indices of structure blocks (data markers) in `blocks`.
    pub markers: Vec<usize>,
}

/// `StructureTemplate.StructureEntityInfo`: recorded, not spawned.
#[derive(Debug)]
pub struct EntityInfo {
    pub pos: [f64; 3],
    pub block_pos: BlockPos,
    pub nbt: Tag,
}

/// `StructureTemplate`.
#[derive(Debug, Default)]
pub struct Template {
    pub size: [i32; 3],
    pub palettes: Vec<Palette>,
    pub entities: Vec<EntityInfo>,
}

/// `LiquidSettings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LiquidSettings {
    IgnoreWaterlogging,
    #[default]
    ApplyWaterlogging,
}

impl LiquidSettings {
    pub fn parse(name: &str) -> Option<LiquidSettings> {
        match name {
            "ignore_waterlogging" => Some(LiquidSettings::IgnoreWaterlogging),
            "apply_waterlogging" => Some(LiquidSettings::ApplyWaterlogging),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LiquidSettings::IgnoreWaterlogging => "ignore_waterlogging",
            LiquidSettings::ApplyWaterlogging => "apply_waterlogging",
        }
    }
}

/// `StructurePlaceSettings`.
pub struct PlaceSettings<'a> {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: BlockPos,
    pub ignore_entities: bool,
    pub bbox: Option<BoundingBox>,
    pub liquid: LiquidSettings,
    /// `setRandom`: when unset, each use seeds a fresh legacy source from the position.
    pub random: Option<WorldgenRandom>,
    /// The settings' random is the one `placeInWorld` is given (`setRandom(random)` with the
    /// same source): every draw of the placement goes to `random` above.
    pub shared_random: bool,
    pub processors: Vec<&'a Processor>,
    pub known_shape: bool,
    pub finalize_entities: bool,
    scratch: WorldgenRandom,
}

impl Default for PlaceSettings<'_> {
    fn default() -> Self {
        Self {
            mirror: Mirror::None,
            rotation: Rotation::None,
            pivot: BlockPos::new(0, 0, 0),
            ignore_entities: false,
            bbox: None,
            liquid: LiquidSettings::ApplyWaterlogging,
            random: None,
            shared_random: false,
            processors: Vec::new(),
            known_shape: false,
            finalize_entities: false,
            scratch: WorldgenRandom::legacy(0),
        }
    }
}

impl<'a> PlaceSettings<'a> {
    pub fn with_rotation(rotation: Rotation) -> Self {
        Self { rotation, ..Self::default() }
    }

    /// `getRandom(pos)`.
    pub fn random_at(&mut self, p: BlockPos) -> &mut WorldgenRandom {
        match &mut self.random {
            Some(r) => r,
            None => {
                self.scratch = WorldgenRandom::legacy(get_seed(p.x, p.y, p.z));
                &mut self.scratch
            }
        }
    }

    /// `popProcessor`.
    pub fn pop_processor(&mut self, p: &Processor) {
        self.processors.retain(|q| !std::ptr::eq(*q, p));
    }

    /// `getRandomPalette(palettes, pos)`: always draws from the source.
    pub fn palette_index(&mut self, count: usize, p: BlockPos) -> usize {
        self.random_at(p).next_int_bounded(count as i32) as usize
    }

    fn apply_waterlogging(&self) -> bool {
        self.liquid == LiquidSettings::ApplyWaterlogging
    }
}

/// `StructureTemplate.transform(pos, mirror, rotation, pivot)`.
pub fn transform(p: BlockPos, m: Mirror, r: Rotation, pivot: BlockPos) -> BlockPos {
    let (mut x, y, mut z) = (p.x, p.y, p.z);
    match m {
        Mirror::LeftRight => z = -z,
        Mirror::FrontBack => x = -x,
        Mirror::None => {}
    }
    let (px, pz) = (pivot.x, pivot.z);
    match r {
        Rotation::CounterClockwise90 => BlockPos::new(px - pz + z, y, px + pz - x),
        Rotation::Clockwise90 => BlockPos::new(px + pz - z, y, pz - px + x),
        Rotation::Clockwise180 => BlockPos::new(px + px - x, y, pz + pz - z),
        Rotation::None => BlockPos::new(x, y, z),
    }
}

/// `StructureTemplate.getZeroPositionWithTransform(pos, mirror, rotation, sizeX, sizeZ)`.
pub fn zero_position_with_transform(p: BlockPos, m: Mirror, r: Rotation, sx: i32, sz: i32) -> BlockPos {
    let (sx, sz) = (sx - 1, sz - 1);
    let x = if m == Mirror::FrontBack { sx } else { 0 };
    let z = if m == Mirror::LeftRight { sz } else { 0 };
    match r {
        Rotation::CounterClockwise90 => p.offset(z, 0, sx - x),
        Rotation::Clockwise90 => p.offset(sz - z, 0, x),
        Rotation::Clockwise180 => p.offset(sx - x, 0, sz - z),
        Rotation::None => p.offset(x, 0, z),
    }
}

impl Template {
    /// `getSize(rotation)`.
    pub fn size_rotated(&self, r: Rotation) -> [i32; 3] {
        match r {
            Rotation::Clockwise90 | Rotation::CounterClockwise90 => [self.size[2], self.size[1], self.size[0]],
            _ => self.size,
        }
    }

    /// `getBoundingBox(settings, pos)`.
    pub fn bounding_box(&self, settings: &PlaceSettings, p: BlockPos) -> BoundingBox {
        bounding_box(p, settings.rotation, settings.pivot, settings.mirror, self.size)
    }

    /// `getJigsaws(pos, rotation)`.
    pub fn jigsaws(&self, p: BlockPos, r: Rotation) -> Vec<JigsawInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }
        let mut settings = PlaceSettings::with_rotation(r);
        let palette = &self.palettes[settings.palette_index(self.palettes.len(), p)];
        palette
            .jigsaws
            .iter()
            .map(|j| {
                let pos = transform(j.pos, settings.mirror, settings.rotation, settings.pivot);
                JigsawInfo { pos: pos.offset(p.x, p.y, p.z), state: rotate(j.state, r), ..j.clone() }
            })
            .collect()
    }

    /// `filterBlocks(pos, settings, block)`: the named block's infos at world positions.
    pub fn filter_blocks(&self, p: BlockPos, settings: &mut PlaceSettings, name: &str) -> Vec<BlockInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }
        let palette = &self.palettes[settings.palette_index(self.palettes.len(), p)];
        let mut out = Vec::new();
        for b in palette.blocks.iter().filter(|b| block_of(b.state).name == name) {
            let pos = transform(b.pos, settings.mirror, settings.rotation, settings.pivot).offset(p.x, p.y, p.z);
            if settings.bbox.is_some_and(|bb| !bb.is_inside(pos)) {
                continue;
            }
            out.push(BlockInfo { pos, state: rotate(b.state, settings.rotation), nbt: b.nbt.clone() });
        }
        out
    }

    /// `filterBlocks(pos, settings, STRUCTURE_BLOCK, relative)`: the data markers.
    pub fn markers(&self, p: BlockPos, settings: &mut PlaceSettings, relative: bool) -> Vec<BlockInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }
        let palette = &self.palettes[settings.palette_index(self.palettes.len(), p)];
        let mut out = Vec::new();
        for &i in &palette.markers {
            let b = &palette.blocks[i];
            let pos = if relative { transform(b.pos, settings.mirror, settings.rotation, settings.pivot).offset(p.x, p.y, p.z) } else { b.pos };
            if settings.bbox.is_some_and(|bb| !bb.is_inside(pos)) {
                continue;
            }
            out.push(BlockInfo { pos, state: rotate(b.state, settings.rotation), nbt: b.nbt.clone() });
        }
        out
    }

    /// `placeInWorld` with `settings.setRandom(random)` on the same source (features).
    pub fn place_with_shared_random(
        &self,
        r: &mut Region,
        p: BlockPos,
        pivot: BlockPos,
        settings: &mut PlaceSettings,
        random: &mut WorldgenRandom,
        flags: i32,
    ) -> bool {
        settings.random = Some(random.clone());
        settings.shared_random = true;
        let placed = self.place_in_world(r, p, pivot, settings, &mut WorldgenRandom::legacy(0), flags);
        if let Some(back) = settings.random.take() {
            *random = back;
        }
        placed
    }

    /// `placeInWorld(level, pos, pivot, settings, random, flags)`.
    #[allow(clippy::too_many_arguments)]
    pub fn place_in_world(
        &self,
        r: &mut Region,
        p: BlockPos,
        pivot: BlockPos,
        settings: &mut PlaceSettings,
        random: &mut WorldgenRandom,
        flags: i32,
    ) -> bool {
        if self.palettes.is_empty() {
            return false;
        }
        let palette = &self.palettes[settings.palette_index(self.palettes.len(), p)];
        if (palette.blocks.is_empty() && (settings.ignore_entities || self.entities.is_empty()))
            || self.size[0] < 1
            || self.size[1] < 1
            || self.size[2] < 1
        {
            return false;
        }
        let bbox = settings.bbox;
        let water = settings.apply_waterlogging();
        let mut to_fill: Vec<BlockPos> = Vec::new();
        let mut sources: Vec<BlockPos> = Vec::new();
        let mut placed: Vec<(BlockPos, bool)> = Vec::new();
        let (mut min, mut max) = ([i32::MAX; 3], [i32::MIN; 3]);
        let infos = process_block_infos(r, p, pivot, settings, &palette.blocks);
        for info in infos {
            let at = info.pos;
            if bbox.is_some_and(|b| !b.is_inside(at)) {
                continue;
            }
            let before = water.then(|| fluid(r.get(at)));
            let s = rotate(mirror(info.state, settings.mirror), settings.rotation);
            if info.nbt.is_some() {
                r.set(at, state::BARRIER, 820);
            }
            if !r.set(at, s, flags) {
                continue;
            }
            min = [min[0].min(at.x), min[1].min(at.y), min[2].min(at.z)];
            max = [max[0].max(at.x), max[1].max(at.y), max[2].max(at.z)];
            placed.push((at, info.nbt.is_some()));
            if let Some(nbt) = &info.nbt
                && kiln_data::block_props::has_block_entity(r.get(at))
            {
                let mut fields = match &**nbt {
                    Tag::Compound(f) => f.clone(),
                    _ => Vec::new(),
                };
                if is_randomizable_container(r.get(at)) {
                    let seed = match (settings.shared_random, settings.random.as_mut()) {
                        (true, Some(shared)) => shared.next_long(),
                        _ => random.next_long(),
                    };
                    put(&mut fields, "LootTableSeed", Tag::Long(seed));
                }
                load_block_entity(r, at, fields);
            }
            if let Some(f) = before {
                if fluid(s).source {
                    sources.push(at);
                } else if is_liquid_container(s) {
                    place_liquid(r, at, s, f.kind);
                    if !f.source {
                        to_fill.push(at);
                    }
                }
            }
        }
        const FILL_DIRS: [Dir; 5] = [Dir::Up, Dir::North, Dir::East, Dir::South, Dir::West];
        let mut changed = true;
        while changed && !to_fill.is_empty() {
            changed = false;
            to_fill.retain(|&at| {
                let mut f = fluid(r.get(at));
                for d in FILL_DIRS {
                    if f.source {
                        break;
                    }
                    let n = at.relative(d);
                    let nf = fluid(r.get(n));
                    if nf.source && !sources.contains(&n) {
                        f = nf;
                    }
                }
                if f.source {
                    let s = r.get(at);
                    if is_liquid_container(s) {
                        place_liquid(r, at, s, f.kind);
                        changed = true;
                        return false;
                    }
                }
                true
            });
        }
        if min[0] <= max[0] && !settings.known_shape {
            let placed: Vec<BlockPos> = placed.iter().map(|(p, _)| *p).collect();
            super::shape::update_placed_shapes(r, flags, &placed, min, max);
        }
        if !settings.ignore_entities {
            self.place_entities(r, p, settings.mirror, settings.rotation, settings.pivot, bbox);
        }
        true
    }

    /// `placeEntities`: the recorded entities inside `bbox`, moved and turned with the
    /// template, go to their chunks' entity lists (`UUID` dropped). Vanilla then runs
    /// `finalizeSpawn` on mobs when the settings ask for it (random equipment and the like
    /// from the level's random); that is left to whoever loads the entity.
    fn place_entities(&self, r: &mut Region, p: BlockPos, m: Mirror, rot: Rotation, pivot: BlockPos, bbox: Option<BoundingBox>) {
        for e in &self.entities {
            let bp = transform(e.block_pos, m, rot, pivot).offset(p.x, p.y, p.z);
            if bbox.is_some_and(|b| !b.is_inside(bp)) {
                continue;
            }
            let v = transform_vec(e.pos, m, rot, pivot);
            let pos = [v[0] + p.x as f64, v[1] + p.y as f64, v[2] + p.z as f64];
            let Tag::Compound(mut fields) = e.nbt.clone() else { continue };
            fields.retain(|(k, _)| k != "UUID");
            let rotation = fields.iter().find(|(k, _)| k == "Rotation").and_then(|(_, v)| v.as_list()).map(|l| l.to_vec());
            let yaw = rotation.as_ref().and_then(|l| l.first()).and_then(|t| t.as_f64()).unwrap_or(0.0) as f32;
            let pitch = rotation.as_ref().and_then(|l| l.get(1)).and_then(|t| t.as_f64()).unwrap_or(0.0) as f32;
            let mut turned = entity_rotate(yaw, rot) + (entity_mirror(yaw, m) - yaw);
            // A hanging entity turns its direction too (`HangingEntity.rotate`/`mirror`), and hangs in the block its new place is in
            // (`snapTo` -> `setPos`; the saved `block_pos` is the old place's).
            let id = fields.iter().find(|(k, _)| k == "id").and_then(|(_, v)| v.as_str()).map(str::to_owned).unwrap_or_default();
            let hanging_key = match id.as_str() {
                "minecraft:item_frame" | "minecraft:glow_item_frame" => Some("Facing"),
                "minecraft:painting" => Some("facing"),
                _ => None,
            };
            let hanging_block = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
            if let Some(key) = hanging_key
                && let Some(slot) = fields.iter_mut().find(|(k, _)| k == key)
                && let Some(value) = slot.1.as_i64()
            {
                // The direction as the horizontal index (south, west, north, east); the item frame's data value has down and up in front.
                let item_frame = key == "Facing";
                let horizontal = if item_frame { [None, None, Some(2), Some(0), Some(1), Some(3)].get(value as usize).copied().flatten() } else { Some(value.rem_euclid(4) as i32) };
                let (f, h) = hanging_turn(horizontal, rot, m);
                turned = f;
                if let Some(h) = h {
                    let written = if item_frame { [3, 4, 2, 5][h as usize] } else { i64::from(h) };
                    slot.1 = Tag::Byte(written as i8);
                }
            }
            for (k, v) in fields.iter_mut() {
                match k.as_str() {
                    "Pos" => *v = Tag::List(pos.iter().map(|&c| Tag::Double(c)).collect()),
                    "Rotation" => *v = Tag::List(vec![Tag::Float(turned), Tag::Float(pitch)]),
                    "block_pos" => *v = Tag::IntArray(if hanging_key.is_some() { hanging_block.to_vec() } else { vec![bp.x, bp.y, bp.z] }),
                    _ => {}
                }
            }
            if !fields.iter().any(|(k, _)| k == "Pos") {
                fields.push(("Pos".into(), Tag::List(pos.iter().map(|&c| Tag::Double(c)).collect())));
            }
            r.add_entity(pos[0], pos[2], Tag::Compound(fields));
        }
    }
}

/// `HangingEntity.rotate(rotation)`: the new horizontal direction (index of south, west, north, east; `None` for one that
/// points up or down, which stays) and the yaw it returns, for the entity facing `direction`.
fn hanging_rotate(direction: Option<i32>, r: Rotation, yaw_of_vertical: f32) -> (Option<i32>, f32) {
    let direction = direction.map(|h| match r {
        Rotation::Clockwise180 => (h + 2) % 4,
        Rotation::CounterClockwise90 => (h + 3) % 4,
        Rotation::Clockwise90 => (h + 1) % 4,
        Rotation::None => h,
    });
    let yaw = wrap_degrees(direction.map_or(yaw_of_vertical, |h| (h * 90) as f32));
    // (The table pairs a counter-clockwise turn of the direction with +90 and a clockwise one with +270.)
    let f = match r {
        Rotation::Clockwise180 => yaw + 180.0,
        Rotation::CounterClockwise90 => yaw + 90.0,
        Rotation::Clockwise90 => yaw + 270.0,
        Rotation::None => yaw,
    };
    (direction, f)
}

/// `rotate` then `mirror` of a hanging entity, as `StructureTemplate.placeEntities` adds them up: the final direction and yaw.
fn hanging_turn(direction: Option<i32>, r: Rotation, m: Mirror) -> (f32, Option<i32>) {
    let (d1, f1) = hanging_rotate(direction, r, 0.0);
    // `Mirror.getRotation(direction)`: a mirror across the axis the entity faces along turns it around.
    let mirror_rotation = match (m, d1) {
        (Mirror::LeftRight, Some(h)) if h % 2 == 0 => Rotation::Clockwise180,
        (Mirror::FrontBack, Some(h)) if h % 2 == 1 => Rotation::Clockwise180,
        _ => Rotation::None,
    };
    let (d2, f2) = hanging_rotate(d1, mirror_rotation, 0.0);
    let y_rot = d2.map_or(0.0, |h| (h * 90) as f32);
    (f1 + (f2 - y_rot), d2)
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(f: f32) -> f32 {
    let mut f = f % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

/// `Entity.rotate(Rotation)`.
fn entity_rotate(yaw: f32, r: Rotation) -> f32 {
    let f = wrap_degrees(yaw);
    match r {
        Rotation::Clockwise180 => f + 180.0,
        Rotation::CounterClockwise90 => f + 270.0,
        Rotation::Clockwise90 => f + 90.0,
        Rotation::None => f,
    }
}

/// `Entity.mirror(Mirror)`.
fn entity_mirror(yaw: f32, m: Mirror) -> f32 {
    let f = wrap_degrees(yaw);
    match m {
        Mirror::FrontBack => -f,
        Mirror::LeftRight => 180.0 - f,
        Mirror::None => f,
    }
}

/// `StructureTemplate.transform(Vec3, mirror, rotation, pivot)`.
pub fn transform_vec(v: [f64; 3], m: Mirror, r: Rotation, pivot: BlockPos) -> [f64; 3] {
    let [mut x, y, mut z] = v;
    match m {
        Mirror::LeftRight => z = 1.0 - z,
        Mirror::FrontBack => x = 1.0 - x,
        Mirror::None => {}
    }
    let (px, pz) = (pivot.x, pivot.z);
    match r {
        Rotation::CounterClockwise90 => [(px - pz) as f64 + z, y, (px + pz + 1) as f64 - x],
        Rotation::Clockwise90 => [(px + pz + 1) as f64 - z, y, (pz - px) as f64 + x],
        Rotation::Clockwise180 => [(px + px + 1) as f64 - x, y, (pz + pz + 1) as f64 - z],
        Rotation::None => [x, y, z],
    }
}

/// `StructureTemplate.getBoundingBox(pos, rotation, pivot, mirror, size)`.
pub fn bounding_box(p: BlockPos, r: Rotation, pivot: BlockPos, m: Mirror, size: [i32; 3]) -> BoundingBox {
    let a = transform(BlockPos::new(0, 0, 0), m, r, pivot);
    let b = transform(BlockPos::new(size[0] - 1, size[1] - 1, size[2] - 1), m, r, pivot);
    BoundingBox::from_corners(a, b).moved(p.x, p.y, p.z)
}

/// `StructureTemplate.processBlockInfos`.
pub fn process_block_infos(
    r: &mut Region,
    p: BlockPos,
    pivot: BlockPos,
    settings: &mut PlaceSettings,
    infos: &[BlockInfo],
) -> Vec<BlockInfo> {
    let entire = settings.processors.iter().any(|q| q.evaluates_entire_piece());
    let bbox = settings.bbox;
    let processors = settings.processors.clone();
    let mut originals: Vec<&BlockInfo> = Vec::new();
    let mut processed: Vec<BlockInfo> = Vec::new();
    for raw in infos {
        let world = transform(raw.pos, settings.mirror, settings.rotation, settings.pivot).offset(p.x, p.y, p.z);
        if !entire && bbox.is_some_and(|b| !b.is_inside(world)) {
            continue;
        }
        let mut info = Some(BlockInfo { pos: world, state: raw.state, nbt: raw.nbt.clone() });
        for q in &processors {
            let Some(i) = info else { break };
            info = q.process(r, p, pivot, raw.pos, i, settings);
        }
        if let Some(i) = info {
            processed.push(i);
            originals.push(raw);
        }
    }
    for q in &processors {
        processed = q.finalize(r, p, pivot, &originals, processed, settings);
    }
    processed
}

/// Whether the state's block entity is a `RandomizableContainer` (its structure NBT gets a
/// fresh `LootTableSeed`).
fn is_randomizable_container(s: u16) -> bool {
    ["ChestBlock", "BarrelBlock", "DispenserBlock", "HopperBlock", "ShulkerBoxBlock", "CrafterBlock", "DecoratedPotBlock"]
        .iter()
        .any(|c| is_instance(s, c))
}

/// `BlockEntity.loadWithComponents` into the placed block's entity data.
fn load_block_entity(r: &mut Region, at: BlockPos, fields: Vec<(String, Tag)>) {
    if let Some(Tag::Compound(be)) = r.block_entity_mut(at) {
        be.retain(|(k, _)| k == "id");
        for (k, v) in fields {
            if k != "id" {
                be.push((k, v));
            }
        }
    }
}

/// `instanceof LiquidBlockContainer`.
fn is_liquid_container(s: u16) -> bool {
    has_prop(s, "waterlogged") || matches!(block_class(s), "KelpBlock" | "KelpPlantBlock" | "SeagrassBlock" | "TallSeagrassBlock")
}

/// `LiquidBlockContainer.placeLiquid`: waterloggable blocks take still water (double slabs
/// do not).
fn place_liquid(r: &mut Region, at: BlockPos, s: u16, kind: FluidKind) -> bool {
    let double_slab = is_instance(s, "SlabBlock") && prop(s, "type") == Some("double");
    if prop(s, "waterlogged") == Some("false") && kind == FluidKind::Water && !double_slab {
        r.set(at, with_prop(s, "waterlogged", "true"), 3);
        r.schedule_fluid_tick(at, "minecraft:water", 5);
        return true;
    }
    false
}

/// `Block.hasDynamicShape`.
fn has_dynamic_shape(s: u16) -> bool {
    let name = block_of(s).name;
    matches!(
        name,
        "minecraft:moving_piston"
            | "minecraft:bamboo"
            | "minecraft:scaffolding"
            | "minecraft:powder_snow"
            | "minecraft:pointed_dripstone"
            | "minecraft:sulfur_spike"
    ) || block_class(s) == "ShulkerBoxBlock"
}

fn int_list(tag: Option<&Tag>) -> [i32; 3] {
    let mut out = [0; 3];
    if let Some(list) = tag.and_then(Tag::as_list) {
        for (o, v) in out.iter_mut().zip(list) {
            *o = v.as_i64().unwrap_or(0) as i32;
        }
    }
    out
}

/// `NbtUtils.readBlockState`: `{id, properties}` (older data: `{Name, Properties}`); unknown
/// blocks read as air, unknown properties are ignored.
fn read_state(tag: &Tag) -> u16 {
    let name = tag.get("id").or_else(|| tag.get("Name")).and_then(Tag::as_str).unwrap_or("minecraft:air");
    let Some(info) = kiln_data::blocks_types::block_by_name(&crate::function::qualify(name)) else { return state::AIR };
    let mut s = info.default;
    if let Some(Tag::Compound(props)) = tag.get("properties").or_else(|| tag.get("Properties")) {
        for (k, v) in props {
            if let Some(v) = v.as_str()
                && let Some(n) = info.with_property(s, k, v)
            {
                s = n;
            }
        }
    }
    s
}

impl Template {
    /// `StructureTemplate.fillFromWorld`: a template of `size` made of `blocks` (positions relative to the area).
    pub fn from_world(size: [i32; 3], blocks: Vec<BlockInfo>, entities: Vec<EntityInfo>) -> Template {
        Template { size, palettes: vec![palette_of(order_infos(blocks))], entities }
    }

    /// `StructureTemplate.save`: the compound a template file holds (`data_version` is `DataVersion`).
    pub fn save(&self, data_version: i32) -> Tag {
        let ints = |v: [i32; 3]| Tag::List(v.iter().map(|&i| Tag::Int(i)).collect());
        let mut out: Vec<(String, Tag)> = Vec::new();
        let write_state = |s: u16| -> Tag {
            let info = block_of(s);
            let mut c = vec![("id".to_owned(), Tag::String(info.name.to_owned()))];
            if !info.properties.is_empty() {
                let props = info.properties.iter().map(|p| (p.name.to_owned(), Tag::String(crate::blocks::prop(s, p.name).unwrap_or("").to_owned()))).collect();
                c.push(("properties".to_owned(), Tag::Compound(props)));
            }
            Tag::Compound(c)
        };
        if self.palettes.is_empty() {
            out.push(("blocks".into(), Tag::List(Vec::new())));
            out.push(("palette".into(), Tag::List(Vec::new())));
        } else {
            // `SimplePalette`: a state gets the next id when it is first seen in the first palette's blocks, the others follow it.
            let mut palettes: Vec<Vec<u16>> = vec![Vec::new(); self.palettes.len()];
            let mut blocks = Vec::new();
            for (i, b) in self.palettes[0].blocks.iter().enumerate() {
                let id = match palettes[0].iter().position(|&s| s == b.state) {
                    Some(id) => id,
                    None => {
                        palettes[0].push(b.state);
                        palettes[0].len() - 1
                    }
                };
                let mut c = vec![("pos".to_owned(), ints([b.pos.x, b.pos.y, b.pos.z])), ("state".to_owned(), Tag::Int(id as i32))];
                if let Some(n) = &b.nbt {
                    c.push(("nbt".to_owned(), (**n).clone()));
                }
                blocks.push(Tag::Compound(c));
                for (k, p) in self.palettes.iter().enumerate().skip(1) {
                    if let Some(o) = p.blocks.get(i) {
                        palettes[k].push(o.state);
                    }
                }
            }
            out.push(("blocks".into(), Tag::List(blocks)));
            if palettes.len() == 1 {
                out.push(("palette".into(), Tag::List(palettes[0].iter().map(|&s| write_state(s)).collect())));
            } else {
                out.push(("palettes".into(), Tag::List(palettes.iter().map(|p| Tag::List(p.iter().map(|&s| write_state(s)).collect())).collect())));
            }
        }
        let entities = self
            .entities
            .iter()
            .map(|e| {
                let mut c = vec![
                    ("pos".to_owned(), Tag::List(e.pos.iter().map(|&d| Tag::Double(d)).collect())),
                    ("blockPos".to_owned(), ints([e.block_pos.x, e.block_pos.y, e.block_pos.z])),
                ];
                c.push(("nbt".to_owned(), e.nbt.clone()));
                Tag::Compound(c)
            })
            .collect();
        out.push(("entities".into(), Tag::List(entities)));
        out.push(("size".into(), ints(self.size)));
        out.push(("DataVersion".into(), Tag::Int(data_version)));
        Tag::Compound(out)
    }

    /// A template file: the gzipped named NBT of [`Template::save`].
    pub fn to_file(&self, data_version: i32) -> Vec<u8> {
        use std::io::Write;
        let mut raw = bytes::BytesMut::new();
        self.save(data_version).write_named("", &mut raw);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = gz.write_all(&raw);
        gz.finish().unwrap_or_default()
    }

    /// `StructureTemplate.load`.
    pub fn load(tag: &Tag) -> Template {
        let size = int_list(tag.get("size"));
        let blocks = tag.get("blocks").and_then(Tag::as_list).unwrap_or(&[]);
        let mut palettes = Vec::new();
        match tag.get("palettes").and_then(Tag::as_list) {
            Some(list) => {
                for p in list {
                    palettes.push(load_palette(p.as_list().unwrap_or(&[]), blocks));
                }
            }
            None => palettes.push(load_palette(tag.get("palette").and_then(Tag::as_list).unwrap_or(&[]), blocks)),
        }
        let entities = tag
            .get("entities")
            .and_then(Tag::as_list)
            .unwrap_or(&[])
            .iter()
            .filter_map(|e| {
                let nbt = e.get("nbt")?.clone();
                let pos = e.get("pos").and_then(Tag::as_list).unwrap_or(&[]);
                let d = |i: usize| pos.get(i).and_then(Tag::as_f64).unwrap_or(0.0);
                let bp = int_list(e.get("blockPos"));
                Some(EntityInfo { pos: [d(0), d(1), d(2)], block_pos: BlockPos::new(bp[0], bp[1], bp[2]), nbt })
            })
            .collect();
        Template { size, palettes, entities }
    }
}

fn load_palette(states: &[Tag], blocks: &[Tag]) -> Palette {
    let states: Vec<u16> = states.iter().map(read_state).collect();
    let infos = blocks
        .iter()
        .map(|b| {
            let p = int_list(b.get("pos"));
            let s = states.get(b.get("state").and_then(Tag::as_i64).unwrap_or(0) as usize).copied().unwrap_or(state::AIR);
            let tag = match b.get("nbt") {
                Some(t @ Tag::Compound(_)) => Some(Arc::new(t.clone())),
                _ => None,
            };
            BlockInfo { pos: BlockPos::new(p[0], p[1], p[2]), state: s, nbt: tag }
        })
        .collect();
    palette_of(order_infos(infos))
}

/// `StructureTemplate.buildInfoList`: the blocks that fill a whole cube, then the others, then those with block entity data,
/// each by y, x, z.
fn order_infos(infos: Vec<BlockInfo>) -> Vec<BlockInfo> {
    let (mut full, mut nbt, mut other) = (Vec::new(), Vec::new(), Vec::new());
    for info in infos {
        let s = info.state;
        if info.nbt.is_some() {
            nbt.push(info);
        } else if !has_dynamic_shape(s) && kiln_data::block_props::full_collision(s) {
            full.push(info);
        } else {
            other.push(info);
        }
    }
    let key = |b: &BlockInfo| (b.pos.y, b.pos.x, b.pos.z);
    full.sort_by_key(key);
    nbt.sort_by_key(key);
    other.sort_by_key(key);
    let mut blocks = full;
    blocks.append(&mut other);
    blocks.append(&mut nbt);
    blocks
}

fn palette_of(blocks: Vec<BlockInfo>) -> Palette {
    let jigsaws = blocks.iter().filter(|b| block_of(b.state).name == "minecraft:jigsaw").map(parse_jigsaw).collect();
    let markers = blocks.iter().enumerate().filter(|(_, b)| block_of(b.state).name == "minecraft:structure_block").map(|(i, _)| i).collect();
    Palette { blocks, jigsaws, markers }
}

/// `JigsawBlockInfo.parse`.
fn parse_jigsaw(b: &BlockInfo) -> JigsawInfo {
    let empty = Tag::Compound(Vec::new());
    let nbt = b.nbt.as_deref().unwrap_or(&empty);
    let id = |k: &str| nbt.get(k).and_then(Tag::as_str).map_or_else(|| "minecraft:empty".to_string(), crate::function::qualify);
    let rollable = match nbt.get("joint").and_then(Tag::as_str) {
        Some("rollable") => true,
        Some("aligned") => false,
        _ => !orientation(b.state).0.is_horizontal(),
    };
    let int = |k: &str| nbt.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
    JigsawInfo {
        pos: b.pos,
        state: b.state,
        rollable,
        name: Some(id("name")),
        pool: id("pool"),
        target: id("target"),
        placement_priority: int("placement_priority"),
        selection_priority: int("selection_priority"),
    }
}

/// Where templates come from: an extracted `data/` tree or a jar.
enum Source {
    Dir(PathBuf),
    Jar { path: PathBuf, entries: HashMap<String, (u64, u32, u16)> },
}

/// `StructureTemplateManager`: templates by id, loaded on first use and kept.
pub struct TemplateManager {
    sources: Vec<Source>,
    cache: Mutex<HashMap<String, Arc<Template>>>,
}

impl TemplateManager {
    /// Sources near a datapack root: `$KILN_TEMPLATES` (a directory holding `data/`, or a
    /// jar), the pack's own `data/*/structure`, then `../versions/*/server-*.jar` (the work
    /// directory layout).
    pub fn near(root: &Path) -> TemplateManager {
        let mut sources = Vec::new();
        let mut add = |p: PathBuf| {
            if p.is_dir() {
                sources.push(Source::Dir(p));
            } else if p.is_file()
                && let Ok(entries) = read_zip_index(&p)
            {
                sources.push(Source::Jar { path: p, entries });
            }
        };
        if let Some(p) = std::env::var_os("KILN_TEMPLATES") {
            add(PathBuf::from(p));
        }
        add(root.to_path_buf());
        if let Some(parent) = root.parent()
            && let Ok(dirs) = std::fs::read_dir(parent.join("versions"))
        {
            let mut jars: Vec<PathBuf> = dirs
                .filter_map(|d| d.ok())
                .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())))
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("server-") && n.ends_with(".jar"))
                })
                .collect();
            jars.sort();
            jars.into_iter().rev().for_each(&mut add);
        }
        TemplateManager { sources, cache: Mutex::new(HashMap::new()) }
    }

    /// `getOrCreate(id)`: a missing template is empty (vanilla logs and returns an empty one).
    pub fn get(&self, id: &str) -> Arc<Template> {
        if let Some(t) = self.cache.lock().unwrap().get(id) {
            return t.clone();
        }
        let t = Arc::new(self.read(id).map(|tag| Template::load(&tag)).unwrap_or_default());
        self.cache.lock().unwrap().entry(id.to_string()).or_insert(t).clone()
    }

    /// A template that exists, from the data pack directories `roots` (the last one wins,
    /// as later packs replace earlier ones; read afresh, packs reload) or else the manager's
    /// own sources. `StructureTemplateManager.get`, which does not make up an empty template.
    pub fn find_in(&self, roots: &[PathBuf], id: &str) -> Option<Arc<Template>> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let rel = format!("data/{ns}/structure/{path}.nbt");
        for root in roots.iter().rev() {
            if let Ok(b) = std::fs::read(root.join(&rel))
                && let Some(tag) = decode_template(&b)
            {
                return Some(Arc::new(Template::load(&tag)));
            }
        }
        if let Some(t) = self.cache.lock().unwrap().get(id) {
            return Some(t.clone());
        }
        let t = Arc::new(Template::load(&self.read(id)?));
        Some(self.cache.lock().unwrap().entry(id.to_string()).or_insert(t).clone())
    }

    fn read(&self, id: &str) -> Option<Tag> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let rel = format!("data/{ns}/structure/{path}.nbt");
        for s in &self.sources {
            let bytes = match s {
                Source::Dir(d) => std::fs::read(d.join(&rel)).ok(),
                Source::Jar { path, entries } => entries.get(&rel).and_then(|e| read_zip_entry(path, *e).ok()),
            };
            if let Some(b) = bytes {
                return decode_template(&b);
            }
        }
        None
    }
}

/// A template file read from `path` (the world's generated structures).
pub fn read_template_file(path: &Path) -> Option<Template> {
    decode_template(&std::fs::read(path).ok()?).map(|t| Template::load(&t))
}

/// A template from the bytes of a file.
pub fn read_template_bytes(bytes: &[u8]) -> Option<Template> {
    decode_template(bytes).map(|t| Template::load(&t))
}

/// A template file: gzipped named NBT.
fn decode_template(bytes: &[u8]) -> Option<Tag> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw).ok()?;
    kiln_proto::nbt::read_named(&raw).ok().map(|(_, t)| t)
}

impl std::fmt::Debug for TemplateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TemplateManager({} sources)", self.sources.len())
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The central directory of a zip: name → (local header offset, compressed size, method).
fn read_zip_index(path: &Path) -> std::io::Result<HashMap<String, (u64, u32, u16)>> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a zip");
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let tail = len.min(65_557);
    f.seek(SeekFrom::Start(len - tail))?;
    let mut buf = vec![0; tail as usize];
    f.read_exact(&mut buf)?;
    let eocd = (0..buf.len().saturating_sub(21)).rev().find(|&i| u32_at(&buf, i) == 0x0605_4b50).ok_or_else(bad)?;
    let cd_size = u32_at(&buf, eocd + 12) as u64;
    let cd_offset = u32_at(&buf, eocd + 16) as u64;
    f.seek(SeekFrom::Start(cd_offset))?;
    let mut cd = vec![0; cd_size as usize];
    f.read_exact(&mut cd)?;
    let mut out = HashMap::new();
    let mut i = 0;
    while i + 46 <= cd.len() && u32_at(&cd, i) == 0x0201_4b50 {
        let method = u16_at(&cd, i + 10);
        let csize = u32_at(&cd, i + 20);
        let (n, e, c) = (u16_at(&cd, i + 28) as usize, u16_at(&cd, i + 30) as usize, u16_at(&cd, i + 32) as usize);
        let offset = u32_at(&cd, i + 42) as u64;
        let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + n]).into_owned();
        if name.ends_with(".nbt") {
            out.insert(name, (offset, csize, method));
        }
        i += 46 + n + e + c;
    }
    Ok(out)
}

fn read_zip_entry(path: &Path, (offset, csize, method): (u64, u32, u16)) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; 30];
    f.read_exact(&mut header)?;
    let skip = u16_at(&header, 26) as i64 + u16_at(&header, 28) as i64;
    f.seek(SeekFrom::Current(skip))?;
    let mut data = vec![0; csize as usize];
    f.read_exact(&mut data)?;
    match method {
        0 => Ok(data),
        8 => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(&data[..]).read_to_end(&mut out)?;
            Ok(out)
        }
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unsupported zip method")),
    }
}

