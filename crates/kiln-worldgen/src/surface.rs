//! The surface step (`MaterialSystem.buildSurface`): material rules evaluated per block, top
//! down per column, plus the eroded badlands and frozen ocean extensions and the badlands
//! clay bands.

use crate::Error;
use crate::aquifer::{WAY_BELOW_MIN_Y, map};
use crate::blocks::{has_fluid, is_air, is_block, state};
use crate::datapack::Datapack;
use crate::function::NodeId;
use crate::generator::ProtoChunk;
use crate::material::{Anchor, CondDef, Ref, RuleDef};
use crate::noise::NoiseStack;
use crate::sampler::{SamplerRef, Scratch};
use crate::simplex::{cold_enough_to_snow, melts_icebergs_slightly};
use crate::state::RandomState;
use crate::volume::Volume;
use kiln_javamath::math as jm;
use kiln_javamath::random::{PositionalRandomFactory, RandomSource};
use std::collections::HashMap;
use std::sync::Arc;

/// A compiled material rule (`RuleEvaluator`).
pub enum Rule {
    Block(u16),
    Bandlands,
    Sequence(Vec<Rule>),
    Condition(Box<Cond>, Box<Rule>),
    /// Index into the ore veins, whose densities are sampled when the rule is compiled.
    OreVein(usize),
}

