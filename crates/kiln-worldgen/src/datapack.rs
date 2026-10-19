//! Loading a datapack directory (vanilla's data generator output, or the vanilla pack
//! extracted): `worldgen/{noise, density_function, noise_settings, material_rule,
//! material_condition, biome, carver, multi_noise_biome_source_parameter_list}` and block
//! tags, plus the `reports/biome_parameters` report that holds the parameter lists of the
//! built-in multi-noise presets.

use crate::Error;
use crate::biome::{Parameter, ParameterSpace, parse_parameter, parse_parameter_list};
use crate::blocks::{BlockSpec, parse_block_state};
use crate::function::{Graph, NodeId, field, qualify};
use crate::json::Json;
use crate::material::{CondDef, Ref, RuleDef, parse_condition, parse_rule};
use crate::noise::{Normalization, NormalNoiseParams};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A `noise_settings` entry.
#[derive(Clone, Debug)]
pub struct NoiseSettings {
    pub min_y: i32,
    pub height: i32,
    pub sea_level: i32,
    pub legacy_random_source: bool,
    /// `disable_mob_generation`: no initial animals (the End).
    pub disable_mob_generation: bool,
    /// `noise_router` fields by name, in vanilla's declaration order.
    pub router: Vec<(String, NodeId)>,
    /// `aquifers` density functions by name, when aquifers are enabled.
    pub aquifers: Vec<(String, NodeId)>,
    pub default_block: BlockSpec,
    pub default_fluid: BlockSpec,
    pub material_rule: Ref<RuleDef>,
    /// `spawn_target`: points of (density function, climate parameter) the world spawn
    /// search scores columns against (`SpawnTargetPoint`).
    pub spawn_target: Vec<Vec<(NodeId, Parameter)>>,
}

pub const ROUTER_FIELDS: [&str; 8] =
    ["temperature", "vegetation", "continents", "erosion", "depth", "ridges", "chunk_surface_level", "final_density"];

pub const AQUIFER_FIELDS: [&str; 6] =
    ["barrier", "fluid_level_floodedness", "fluid_level_spread", "lava", "exclusion", "surface_level"];

pub struct Datapack {
    pub graph: Graph,
    pub settings: HashMap<String, NoiseSettings>,
    pub rules: HashMap<String, Ref<RuleDef>>,
    pub conditions: HashMap<String, Ref<CondDef>>,
    /// `worldgen/biome` entries, sorted by id.
    pub biomes: Vec<(String, Json)>,
    pub carvers: HashMap<String, Json>,
    /// `worldgen/multi_noise_biome_source_parameter_list` entries: their parameter lists,
    /// from the biome parameters report (vanilla builds presets in code).
    pub parameter_lists: HashMap<String, Vec<(ParameterSpace, String)>>,
    /// `tags/block` entries as raw lists (`#tag` entries unexpanded).
    pub block_tags: HashMap<String, Vec<String>>,
    /// Other tag registries by directory (`fluid`, `worldgen/biome`, `worldgen/structure`...),
    /// entries as raw lists.
    pub tags: HashMap<String, HashMap<String, Vec<String>>>,
    /// `worldgen/feature` (configured features), sorted by id.
    pub features: Vec<(String, Json)>,
    /// `worldgen/placed_feature`, sorted by id.
    pub placed_features: Vec<(String, Json)>,
    /// `worldgen/block_state_provider`.
    pub state_providers: HashMap<String, Json>,
    /// Other `worldgen/*` registries loaded as raw JSON by directory (`structure`,
    /// `structure_set`, `template_pool`, `processor_list`), entries sorted by id.
    pub registries: HashMap<String, Vec<(String, Json)>>,
    /// `dimension_type` entries: (`min_y`, `height`), the level geometry chunks are built in.
    pub dimension_types: HashMap<String, (i32, i32)>,
    /// `dimension/*.json` entries (`LevelStem`s the packs define), sorted by id.
    pub dimensions: Vec<(String, Json)>,
    /// The bottom pack root (vanilla's data): structure templates are looked up near it.
    pub root: PathBuf,
    /// Every pack root loaded, the bottom one first.
    pub roots: Vec<PathBuf>,
}

impl Datapack {
    /// Loads every namespace under `root/data/*/worldgen` (the data of one pack).
    pub fn load(root: &Path) -> Result<Datapack, Error> {
        Self::load_stack(&[root.to_path_buf()])
    }

