//! Per-seed worldgen state (vanilla's `RandomState`): the positional random factory derived
//! from the world seed, the noise instances seeded from it, and the compiled noise router.

use crate::Error;
use crate::compile::{Compiler, NoiseSource, compile_random};
use crate::datapack::Datapack;
use crate::function::{Graph, NodeId};
use crate::noise::{NoiseStack, instantiate};
use crate::sampler::SamplerRef;
use kiln_javamath::random::{LegacyRandom, PositionalRandomFactory, RandomSource, WorldgenRandom, XoroshiroRandom};
use std::collections::HashMap;
use std::sync::Arc;

pub struct RandomState {
    seed: i64,
    legacy: bool,
    factory: PositionalRandomFactory,
    noises: HashMap<String, Arc<NoiseStack>>,
}

struct Source<'a> {
    graph: &'a Graph,
    state: &'a mut RandomState,
}

impl NoiseSource for Source<'_> {
    /// `CompileContext.createNoiseSampler`: the two nether biome noises (whatever the random
    /// source setting) get the legacy octave setup on a `LegacyRandomSource(seed + 0)` and
    /// `(seed + 1)`; every other noise is the world's shared instance.
    fn noise(&mut self, id: &str) -> Result<Arc<NoiseStack>, Error> {
        let offset = match id {
            "minecraft:nether/temperature" => 0,
            "minecraft:nether/vegetation" => 1,
            _ => return self.state.noise(self.graph, id),
        };
        let params = self.graph.noise(id)?;
        let mut random = LegacyRandom::new(self.state.seed.wrapping_add(offset));
        Ok(Arc::new(params.create_legacy_nether_biome(&mut random)))
    }

    fn random(&mut self, id: &str) -> WorldgenRandom {
        compile_random(&self.state.factory, self.state.legacy.then_some(self.state.seed), id)
    }
}

impl RandomState {
    /// `RandomState.create`: Xoroshiro seeding unless the settings ask for the legacy LCG.
    pub fn new(seed: i64, legacy_random_source: bool) -> Self {
        let factory = if legacy_random_source {
            LegacyRandom::new(seed).fork_positional()
        } else {
            XoroshiroRandom::new(seed).fork_positional()
        };
        Self { seed, legacy: legacy_random_source, factory, noises: HashMap::new() }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    pub fn factory(&self) -> &PositionalRandomFactory {
        &self.factory
    }

    /// `RandomState.getOrCreateNoise`: the registered noise `id` seeded for this world.
    pub fn noise(&mut self, graph: &Graph, id: &str) -> Result<Arc<NoiseStack>, Error> {
        if let Some(n) = self.noises.get(id) {
            return Ok(n.clone());
        }
        let n = Arc::new(instantiate(graph.noise(id)?, &self.factory, id));
        self.noises.insert(id.to_string(), n.clone());
        Ok(n)
    }

    /// Compiles `roots` with one shared compiler, so common subfunctions and caches are shared.
    pub fn compile(&mut self, graph: &Graph, roots: &[NodeId]) -> Result<Vec<SamplerRef>, Error> {
        let mut source = Source { graph, state: self };
        let mut compiler = Compiler::new(graph, &mut source);
        roots.iter().map(|&r| compiler.compile(r)).collect()
    }
}

/// The compiled `noise_router` of a noise settings entry, plus its aquifer functions.
pub struct NoiseRouter {
    /// Router fields in vanilla order, then aquifer functions as `aquifers/<name>`.
    pub outputs: Vec<(String, SamplerRef)>,
}

impl NoiseRouter {
    pub fn new(pack: &Datapack, settings: &str, seed: i64) -> Result<(RandomState, NoiseRouter), Error> {
        let s = pack.settings(settings)?;
        let mut state = RandomState::new(seed, s.legacy_random_source);
        let names: Vec<String> = s
            .router
            .iter()
            .map(|(n, _)| n.clone())
            .chain(s.aquifers.iter().map(|(n, _)| format!("aquifers/{n}")))
            .collect();
        let roots: Vec<NodeId> = s.router.iter().chain(&s.aquifers).map(|(_, id)| *id).collect();
        let samplers = state.compile(&pack.graph, &roots)?;
        Ok((state, NoiseRouter { outputs: names.into_iter().zip(samplers).collect() }))
    }

    pub fn get(&self, name: &str) -> Option<&SamplerRef> {
        self.outputs.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }
}
