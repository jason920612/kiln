//! Configured features (`worldgen/feature`) and placed features (`worldgen/placed_feature`):
//! loading and dispatch. Feature types without an implementation load as
//! [`Feature::Unsupported`] and place nothing; [`Features::gaps`] counts how often each was
//! asked to.

pub mod cuboid;
pub mod ore;
pub mod simple;
pub mod template;
pub mod terrain;
pub mod trees;
pub mod vegetation;

use crate::Error;
use crate::json::Json;
use crate::placement::{Modifier, PlacedFeature};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// A configured feature.
#[derive(Debug)]
pub enum Feature {
    NoOp,
    SimpleBlock(simple::SimpleBlock),
    Ore(ore::Ore),
    ScatteredOre(ore::Ore),
    RandomSelector { features: Vec<(usize, f32)>, default: usize },
    SimpleRandomSelector(Vec<usize>),
    WeightedRandomSelector(crate::providers::Weighted<usize>),
    RandomBooleanSelector { if_true: usize, if_false: usize },
    Sequence(Vec<usize>),
    Overlay(Vec<usize>),
    Trees(trees::Kind),
    Terrain(terrain::Kind),
    Vegetation(vegetation::Kind),
    Template(template::TemplateFeature),
    Fossil(template::Fossil),
    /// A type Kiln does not implement: places nothing. Index into [`Features::gaps`].
    Unsupported(usize),
}

#[derive(Debug)]
pub struct Configured {
    pub name: String,
    pub feature: Feature,
}

/// All configured and placed features of a datapack.
#[derive(Debug)]
pub struct Features {
    pub configured: Vec<Configured>,
    ids: HashMap<String, usize>,
    pub placed: Vec<PlacedFeature>,
    placed_ids: HashMap<String, usize>,
    /// Placed features in each biome's feature lists, for biome filters.
    biome_features: Vec<HashSet<usize>>,
    /// Unimplemented feature types and how many placements of each were skipped.
    gaps: Vec<(String, AtomicU64)>,
    /// Structure templates, for template and fossil features.
    templates: std::sync::Arc<crate::structure::template::TemplateManager>,
    /// Processor lists, while features are parsed.
    lists: Option<crate::structure::processor::ProcessorLists>,
}

impl Features {
    /// Loads every configured and placed feature of the datapack.
    pub fn load(l: &Loader) -> Result<Features, Error> {
        let mut f = Features {
            configured: Vec::new(),
            ids: HashMap::new(),
            placed: Vec::new(),
            placed_ids: HashMap::new(),
            biome_features: Vec::new(),
            gaps: Vec::new(),
            templates: template::manager(l),
            lists: Some(crate::structure::processor::ProcessorLists::load(l)?),
        };
        for (id, _) in &l.pack.features {
            f.ids.insert(id.clone(), f.configured.len());
            f.configured.push(Configured { name: id.clone(), feature: Feature::NoOp });
        }
        for (id, _) in &l.pack.placed_features {
            f.placed_ids.insert(id.clone(), f.placed.len());
            f.placed.push(PlacedFeature { name: id.clone(), feature: 0, placement: Vec::new() });
        }
        for (i, (id, json)) in l.pack.features.iter().enumerate() {
            let feature = f.parse_feature(json, l).map_err(|e| e.context(id))?;
            f.configured[i].feature = feature;
        }
        for (i, (id, json)) in l.pack.placed_features.iter().enumerate() {
            let (feature, placement) = f.parse_placed_body(json, l).map_err(|e| e.context(id))?;
            f.placed[i].feature = feature;
            f.placed[i].placement = placement;
        }
        f.lists = None;
        Ok(f)
    }

    pub fn placed_id(&self, name: &str) -> Option<usize> {
        self.placed_ids.get(name).copied()
    }

    pub fn feature_id(&self, name: &str) -> Option<usize> {
        self.ids.get(name).copied()
    }

    /// Records which placed features each biome lists (`BiomeGenerationSettings.featureSet`).
    pub fn set_biome_features(&mut self, sets: Vec<HashSet<usize>>) {
        self.biome_features = sets;
    }

    /// `BiomeGenerationSettings.hasFeature`.
    pub fn biome_has(&self, biome: u16, placed: usize) -> bool {
        self.biome_features.get(biome as usize).is_some_and(|s| s.contains(&placed))
    }

