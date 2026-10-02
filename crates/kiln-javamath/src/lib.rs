//! Java-exact arithmetic and random sources.
//!
//! Everything here reproduces the JVM's results bit for bit: `java.lang.Math` float semantics
//! (NaN propagation, signed zeros), Minecraft's `Mth` helpers, `java.util.Random`-style LCG
//! and Xoroshiro128++ sources with Minecraft's seeding (including MD5 name hashing).
//! Rust never contracts `a * b + c` into an FMA, so plain `f32` expressions written in Java's
//! evaluation order give Java's results.

pub mod atan;
pub mod math;
pub mod random;
pub mod trig;
