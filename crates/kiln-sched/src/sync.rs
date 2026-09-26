//! Atomics used by the publication protocol, swapped for loom's under `--cfg loom`.

#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};