/// A compiled material condition (`ConditionEvaluator`).
pub enum Cond {
    Biome(Vec<u16>),
    Noise { slot: usize, min: f64, max: f64 },
    VerticalGradient { true_at_and_below: i32, false_at_and_above: i32, random: PositionalRandomFactory },
    YAbove { y: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    Water { offset: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    Temperature,
    Steep,
    Not(Box<Cond>),
    Hole,
    AbovePreliminarySurface,
    StoneDepth { offset: i32, add_surface_depth: bool, secondary_depth_range: i32, ceiling: bool },
}

pub struct OreVein {
    ore: u16,
    raw_ore: u16,
    filler: u16,
    raw_ore_chance: f32,
    density: SamplerRef,
    richness: SamplerRef,
    gap: SamplerRef,
    random: PositionalRandomFactory,
}

/// What the material rules need of the biome table.
pub struct BiomeClimate {
    pub temperature: f32,
    pub frozen: bool,
}

/// The per-seed surface system.
pub struct MaterialSystem {
    default_block: u16,
    sea_level: i32,
    rule: Rule,
    ore_veins: Vec<OreVein>,
    /// `(noise, is_3d)` per noise-threshold slot.
    noises: Vec<(Arc<NoiseStack>, bool)>,
    preliminary_surface: SamplerRef,
    clay_bands: [u16; 192],
    clay_bands_offset: Arc<NoiseStack>,
    badlands_pillar: Arc<NoiseStack>,
    badlands_pillar_roof: Arc<NoiseStack>,
    badlands_surface: Arc<NoiseStack>,
    iceberg_pillar: Arc<NoiseStack>,
    iceberg_pillar_roof: Arc<NoiseStack>,
    iceberg_surface: Arc<NoiseStack>,
    surface: Arc<NoiseStack>,
    surface_secondary: Arc<NoiseStack>,
    noise_random: PositionalRandomFactory,
    climate: Vec<BiomeClimate>,
    eroded_badlands: Option<u16>,
    frozen_oceans: Vec<u16>,
}

/// The density functions the material rules sample, collected before compilation so they
/// share the generator's compiler.
pub fn rule_densities(pack: &Datapack, rule: &Ref<RuleDef>, out: &mut Vec<NodeId>) -> Result<(), Error> {
    walk_rule(pack, rule, 0, &mut |def| {
        if let RuleDef::OreVein { density, richness, filler_gap, .. } = def {
            out.extend([*density, *richness, *filler_gap]);
        }
    })
}

fn walk_rule(pack: &Datapack, rule: &Ref<RuleDef>, depth: u32, f: &mut dyn FnMut(&RuleDef)) -> Result<(), Error> {
    if depth > 256 {
        return Err(Error::Invalid("material rule reference cycle".into()));
    }
    let def = resolve_rule(pack, rule)?;
    f(def);
    match def {
        RuleDef::Sequence(list) => {
            for r in list {
                walk_rule(pack, r, depth + 1, f)?;
            }
        }
        RuleDef::Condition { then_run, .. } => walk_rule(pack, then_run, depth + 1, f)?,
        _ => {}
    }
    Ok(())
}

fn resolve_rule<'a>(pack: &'a Datapack, mut rule: &'a Ref<RuleDef>) -> Result<&'a RuleDef, Error> {
    for _ in 0..64 {
        match rule {
            Ref::Inline(def) => return Ok(def),
            Ref::Named(name) => {
                rule = pack.rules.get(name).ok_or_else(|| Error::Invalid(format!("unknown material rule {name}")))?;
            }
        }
    }
    Err(Error::Invalid("material rule reference cycle".into()))
}

fn resolve_condition<'a>(pack: &'a Datapack, mut cond: &'a Ref<CondDef>) -> Result<&'a CondDef, Error> {
    for _ in 0..64 {
        match cond {
            Ref::Inline(def) => return Ok(def),
            Ref::Named(name) => {
                cond = pack.conditions.get(name).ok_or_else(|| Error::Invalid(format!("unknown material condition {name}")))?;
            }
        }
    }
    Err(Error::Invalid("material condition reference cycle".into()))
}

/// Inputs for [`MaterialSystem::new`].
pub struct MaterialInputs<'a> {
    pub pack: &'a Datapack,
    pub rule: &'a Ref<RuleDef>,
    pub default_block: u16,
    pub sea_level: i32,
    pub min_y: i32,
    pub height: i32,
    pub preliminary_surface: SamplerRef,
    /// Compiled samplers of the ids [`rule_densities`] returned.
    pub densities: HashMap<NodeId, SamplerRef>,
    pub biome_names: &'a [String],
    pub climate: Vec<BiomeClimate>,
}

impl MaterialSystem {
    /// `MaterialSystem`'s constructor and `RandomState`'s noise and random factories.
    pub fn new(state: &mut RandomState, input: MaterialInputs) -> Result<MaterialSystem, Error> {
        let pack = input.pack;
        let mut noise = |id: &str| state.noise(&pack.graph, id);
        let clay_bands_offset = noise("minecraft:clay_bands_offset")?;
        let surface = noise("minecraft:surface")?;
        let surface_secondary = noise("minecraft:surface_secondary")?;
        let badlands_pillar = noise("minecraft:badlands_pillar")?;
        let badlands_pillar_roof = noise("minecraft:badlands_pillar_roof")?;
        let badlands_surface = noise("minecraft:badlands_surface")?;
        let iceberg_pillar = noise("minecraft:iceberg_pillar")?;
        let iceberg_pillar_roof = noise("minecraft:iceberg_pillar_roof")?;
        let iceberg_surface = noise("minecraft:iceberg_surface")?;
        let noise_random = *state.factory();
        let clay_bands = generate_bands(&mut noise_random.from_hash_of("minecraft:clay_bands"));
        let biome_index = |name: &str| input.biome_names.iter().position(|b| b == name).map(|i| i as u16);
        let mut compiler = RuleCompiler {
            pack,
            state,
            densities: &input.densities,
            biome_index: &biome_index,
            min_y: input.min_y,
            height: input.height,
            sea_level: input.sea_level,
            ore_veins: Vec::new(),
            noises: Vec::new(),
            noise_slots: HashMap::new(),
        };
        let rule = compiler.rule(input.rule, 0)?;
        let (ore_veins, noises) = (compiler.ore_veins, compiler.noises);
        Ok(MaterialSystem {
            default_block: input.default_block,
            sea_level: input.sea_level,
            rule,
            ore_veins,
            noises,
            preliminary_surface: input.preliminary_surface,
            clay_bands,
            clay_bands_offset,
            badlands_pillar,
            badlands_pillar_roof,
            badlands_surface,
            iceberg_pillar,
            iceberg_pillar_roof,
            iceberg_surface,
            surface,
            surface_secondary,
            noise_random,
            climate: input.climate,
            eroded_badlands: biome_index("minecraft:eroded_badlands"),
            frozen_oceans: ["minecraft:frozen_ocean", "minecraft:deep_frozen_ocean"].iter().filter_map(|n| biome_index(n)).collect(),
        })
    }

