//! Level stems (`LevelStem`): the dimension type and the chunk generator a dimension is made
//! with, as `dimension/*.json` of a data pack and the saved `world_gen_settings` write them.
//!
//! `minecraft:noise` generators (a noise settings id or inline object, and a biome source:
//! `multi_noise` with a preset or the parameter list written out, `the_end`, `fixed`,
//! `checkerboard`) run the noise pipeline; `minecraft:flat` generators lay their layers
//! ([`FlatChunks`]).

use crate::Error;
use crate::biome::{ParameterSpace, parse_parameter_list};
use crate::blocks::block;
use crate::blocks::state::AIR;
use crate::datapack::Datapack;
use crate::function::{field, qualify};
use crate::generator::BiomeSourceKind;
use crate::json::Json;
use kiln_world::chunk::Chunk;
use kiln_world::section::Section;
use kiln_world::{ChunkGenerator, ChunkPos, Dimension};
use std::sync::Arc;

/// A parsed `LevelStem`.
#[derive(Clone, Debug)]
pub struct Stem {
    /// The `dimension_type` entry the dimension uses.
    pub dimension_type: String,
    pub generator: StemGenerator,
}

#[derive(Clone, Debug)]
pub enum StemGenerator {
    /// `NoiseBasedChunkGenerator`: the id of its noise settings (inline settings are registered
    /// in the [`Datapack`] under a made-up id) and its biome source.
    Noise { settings: String, source: SourceSpec },
    /// `FlatLevelSource`.
    Flat(FlatSettings),
    /// `DebugLevelSource`: not generated (the dimension is void).
    Debug,
}

/// A noise generator's biome source.
#[derive(Clone, Debug)]
pub enum SourceSpec {
    /// `MultiNoiseBiomeSource` with a `multi_noise_biome_source_parameter_list` entry.
    Preset(String),
    /// `MultiNoiseBiomeSource` with the list in the stem.
    Inline(Vec<(ParameterSpace, String)>),
    TheEnd,
    Fixed(String),
    /// `CheckerboardColumnBiomeSource`: its biomes and `scale`.
    Checkerboard(Vec<String>, i32),
}

impl SourceSpec {
    pub fn kind(&self) -> BiomeSourceKind<'_> {
        match self {
            SourceSpec::Preset(id) => BiomeSourceKind::MultiNoise(id),
            SourceSpec::Inline(list) => BiomeSourceKind::Inline(list),
            SourceSpec::TheEnd => BiomeSourceKind::TheEnd,
            SourceSpec::Fixed(b) => BiomeSourceKind::Fixed(b),
            SourceSpec::Checkerboard(biomes, scale) => BiomeSourceKind::Checkerboard(biomes, *scale),
        }
    }
}

/// `FlatLevelGeneratorSettings`: the layers from the bottom of the level up (a state id each, air
/// where nothing is placed) and the single biome.
#[derive(Clone, Debug)]
pub struct FlatSettings {
    pub biome: String,
    pub layers: Vec<u16>,
    /// Decoration of the biome (`features`) and lakes: not generated.
    pub features: bool,
    pub lakes: bool,
}

impl Datapack {
    /// Parses the `LevelStem` `json` of dimension `key`. Noise settings written inline are added to
    /// the pack under the id `inline:<key>`.
    pub fn parse_stem(&mut self, key: &str, json: &Json) -> Result<Stem, Error> {
        let dimension_type = qualify(field(json, "type")?.as_str().ok_or_else(|| Error::Invalid("the dimension type must be an id".into()))?);
        let generator = field(json, "generator")?;
        let kind = qualify(field(generator, "type")?.as_str().ok_or_else(|| Error::Invalid("bad generator type".into()))?);
        let generator = match kind.as_str() {
            "minecraft:noise" => {
                let settings = match field(generator, "settings")? {
                    Json::String(id) => qualify(id),
                    inline @ Json::Object(_) => {
                        let id = format!("inline:{}", key.replace(':', "/"));
                        let parsed = crate::datapack::parse_settings(&mut self.graph, inline).map_err(|e| e.context("settings"))?;
                        self.settings.insert(id.clone(), parsed);
                        id
                    }
                    other => return Err(Error::Invalid(format!("bad noise settings {other:?}"))),
                };
                let source = self.parse_source(field(generator, "biome_source")?)?;
                StemGenerator::Noise { settings, source }
            }
            "minecraft:flat" => StemGenerator::Flat(parse_flat(field(generator, "settings")?)?),
            "minecraft:debug" => StemGenerator::Debug,
            other => return Err(Error::Invalid(format!("unknown chunk generator {other}"))),
        };
        Ok(Stem { dimension_type, generator })
    }

