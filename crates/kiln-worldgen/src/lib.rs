//! Vanilla 26.3 world generation: density functions and noise, bit-exact in f32.
//!
//! The datapack's `worldgen/{noise, density_function, noise_settings}` JSON is loaded into a
//! [`function::Graph`], compiled per world seed like vanilla's `RandomState` does, and
//! evaluated either per position ([`sampler::Sampler::point`]) or per volume
//! ([`sampler::Sampler::fill`]), matching `sampleValue` and `sampleVolume` respectively.

// Negated float comparisons mirror Java's fcmpl/fcmpg branches: they differ from the
// positive comparison exactly when an operand is NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod aquifer;
pub mod biome;
pub mod blocks;
pub mod compile;
pub mod datapack;
pub mod function;
pub mod generator;
pub mod interval;
pub mod json;
pub mod material;
pub mod noise;
pub mod sampler;
pub mod simplex;
pub mod spline;
pub mod state;
pub mod surface;
pub mod volume;

pub use datapack::{Datapack, NoiseSettings};
pub use generator::{GenScratch, Generator, ProtoChunk};
pub use sampler::{Sampler, SamplerRef, Scratch};
pub use state::{NoiseRouter, RandomState};
pub use volume::Volume;

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
