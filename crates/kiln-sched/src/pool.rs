//! The pool: worker threads, the unit fork, parking and waking, housekeeping.

use std::any::Any;
use std::cell::Cell;
use std::hint;
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::Ordering::{Acquire, Relaxed, SeqCst};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::{self, JoinHandle, Thread, ThreadId};
use std::time::{Duration, Instant};

use crossbeam_deque::{Injector, Steal};
use crossbeam_utils::CachePadded;

use crate::config::{PoolConfig, Tuning};
use crate::job::{AbortOnUnwind, Header};
use crate::rng::Rng;
use crate::slot::Slot;
use crate::stats::{ForkReport, StatCells, WorkerStats};
use crate::window::Ctx;

/// Upper bound on [`PoolConfig::workers`] (the sleeper and live-window sets are `u64` masks).
pub const MAX_WORKERS: usize = 64;

/// Family of a worker outside any unit: it may help every window.
pub(crate) const ANY: u64 = 0;

/// How many parked workers one thread unparks for a wake-up; the workers it wakes pass the rest
/// on. An unpark costs the caller about 5 µs on Windows, so this keeps a publisher from spending
/// 30 µs waking six helpers before it gets to its own work.
const WAKE_FANOUT: usize = 2;

type HkJob = Box<dyn FnOnce() + Send>;

pub(crate) struct Shared {
    pub tuning: Tuning,
    pub workers: usize,
    pub windows: Box<[CachePadded<Slot>]>,
    /// Bit `i`: window slot `i` may have unclaimed chunks.
    pub live: CachePadded<AtomicU64>,
    fork: CachePadded<Slot>,
    /// The published fork may have unclaimed batches.
    fork_pending: CachePadded<AtomicBool>,
    /// A fork is in progress; housekeeping does not start meanwhile.
    fork_active: AtomicBool,
    /// Bit `i`: worker `i` is parked (or about to park) and wants to be woken for work.
    sleepers: CachePadded<AtomicU64>,
    /// Wake-ups requested but not yet passed on (a hint: nothing depends on them for progress).
    wake_debt: CachePadded<AtomicUsize>,
    epoch: Instant,
    /// Until this time (ns since `epoch`) idle workers keep spinning; see `prewake`.
    hot_until: AtomicU64,
    hk: Injector<HkJob>,
    hk_pending: AtomicUsize,
    shutdown: AtomicBool,
    threads: Box<[OnceLock<Thread>]>,
    /// The thread currently acting as worker 0.
    coord: Mutex<Option<Thread>>,
    pub stats: Box<[CachePadded<StatCells>]>,
}

/// Per-thread worker state; lives on the worker's stack (in the pool for worker 0).
pub(crate) struct WorkerLocal {
    pub shared: Arc<Shared>,
    pub idx: usize,
    rng: Cell<u64>,
    parked_ns: Cell<u64>,
}

impl WorkerLocal {
    fn new(shared: Arc<Shared>, idx: usize, seed: u64) -> Self {
        let mut rng = Rng(seed ^ (idx as u64).wrapping_mul(0xA24B_AED4_963E_E407));
        rng.next_u64();
        WorkerLocal { shared, idx, rng: Cell::new(rng.0), parked_ns: Cell::new(0) }
    }

    pub fn with_rng<T>(&self, f: impl FnOnce(&mut Rng) -> T) -> T {
        let mut rng = Rng(self.rng.get());
        let v = f(&mut rng);
        self.rng.set(rng.0);
        v
    }

    pub fn rand_below(&self, n: usize) -> usize {
        self.with_rng(|r| r.below(n))
    }

    pub fn chaos(&self) -> bool {
        self.shared.tuning.chaos
    }

    /// Delays now and then in chaos mode to shake up interleavings. Spins rather than yields:
    /// a yield can hand the core away for a whole OS quantum on a busy machine.
    pub fn chaos_point(&self) {
        if self.chaos() && self.rand_below(8) == 0 {
            for _ in 0..self.rand_below(1024) {
                hint::spin_loop();
            }
        }
    }

