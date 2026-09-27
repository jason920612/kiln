//! Template pools (`StructureTemplatePool`) and their elements (`StructurePoolElement`).

use crate::Error;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::structure::bbox::BoundingBox;
use crate::structure::piece::PlaceContext;
use crate::structure::processor::{
    IGNORE_STRUCTURE_AND_AIR, IGNORE_STRUCTURE_BLOCK, JIGSAW_REPLACEMENT, ProcessorList, ProcessorLists, TERRAIN_MATCHING_GRAVITY,
    json_to_tag,
};
use crate::structure::template::{JigsawInfo, LiquidSettings, PlaceSettings, TemplateManager};
use crate::structure::transform::Rotation;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// `StructureTemplatePool.Projection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    TerrainMatching,
    Rigid,
}

impl Projection {
    pub fn parse(name: &str) -> Result<Projection, Error> {
        match name {
            "rigid" => Ok(Projection::Rigid),
            "terrain_matching" => Ok(Projection::TerrainMatching),
            p => Err(Error::Invalid(format!("unknown projection {p}"))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Projection::Rigid => "rigid",
            Projection::TerrainMatching => "terrain_matching",
        }
    }
}

/// A pool element's type and configuration.
#[derive(Debug)]
pub enum ElementKind {
    /// `SinglePoolElement` (`legacy`: `LegacySinglePoolElement`).
    Single { template: String, processors: ProcessorList, liquid: Option<LiquidSettings>, legacy: bool },
    /// `FeaturePoolElement`: a placed feature by id.
    Feature(String),
    List(Vec<PoolElement>),
    Empty,
}

/// `StructurePoolElement`.
#[derive(Debug)]
pub struct PoolElement {
    pub kind: ElementKind,
    pub projection: Projection,
    /// The element as `StructurePoolElement.CODEC` saves it in piece NBT.
    pub nbt: Tag,
}

/// Utility `Util.shuffle`.
pub fn shuffle<T>(list: &mut [T], random: &mut WorldgenRandom) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
}

impl PoolElement {
    fn parse(json: &Json, lists: &ProcessorLists, l: &Loader, projection_override: Option<Projection>) -> Result<PoolElement, Error> {
        let ty = json.get("element_type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("pool element without type".into()))?;
        let ty = ty.strip_prefix("minecraft:").unwrap_or(ty);
        let projection = match projection_override {
            Some(p) => p,
            None if ty == "empty_pool_element" => Projection::TerrainMatching,
            None => Projection::parse(json.get("projection").and_then(Json::as_str).unwrap_or(""))?,
        };
        let mut nbt = vec![("element_type".to_string(), Tag::String(format!("minecraft:{ty}")))];
        let kind = match ty {
            "single_pool_element" | "legacy_single_pool_element" => {
                let location = crate::function::qualify(json.get("location").and_then(Json::as_str).unwrap_or(""));
                let pj = json.get("processors").ok_or_else(|| Error::Invalid("pool element without processors".into()))?;
                let liquid = match json.get("override_liquid_settings").and_then(Json::as_str) {
                    Some(n) => Some(LiquidSettings::parse(n).ok_or_else(|| Error::Invalid(format!("bad liquid settings {n}")))?),
                    None => None,
                };
                nbt.push(("location".into(), Tag::String(location.clone())));
                nbt.push((
                    "processors".into(),
                    match pj {
                        Json::String(id) => Tag::String(crate::function::qualify(id)),
                        other => json_to_tag(other),
                    },
                ));
                if let Some(ls) = liquid {
                    nbt.push(("override_liquid_settings".into(), Tag::String(ls.name().into())));
                }
                ElementKind::Single {
                    template: location,
                    processors: lists.get(pj, l)?,
                    liquid,
                    legacy: ty == "legacy_single_pool_element",
                }
            }
            "feature_pool_element" => {
                let feature = crate::function::qualify(json.get("feature").and_then(Json::as_str).unwrap_or(""));
                nbt.push(("feature".into(), Tag::String(feature.clone())));
                ElementKind::Feature(feature)
            }
            "list_pool_element" => {
                let elements = json
                    .get("elements")
                    .and_then(Json::as_array)
                    .ok_or_else(|| Error::Invalid("list element without elements".into()))?
                    .iter()
                    .map(|e| PoolElement::parse(e, lists, l, Some(projection)))
                    .collect::<Result<Vec<_>, _>>()?;
                nbt.push(("elements".into(), Tag::List(elements.iter().map(|e| e.nbt.clone()).collect())));
                ElementKind::List(elements)
            }
            "empty_pool_element" => ElementKind::Empty,
            t => return Err(Error::Invalid(format!("unsupported pool element {t}"))),
        };
        if !matches!(kind, ElementKind::Empty) {
            nbt.push(("projection".into(), Tag::String(projection.name().into())));
        }
        Ok(PoolElement { kind, projection, nbt: Tag::Compound(nbt) })
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.kind, ElementKind::Empty)
    }