    /// `getSurfaceDepth`.
    fn surface_depth(&self, x: i32, z: i32) -> i32 {
        let n = self.surface.get3(x as f64, 0.0, z as f64) as f64;
        (n * 2.75 + 3.0 + self.noise_random.at(x, 0, z).next_double() * 0.25) as i32
    }

    /// `getBand`.
    fn band(&self, x: i32, y: i32, z: i32) -> u16 {
        let offset = java_round(self.clay_bands_offset.get3(x as f64, 0.0, z as f64) * 4.0);
        let n = self.clay_bands.len() as i32;
        self.clay_bands[((y.wrapping_add(offset).wrapping_add(n)) % n) as usize]
    }

    /// `MaterialSystem.buildSurface`.
    pub fn build_surface(&self, s: &mut Scratch, biome: &mut dyn FnMut(i32, i32, i32) -> u16, chunk: &mut ProtoChunk) {
        let (min_x, min_z) = (chunk.x << 4, chunk.z << 4);
        let highest = (0..chunk.sections()).rev().find(|&i| chunk.blocks[i << 12..(i + 1) << 12].iter().any(|&b| !is_air(b)));
        let top = match highest {
            Some(i) => chunk.min_y + (i as i32) * 16 + 15,
            None => chunk.min_y - 1,
        };
        let expected = Volume::blocks([16, (top - chunk.min_y + 1).max(1), 16], [min_x, chunk.min_y, min_z]);
        let mut ctx = Context::new(self, s, biome, expected);
        for x in 0..16usize {
            for z in 0..16usize {
                let (bx, bz) = (min_x + x as i32, min_z + z as i32);
                let height = chunk.surface_height(x, z);
                let column_biome = (ctx.biome)(bx, height, bz);
                if Some(column_biome) == self.eroded_badlands {
                    self.eroded_badlands_extension(chunk, x, z, bx, bz, height);
                }
                let start = chunk.surface_height(x, z);
                ctx.update_xz(bx, bz, gradient_x(chunk, x, z), gradient_z(chunk, x, z));
                let mut stone_above = 0;
                let mut water_height = i32::MIN;
                let mut next_ceiling = i32::MAX;
                for y in (chunk.min_y..=start).rev() {
                    let block = chunk.get(x, y, z);
                    if is_air(block) {
                        stone_above = 0;
                        water_height = i32::MIN;
                        continue;
                    }
                    if has_fluid(block) {
                        if water_height == i32::MIN {
                            water_height = y + 1;
                        }
                        continue;
                    }
                    if next_ceiling >= y {
                        next_ceiling = WAY_BELOW_MIN_Y;
                        for yy in (chunk.min_y - 1..y).rev() {
                            if !is_stone(chunk.get(x, yy, z)) {
                                next_ceiling = yy + 1;
                                break;
                            }
                        }
                    }
                    stone_above += 1;
                    ctx.update_y(stone_above, y - next_ceiling + 1, water_height, y);
                    if let Some(b) = ctx.apply(&self.rule, bx, y, bz) {
                        chunk.set(x, y, z, b);
                    }
                }
                if self.frozen_oceans.contains(&column_biome) {
                    let min_surface = ctx.min_surface_level();
                    self.frozen_ocean_extension(min_surface, column_biome, chunk, x, z, bx, bz, height);
                }
            }
        }
    }

    /// `MaterialSystem.topMaterial`: the rules' block for one position with a fresh context
    /// (used when carving exposes dirt under grass).
    #[allow(clippy::too_many_arguments)]
    pub fn top_material(
        &self,
        s: &mut Scratch,
        biome: &mut dyn FnMut(i32, i32, i32) -> u16,
        chunk: &ProtoChunk,
        x: i32,
        y: i32,
        z: i32,
        fluid_above: bool,
    ) -> Option<u16> {
        let expected = Volume::blocks([1, 1, 1], [x, y, z]);
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let mut ctx = Context::new(self, s, biome, expected);
        ctx.update_xz(x, z, gradient_x(chunk, lx, lz), gradient_z(chunk, lx, lz));
        ctx.update_y(1, 1, if fluid_above { y + 1 } else { i32::MIN }, y);
        ctx.apply(&self.rule, x, y, z)
    }

