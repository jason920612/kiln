//! Java-exact arithmetic and random sources.
//!
//! Everything here reproduces the JVM's results bit for bit: `java.lang.Math` float semantics
//! (NaN propagation, signed zeros), Minecraft's `Mth` helpers, `java.util.Random`-style LCG
//! and Xoroshiro128++ sources with Minecraft's seeding (including MD5 name hashing).
//! Rust never contracts `a * b + c` into an FMA, so plain `f32` expressions written in Java's
//! evaluation order give Java's results.
//!
//! The transcendental functions are the reason this crate exists: the platform libm (MSVC,
//! glibc) rounds their last bit differently from Java and from each other, so the same seed
//! gives different worlds and mob decisions per operating system. Simulation and world
//! generation call these instead of `f64::sin` and friends (a test fails if they do not):
//!
//! | Java | here | agreement with a JDK |
//! |---|---|---|
//! | `Math.atan2`, `asin`, `acos`, `log1p` (no intrinsic, so fdlibm) | [`atan::atan2`], [`strict`] | bit exact |
//! | `Math.sin`, `cos`, `log`, `pow` (HotSpot's Intel libm stubs) | [`trig`], [`pow`] | correctly rounded, which the stubs are on nearly every argument (rates in `tests/jvm_dump.rs`) |

pub mod atan;
pub(crate) mod dd;
pub mod math;
pub mod mth;
pub mod pow;
pub mod random;
pub mod strict;
pub mod trig;