    pub fn park(&self) {
        let t = Instant::now();
        thread::park();
        let ns = t.elapsed().as_nanos() as u64;
        self.parked_ns.set(self.parked_ns.get() + ns);
        StatCells::add(&self.shared.stats[self.idx].parked_ns, ns);
    }
}

/// Bounded spinning before parking.
#[derive(Default)]
pub(crate) struct Idle {
    since: Option<Instant>,
    rounds: u32,
}

impl Idle {
    pub fn reset(&mut self) {
        self.since = None;
    }

    /// Spins briefly; false once the spin budget (or a longer `prewake` hold) is used up and
    /// the caller should park. Only idle workers `may_yield`: a waiting owner is on the
    /// critical path, and a yield can cost it a whole OS quantum when the machine is busy.
    pub fn spin(&mut self, sh: &Shared, may_yield: bool) -> bool {
        let since = *self.since.get_or_insert_with(Instant::now);
        if since.elapsed().as_nanos() as u64 >= sh.tuning.spin_ns && sh.now_ns() >= sh.hot_until.load(Relaxed) {
            return false;
        }
        self.rounds = self.rounds.wrapping_add(1);
        if may_yield && self.rounds.is_multiple_of(16) {
            thread::yield_now();
        } else {
            for _ in 0..64 {
                hint::spin_loop();
            }
        }
        true
    }
}

impl Shared {
    fn new(cfg: &PoolConfig) -> Self {
        let slots = (cfg.workers * 4).clamp(16, 64);
        Shared {
            tuning: Tuning::new(cfg),
            workers: cfg.workers,
            windows: (0..slots).map(|_| CachePadded::new(Slot::new())).collect(),
            live: CachePadded::new(AtomicU64::new(0)),
            fork: CachePadded::new(Slot::new()),
            fork_pending: CachePadded::new(AtomicBool::new(false)),
            fork_active: AtomicBool::new(false),
            sleepers: CachePadded::new(AtomicU64::new(0)),
            wake_debt: CachePadded::new(AtomicUsize::new(0)),
            epoch: Instant::now(),
            hot_until: AtomicU64::new(0),
            hk: Injector::new(),
            hk_pending: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            threads: (0..cfg.workers).map(|_| OnceLock::new()).collect(),
            coord: Mutex::new(None),
            stats: (0..cfg.workers).map(|_| CachePadded::new(StatCells::default())).collect(),
        }
    }

    pub fn unpark(&self, worker: usize) {
        if worker == 0 {
            if let Some(t) = &*self.coord.lock().unwrap_or_else(PoisonError::into_inner) {
                t.unpark();
            }
        } else if let Some(t) = self.threads[worker].get() {
            t.unpark();
        }
    }

    /// Wakes up to `k` parked workers: a couple directly, the rest through the workers woken.
    /// Publishers call this after making work visible with a `SeqCst` write; `sleep` registers
    /// before its `SeqCst` re-check, so the direct wake-up is never lost. Progress never depends
    /// on it anyway: every publisher completes its own work if nobody helps.
    pub fn wake(&self, k: usize) {
        if k == 0 || self.sleepers.load(SeqCst) == 0 {
            return;
        }
        let direct = k.min(WAKE_FANOUT);
        if k > direct {
            self.wake_debt.fetch_add(k - direct, Relaxed);
        }
        if self.wake_now(direct) < direct {
            self.wake_debt.store(0, Relaxed);
        }
    }

    /// Unparks up to `k` sleepers; returns how many.
    fn wake_now(&self, k: usize) -> usize {
        let mut woke = 0;
        while woke < k {
            let s = self.sleepers.load(SeqCst);
            if s == 0 {
                break;
            }
            let bit = 1u64 << s.trailing_zeros();
            if self.sleepers.fetch_and(!bit, SeqCst) & bit != 0 {
                self.unpark(bit.trailing_zeros() as usize);
                woke += 1;
            }
        }
        woke
    }