    /// `getGroundLevelDelta`.
    pub fn ground_level_delta(&self) -> i32 {
        1
    }

    /// `getSize(manager, rotation)`.
    pub fn size(&self, tm: &TemplateManager, r: Rotation) -> [i32; 3] {
        match &self.kind {
            ElementKind::Single { template, .. } => tm.get(template).size_rotated(r),
            ElementKind::List(es) => es.iter().fold([0; 3], |a, e| {
                let s = e.size(tm, r);
                [a[0].max(s[0]), a[1].max(s[1]), a[2].max(s[2])]
            }),
            ElementKind::Feature(_) | ElementKind::Empty => [0; 3],
        }
    }

    /// `getShuffledJigsawBlocks(manager, pos, rotation, random)`.
    pub fn shuffled_jigsaws(&self, tm: &TemplateManager, p: BlockPos, r: Rotation, random: &mut WorldgenRandom) -> Vec<JigsawInfo> {
        match &self.kind {
            ElementKind::Single { template, .. } => {
                let mut list = tm.get(template).jigsaws(p, r);
                shuffle(&mut list, random);
                list.sort_by(|a, b| b.selection_priority.cmp(&a.selection_priority));
                list
            }
            ElementKind::List(es) => es[0].shuffled_jigsaws(tm, p, r, random),
            ElementKind::Feature(_) => vec![JigsawInfo {
                pos: p,
                state: crate::blocks::with_prop(crate::blocks::state::JIGSAW, "orientation", "down_south"),
                rollable: true,
                name: None,
                pool: "minecraft:empty".into(),
                target: "minecraft:empty".into(),
                placement_priority: 0,
                selection_priority: 0,
            }],
            ElementKind::Empty => Vec::new(),
        }
    }

    /// `getBoundingBox(manager, pos, rotation)`.
    pub fn bounding_box(&self, tm: &TemplateManager, p: BlockPos, r: Rotation) -> BoundingBox {
        match &self.kind {
            ElementKind::Single { template, .. } => tm.get(template).bounding_box(&PlaceSettings::with_rotation(r), p),
            ElementKind::List(es) => {
                let mut boxes = es.iter().filter(|e| !e.is_empty()).map(|e| e.bounding_box(tm, p, r));
                let first = boxes.next().expect("Unable to calculate boundingbox for ListPoolElement");
                boxes.fold(first, |mut a, b| {
                    a.encapsulate(&b);
                    a
                })
            }
            ElementKind::Feature(_) => {
                let s = self.size(tm, r);
                BoundingBox::new(p.x, p.y, p.z, p.x + s[0], p.y + s[1], p.z + s[2])
            }
            ElementKind::Empty => panic!("Invalid call to EmptyPoolElement.getBoundingBox, filter me!"),
        }
    }