    fn eroded_badlands_extension(&self, chunk: &mut ProtoChunk, x: usize, z: usize, bx: i32, bz: i32, height: i32) {
        let (fx, fz) = (bx as f64, bz as f64);
        let surface = (self.badlands_surface.get3(fx, 0.0, fz) as f64 * 8.25).abs();
        let pillar = (self.badlands_pillar.get3(fx * 0.2, 0.0, fz * 0.2) * 15.0) as f64;
        let d = java_min(surface, pillar);
        if d <= 0.0 {
            return;
        }
        let roof = (self.badlands_pillar_roof.get3(fx * 0.75, 0.0, fz * 0.75) as f64 * 1.5).abs();
        let top = jm::floor(64.0 + java_min(d * d * 2.5, (roof * 50.0).ceil() + 24.0));
        if height > top {
            return;
        }
        for y in (chunk.min_y..=top).rev() {
            let b = chunk.get(x, y, z);
            if same_block(b, self.default_block) {
                break;
            }
            if is_block(b, "minecraft:water") {
                return;
            }
        }
        let mut y = top;
        while y >= chunk.min_y && is_air(chunk.get(x, y, z)) {
            chunk.set(x, y, z, self.default_block);
            y -= 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn frozen_ocean_extension(
        &self,
        min_surface: i32,
        biome: u16,
        chunk: &mut ProtoChunk,
        x: usize,
        z: usize,
        bx: i32,
        bz: i32,
        height: i32,
    ) {
        let (fx, fz) = (bx as f64, bz as f64);
        let surface = (self.iceberg_surface.get3(fx, 0.0, fz) as f64 * 8.25).abs();
        let pillar = (self.iceberg_pillar.get3(fx * 1.28, 0.0, fz * 1.28) * 15.0) as f64;
        let d = java_min(surface, pillar);
        if d <= 1.8 {
            return;
        }
        let roof = (self.iceberg_pillar_roof.get3(fx * 1.17, 0.0, fz * 1.17) as f64 * 1.5).abs();
        let mut top = java_min(d * d * 1.2, (roof * 40.0).ceil() + 14.0);
        let c = &self.climate[biome as usize];
        if melts_icebergs_slightly(c.temperature, c.frozen, self.sea_level, bx, self.sea_level, bz) {
            top -= 2.0;
        }
        if top <= 2.0 {
            return;
        }
        let bottom = self.sea_level as f64 - top - 7.0;
        let top = top + self.sea_level as f64;
        let mut random = self.noise_random.at(bx, 0, bz);
        let snow_limit = 2 + random.next_int_bounded(4);
        let snow_min_y = self.sea_level + 18 + random.next_int_bounded(10);
        let mut snow = 0;
        let mut y = height.max(top as i32 + 1);
        while y >= min_surface {
            let b = chunk.get(x, y, z);
            let place = (is_air(b) && y < top as i32 && random.next_double() > 0.01)
                || (is_block(b, "minecraft:water")
                    && y > bottom as i32
                    && y < self.sea_level
                    && random.next_double() > 0.15);
            if place {
                if snow <= snow_limit && y > snow_min_y {
                    chunk.set(x, y, z, state::SNOW_BLOCK);
                    snow += 1;
                } else {
                    chunk.set(x, y, z, state::PACKED_ICE);
                }
            }
            y -= 1;
        }
    }
}

/// `Math.min(double, double)`.
#[inline]
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

/// `Math.round(float)`: nearest integer, ties toward positive infinity, saturating.
fn java_round(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    let f = v.floor();
    let r = if v - f >= 0.5 { f + 1.0 } else { f };
    r as i32
}

fn same_block(a: u16, b: u16) -> bool {
    kiln_data::blocks_types::block_of(a).name == kiln_data::blocks_types::block_of(b).name
}

/// `isStone`: neither air nor fluid.
#[inline]
fn is_stone(b: u16) -> bool {
    !is_air(b) && !has_fluid(b)
}

/// `getSurfaceGradientX`.
fn gradient_x(chunk: &ProtoChunk, x: usize, z: usize) -> i32 {
    chunk.surface_height((x + 1).min(15), z) - chunk.surface_height(x.saturating_sub(1), z)
}

/// `getSurfaceGradientZ`.
fn gradient_z(chunk: &ProtoChunk, x: usize, z: usize) -> i32 {
    chunk.surface_height(x, (z + 1).min(15)) - chunk.surface_height(x, z.saturating_sub(1))
}

/// `generateBands`.
fn generate_bands(r: &mut impl RandomSource) -> [u16; 192] {
    let mut bands = [state::TERRACOTTA; 192];
    let n = bands.len() as i32;
    let mut i = 0;
    while i < n {
        i += r.next_int_bounded(5) + 1;
        if i < n {
            bands[i as usize] = state::ORANGE_TERRACOTTA;
        }
        i += 1;
    }
    make_bands(r, &mut bands, 1, state::YELLOW_TERRACOTTA);
    make_bands(r, &mut bands, 2, state::BROWN_TERRACOTTA);
    make_bands(r, &mut bands, 1, state::RED_TERRACOTTA);
    let count = r.next_int_bounded(15 - 9 + 1) + 9;
    let (mut k, mut j) = (0, 0);
    while k < count && j < n {
        bands[j as usize] = state::WHITE_TERRACOTTA;
        if j - 1 > 0 && r.next_bool() {
            bands[(j - 1) as usize] = state::LIGHT_GRAY_TERRACOTTA;
        }
        if j + 1 < n && r.next_bool() {
            bands[(j + 1) as usize] = state::LIGHT_GRAY_TERRACOTTA;
        }
        k += 1;
        j += r.next_int_bounded(16) + 4;
    }
    bands
}

/// `makeBands`.
fn make_bands(r: &mut impl RandomSource, bands: &mut [u16; 192], base: i32, block: u16) {
    let n = bands.len() as i32;
    let count = r.next_int_bounded(15 - 6 + 1) + 6;
    for _ in 0..count {
        let len = base + r.next_int_bounded(3);
        let start = r.next_int_bounded(n);
        let mut k = 0;
        while start + k < n && k < len {
            bands[(start + k) as usize] = block;
            k += 1;
        }
    }
}

struct RuleCompiler<'a> {
    pack: &'a Datapack,
    state: &'a mut RandomState,
    densities: &'a HashMap<NodeId, SamplerRef>,
    biome_index: &'a dyn Fn(&str) -> Option<u16>,
    min_y: i32,
    height: i32,
    sea_level: i32,
    ore_veins: Vec<OreVein>,
    noises: Vec<(Arc<NoiseStack>, bool)>,
    noise_slots: HashMap<(String, bool), usize>,
}

impl RuleCompiler<'_> {
    fn anchor(&self, a: Anchor) -> i32 {
        a.resolve(self.min_y, self.height, self.sea_level)
    }

