//! Loading a datapack directory (vanilla's data generator output, or the vanilla pack
//! extracted): `worldgen/{noise, density_function, noise_settings, material_rule,
//! material_condition, biome, carver, multi_noise_biome_source_parameter_list}` and block
//! tags, plus the `reports/biome_parameters` report that holds the parameter lists of the
//! built-in multi-noise presets.

use crate::Error;
use crate::biome::{ParameterSpace, parse_parameter_list};
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
    /// `noise_router` fields by name, in vanilla's declaration order.
    pub router: Vec<(String, NodeId)>,
    /// `aquifers` density functions by name, when aquifers are enabled.
    pub aquifers: Vec<(String, NodeId)>,
    pub default_block: BlockSpec,
    pub default_fluid: BlockSpec,
    pub material_rule: Ref<RuleDef>,
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
}

impl Datapack {
    /// Loads every namespace under `root/data/*/worldgen`.
    pub fn load(root: &Path) -> Result<Datapack, Error> {
        let mut graph = Graph::default();
        let mut settings = HashMap::new();
        let mut rules = HashMap::new();
        let mut conditions = HashMap::new();
        let mut biomes = Vec::new();
        let mut carvers = HashMap::new();
        let mut parameter_lists = HashMap::new();
        let mut block_tags = HashMap::new();
        let data = root.join("data");
        let mut namespaces: Vec<_> = fs::read_dir(&data)
            .map_err(|e| Error::from(e).context(data.display().to_string()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("worldgen").is_dir())
            .collect();
        namespaces.sort();
        for ns_dir in &namespaces {
            let ns = ns_dir.file_name().unwrap().to_string_lossy().into_owned();
            let worldgen = ns_dir.join("worldgen");
            for (id, json) in entries(&worldgen.join("noise"), &ns)? {
                let params = parse_noise(&json).map_err(|e| e.context(&id))?;
                graph.add_noise(&id, params);
            }
            for (id, json) in entries(&worldgen.join("density_function"), &ns)? {
                graph.add_function(&id, &json)?;
            }
        }
        for ns_dir in &namespaces {
            let ns = ns_dir.file_name().unwrap().to_string_lossy().into_owned();
            let worldgen = ns_dir.join("worldgen");
            for (id, json) in entries(&worldgen.join("noise_settings"), &ns)? {
                let s = parse_settings(&mut graph, &json).map_err(|e| e.context(&id))?;
                settings.insert(id, s);
            }
            for (id, json) in entries(&worldgen.join("material_rule"), &ns)? {
                let r = parse_rule(&mut graph, &json).map_err(|e| e.context(&id))?;
                rules.insert(id, r);
            }
            for (id, json) in entries(&worldgen.join("material_condition"), &ns)? {
                let c = parse_condition(&mut graph, &json).map_err(|e| e.context(&id))?;
                conditions.insert(id, c);
            }
            biomes.extend(entries(&worldgen.join("biome"), &ns)?);
            carvers.extend(entries(&worldgen.join("carver"), &ns)?);
            for (id, json) in entries(&worldgen.join("multi_noise_biome_source_parameter_list"), &ns)? {
                let preset = field(&json, "preset")?.as_str().map(qualify).ok_or_else(|| Error::Invalid("bad preset".into()))?;
                let (pns, path) = preset.split_once(':').expect("qualified");
                let report = root.join("reports").join("biome_parameters").join(pns).join(format!("{path}.json"));
                if report.is_file() {
                    let json = Json::parse(&fs::read_to_string(&report)?)?;
                    parameter_lists.insert(id.clone(), parse_parameter_list(&json).map_err(|e| e.context(&id))?);
                }
            }
            for (id, json) in entries(&ns_dir.join("tags").join("block"), &ns)? {
                let values = field(&json, "values")?
                    .as_array()
                    .ok_or_else(|| Error::Invalid("tag values must be a list".into()))?
                    .iter()
                    .filter_map(|v| v.as_str().or_else(|| v.get("id").and_then(Json::as_str)).map(str::to_string))
                    .collect();
                block_tags.insert(id, values);
            }
        }
        biomes.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Datapack { graph, settings, rules, conditions, biomes, carvers, parameter_lists, block_tags })
    }

    pub fn settings(&self, id: &str) -> Result<&NoiseSettings, Error> {
        self.settings.get(&qualify(id)).ok_or_else(|| Error::UnknownSettings(id.to_string()))
    }

    /// Block names in a block tag, with nested tags expanded.
    pub fn block_tag(&self, id: &str) -> Result<Vec<String>, Error> {
        let mut out = Vec::new();
        let mut stack = vec![qualify(id.trim_start_matches('#'))];
        let mut seen = Vec::new();
        while let Some(tag) = stack.pop() {
            if seen.contains(&tag) {
                continue;
            }
            let values = self.block_tags.get(&tag).ok_or_else(|| Error::Invalid(format!("unknown block tag {tag}")))?;
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
fn parse_noise(json: &Json) -> Result<NormalNoiseParams, Error> {
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

fn parse_settings(graph: &mut Graph, json: &Json) -> Result<NoiseSettings, Error> {
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
    Ok(NoiseSettings {
        min_y: int(noise, "min_y")?,
        height: int(noise, "height")?,
        sea_level: int(json, "sea_level")?,
        legacy_random_source: field(json, "legacy_random_source")?.as_bool().unwrap_or(false),
        router,
        aquifers,
        default_block: parse_block_state(field(json, "default_block")?)?,
        default_fluid: parse_block_state(field(json, "default_fluid")?)?,
        material_rule: parse_rule(graph, field(json, "material_rule")?)?,
    })
}