    /// Loads a stack of pack roots, vanilla's data at the bottom and the world's enabled packs
    /// above it in load order (`roots[0]` first). As in vanilla's resource manager a later pack's
    /// file replaces an earlier one's with the same id, and tags merge (`TagLoader`: entries add up
    /// in pack order, `replace` drops what the earlier packs listed). A root without `data/` is a
    /// pack with no data (the bottom root needs it).
    pub fn load_stack(roots: &[PathBuf]) -> Result<Datapack, Error> {
        let Some(bottom) = roots.first() else { return Err(Error::Invalid("no data pack".into())) };
        let data = bottom.join("data");
        if !data.is_dir() {
            return Err(Error::from(std::io::Error::from(std::io::ErrorKind::NotFound)).context(data.display().to_string()));
        }
        let mut graph = Graph::default();
        let mut settings = HashMap::new();
        let mut rules = HashMap::new();
        let mut conditions = HashMap::new();
        let mut parameter_lists = HashMap::new();
        let mut registries: HashMap<String, Vec<(String, Json)>> = HashMap::new();
        let mut dimension_types = HashMap::new();
        let mut tags: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        let entries = |sub: &str| stacked_entries(roots, sub);
        for (id, json) in entries("worldgen/noise")? {
            let params = parse_noise(&json).map_err(|e| e.context(&id))?;
            graph.add_noise(&id, params);
        }
        for (id, json) in entries("worldgen/density_function")? {
            graph.add_function(&id, &json)?;
        }
        for (id, json) in entries("worldgen/noise_settings")? {
            let s = parse_settings(&mut graph, &json).map_err(|e| e.context(&id))?;
            settings.insert(id, s);
        }
        for (id, json) in entries("worldgen/material_rule")? {
            let r = parse_rule(&mut graph, &json).map_err(|e| e.context(&id))?;
            rules.insert(id, r);
        }
        for (id, json) in entries("worldgen/material_condition")? {
            let c = parse_condition(&json).map_err(|e| e.context(&id))?;
            conditions.insert(id, c);
        }
        let biomes = entries("worldgen/biome")?;
        let carvers: HashMap<String, Json> = entries("worldgen/carver")?.into_iter().collect();
        for (id, json) in entries("worldgen/multi_noise_biome_source_parameter_list")? {
            let preset = field(&json, "preset")?.as_str().map(qualify).ok_or_else(|| Error::Invalid("bad preset".into()))?;
            let (pns, path) = preset.split_once(':').expect("qualified");
            // The presets are built in code in vanilla; the data generator's report holds their lists.
            let report = roots
                .iter()
                .rev()
                .map(|r| r.join("reports").join("biome_parameters").join(pns).join(format!("{path}.json")))
                .find(|p| p.is_file());
            if let Some(report) = report {
                let json = Json::parse(&fs::read_to_string(&report)?)?;
                parameter_lists.insert(id.clone(), parse_parameter_list(&json).map_err(|e| e.context(&id))?);
            }
        }
        let features = entries("worldgen/feature")?;
        let placed_features = entries("worldgen/placed_feature")?;
        let state_providers: HashMap<String, Json> = entries("worldgen/block_state_provider")?.into_iter().collect();
        for dir in ["structure", "structure_set", "template_pool", "processor_list"] {
            registries.insert(dir.to_string(), entries(&format!("worldgen/{dir}"))?);
        }
        for (id, json) in entries("dimension_type")? {
            let geometry = (i32_of(&json, "min_y")?, i32_of(&json, "height")?);
            dimension_types.insert(id, geometry);
        }
        let dimensions = entries("dimension")?;
        let block_tags = stacked_tags(roots, "block")?;
        for dir in ["fluid", "worldgen/biome", "worldgen/structure", "worldgen/feature"] {
            tags.insert(dir.to_string(), stacked_tags(roots, dir)?);
        }
        Ok(Datapack {
            root: bottom.clone(),
            roots: roots.to_vec(),
            graph,
            settings,
            rules,
            conditions,
            biomes,
            carvers,
            parameter_lists,
            block_tags,
            tags,
            features,
            placed_features,
            state_providers,
            registries,
            dimension_types,
            dimensions,
        })
    }

    pub fn settings(&self, id: &str) -> Result<&NoiseSettings, Error> {
        self.settings.get(&qualify(id)).ok_or_else(|| Error::UnknownSettings(id.to_string()))
    }

    /// Block names in a block tag, with nested tags expanded.
    pub fn block_tag(&self, id: &str) -> Result<Vec<String>, Error> {
        expand(&self.block_tags, "block", id)
    }

