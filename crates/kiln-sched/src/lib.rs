//! The tick pool: one priority work pool for region ticks and the parallel phase windows inside
//! them (design §4.2–4.6). Self-contained; it knows nothing about regions or worlds.
//!
//! # Workers
//! [`TickPool::new(n)`](TickPool::new) spawns `n - 1` threads; the thread calling into the pool
//! (the sim's coordinator) is worker 0 while it is inside a pool call, so `TickPool::new(7)`
//! uses 7 threads in total. Idle workers spin for at most [`PoolConfig::spin`] and then park,
//! so they cost no CPU between ticks; each worker shortens its spin while spinning keeps ending
//! in a park (down to a sixteenth) and lengthens it again when work arrives mid-spin. Waking parked workers costs tens of microseconds;
//! [`TickPool::prewake`] can hide that at the start of a tick.
//!
//! An idle worker always takes the highest-priority work available:
//! 1. **phase chunks**: pieces of a window that some unit (or the coordinator) is waiting for;
//! 2. **unit batches** of the current [`run_units`](TickPool::run_units) fork;
//! 3. **housekeeping** jobs ([`TickPool::spawn_housekeeping`]), only while no fork is running.
//!
//! # Region fork-join
//! [`TickPool::run_units`]`(&mut units, cost, f)` calls `f(&mut unit, &ctx)` once per unit in
//! parallel and returns when every call has finished.
//! - Each unit is handed out exactly once, as a `&mut` no other thread holds: the slice is
//!   borrowed mutably for the call, `U: Send`, `f: Sync`. `f` may borrow non-`'static` data.
//! - Units start in decreasing `cost` order (LPT, ties by index). Units estimated below
//!   [`PoolConfig::small_unit`] are batched up to [`PoolConfig::unit_batch`] of estimate; a
//!   batch never holds more than `n / (2 * workers)` units, so zero estimates still spread out.
//! - A panicking unit does not stop the others; once all have finished, the first panic is
//!   resumed in the caller. The pool stays usable.
//! - [`ForkReport::unit_ns`] has each unit's wall time, for the caller's cost EMA.
//!
//! # Phase windows
//! [`Ctx::map_indexed`] maps a slice and returns the results in input order (an indexed
//! collect, never a reduction). It runs inline or splits the items into chunks of about
//! [`PoolConfig::chunk_target`] that other workers may take; the caller runs chunks too.
//! While it waits for chunks others took, the caller only helps windows of its own unit
//! (including nested ones) and never starts another unit or a housekeeping job, so a worker
//! inside a region tick cannot get stuck behind an unrelated multi-millisecond tick (the
//! priority inversion rayon's `join` allows). Workers with nothing else to do help any window.
//! - Windows nest: [`Ctx::map_indexed_with`] passes the executing worker's `Ctx` to `run`.
//! - The coordinator opens windows in serial segments through [`TickPool::serial`].
//! - A panic in `run` is resumed from the `map_indexed` call after every chunk has finished or
//!   been skipped; outputs already produced are dropped exactly once.
//!
//! **Strategy** ([`Strategy`]): under [`PhaseMode::Auto`] a window runs inline when its
//! estimate is below [`PoolConfig::inline_below`]. The estimate comes from [`Window::item_ns`];
//! without one, the window maps a prefix inline while timing it (at the rate of its fastest
//! large block, so a block that lost its core does not inflate it) and extrapolates. A split
//! window wakes only as many parked helpers as get [`PoolConfig::helper_share`] of the estimate
//! each, and stays inline when that is none: a helper costs a wake-up and a spin whether or not
//! it finds much to do. A single worker always runs inline. The strategy never changes a result: inline and parallel call
//! the same closure on the same items, and outputs land at their index.
//!
//! # Determinism, strict and chaos modes
//! The pool decides only where and when code runs, never what it computes. If units touch only
//! their own `&mut U` plus shared immutable data and window closures are pure, results are
//! identical for any worker count and schedule. To test that, [`PoolConfig::phase`] forces
//! windows [`Inline`](PhaseMode::Inline), [`Parallel`](PhaseMode::Parallel) or a random
//! [`Mixed`](PhaseMode::Mixed) choice, and [`PoolConfig::chaos`] (seeded) randomizes unit
//! start order, unit batches, chunk boundaries and claim order, the order idle workers scan
//! windows in, how many workers get woken, and adds random yields.
//!
//! # Metrics
//! [`ForkReport`] per fork; [`TickPool::stats`] gives each worker's working and parked time and
//! how many units, chunks and housekeeping jobs it ran.
//!
//! # Limits
//! - At most [`MAX_WORKERS`] workers.
//! - At most 64 windows published at once (four slots per worker, at least 16); a window that
//!   finds no free slot runs inline, with the same result.
//! - One fork at a time: `run_units` takes `&mut self`. `TickPool` is `Send` but not `Sync`;
//!   other threads queue housekeeping through a [`Housekeeper`].
//! - A housekeeping job runs to completion once started, so keep jobs around a millisecond.
//!
//! # Costs
//! Measured with `examples/forkjoin.rs` on Windows 11 (Ryzen 7 5700X3D, 8C/16T), 7 workers:
//! an empty fork of 7 units takes about 3 µs back to back and about 20 µs after 50 ms idle
//! (workers parked); a parked worker's wake-up costs its waker about 5 µs, so each thread
//! unparks at most two and the woken workers pass the rest on.

mod config;
mod job;
mod pool;
mod rng;
mod slot;
mod stats;
mod sync;
mod window;

pub use config::{PhaseMode, PoolConfig, Strategy};
pub use pool::{Housekeeper, MAX_WORKERS, TickPool};
pub use stats::{ForkReport, WorkerStats};
pub use window::{Ctx, Window};