    fn random(&self, name: &str) -> PositionalRandomFactory {
        self.state.factory().from_hash_of(name).fork_positional()
    }

    fn rule(&mut self, rule: &Ref<RuleDef>, depth: u32) -> Result<Rule, Error> {
        if depth > 256 {
            return Err(Error::Invalid("material rule nesting too deep".into()));
        }
        let pack = self.pack;
        Ok(match resolve_rule(pack, rule)? {
            RuleDef::Block(b) => Rule::Block(b.resolve()?),
            RuleDef::Bandlands => Rule::Bandlands,
            RuleDef::Sequence(list) => {
                let mut rules = list.iter().map(|r| self.rule(r, depth + 1)).collect::<Result<Vec<_>, _>>()?;
                if rules.len() == 1 { rules.pop().unwrap() } else { Rule::Sequence(rules) }
            }
            RuleDef::Condition { if_true, then_run } => {
                let c = self.condition(if_true, depth + 1)?;
                let r = self.rule(then_run, depth + 1)?;
                Rule::Condition(Box::new(c), Box::new(r))
            }
            RuleDef::OreVein { ore_block, raw_ore_block, filler_block, raw_ore_chance, density, richness, filler_gap } => {
                let d = |id: &NodeId| self.densities.get(id).cloned().ok_or_else(|| Error::Invalid("uncompiled ore vein density".into()));
                let vein = OreVein {
                    ore: ore_block.resolve()?,
                    raw_ore: raw_ore_block.resolve()?,
                    filler: filler_block.resolve()?,
                    raw_ore_chance: *raw_ore_chance,
                    density: d(density)?,
                    richness: d(richness)?,
                    gap: d(filler_gap)?,
                    random: self.random("minecraft:ore"),
                };
                self.ore_veins.push(vein);
                Rule::OreVein(self.ore_veins.len() - 1)
            }
        })
    }