    /// Called by a worker that was just unparked: takes over part of a pending wake-up.
    fn pass_on_wake(&self) {
        let mut debt = self.wake_debt.load(Relaxed);
        while debt > 0 {
            let take = debt.min(WAKE_FANOUT);
            match self.wake_debt.compare_exchange_weak(debt, debt - take, Relaxed, Relaxed) {
                Ok(_) => {
                    if self.wake_now(take) < take {
                        self.wake_debt.store(0, Relaxed);
                    }
                    return;
                }
                Err(d) => debt = d,
            }
        }
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// How many helpers to wake for `extra` pieces beyond the one the publisher runs.
    pub fn wake_count(&self, local: &WorkerLocal, extra: usize) -> usize {
        let k = extra.min(self.workers - 1);
        if local.chaos() { local.rand_below(k + 1) } else { k }
    }

    /// Parks until woken, unless `has_work` (which must read with `SeqCst`) finds work after
    /// the worker registered as a sleeper.
    pub fn sleep(&self, local: &WorkerLocal, has_work: impl Fn() -> bool) {
        let bit = 1u64 << local.idx;
        self.sleepers.fetch_or(bit, SeqCst);
        if !has_work() {
            local.park();
            self.pass_on_wake();
        }
        self.sleepers.fetch_and(!bit, SeqCst);
    }

    fn idle_has_work(&self) -> bool {
        self.live.load(SeqCst) != 0
            || self.fork_pending.load(SeqCst)
            || self.shutdown.load(SeqCst)
            || (self.hk_pending.load(SeqCst) != 0 && !self.fork_active.load(SeqCst))
    }

    /// Runs `f`; if it did something, adds its duration minus time parked inside it to the
    /// worker's working time.
    pub fn timed(&self, local: &WorkerLocal, f: impl FnOnce() -> bool) -> bool {
        let t = Instant::now();
        let parked = local.parked_ns.get();
        let did = f();
        if did {
            let ns = (t.elapsed().as_nanos() as u64).saturating_sub(local.parked_ns.get() - parked);
            StatCells::add(&self.stats[local.idx].working_ns, ns);
        }
        did
    }

    /// Takes chunks from published windows of `family` (any window for [`ANY`]) until one has
    /// nothing left. Returns whether it ran a chunk.
    pub fn help_windows(&self, local: &WorkerLocal, family: u64) -> bool {
        let mask = self.live.load(Acquire);
        if mask == 0 {
            return false;
        }
        let rot = if local.chaos() { local.rand_below(64) as u32 } else { 0 };
        let mut m = mask.rotate_right(rot);
        while m != 0 {
            let i = ((m.trailing_zeros() + rot) % 64) as usize;
            m &= m - 1;
            let slot = &self.windows[i];
            if family != ANY && slot.family_hint() != family {
                continue;
            }
            let Some(e) = slot.enter() else { continue };
            let mut ran = false;
            if family == ANY || e.family == family {
                let job = e.job.cast::<Header>();
                // SAFETY: while inside the slot its job stays alive; pieces come from `claim`.
                while let Some(p) = unsafe { (*job).claim() } {
                    unsafe { Header::exec(job, p, local) };
                    ran = true;
                }
                // Still inside, so bit `i` belongs to this job, which has nothing left to claim.
                self.live.fetch_and(!(1 << i), SeqCst);
            }
            if let Some(owner) = slot.leave() {
                self.unpark(owner);
            }
            if ran {
                return true;
            }
        }
        false
    }

    /// Waits until a closed window slot is drained. Meanwhile it helps only windows of the
    /// same family: the waiting worker is inside a unit and must not start other work.
    pub fn wait_drained(&self, slot: &Slot, local: &WorkerLocal, family: u64) {
        let mut idle = Idle::default();
        while !slot.drained() {
            if self.help_windows(local, family) {
                idle.reset();
            } else if !idle.spin(self, false) {
                // The last helper to leave unparks us; a wakeup that raced ahead leaves a token.
                local.park();
                idle.reset();
            }
        }
    }

    pub fn reserve_window(&self, local: &WorkerLocal) -> Option<usize> {
        let n = self.windows.len();
        let start = local.idx * 4 % n;
        (0..n).map(|k| (start + k) % n).find(|&i| self.windows[i].try_reserve())
    }

    fn run_fork_batch(&self, local: &WorkerLocal) -> bool {
        if !self.fork_pending.load(Acquire) {
            return false;
        }
        let Some(e) = self.fork.enter() else { return false };
        let job = e.job.cast::<Header>();
        // SAFETY: while inside the slot the fork job stays alive.
        let ran = match unsafe { (*job).claim() } {
            Some(p) => {
                unsafe { Header::exec(job, p, local) };
                true
            }
            None => {
                self.fork_pending.store(false, SeqCst);
                false
            }
        };
        if let Some(owner) = self.fork.leave() {
            self.unpark(owner);
        }
        ran
    }

    fn run_housekeeping(&self, local: &WorkerLocal) -> bool {
        loop {
            match self.hk.steal() {
                Steal::Success(job) => {
                    self.hk_pending.fetch_sub(1, SeqCst);
                    let cells = &self.stats[local.idx];
                    StatCells::add(&cells.housekeeping, 1);
                    if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
                        StatCells::add(&cells.housekeeping_panics, 1);
                    }
                    return true;
                }
                Steal::Empty => return false,
                Steal::Retry => {}
            }
        }
    }