    /// `place(manager, level, structureManager, generator, pos, pivot, rotation, box, random,
    /// liquidSettings, keepJigsaws)`.
    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &self,
        cx: &PlaceContext,
        r: &mut Region,
        p: BlockPos,
        pivot: BlockPos,
        rotation: Rotation,
        bbox: &BoundingBox,
        random: &mut WorldgenRandom,
        liquid: LiquidSettings,
        keep_jigsaws: bool,
    ) -> bool {
        match &self.kind {
            ElementKind::Single { template, processors, liquid: over, legacy } => {
                let t = cx.structures.templates.get(template);
                let mut settings = PlaceSettings::with_rotation(rotation);
                settings.bbox = Some(*bbox);
                settings.known_shape = true;
                settings.ignore_entities = false;
                settings.processors.push(&IGNORE_STRUCTURE_BLOCK);
                settings.finalize_entities = true;
                settings.liquid = over.unwrap_or(liquid);
                if !keep_jigsaws {
                    settings.processors.push(&JIGSAW_REPLACEMENT);
                }
                settings.processors.extend(processors.iter());
                if self.projection == Projection::TerrainMatching {
                    settings.processors.push(&TERRAIN_MATCHING_GRAVITY);
                }
                if *legacy {
                    settings.pop_processor(&IGNORE_STRUCTURE_BLOCK);
                    settings.processors.push(&IGNORE_STRUCTURE_AND_AIR);
                }
                // Data markers go through the processors too, but nothing in them draws from
                // shared state and `handleDataMarker` does nothing for pool elements.
                t.place_in_world(r, p, pivot, &mut settings, random, 18)
            }
            ElementKind::Feature(name) => {
                let Some(features) = cx.features else { return false };
                let Some(id) = features.placed_id(name) else { return false };
                features.place_placed(id, r, random, p, false)
            }
            ElementKind::List(es) => es.iter().all(|e| e.place(cx, r, p, pivot, rotation, bbox, random, liquid, keep_jigsaws)),
            ElementKind::Empty => true,
        }
    }
}

/// `StructureTemplatePool`.
#[derive(Debug)]
pub struct Pool {
    pub name: String,
    /// Pool index of the fallback.
    pub fallback: usize,
    /// Elements repeated by weight.
    pub templates: Vec<Arc<PoolElement>>,
    max_size: OnceLock<i32>,
}

impl Pool {
    /// `getRandomTemplate`: `None` for the empty element.
    pub fn random_template(&self, random: &mut WorldgenRandom) -> Option<&Arc<PoolElement>> {
        if self.templates.is_empty() {
            return None;
        }
        Some(&self.templates[random.next_int_bounded(self.templates.len() as i32) as usize])
    }

    /// `getShuffledTemplates`.
    pub fn shuffled(&self, random: &mut WorldgenRandom) -> Vec<Arc<PoolElement>> {
        let mut list = self.templates.clone();
        shuffle(&mut list, random);
        list
    }

    /// `getMaxSize`: the tallest non-empty element.
    pub fn max_size(&self, tm: &TemplateManager) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|e| !e.is_empty())
                .map(|e| e.bounding_box(tm, BlockPos::new(0, 0, 0), Rotation::None).y_span())
                .max()
                .unwrap_or(0)
        })
    }
}

/// Every template pool of the datapack.
#[derive(Debug)]
pub struct Pools {
    pub pools: Vec<Pool>,
    ids: HashMap<String, usize>,
}

impl Pools {
    pub fn load(l: &Loader) -> Result<Pools, Error> {
        let lists = ProcessorLists::load(l)?;
        let empty = Vec::new();
        let raw = l.pack.registries.get("template_pool").unwrap_or(&empty);
        let ids: HashMap<String, usize> = raw.iter().enumerate().map(|(i, (id, _))| (id.clone(), i)).collect();
        let mut pools = Vec::with_capacity(raw.len());
        for (id, json) in raw {
            let parse = || -> Result<Pool, Error> {
                let fallback = crate::function::qualify(json.get("fallback").and_then(Json::as_str).unwrap_or("minecraft:empty"));
                let fallback = *ids.get(&fallback).ok_or_else(|| Error::Invalid(format!("unknown fallback pool {fallback}")))?;
                let mut templates = Vec::new();
                for e in json.get("elements").and_then(Json::as_array).unwrap_or(&[]) {
                    let element =
                        Arc::new(PoolElement::parse(e.get("element").ok_or_else(|| Error::Invalid("entry without element".into()))?, &lists, l, None)?);
                    let weight = e.get("weight").and_then(Json::as_i32).unwrap_or(1);
                    for _ in 0..weight {
                        templates.push(element.clone());
                    }
                }
                Ok(Pool { name: id.clone(), fallback, templates, max_size: OnceLock::new() })
            };
            pools.push(parse().map_err(|e| e.context(id))?);
        }
        Ok(Pools { pools, ids })
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.ids.get(name).copied()
    }
}
