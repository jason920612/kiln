//! Structure placement (`StructurePlacement`): which chunks may start a structure of a set.

use super::{StructureDef, StructureSet, Structures};
use crate::Error;
use crate::biome::LastResult;
use crate::generator::Generator;
use crate::json::Json;
use crate::random::WorldgenRandom;
use crate::sampler::Scratch;
use crate::sets::{BiomeSet, Loader};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::collections::HashMap;
use std::sync::OnceLock;

/// `FrequencyReductionMethod`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrequencyReduction {
    Default,
    /// Pillager outposts.
    LegacyType1,
    LegacyType2,
    LegacyType3,
}

impl FrequencyReduction {
    /// `FrequencyReductionMethod.shouldGenerate(seed, salt, chunkX, chunkZ, probability)`.
    fn should_generate(self, seed: i64, salt: i32, x: i32, z: i32, p: f32) -> bool {
        match self {
            FrequencyReduction::Default => {
                // Vanilla passes (salt, x, z) as setLargeFeatureWithSalt's (x, z, salt).
                let mut r = WorldgenRandom::legacy(0);
                r.set_large_feature_with_salt(seed, salt, x, z);
                r.next_float() < p
            }
            FrequencyReduction::LegacyType1 => {
                let (a, b) = (x >> 4, z >> 4);
                let mut r = WorldgenRandom::legacy(0);
                r.set_seed((a ^ (b << 4)) as i64 ^ seed);
                r.next_int();
                r.next_int_bounded((1.0 / p) as i32) == 0
            }
            FrequencyReduction::LegacyType2 => {
                let mut r = WorldgenRandom::legacy(0);
                r.set_large_feature_with_salt(seed, x, z, 10_387_320);
                r.next_float() < p
            }
            FrequencyReduction::LegacyType3 => {
                let mut r = WorldgenRandom::legacy(0);
                r.set_large_feature_seed(seed, x, z);
                r.next_double() < p as f64
            }
        }
    }
}

/// `AbstractSpreadingStructurePlacement`'s shared settings.
#[derive(Clone, Debug)]
pub struct Spreading {
    pub locate_offset: (i32, i32, i32),
    pub reduction: FrequencyReduction,
    pub frequency: f32,
    pub salt: i32,
    /// `ExclusionZone`: (other set, chunk count).
    pub exclusion: Option<(usize, i32)>,
}

#[derive(Clone, Debug)]
pub enum Placement {
    RandomSpread { spacing: i32, separation: i32, triangular: bool, common: Spreading },
    ConcentricRings { distance: i32, spread: i32, count: i32, preferred: BiomeSet, common: Spreading },
}