    fn condition(&mut self, cond: &Ref<CondDef>, depth: u32) -> Result<Cond, Error> {
        if depth > 256 {
            return Err(Error::Invalid("material condition nesting too deep".into()));
        }
        let pack = self.pack;
        Ok(match resolve_condition(pack, cond)? {
            CondDef::Biome(names) => Cond::Biome(names.iter().filter_map(|n| (self.biome_index)(n)).collect()),
            CondDef::NoiseThreshold { noise, min, max, is_3d } => {
                let key = (noise.clone(), *is_3d);
                let slot = match self.noise_slots.get(&key) {
                    Some(&s) => s,
                    None => {
                        let n = self.state.noise(&pack.graph, noise)?;
                        self.noises.push((n, *is_3d));
                        self.noise_slots.insert(key, self.noises.len() - 1);
                        self.noises.len() - 1
                    }
                };
                Cond::Noise { slot, min: *min, max: *max }
            }
            CondDef::VerticalGradient { random_name, true_at_and_below, false_at_and_above } => Cond::VerticalGradient {
                true_at_and_below: self.anchor(*true_at_and_below),
                false_at_and_above: self.anchor(*false_at_and_above),
                random: self.random(random_name),
            },
            CondDef::YAbove { anchor, surface_depth_multiplier, add_stone_depth } => Cond::YAbove {
                y: self.anchor(*anchor),
                surface_depth_multiplier: *surface_depth_multiplier,
                add_stone_depth: *add_stone_depth,
            },
            CondDef::Water { offset, surface_depth_multiplier, add_stone_depth } => Cond::Water {
                offset: *offset,
                surface_depth_multiplier: *surface_depth_multiplier,
                add_stone_depth: *add_stone_depth,
            },
            CondDef::Temperature => Cond::Temperature,
            CondDef::Steep => Cond::Steep,
            CondDef::Not(c) => Cond::Not(Box::new(self.condition(c, depth + 1)?)),
            CondDef::Hole => Cond::Hole,
            CondDef::AbovePreliminarySurface => Cond::AbovePreliminarySurface,
            CondDef::StoneDepth { offset, add_surface_depth, secondary_depth_range, ceiling } => Cond::StoneDepth {
                offset: *offset,
                add_surface_depth: *add_surface_depth,
                secondary_depth_range: *secondary_depth_range,
                ceiling: *ceiling,
            },
        })
    }
}

/// `MaterialRuleContext` with the compiled rule's per-context state (ore vein densities
/// sampled at compile time, memoized noises).
struct Context<'a, 'b> {
    sys: &'a MaterialSystem,
    s: &'b mut Scratch,
    biome: &'b mut dyn FnMut(i32, i32, i32) -> u16,
    expected: Volume,
    preliminary_volume: Volume,
    preliminary: Option<Vec<f32>>,
    /// Density and richness per ore vein, over `expected`.
    ore: Vec<(Vec<f32>, Vec<f32>)>,
    noise_cache: Vec<(u64, f64)>,
    xz_update: u64,
    y_update: u64,
    x: i32,
    z: i32,
    gradient_x: i32,
    gradient_z: i32,
    surface_depth: i32,
    secondary: Option<f64>,
    min_surface: Option<i32>,
    y: i32,
    water_height: i32,
    stone_below: i32,
    stone_above: i32,
    column_biome: Option<u16>,
}

impl<'a, 'b> Context<'a, 'b> {
    /// A new context, then `MaterialRule.compile`: ore veins sample their density and
    /// richness over the expected volume, in rule order.
    fn new(sys: &'a MaterialSystem, s: &'b mut Scratch, biome: &'b mut dyn FnMut(i32, i32, i32) -> u16, expected: Volume) -> Self {
        let ore = sys
            .ore_veins
            .iter()
            .map(|v| {
                let mut d = vec![0f32; expected.len()];
                v.density.fill(s, &expected, &mut d);
                let mut r = vec![0f32; expected.len()];
                v.richness.fill(s, &expected, &mut r);
                (d, r)
            })
            .collect();
        let preliminary_volume = Volume::blocks([expected.size[0], 1, expected.size[2]], [expected.min[0], 0, expected.min[2]]);
        Context {
            sys,
            s,
            biome,
            expected,
            preliminary_volume,
            preliminary: None,
            ore,
            noise_cache: vec![(u64::MAX, 0.0); sys.noises.len()],
            xz_update: 0,
            y_update: 0,
            x: 0,
            z: 0,
            gradient_x: 0,
            gradient_z: 0,
            surface_depth: 0,
            secondary: None,
            min_surface: None,
            y: 0,
            water_height: 0,
            stone_below: 0,
            stone_above: 0,
            column_biome: None,
        }
    }

