//! Vanilla 26.3 world generation, block for block: density functions and noise (bit-exact in
//! f32), multi-noise biomes, and the TERRAIN status (noise fill with aquifers, material
//! rules, carvers). Structures, features and spawning are not generated.
//!
//! The datapack's `worldgen/{noise, density_function, noise_settings}` JSON is loaded into a
//! [`function::Graph`], compiled per world seed like vanilla's `RandomState` does, and
//! evaluated either per position ([`sampler::Sampler::point`]) or per volume
//! ([`sampler::Sampler::fill`]), matching `sampleValue` and `sampleVolume` respectively.
//! [`Generator`] builds chunks on top ([`generator::Step`] lists the steps), and
//! [`NoiseChunks`] hands them to `kiln-world` as a `ChunkGenerator`.

// Negated float comparisons mirror Java's fcmpl/fcmpg branches: they differ from the
// positive comparison exactly when an operand is NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod aquifer;
pub mod biome;
pub mod block_facts;
pub mod blocks;
pub mod carver;
pub mod compile;
pub mod datapack;
pub mod decorate;
pub mod feature;
pub mod function;
pub mod generator;
pub mod interval;
pub mod json;
pub mod material;
pub mod noise;
pub mod order;
pub mod pipeline;
pub mod placement;
pub mod pos;
pub mod postprocess;
pub mod predicate;
pub mod proto;
pub mod providers;
pub mod random;
pub mod region;
pub mod sampler;
pub mod sets;
pub mod simplex;
pub mod spawn;
pub mod spline;
pub mod state;
pub mod state_provider;
pub mod structure;
pub mod surface;
pub mod survive;
pub mod volume;
pub mod vtags;
pub mod world;

pub use datapack::{Datapack, NoiseSettings};
pub use generator::{GenScratch, Generator, ProtoChunk};
pub use sampler::{Sampler, SamplerRef, Scratch};
pub use state::{NoiseRouter, RandomState};
pub use volume::Volume;
pub use pipeline::{Pipeline, Worldgen};
pub use world::{FullChunks, NoiseChunks};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported density function type {0}")]
    UnsupportedFunction(String),
    #[error("unknown density function {0}")]
    UnknownFunction(String),
    #[error("unknown noise {0}")]
    UnknownNoise(String),
    #[error("unknown noise settings {0}")]
    UnknownSettings(String),
    #[error("invalid worldgen data: {0}")]
    Invalid(String),
    #[error("{0}: {1}")]
    Context(String, Box<Error>),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] json::ParseError),
}

impl Error {
    pub fn context(self, what: impl Into<String>) -> Error {
        Error::Context(what.into(), Box::new(self))
    }
}