    /// One step of an idle worker, in priority order: window chunks, then unit batches, then
    /// (outside forks) housekeeping.
    fn find_work(&self, local: &WorkerLocal) -> bool {
        self.help_windows(local, ANY)
            || self.run_fork_batch(local)
            || (!self.fork_active.load(Acquire) && self.run_housekeeping(local))
    }

    fn submit(&self, job: HkJob) {
        self.hk_pending.fetch_add(1, SeqCst);
        self.hk.push(job);
        if !self.fork_active.load(SeqCst) {
            self.wake(1);
        }
    }
}

fn worker_main(shared: Arc<Shared>, idx: usize, seed: u64) {
    let _ = shared.threads[idx].set(thread::current());
    let local = WorkerLocal::new(shared.clone(), idx, seed);
    let sh = &*shared;
    let mut idle = Idle::default();
    loop {
        if sh.timed(&local, || sh.find_work(&local)) {
            idle.reset();
            continue;
        }
        if sh.shutdown.load(SeqCst) && sh.hk_pending.load(SeqCst) == 0 {
            return;
        }
        if !idle.spin(sh, true) {
            sh.sleep(&local, || sh.idle_has_work());
            idle.reset();
        }
    }
}

/// The single priority work pool that runs region ticks and their parallel phase windows.
/// See the crate docs for the guarantees.
pub struct TickPool {
    shared: Arc<Shared>,
    local: WorkerLocal,
    threads: Vec<JoinHandle<()>>,
    coord: Option<ThreadId>,
    next_family: u64,
}

impl TickPool {
    /// A pool of `workers` workers in total: the calling thread is worker 0 while it is inside a
    /// pool call, so `workers - 1` threads are spawned.
    pub fn new(workers: usize) -> Self {
        Self::with_config(PoolConfig::new(workers))
    }

    pub fn with_config(cfg: PoolConfig) -> Self {
        assert!(
            (1..=MAX_WORKERS).contains(&cfg.workers),
            "a tick pool needs 1..={MAX_WORKERS} workers, got {}",
            cfg.workers
        );
        let seed = cfg.chaos.unwrap_or(0x6b69_6c6e);
        let shared = Arc::new(Shared::new(&cfg));
        let threads = (1..cfg.workers)
            .map(|i| {
                let sh = shared.clone();
                let mut b = thread::Builder::new().name(format!("kiln-tick-{i}"));
                if let Some(size) = cfg.stack_size {
                    b = b.stack_size(size);
                }
                b.spawn(move || worker_main(sh, i, seed)).expect("failed to spawn a tick worker")
            })
            .collect();
        TickPool { local: WorkerLocal::new(shared.clone(), 0, seed), shared, threads, coord: None, next_family: 1 }
    }

    pub fn workers(&self) -> usize {
        self.shared.workers
    }