    fn update_xz(&mut self, x: i32, z: i32, gradient_x: i32, gradient_z: i32) {
        self.xz_update += 1;
        self.y_update += 1;
        self.x = x;
        self.z = z;
        self.gradient_x = gradient_x;
        self.gradient_z = gradient_z;
        self.surface_depth = self.sys.surface_depth(x, z);
        self.secondary = None;
        self.min_surface = None;
    }

    fn update_y(&mut self, stone_above: i32, stone_below: i32, water_height: i32, y: i32) {
        self.y_update += 1;
        self.column_biome = None;
        self.y = y;
        self.water_height = water_height;
        self.stone_below = stone_below;
        self.stone_above = stone_above;
    }

    fn surface_secondary(&mut self) -> f64 {
        *self.secondary.get_or_insert_with(|| self.sys.surface_secondary.get3(self.x as f64, 0.0, self.z as f64) as f64)
    }

    fn biome(&mut self) -> u16 {
        match self.column_biome {
            Some(b) => b,
            None => {
                let b = (self.biome)(self.x, self.y, self.z);
                self.column_biome = Some(b);
                b
            }
        }
    }

    /// `getMinSurfaceLevel`: the chunk's preliminary surface (sampled in volume mode on
    /// first use) plus the surface depth, minus 8.
    fn min_surface_level(&mut self) -> i32 {
        if let Some(v) = self.min_surface {
            return v;
        }
        let v = match self.preliminary_volume.index_of_block(self.x, 0, self.z) {
            Some(i) => {
                if self.preliminary.is_none() {
                    let mut buf = vec![0f32; self.preliminary_volume.len()];
                    self.sys.preliminary_surface.fill(self.s, &self.preliminary_volume, &mut buf);
                    self.preliminary = Some(buf);
                }
                self.preliminary.as_ref().unwrap()[i]
            }
            None => self.sys.preliminary_surface.point(self.s, self.x, 0, self.z),
        };
        let level = jm::floor_f32(v) + self.surface_depth - 8;
        self.min_surface = Some(level);
        level
    }

    fn noise(&mut self, slot: usize) -> f64 {
        let (n, is_3d) = &self.sys.noises[slot];
        let key = if *is_3d { self.y_update } else { self.xz_update };
        let cached = &mut self.noise_cache[slot];
        if cached.0 != key {
            let y = if *is_3d { self.y as f64 } else { 0.0 };
            *cached = (key, n.get3(self.x as f64, y, self.z as f64) as f64);
        }
        cached.1
    }