    /// Skipped placements per unimplemented feature type.
    pub fn gaps(&self) -> BTreeMap<String, u64> {
        self.gaps.iter().map(|(n, c)| (n.clone(), c.load(Ordering::Relaxed))).filter(|(_, c)| *c > 0).collect()
    }

    /// Unimplemented feature types referenced by the loaded data.
    pub fn unsupported_types(&self) -> Vec<&str> {
        self.gaps.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// A feature reference: a registry id or an inline definition.
    pub fn feature_ref(&mut self, json: &Json, l: &Loader) -> Result<usize, Error> {
        if let Some(id) = json.as_str() {
            return self
                .ids
                .get(&crate::function::qualify(id))
                .copied()
                .ok_or_else(|| Error::Invalid(format!("unknown feature {id}")));
        }
        let feature = self.parse_feature(json, l)?;
        self.configured.push(Configured { name: String::new(), feature });
        Ok(self.configured.len() - 1)
    }

    /// A placed feature reference: a registry id or an inline `{feature, placement}`.
    pub fn placed_ref(&mut self, json: &Json, l: &Loader) -> Result<usize, Error> {
        if let Some(id) = json.as_str() {
            return self
                .placed_ids
                .get(&crate::function::qualify(id))
                .copied()
                .ok_or_else(|| Error::Invalid(format!("unknown placed feature {id}")));
        }
        let (feature, placement) = self.parse_placed_body(json, l)?;
        self.placed.push(PlacedFeature { name: String::new(), feature, placement });
        Ok(self.placed.len() - 1)
    }

    /// `HolderSet<PlacedFeature>`: a list of references (or a single one).
    pub fn placed_list(&mut self, json: &Json, l: &Loader) -> Result<Vec<usize>, Error> {
        match json.as_array() {
            Some(items) => items.iter().map(|p| self.placed_ref(p, l)).collect(),
            None => Ok(vec![self.placed_ref(json, l)?]),
        }
    }

    fn parse_placed_body(&mut self, json: &Json, l: &Loader) -> Result<(usize, Vec<Modifier>), Error> {
        let feature = self.feature_ref(json.get("feature").ok_or_else(|| Error::Invalid("placed feature without feature".into()))?, l)?;
        let placement = match json.get("placement").and_then(Json::as_array) {
            Some(list) => list.iter().map(|m| Modifier::parse(m, l)).collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        Ok((feature, placement))
    }

    fn parse_feature(&mut self, json: &Json, l: &Loader) -> Result<Feature, Error> {
        let ty = json.get("type").and_then(Json::as_str).ok_or_else(|| Error::Invalid("feature without type".into()))?;
        let ty = ty.strip_prefix("minecraft:").unwrap_or(ty);
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("feature {ty} without {k}")));
        Ok(match ty {
            "no_op" => Feature::NoOp,
            "simple_block" => Feature::SimpleBlock(simple::SimpleBlock::parse(json, l)?),
            "ore" => Feature::Ore(ore::Ore::parse(json, l)?),
            "scattered_ore" => Feature::ScatteredOre(ore::Ore::parse(json, l)?),
            "random_selector" => {
                let features = field("features")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("features must be a list".into()))?
                    .iter()
                    .map(|e| {
                        let chance = crate::providers::float(e, "chance")?;
                        Ok((self.placed_ref(e.get("feature").ok_or_else(|| Error::Invalid("entry without feature".into()))?, l)?, chance))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let default = self.placed_ref(field("default")?, l)?;
                Feature::RandomSelector { features, default }
            }
            "simple_random_selector" => Feature::SimpleRandomSelector(self.placed_list(field("features")?, l)?),
            "weighted_random_selector" => {
                let list = field("features")?.as_array().ok_or_else(|| Error::Invalid("features must be a list".into()))?;
                let mut entries = Vec::with_capacity(list.len());
                for e in list {
                    let data = e.get("data").ok_or_else(|| Error::Invalid("weighted entry without data".into()))?;
                    entries.push((self.placed_ref(data, l)?, crate::providers::int(e, "weight")?));
                }
                Feature::WeightedRandomSelector(crate::providers::Weighted::new(entries))
            }
            "random_boolean_selector" => Feature::RandomBooleanSelector {
                if_true: self.placed_ref(field("feature_true")?, l)?,
                if_false: self.placed_ref(field("feature_false")?, l)?,
            },
            "sequence" => Feature::Sequence(self.placed_list(field("features")?, l)?),
            "overlay" => Feature::Overlay(self.placed_list(field("features")?, l)?),
            "template" | "fossil" => {
                let lists = match &self.lists {
                    Some(lists) => lists,
                    None => &crate::structure::processor::ProcessorLists::load(l)?,
                };
                if ty == "template" {
                    Feature::Template(template::TemplateFeature::parse(json, lists, l)?)
                } else {
                    Feature::Fossil(template::Fossil::parse(json, lists, l)?)
                }
            }
            other => {
                if let Some(k) = trees::parse(other, json, self, l) {
                    return Ok(Feature::Trees(k?));
                }
                if let Some(k) = terrain::parse(other, json, self, l) {
                    return Ok(Feature::Terrain(k?));
                }
                if let Some(k) = vegetation::parse(other, json, self, l) {
                    return Ok(Feature::Vegetation(k?));
                }
                let name = format!("minecraft:{other}");
                let i = match self.gaps.iter().position(|(n, _)| *n == name) {
                    Some(i) => i,
                    None => {
                        self.gaps.push((name, AtomicU64::new(0)));
                        self.gaps.len() - 1
                    }
                };
                Feature::Unsupported(i)
            }
        })
    }

    /// `Feature.place(level, generator, random, origin)`.
    pub fn place_feature(&self, id: usize, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        use kiln_javamath::random::RandomSource;
        match &self.configured[id].feature {
            Feature::NoOp => true,
            Feature::SimpleBlock(f) => f.place(r, random, p),
            Feature::Ore(f) => f.place(r, random, p),
            Feature::ScatteredOre(f) => f.place_scattered(r, random, p),
            Feature::RandomSelector { features, default } => {
                for (f, chance) in features {
                    if random.next_float() < *chance {
                        return self.place_placed(*f, r, random, p, false);
                    }
                }
                self.place_placed(*default, r, random, p, false)
            }
            Feature::SimpleRandomSelector(list) => {
                let i = random.next_int_bounded(list.len() as i32) as usize;
                self.place_placed(list[i], r, random, p, false)
            }
            Feature::WeightedRandomSelector(w) => match w.pick(random) {
                Some(&f) => self.place_placed(f, r, random, p, false),
                None => false,
            },
            Feature::RandomBooleanSelector { if_true, if_false } => {
                let f = if random.next_bool() { *if_true } else { *if_false };
                self.place_placed(f, r, random, p, false)
            }
            Feature::Sequence(list) => {
                for &f in list {
                    if !self.place_placed(f, r, random, p, false) {
                        return false;
                    }
                }
                true
            }
            Feature::Overlay(list) => {
                let mut any = false;
                for &f in list {
                    any |= self.place_placed(f, r, random, p, false);
                }
                any
            }
            Feature::Trees(k) => k.place(self, r, random, p),
            Feature::Terrain(k) => k.place(self, r, random, p),
            Feature::Vegetation(k) => k.place(self, r, random, p),
            Feature::Template(f) => f.place(&self.templates, r, random, p),
            Feature::Fossil(f) => f.place(&self.templates, r, random, p),
            Feature::Unsupported(i) => {
                self.gaps[*i].1.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// `PlacedFeature.getFeatures()`: the configured feature and what its configuration nests,
    /// appended to `out` in vanilla's order.
    pub fn placed_features_of(&self, placed: usize, out: &mut Vec<usize>) {
        self.configured_features_of(self.placed[placed].feature, out);
    }

    /// `ConfiguredFeature.getFeatures()`.
    pub fn configured_features_of(&self, id: usize, out: &mut Vec<usize>) {
        out.push(id);
        match &self.configured[id].feature {
            Feature::RandomSelector { features, default } => {
                for (f, _) in features {
                    self.placed_features_of(*f, out);
                }
                self.placed_features_of(*default, out);
            }
            Feature::SimpleRandomSelector(l) | Feature::Sequence(l) | Feature::Overlay(l) => {
                for f in l {
                    self.placed_features_of(*f, out);
                }
            }
            Feature::WeightedRandomSelector(w) => {
                for (f, _) in &w.entries {
                    self.placed_features_of(*f, out);
                }
            }
            Feature::RandomBooleanSelector { if_true, if_false } => {
                self.placed_features_of(*if_true, out);
                self.placed_features_of(*if_false, out);
            }
            Feature::Terrain(k) => {
                for f in k.nested() {
                    self.placed_features_of(f, out);
                }
            }
            Feature::Vegetation(k) => {
                for f in k.nested() {
                    self.placed_features_of(f, out);
                }
            }
            _ => {}
        }
    }

    /// Whether a configured feature (or anything it places) is unimplemented.
    pub fn is_supported(&self, id: usize) -> bool {
        let mut seen = HashSet::new();
        self.supported_rec(id, &mut seen)
    }

    fn supported_rec(&self, id: usize, seen: &mut HashSet<usize>) -> bool {
        if !seen.insert(id) {
            return true;
        }
        let placed = |p: usize, seen: &mut HashSet<usize>| self.supported_rec(self.placed[p].feature, seen);
        match &self.configured[id].feature {
            Feature::Unsupported(_) => false,
            Feature::RandomSelector { features, default } => {
                features.iter().all(|(f, _)| placed(*f, seen)) && placed(*default, seen)
            }
            Feature::SimpleRandomSelector(l) | Feature::Sequence(l) | Feature::Overlay(l) => l.iter().all(|f| placed(*f, seen)),
            Feature::WeightedRandomSelector(w) => w.entries.iter().all(|(f, _)| placed(*f, seen)),
            Feature::RandomBooleanSelector { if_true, if_false } => placed(*if_true, seen) && placed(*if_false, seen),
            Feature::Trees(k) => k.nested().into_iter().all(|f| placed(f, seen)),
            Feature::Terrain(k) => k.nested().into_iter().all(|f| placed(f, seen)),
            Feature::Vegetation(k) => k.nested().into_iter().all(|f| placed(f, seen)),
            _ => true,
        }
    }

    /// The feature type name of a configured feature (`minecraft:ore`...).
    pub fn type_name(&self, id: usize) -> &str {
        match &self.configured[id].feature {
            Feature::NoOp => "minecraft:no_op",
            Feature::SimpleBlock(_) => "minecraft:simple_block",
            Feature::Ore(_) => "minecraft:ore",
            Feature::ScatteredOre(_) => "minecraft:scattered_ore",
            Feature::RandomSelector { .. } => "minecraft:random_selector",
            Feature::SimpleRandomSelector(_) => "minecraft:simple_random_selector",
            Feature::WeightedRandomSelector(_) => "minecraft:weighted_random_selector",
            Feature::RandomBooleanSelector { .. } => "minecraft:random_boolean_selector",
            Feature::Sequence(_) => "minecraft:sequence",
            Feature::Overlay(_) => "minecraft:overlay",
            Feature::Trees(k) => k.type_name(),
            Feature::Terrain(k) => k.type_name(),
            Feature::Vegetation(k) => k.type_name(),
            Feature::Template(_) => "minecraft:template",
            Feature::Fossil(_) => "minecraft:fossil",
            Feature::Unsupported(i) => &self.gaps[*i].0,
        }
    }
}

/// The saved form of an entity generation spawns (`Entity.saveWithoutId` plus `id`), with the
/// fields worldgen decides: position, yaw (pitch 0), no motion, not invulnerable; `extra`
/// holds the type's own fields. Vanilla also writes a random `UUID` and air, fire and fall
/// distance defaults; whoever loads the entity supplies those.
pub fn entity_tag(id: &str, pos: [f64; 3], yaw: f32, extra: Vec<(String, kiln_proto::nbt::Tag)>) -> kiln_proto::nbt::Tag {
    use kiln_proto::nbt::Tag;
    let mut fields = vec![
        ("id".to_string(), Tag::String(id.to_string())),
        ("Pos".to_string(), Tag::List(pos.iter().map(|&v| Tag::Double(v)).collect())),
        ("Motion".to_string(), Tag::List(vec![Tag::Double(0.0); 3])),
        ("Rotation".to_string(), Tag::List(vec![Tag::Float(yaw), Tag::Float(0.0)])),
        ("Invulnerable".to_string(), Tag::Byte(0)),
        ("OnGround".to_string(), Tag::Byte(0)),
    ];
    fields.extend(extra);
    Tag::Compound(fields)
}