    fn parse_source(&self, json: &Json) -> Result<SourceSpec, Error> {
        let kind = qualify(field(json, "type")?.as_str().ok_or_else(|| Error::Invalid("bad biome source type".into()))?);
        match kind.as_str() {
            "minecraft:multi_noise" => match json.get("preset") {
                Some(id) => Ok(SourceSpec::Preset(qualify(id.as_str().ok_or_else(|| Error::Invalid("bad preset".into()))?))),
                None => Ok(SourceSpec::Inline(parse_parameter_list(json)?)),
            },
            "minecraft:the_end" => Ok(SourceSpec::TheEnd),
            "minecraft:fixed" => {
                let biome = field(json, "biome")?.as_str().ok_or_else(|| Error::Invalid("bad fixed biome".into()))?;
                Ok(SourceSpec::Fixed(qualify(biome)))
            }
            "minecraft:checkerboard" => {
                let biomes = match field(json, "biomes")? {
                    Json::String(s) if s.starts_with('#') => self.tag("worldgen/biome", s)?,
                    Json::String(s) => vec![qualify(s)],
                    Json::Array(items) => items.iter().filter_map(|i| i.as_str().map(qualify)).collect(),
                    other => return Err(Error::Invalid(format!("bad checkerboard biomes {other:?}"))),
                };
                let scale = match json.get("scale") {
                    Some(v) => v.as_i32().filter(|s| (0..=62).contains(s)).ok_or_else(|| Error::Invalid("bad checkerboard scale".into()))?,
                    None => 2,
                };
                Ok(SourceSpec::Checkerboard(biomes, scale))
            }
            other => Err(Error::Invalid(format!("unknown biome source {other}"))),
        }
    }
}

/// A stem read without the data pack: superflat generators parse, a noise generator has nothing to run on
/// (it is left to the default terrain).
pub fn parse_stem_without_pack(json: &Json) -> Result<Stem, Error> {
    let dimension_type = qualify(field(json, "type")?.as_str().ok_or_else(|| Error::Invalid("the dimension type must be an id".into()))?);
    let generator = field(json, "generator")?;
    let kind = qualify(field(generator, "type")?.as_str().ok_or_else(|| Error::Invalid("bad generator type".into()))?);
    let generator = match kind.as_str() {
        "minecraft:flat" => StemGenerator::Flat(parse_flat(field(generator, "settings")?)?),
        _ => StemGenerator::Debug,
    };
    Ok(Stem { dimension_type, generator })
}

/// `FlatLevelGeneratorSettings.CODEC`.
fn parse_flat(json: &Json) -> Result<FlatSettings, Error> {
    let mut layers = Vec::new();
    for layer in field(json, "layers")?.as_array().ok_or_else(|| Error::Invalid("flat layers must be a list".into()))? {
        let height = field(layer, "height")?.as_i32().filter(|h| (0..=384).contains(h)).ok_or_else(|| Error::Invalid("bad layer height".into()))?;
        let name = field(layer, "block")?.as_str().ok_or_else(|| Error::Invalid("bad layer block".into()))?;
        let default = block(name)?.default;
        layers.extend(std::iter::repeat_n(default, height as usize));
    }
    if layers.len() > 384 {
        return Err(Error::Invalid(format!("flat layers are {} high, at most 384", layers.len())));
    }
    // (Saved stems come from NBT, where a boolean is a byte.)
    let flag = |k: &str| json.get(k).is_some_and(|v| matches!(v, Json::Bool(true)) || matches!(v, Json::Number(n) if n == "1"));
    let biome = json.get("biome").and_then(Json::as_str).map_or_else(|| "minecraft:plains".to_owned(), qualify);
    Ok(FlatSettings { biome, layers, features: flag("features"), lakes: flag("lakes") })
}

/// A `ChunkGenerator` for `FlatLevelSource`: the layers from the level's bottom up, one biome.
#[derive(Clone)]
pub struct FlatChunks {
    layers: Arc<Vec<u16>>,
    biome: u16,
}

impl FlatChunks {
    /// `biome` is the network id of the flat biome.
    pub fn new(settings: &FlatSettings, biome: u16) -> Self {
        FlatChunks { layers: Arc::new(settings.layers.clone()), biome }
    }

    /// `FlatLevelSource.getSpawnHeight`: the level's bottom plus the layers (at most its height).
    pub fn spawn_height(&self, dimension: Dimension) -> i32 {
        dimension.min_y + dimension.height.min(self.layers.len() as i32)
    }
}

impl ChunkGenerator for FlatChunks {
    fn generate(&mut self, _pos: ChunkPos, dimension: Dimension) -> Chunk {
        let sections = (dimension.height / 16) as usize;
        let mut out = vec![Section::filled(AIR, self.biome); sections];
        for (y, &layer) in self.layers.iter().enumerate().take(dimension.height as usize) {
            if layer == AIR {
                continue;
            }
            for x in 0..16 {
                for z in 0..16 {
                    out[y >> 4].set(x, y & 15, z, layer);
                }
            }
        }
        Chunk::new(out, dimension.min_y)
    }

    fn fork(&self) -> Box<dyn ChunkGenerator> {
        Box::new(self.clone())
    }
}