    fn test(&mut self, c: &Cond) -> bool {
        match c {
            Cond::Biome(list) => {
                let b = self.biome();
                list.contains(&b)
            }
            Cond::Noise { slot, min, max } => {
                let v = self.noise(*slot);
                v >= *min && v <= *max
            }
            Cond::VerticalGradient { true_at_and_below, false_at_and_above, random } => {
                let y = self.y;
                if y <= *true_at_and_below {
                    return true;
                }
                if y >= *false_at_and_above {
                    return false;
                }
                let d = map(y as f64, *true_at_and_below as f64, *false_at_and_above as f64, 1.0, 0.0);
                (random.at(self.x, y, self.z).next_float() as f64) < d
            }
            Cond::YAbove { y, surface_depth_multiplier, add_stone_depth } => {
                let extra = if *add_stone_depth { self.stone_above } else { 0 };
                self.y + extra >= y + self.surface_depth * surface_depth_multiplier
            }
            Cond::Water { offset, surface_depth_multiplier, add_stone_depth } => {
                if self.water_height == i32::MIN {
                    return true;
                }
                let extra = if *add_stone_depth { self.stone_above } else { 0 };
                self.y + extra >= self.water_height + offset + self.surface_depth * surface_depth_multiplier
            }
            Cond::Temperature => {
                let b = self.biome();
                let c = &self.sys.climate[b as usize];
                cold_enough_to_snow(c.temperature, c.frozen, self.sys.sea_level, self.x, self.y, self.z)
            }
            Cond::Steep => self.gradient_x <= -4 || self.gradient_z >= 4,
            Cond::Not(c) => !self.test(c),
            Cond::Hole => self.surface_depth <= 0,
            Cond::AbovePreliminarySurface => self.y >= self.min_surface_level(),
            Cond::StoneDepth { offset, add_surface_depth, secondary_depth_range, ceiling } => {
                let depth = if *ceiling { self.stone_below } else { self.stone_above };
                let surface = if *add_surface_depth { self.surface_depth } else { 0 };
                let secondary = if *secondary_depth_range == 0 {
                    0
                } else {
                    map(self.surface_secondary(), -1.0, 1.0, 0.0, *secondary_depth_range as f64) as i32
                };
                depth <= 1 + offset + surface + secondary
            }
        }
    }

    /// Density getters of an ore vein (`getDensitiesInChunk`).
    fn ore_density(&mut self, vein: usize, richness: bool) -> f32 {
        let v = &self.sys.ore_veins[vein];
        match self.expected.index_of_block(self.x, self.y, self.z) {
            Some(i) => {
                let (d, r) = &self.ore[vein];
                if richness { r[i] } else { d[i] }
            }
            None => {
                let f = if richness { &v.richness } else { &v.density };
                f.point(self.s, self.x, self.y, self.z)
            }
        }
    }

    fn apply(&mut self, rule: &Rule, x: i32, y: i32, z: i32) -> Option<u16> {
        match rule {
            Rule::Block(b) => Some(*b),
            Rule::Bandlands => Some(self.sys.band(x, y, z)),
            Rule::Sequence(rules) => rules.iter().find_map(|r| self.apply(r, x, y, z)),
            Rule::Condition(c, r) => {
                if self.test(c) {
                    self.apply(r, x, y, z)
                } else {
                    None
                }
            }
            Rule::OreVein(i) => {
                let d = self.ore_density(*i, false);
                if !(d > 0.0) {
                    return None;
                }
                let v = &self.sys.ore_veins[*i];
                let mut random = v.random.at(x, y, z);
                if random.next_float() > d {
                    return None;
                }
                let richness = self.ore_density(*i, true);
                if random.next_float() < richness && v.gap.point(self.s, self.x, self.y, self.z) < 0.0 {
                    Some(if random.next_float() < v.raw_ore_chance { v.raw_ore } else { v.ore })
                } else {
                    Some(v.filler)
                }
            }
        }
    }
}