    /// Entry names in a tag of the registry directory `dir` (`fluid`, `worldgen/biome`...),
    /// with nested tags expanded.
    pub fn tag(&self, dir: &str, id: &str) -> Result<Vec<String>, Error> {
        let empty = HashMap::new();
        expand(self.tags.get(dir).unwrap_or(&empty), dir, id)
    }
}

impl Datapack {
    /// Block names of a block tag in vanilla's element order (`TagLoader`: entries in file
    /// order, nested tags expanded in place, duplicates dropped).
    pub fn block_tag_ordered(&self, id: &str) -> Result<Vec<String>, Error> {
        fn walk(tags: &HashMap<String, Vec<String>>, tag: &str, out: &mut Vec<String>, depth: usize) -> Result<(), Error> {
            let values = tags.get(tag).ok_or_else(|| Error::Invalid(format!("unknown block tag {tag}")))?;
            if depth > 32 {
                return Err(Error::Invalid(format!("block tag cycle at {tag}")));
            }
            for v in values {
                match v.strip_prefix('#') {
                    Some(t) => walk(tags, &qualify(t), out, depth + 1)?,
                    None => {
                        let v = qualify(v);
                        if !out.contains(&v) {
                            out.push(v);
                        }
                    }
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk(&self.block_tags, &qualify(id.trim_start_matches('#')), &mut out, 0)?;
        Ok(out)
    }
}

fn i32_of(json: &Json, key: &str) -> Result<i32, Error> {
    field(json, key)?.as_i32().ok_or_else(|| Error::Invalid(format!("{key} must be an integer")))
}

/// The entries of tag directory `dir` (`block`, `worldgen/biome`...) over the stack: `TagLoader` adds
/// a pack's entries to the earlier packs' and `replace` drops what came before.
fn stacked_tags(roots: &[PathBuf], dir: &str) -> Result<HashMap<String, Vec<String>>, Error> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for root in roots {
        for ns_dir in namespaces(root) {
            let ns = ns_dir.file_name().unwrap().to_string_lossy().into_owned();
            for (id, json) in entries(&ns_dir.join("tags").join(dir), &ns)? {
                let values = tag_values(&json)?;
                let replace = matches!(json.get("replace"), Some(Json::Bool(true)));
                let list = out.entry(id).or_default();
                if replace {
                    list.clear();
                }
                list.extend(values);
            }
        }
    }
    Ok(out)
}

/// `data/*` of one pack root, sorted.
fn namespaces(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root.join("data"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// The entries `data/<namespace>/<sub>/**.json` over the stack of pack roots, a later root's entry
/// replacing an earlier one's with the same id; sorted by id.
fn stacked_entries(roots: &[PathBuf], sub: &str) -> Result<Vec<(String, Json)>, Error> {
    let mut merged: std::collections::BTreeMap<String, Json> = std::collections::BTreeMap::new();
    for root in roots {
        for ns_dir in namespaces(root) {
            let ns = ns_dir.file_name().unwrap().to_string_lossy().into_owned();
            merged.extend(entries(&ns_dir.join(sub), &ns)?);
        }
    }
    Ok(merged.into_iter().collect())
}

/// A tag's `values`: plain ids or `{"id", "required"}` entries.
fn tag_values(json: &Json) -> Result<Vec<String>, Error> {
    Ok(field(json, "values")?
        .as_array()
        .ok_or_else(|| Error::Invalid("tag values must be a list".into()))?
        .iter()
        .filter_map(|v| v.as_str().or_else(|| v.get("id").and_then(Json::as_str)).map(str::to_string))
        .collect())
}

/// Entries of tag `id`, nested tags expanded.
fn expand(tags: &HashMap<String, Vec<String>>, dir: &str, id: &str) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    let mut stack = vec![qualify(id.trim_start_matches('#'))];
    let mut seen = Vec::new();
    while let Some(tag) = stack.pop() {
        if seen.contains(&tag) {
            continue;
        }
        let values = tags.get(&tag).ok_or_else(|| Error::Invalid(format!("unknown {dir} tag {tag}")))?;
        for v in values {
            match v.strip_prefix('#') {
                Some(t) => stack.push(qualify(t)),
                None => out.push(qualify(v)),
            }
        }
        seen.push(tag);
    }
    Ok(out)
}

/// `(namespace:path, json)` for every `.json` below `dir`, sorted by id.
fn entries(dir: &Path, ns: &str) -> Result<Vec<(String, Json)>, Error> {
    let mut files = Vec::new();
    if dir.is_dir() {
        collect(dir, &mut files)?;
    }
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let rel = path.strip_prefix(dir).unwrap().with_extension("");
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            let text = fs::read_to_string(&path)?;
            let json = Json::parse(&text).map_err(|e| Error::from(e).context(path.display().to_string()))?;
            Ok((format!("{ns}:{rel}"), json))
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// `NormalNoise.Parameters.CODEC`.
pub(crate) fn parse_noise(json: &Json) -> Result<NormalNoiseParams, Error> {
    let base_amplitude = match json.get("base_amplitude") {
        Some(v) => v.as_f64().ok_or_else(|| Error::Invalid("base_amplitude must be a number".into()))?,
        None => 1.0,
    };
    let base_octave = field(json, "base_octave")?.as_i32().ok_or_else(|| Error::Invalid("bad base_octave".into()))?;
    let octave_count = match json.get("octave_count") {
        Some(v) => v.as_i32().ok_or_else(|| Error::Invalid("bad octave_count".into()))?,
        None => 1,
    };
    let normalize = match json.get("normalize") {
        None | Some(Json::Bool(true)) => Normalization::Enabled,
        Some(Json::Bool(false)) => Normalization::Disabled,
        Some(Json::String(s)) if s == "legacy" => Normalization::Legacy,
        Some(v) => return Err(Error::Invalid(format!("bad normalize {v:?}"))),
    };
    let amplitude_modifiers = match json.get("amplitude_modifiers") {
        Some(v) => v
            .as_array()
            .ok_or_else(|| Error::Invalid("amplitude_modifiers must be a list".into()))?
            .iter()
            .map(|a| a.as_f64().ok_or_else(|| Error::Invalid("bad amplitude modifier".into())))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    if !(1e-5..=1e6).contains(&base_amplitude) || !(-32..=32).contains(&base_octave) || !(1..=32).contains(&octave_count) {
        return Err(Error::Invalid("noise parameters out of range".into()));
    }
    if !amplitude_modifiers.is_empty() && amplitude_modifiers.len() != octave_count as usize {
        return Err(Error::Invalid(format!(
            "amplitude_modifiers had size {}, but octave_count was {octave_count}",
            amplitude_modifiers.len()
        )));
    }
    Ok(NormalNoiseParams { base_amplitude, base_octave, octave_count, normalize, amplitude_modifiers })
}

pub(crate) fn parse_settings(graph: &mut Graph, json: &Json) -> Result<NoiseSettings, Error> {
    let noise = field(json, "noise")?;
    let int = |j: &Json, k: &str| -> Result<i32, Error> {
        field(j, k)?.as_i32().ok_or_else(|| Error::Invalid(format!("{k} must be an integer")))
    };
    let router_json = field(json, "noise_router")?;
    let mut router = Vec::new();
    for name in ROUTER_FIELDS {
        let node = graph.parse(field(router_json, name)?).map_err(|e| e.context(name))?;
        router.push((name.to_string(), node));
    }
    let mut aquifers = Vec::new();
    if let Some(a) = json.get("aquifers") {
        for name in AQUIFER_FIELDS {
            let node = graph.parse(field(a, name)?).map_err(|e| e.context(name))?;
            aquifers.push((name.to_string(), node));
        }
    }
    let mut spawn_target = Vec::new();
    for point in json.get("spawn_target").and_then(Json::as_array).unwrap_or(&[]) {
        let Json::Object(entries) = point else { return Err(Error::Invalid("bad spawn_target point".into())) };
        let mut p = Vec::new();
        for (id, param) in entries {
            p.push((graph.parse(&Json::String(id.clone())).map_err(|e| e.context(id))?, parse_parameter(param)?));
        }
        spawn_target.push(p);
    }
    Ok(NoiseSettings {
        spawn_target,
        min_y: int(noise, "min_y")?,
        height: int(noise, "height")?,
        sea_level: int(json, "sea_level")?,
        legacy_random_source: field(json, "legacy_random_source")?.as_bool().unwrap_or(false),
        disable_mob_generation: field(json, "disable_mob_generation").ok().and_then(|v| v.as_bool()).unwrap_or(false),
        router,
        aquifers,
        default_block: parse_block_state(field(json, "default_block")?)?,
        default_fluid: parse_block_state(field(json, "default_fluid")?)?,
        material_rule: parse_rule(graph, field(json, "material_rule")?)?,
    })
}