    /// Makes the calling thread worker 0.
    fn bind(&mut self) {
        let t = thread::current();
        if self.coord != Some(t.id()) {
            self.coord = Some(t.id());
            *self.shared.coord.lock().unwrap_or_else(PoisonError::into_inner) = Some(t);
        }
    }

    fn families(&mut self, n: usize) -> u64 {
        let first = self.next_family;
        self.next_family += n as u64;
        first
    }

    /// Runs `f` once on every unit, in parallel, and returns when all are done.
    ///
    /// `cost` estimates a unit's tick in nanoseconds (for example a cost EMA). Units start
    /// largest estimate first (LPT); units estimated below [`PoolConfig::small_unit`] are
    /// batched into jobs of about [`PoolConfig::unit_batch`]. `f` gets a [`Ctx`] for opening
    /// phase windows. If units panic, every other unit still runs, and the first panic is
    /// resumed here after all of them finished.
    pub fn run_units<U, C, F>(&mut self, units: &mut [U], cost: C, f: F) -> ForkReport
    where
        U: Send,
        C: Fn(&U) -> u64,
        F: Fn(&mut U, &Ctx<'_>) + Sync,
    {
        let start = Instant::now();
        let n = units.len();
        let mut unit_ns = vec![0u64; n];
        if n == 0 {
            return ForkReport { unit_ns, wall: start.elapsed() };
        }
        assert!(n <= u32::MAX as usize, "too many units");
        self.bind();
        let family0 = self.families(n);
        let costs: Vec<u64> = units.iter().map(cost).collect();
        let (order, bounds) =
            self.local.with_rng(|rng| plan_units(&costs, &self.shared.tuning, self.shared.workers, rng));
        let batches = bounds.len() - 1;
        let job = ForkJob {
            header: Header::new(exec_fork::<U, F>, batches),
            units: units.as_mut_ptr(),
            order,
            bounds,
            f: &f,
            unit_ns: unit_ns.as_mut_ptr(),
            family0,
            panic: Mutex::new(None),
        };
        let jp = (&raw const job).cast::<Header>();
        let sh = &*self.shared;
        let local = &self.local;
        assert!(sh.fork.try_reserve(), "fork slot busy");
        let abort = AbortOnUnwind;
        sh.fork_active.store(true, SeqCst);
        sh.fork.publish(jp.cast(), ANY, 0);
        sh.fork_pending.store(true, SeqCst);
        sh.wake(sh.wake_count(local, batches - 1));
        coordinate(sh, local, jp);
        sh.fork.free();
        sh.fork_active.store(false, SeqCst);
        if sh.hk_pending.load(SeqCst) != 0 {
            sh.wake(1);
        }
        mem::forget(abort);
        if let Some(p) = job.panic.into_inner().unwrap_or_else(PoisonError::into_inner) {
            panic::resume_unwind(p);
        }
        ForkReport { unit_ns, wall: start.elapsed() }
    }

    /// Runs a serial segment on the calling thread with a [`Ctx`], so it can open windows
    /// that idle workers help with.
    pub fn serial<R>(&mut self, f: impl FnOnce(&Ctx<'_>) -> R) -> R {
        self.bind();
        let family = self.families(1);
        f(&Ctx::new(&self.local, family))
    }

    /// [`Ctx::map_indexed`] from a serial segment.
    pub fn map_indexed<In, Out>(&mut self, items: &[In], run: impl Fn(&In) -> Out + Sync) -> Vec<Out>
    where
        In: Sync,
        Out: Send,
    {
        self.serial(|ctx| ctx.map_indexed(items, run))
    }

    /// Queues a lowest-priority job. Idle workers run housekeeping only while no
    /// [`run_units`](Self::run_units) is in progress, so keep jobs short (about a
    /// millisecond). A panicking job is counted in [`WorkerStats::housekeeping_panics`].
    pub fn spawn_housekeeping(&self, job: impl FnOnce() + Send + 'static) {
        self.shared.submit(Box::new(job));
    }

    /// A handle for queueing housekeeping from other threads.
    pub fn housekeeper(&self) -> Housekeeper {
        Housekeeper(self.shared.clone())
    }

    /// Runs queued housekeeping on the calling thread until the queue is empty or `until`
    /// has passed (useful with a single worker). Returns how many jobs ran.
    pub fn run_housekeeping(&mut self, until: Instant) -> usize {
        let mut n = 0;
        while Instant::now() < until && self.shared.run_housekeeping(&self.local) {
            n += 1;
        }
        n
    }

    /// Wakes every parked worker and keeps idle workers spinning for `hold`, so forks and
    /// windows in the next `hold` do not pay the OS wake-up (tens of µs). Meant for the start
    /// of a tick, just before its first serial segment; costs up to `hold` of CPU per worker.
    pub fn prewake(&self, hold: Duration) {
        let sh = &*self.shared;
        sh.hot_until.fetch_max(sh.now_ns().saturating_add(hold.as_nanos() as u64), Relaxed);
        sh.wake(sh.workers - 1);
    }

    /// Per-worker counters, index 0 being the coordinator.
    pub fn stats(&self) -> Vec<WorkerStats> {
        self.shared.stats.iter().map(|c| c.snapshot()).collect()
    }

    pub fn reset_stats(&self) {
        for c in self.shared.stats.iter() {
            c.reset();
        }
    }
}

impl Drop for TickPool {
    /// Stops the workers after they drained the housekeeping queue.
    fn drop(&mut self) {
        self.shared.shutdown.store(true, SeqCst);
        self.shared.wake_now(self.shared.workers);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        while self.shared.run_housekeeping(&self.local) {}
    }
}

/// Queues housekeeping from any thread. Jobs queued after the pool was dropped never run.
#[derive(Clone)]
pub struct Housekeeper(Arc<Shared>);

impl Housekeeper {
    pub fn spawn(&self, job: impl FnOnce() + Send + 'static) {
        self.0.submit(Box::new(job));
    }
}

/// Worker 0 during a fork: runs windows and batches like any worker, then waits for the
/// batches others took, helping any window meanwhile (it is not inside a unit).
fn coordinate(sh: &Shared, local: &WorkerLocal, jp: *const Header) {
    loop {
        if sh.timed(local, || sh.help_windows(local, ANY)) {
            continue;
        }
        // SAFETY: the fork job outlives this function and `p` comes from `claim`.
        let Some(p) = (unsafe { (*jp).claim() }) else { break };
        sh.timed(local, || {
            unsafe { Header::exec(jp, p, local) };
            true
        });
    }
    sh.fork_pending.store(false, SeqCst);
    if sh.fork.close() {
        return;
    }
    let mut idle = Idle::default();
    while !sh.fork.drained() {
        if sh.timed(local, || sh.help_windows(local, ANY)) {
            idle.reset();
        } else if !idle.spin(sh, false) {
            sh.sleep(local, || sh.live.load(SeqCst) != 0 || sh.fork.drained());
            idle.reset();
        }
    }
}

#[repr(C)]
struct ForkJob<'a, U, F> {
    header: Header,
    units: *mut U,
    /// A permutation of the unit indices: the start order.
    order: Vec<u32>,
    /// Batch `b` is `order[bounds[b]..bounds[b + 1]]`.
    bounds: Vec<u32>,
    f: &'a F,
    unit_ns: *mut u64,
    family0: u64,
    panic: Mutex<Option<Box<dyn Any + Send>>>,
}

unsafe fn exec_fork<U, F>(h: *const Header, batch: usize, local: &WorkerLocal)
where
    U: Send,
    F: Fn(&mut U, &Ctx<'_>) + Sync,
{
    // SAFETY: `h` points to a live ForkJob<U, F> (see `Header::exec`).
    let job = unsafe { &*h.cast::<ForkJob<'_, U, F>>() };
    let sh = &*local.shared;
    let (a, b) = (job.bounds[batch] as usize, job.bounds[batch + 1] as usize);
    for k in a..b {
        if k > a {
            // Between two units this worker is inside none, so it may help any window.
            sh.help_windows(local, ANY);
        }
        local.chaos_point();
        let idx = job.order[k] as usize;
        // SAFETY: `order` is a permutation and each batch is claimed once, so this is the only
        // reference to the unit; the slice outlives the fork.
        let unit = unsafe { &mut *job.units.add(idx) };
        let ctx = Ctx::new(local, job.family0 + idx as u64);
        let t = Instant::now();
        let r = panic::catch_unwind(AssertUnwindSafe(|| (job.f)(unit, &ctx)));
        // SAFETY: same exclusivity argument as for the unit.
        unsafe { *job.unit_ns.add(idx) = t.elapsed().as_nanos() as u64 };
        StatCells::add(&sh.stats[local.idx].units, 1);
        if let Err(p) = r {
            job.panic.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(p);
        }
    }
}

/// Start order (LPT: largest estimate first, ties by index) and batch boundaries. Consecutive
/// small units share a batch up to `unit_batch` of estimated cost, and a batch never holds more
/// than `n / (2 * workers)` units so poor estimates still leave work for every worker.
/// In chaos mode both the order and the batch sizes are random.
fn plan_units(costs: &[u64], t: &Tuning, workers: usize, rng: &mut Rng) -> (Vec<u32>, Vec<u32>) {
    let n = costs.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut bounds = vec![0u32];
    if t.chaos {
        rng.shuffle(&mut order);
        let mut i = 0;
        while i < n {
            i = (i + 1 + rng.below(4)).min(n);
            bounds.push(i as u32);
        }
        return (order, bounds);
    }
    order.sort_by(|&a, &b| costs[b as usize].cmp(&costs[a as usize]).then(a.cmp(&b)));
    let max_len = n.div_ceil(2 * workers).max(1);
    let mut i = 0;
    while i < n {
        let start = i;
        let mut sum = 0u64;
        while i < n {
            let c = costs[order[i] as usize];
            let big = c >= t.small_unit_ns;
            if i > start && (big || i - start >= max_len || sum.saturating_add(c) > t.unit_batch_ns) {
                break;
            }
            sum = sum.saturating_add(c);
            i += 1;
            if big {
                break;
            }
        }
        bounds.push(i as u32);
    }
    (order, bounds)
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn plan(costs: &[u64], workers: usize) -> (Vec<u32>, Vec<u32>) {
        let t = Tuning::new(&PoolConfig::new(workers));
        plan_units(costs, &t, workers, &mut Rng(1))
    }

    #[test]
    fn lpt_order_with_stable_ties() {
        let (order, _) = plan(&[5, 9_000_000, 7, 9_000_000, 0], 4);
        assert_eq!(order, [1, 3, 2, 0, 4]);
    }

    #[test]
    fn big_units_alone_small_units_batched() {
        let us = 1_000;
        // Two big units, then 40 units of 100 us: batches of at most ceil(42 / 8) = 6 units
        // and 1 ms of estimate.
        let mut costs = vec![100 * us; 40];
        costs.extend([5_000 * us, 300 * us]);
        let (order, bounds) = plan(&costs, 4);
        assert_eq!(&order[..2], &[40, 41]);
        assert_eq!(&bounds[..3], &[0, 1, 2]);
        for w in bounds[2..].windows(2) {
            assert!(w[1] - w[0] <= 6, "{bounds:?}");
        }
        assert_eq!(*bounds.last().unwrap(), 42);
        // Estimates of zero still spread over the workers.
        let (_, bounds) = plan(&[0; 50], 7);
        assert_eq!(bounds.len() - 1, 50usize.div_ceil(4));
    }

    #[test]
    fn chaos_plan_is_a_permutation_partitioned_into_batches() {
        let mut cfg = PoolConfig::new(4);
        cfg.chaos = Some(9);
        let t = Tuning::new(&cfg);
        for n in [1, 2, 17, 100] {
            let (mut order, bounds) = plan_units(&vec![1; n], &t, 4, &mut Rng(n as u64));
            order.sort_unstable();
            assert_eq!(order, (0..n as u32).collect::<Vec<_>>());
            assert_eq!(bounds[0], 0);
            assert_eq!(*bounds.last().unwrap(), n as u32);
            assert!(bounds.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