impl Placement {
    pub fn parse(json: &Json, l: &Loader, set_ids: &HashMap<&str, usize>) -> Result<Placement, Error> {
        let int = |k: &str| json.get(k).and_then(Json::as_i32).ok_or_else(|| Error::Invalid(format!("placement without {k}")));
        let offset = match json.get("locate_offset").and_then(Json::as_array) {
            Some([x, y, z]) => (x.as_i32().unwrap_or(0), y.as_i32().unwrap_or(0), z.as_i32().unwrap_or(0)),
            _ => (0, 0, 0),
        };
        let reduction = match json.get("frequency_reduction_method").and_then(Json::as_str).unwrap_or("default") {
            "default" => FrequencyReduction::Default,
            "legacy_type_1" => FrequencyReduction::LegacyType1,
            "legacy_type_2" => FrequencyReduction::LegacyType2,
            "legacy_type_3" => FrequencyReduction::LegacyType3,
            m => return Err(Error::Invalid(format!("unknown frequency reduction method {m}"))),
        };
        let exclusion = match json.get("exclusion_zone") {
            Some(z) => {
                let other = z.get("other_set").and_then(Json::as_str).unwrap_or("");
                let set = *set_ids.get(crate::function::qualify(other).as_str()).ok_or_else(|| Error::Invalid(format!("unknown set {other}")))?;
                Some((set, z.get("chunk_count").and_then(Json::as_i32).unwrap_or(1)))
            }
            None => None,
        };
        let common = Spreading {
            locate_offset: offset,
            reduction,
            frequency: json.get("frequency").and_then(Json::as_f32).unwrap_or(1.0),
            salt: int("salt")?,
            exclusion,
        };
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "random_spread" => Placement::RandomSpread {
                spacing: int("spacing")?,
                separation: int("separation")?,
                triangular: json.get("spread_type").and_then(Json::as_str) == Some("triangular"),
                common,
            },
            "concentric_rings" => Placement::ConcentricRings {
                distance: int("distance")?,
                spread: int("spread")?,
                count: int("count")?,
                preferred: l.biomes(json.get("preferred_biomes").ok_or_else(|| Error::Invalid("rings without preferred_biomes".into()))?)?,
                common,
            },
            t => return Err(Error::Invalid(format!("unsupported structure placement {t}"))),
        })
    }

    pub fn common(&self) -> &Spreading {
        match self {
            Placement::RandomSpread { common, .. } | Placement::ConcentricRings { common, .. } => common,
        }
    }

    /// `RandomSpreadStructurePlacement.getPotentialStructureChunk`.
    pub fn potential_chunk(&self, seed: i64, x: i32, z: i32) -> Option<(i32, i32)> {
        let Placement::RandomSpread { spacing, separation, triangular, common } = self else { return None };
        let (i, j) = (x.div_euclid(*spacing), z.div_euclid(*spacing));
        let mut r = WorldgenRandom::legacy(0);
        r.set_large_feature_with_salt(seed, i, j, common.salt);
        let k = spacing - separation;
        let eval = |r: &mut WorldgenRandom| {
            if *triangular {
                (r.next_int_bounded(k) + r.next_int_bounded(k)) / 2
            } else {
                r.next_int_bounded(k)
            }
        };
        let a = eval(&mut r);
        let b = eval(&mut r);
        Some((i * spacing + a, j * spacing + b))
    }

    /// `AbstractSpreadingStructurePlacement.isStructureChunk`.
    pub fn is_structure_chunk(&self, structures: &Structures, generator: &Generator, set: usize, x: i32, z: i32) -> bool {
        let seed = structures.seed;
        let placement = match self {
            Placement::RandomSpread { .. } => self.potential_chunk(seed, x, z) == Some((x, z)),
            Placement::ConcentricRings { .. } => {
                structures.ring_positions(generator, set).is_some_and(|p| p.contains(&(x, z)))
            }
        };
        if !placement {
            return false;
        }
        let c = self.common();
        if c.frequency < 1.0 && !c.reduction.should_generate(seed, c.salt, x, z, c.frequency) {
            return false;
        }
        if let Some((other, count)) = c.exclusion {
            for dx in x - count..=x + count {
                for dz in z - count..=z + count {
                    if structures.sets[other].placement.is_structure_chunk(structures, generator, other, dx, dz) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Ring positions of concentric-rings sets, computed on first use
/// (`ChunkGeneratorStructureState.generateRingPositions`).
pub struct Rings {
    /// Per set: whether it gets ring positions (a structure of it can generate), and the
    /// positions once computed.
    slots: Vec<Option<OnceLock<Vec<(i32, i32)>>>>,
}

impl Rings {
    pub fn new(sets: &[StructureSet], structures: &[StructureDef], possible_biomes: &[bool]) -> Rings {
        let slots = sets
            .iter()
            .map(|s| {
                let concentric = matches!(s.placement, Placement::ConcentricRings { .. });
                let can_generate = s.structures.iter().any(|&(st, _)| {
                    (0..possible_biomes.len()).any(|b| possible_biomes[b] && structures[st].biomes.contains(b as u16))
                });
                (concentric && can_generate).then(OnceLock::new)
            })
            .collect();
        Rings { slots }
    }

    pub fn positions(&self, structures: &Structures, generator: &Generator, set: usize) -> Option<&[(i32, i32)]> {
        let slot = self.slots.get(set)?.as_ref()?;
        Some(slot.get_or_init(|| compute_rings(structures, generator, set)))
    }
}

fn compute_rings(structures: &Structures, generator: &Generator, set: usize) -> Vec<(i32, i32)> {
    let Placement::ConcentricRings { distance, spread, count, preferred, .. } = &structures.sets[set].placement else {
        return Vec::new();
    };
    let (distance, count) = (*distance, *count);
    let mut spread = *spread;
    if count == 0 {
        return Vec::new();
    }
    let mut random = LegacyRandom::new(structures.seed);
    let mut angle = random.next_double() * std::f64::consts::PI * 2.0;
    let (mut in_circle, mut circle) = (0, 0);
    let mut starts = Vec::with_capacity(count as usize);
    for i in 0..count {
        let dist = (4 * distance + distance * circle * 6) as f64 + (random.next_double() - 0.5) * (distance as f64 * 2.5);
        let x = (angle.cos() * dist).round_ties_even_java();
        let z = (angle.sin() * dist).round_ties_even_java();
        let forked = LegacyRandom::new(random.next_long());
        starts.push((x, z, forked));
        angle += std::f64::consts::PI * 2.0 / spread as f64;
        in_circle += 1;
        if in_circle == spread {
            circle += 1;
            in_circle = 0;
            spread += 2 * spread / (circle + 1);
            spread = spread.min(count - i);
            angle += random.next_double() * std::f64::consts::PI * 2.0;
        }
    }
    starts
        .into_iter()
        .map(|(x, z, mut r)| {
            match find_biome_horizontal(generator, (x << 4) + 8, 0, (z << 4) + 8, 112, preferred, &mut r) {
                Some((bx, bz)) => (bx >> 4, bz >> 4),
                None => (x, z),
            }
        })
        .collect()
}

/// `Math.round(double)` as an int.
trait JavaRound {
    fn round_ties_even_java(self) -> i32;
}

impl JavaRound for f64 {
    fn round_ties_even_java(self) -> i32 {
        if self.is_nan() { 0 } else { (self + 0.5).floor() as i64 as i32 }
    }
}

/// `BiomeSource.findBiomeHorizontal(x, y, z, radius, 1, predicate, random, false, state)`:
/// a random matching quart on the ring of `radius` blocks (the full square, as vanilla scans
/// it), chosen by reservoir sampling; the block position of its corner.
pub fn find_biome_horizontal(
    generator: &Generator,
    x: i32,
    y: i32,
    z: i32,
    radius: i32,
    wanted: &BiomeSet,
    random: &mut LegacyRandom,
) -> Option<(i32, i32)> {
    let (qx, qz, r, qy) = (x >> 2, z >> 2, radius >> 2, y >> 2);
    let mut s = Scratch::caching();
    let mut last: LastResult = None;
    let mut found = None;
    let mut n = 0;
    for dz in -r..=r {
        for dx in -r..=r {
            let b = generator.point_biome(&mut s, &mut last, qx + dx, qy, qz + dz);
            if wanted.contains(b) {
                if found.is_none() || random.next_int_bounded(n + 1) == 0 {
                    found = Some(((qx + dx) << 2, (qz + dz) << 2));
                }
                n += 1;
            }
        }
    }
    found
}
