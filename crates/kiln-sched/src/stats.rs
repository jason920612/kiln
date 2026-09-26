use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Duration;

/// Cumulative counters of one worker since the pool was created (or [`reset`]).
///
/// `working` is time spent running units, window chunks taken as a helper and housekeeping
/// jobs, minus time parked inside them; `parked` is time blocked in the OS. For worker 0 only
/// time inside pool calls counts. Idle spinning is the remainder.
///
/// [`reset`]: crate::TickPool::reset_stats
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStats {
    pub working: Duration,
    pub parked: Duration,
    pub units: u64,
    pub chunks: u64,
    pub housekeeping: u64,
    pub housekeeping_panics: u64,
}

/// What one [`run_units`](crate::TickPool::run_units) call measured.
#[derive(Clone, Debug, Default)]
pub struct ForkReport {
    /// Wall time of each unit's `f`, indexed like the units slice (feed a cost EMA with it).
    pub unit_ns: Vec<u64>,
    /// Wall time of the whole call, including fork and join.
    pub wall: Duration,
}

#[derive(Default)]
pub(crate) struct StatCells {
    pub working_ns: AtomicU64,
    pub parked_ns: AtomicU64,
    pub units: AtomicU64,
    pub chunks: AtomicU64,
    pub housekeeping: AtomicU64,
    pub housekeeping_panics: AtomicU64,
}

impl StatCells {
    pub fn add(c: &AtomicU64, v: u64) {
        c.fetch_add(v, Relaxed);
    }

    pub fn snapshot(&self) -> WorkerStats {
        WorkerStats {
            working: Duration::from_nanos(self.working_ns.load(Relaxed)),
            parked: Duration::from_nanos(self.parked_ns.load(Relaxed)),
            units: self.units.load(Relaxed),
            chunks: self.chunks.load(Relaxed),
            housekeeping: self.housekeeping.load(Relaxed),
            housekeeping_panics: self.housekeeping_panics.load(Relaxed),
        }
    }

    pub fn reset(&self) {
        for c in [
            &self.working_ns,
            &self.parked_ns,
            &self.units,
            &self.chunks,
            &self.housekeeping,
            &self.housekeeping_panics,
        ] {
            c.store(0, Relaxed);
        }
    }
}
