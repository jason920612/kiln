use std::time::Duration;

/// How one phase window runs. The choice never changes the window's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    /// The calling worker maps every item itself.
    Inline,
    /// The items are split into chunks that idle workers may take.
    Parallel,
}

/// Pool-wide override of the per-window [`Strategy`] choice (tests and strict mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PhaseMode {
    /// Inline when the estimate is below [`PoolConfig::inline_below`], else parallel. A
    /// window without an estimate times a prefix of its items and extrapolates.
    #[default]
    Auto,
    /// Every window runs inline.
    Inline,
    /// Every window of two or more items is split and published, even on a single worker.
    Parallel,
    /// Each window picks inline or parallel (with a random chunk size) at random.
    Mixed,
}

#[derive(Clone, Debug)]
pub struct PoolConfig {
    /// Total workers including the calling thread, which acts as worker 0: `workers - 1`
    /// threads are spawned. At most 64.
    pub workers: usize,
    /// How long an idle worker spins (checking for work) before it parks.
    pub spin: Duration,
    /// Windows estimated to take less than this run inline.
    pub inline_below: Duration,
    /// Target duration of one window chunk.
    pub chunk_target: Duration,
    /// Units estimated below this are batched with other small units.
    pub small_unit: Duration,
    /// Target duration of one batch of small units.
    pub unit_batch: Duration,
    pub phase: PhaseMode,
    /// Seeded chaos scheduling for determinism tests: unit start order, unit batches, chunk
    /// boundaries, chunk claim order, the order idle workers scan windows in, how many
    /// workers get woken, and random yields are all randomized. Results must not change.
    pub chaos: Option<u64>,
    /// Stack size of the spawned workers (the caller keeps its own stack).
    pub stack_size: Option<usize>,
}

impl PoolConfig {
    pub fn new(workers: usize) -> Self {
        PoolConfig {
            workers,
            spin: Duration::from_micros(50),
            inline_below: Duration::from_micros(500),
            chunk_target: Duration::from_micros(100),
            small_unit: Duration::from_micros(200),
            unit_batch: Duration::from_millis(1),
            phase: PhaseMode::Auto,
            chaos: None,
            stack_size: None,
        }
    }
}

/// The numeric part of [`PoolConfig`], in nanoseconds.
#[derive(Clone, Copy)]
pub(crate) struct Tuning {
    pub spin_ns: u64,
    pub inline_below_ns: u64,
    pub chunk_target_ns: u64,
    pub small_unit_ns: u64,
    pub unit_batch_ns: u64,
    pub phase: PhaseMode,
    pub chaos: bool,
}

impl Tuning {
    pub fn new(c: &PoolConfig) -> Self {
        let ns = |d: Duration| d.as_nanos().min(u64::MAX as u128) as u64;
        Tuning {
            spin_ns: ns(c.spin),
            inline_below_ns: ns(c.inline_below),
            chunk_target_ns: ns(c.chunk_target).max(1),
            small_unit_ns: ns(c.small_unit),
            unit_batch_ns: ns(c.unit_batch),
            phase: c.phase,
            chaos: c.chaos.is_some(),
        }
    }

    /// Items per chunk for items costing `item_ns` each.
    pub fn chunk_for(&self, item_ns: u64, n: usize) -> usize {
        ((self.chunk_target_ns / item_ns.max(1)) as usize).clamp(1, n.max(1))
    }
}
