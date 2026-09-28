//! Phase windows: an indexed map over items, split into chunks that idle workers may take.

use std::any::Any;
use std::marker::PhantomData;
use std::mem;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use crate::config::{PhaseMode, Strategy};
use crate::job::{AbortOnUnwind, Header};
use crate::pool::WorkerLocal;
use crate::stats::StatCells;

/// Per-window hints. All optional; none of them can change a result.
#[derive(Clone, Copy, Debug, Default)]
pub struct Window {
    item_ns: Option<u64>,
    chunk: Option<usize>,
    strategy: Option<Strategy>,
}

impl Window {
    pub const fn new() -> Self {
        Window { item_ns: None, chunk: None, strategy: None }
    }

    /// Estimated cost of one item. Without an estimate, [`PhaseMode::Auto`] times a prefix of
    /// the items and extrapolates.
    pub const fn item_ns(mut self, ns: u64) -> Self {
        self.item_ns = Some(ns);
        self
    }

    /// Items per chunk, instead of deriving it from the estimate and the chunk target.
    pub const fn chunk(mut self, items: usize) -> Self {
        self.chunk = Some(items);
        self
    }

    /// Forces this window's strategy under [`PhaseMode::Auto`]; other modes override it.
    pub const fn strategy(mut self, strategy: Strategy) -> Self {
        self.strategy = Some(strategy);
        self
    }
}

/// The context of the worker running a unit tick, a serial segment or a window item.
///
/// It is neither `Send` nor `Sync`: code running on another worker gets its own `Ctx` (the
/// `run` closure of [`map_indexed_with`](Self::map_indexed_with) receives one), so a window
/// always knows which worker waits for it and which unit it belongs to.
pub struct Ctx<'a> {
    local: &'a WorkerLocal,
    /// The unit (or serial segment) this code runs for; waiting here helps only this family.
    family: u64,
    _not_send: PhantomData<*mut ()>,
}

enum Plan {
    Inline,
    /// Chunk size, and the estimated work of the whole window when there is one.
    Parallel(usize, Option<u64>),
    /// Time a prefix, then decide; carries the chunk-size hint.
    Probe(Option<usize>),
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(local: &'a WorkerLocal, family: u64) -> Self {
        Ctx { local, family, _not_send: PhantomData }
    }

    /// Index of the worker running this code (0 is the coordinator).
    pub fn worker(&self) -> usize {
        self.local.idx
    }

    pub fn workers(&self) -> usize {
        self.local.shared.workers
    }

    /// Maps `items` in input order: `result[i] == run(&items[i])`.
    ///
    /// Short windows run inline. Longer ones are split into chunks of about
    /// [`PoolConfig::chunk_target`](crate::PoolConfig::chunk_target) that idle workers may
    /// take; the calling worker runs chunks too, and while it waits for the rest it only helps
    /// windows of its own unit, never starting another unit. A panic in `run` is resumed here
    /// once every chunk has finished or been skipped; outputs already produced are dropped.
    pub fn map_indexed<In, Out>(&self, items: &[In], run: impl Fn(&In) -> Out + Sync) -> Vec<Out>
    where
        In: Sync,
        Out: Send,
    {
        self.map_indexed_with(Window::new(), items, |_, x| run(x))
    }

    /// [`map_indexed`](Self::map_indexed) with hints, where `run` gets the executing worker's
    /// `Ctx` so items can open nested windows.
    pub fn map_indexed_with<In, Out, R>(&self, window: Window, items: &[In], run: R) -> Vec<Out>
    where
        In: Sync,
        Out: Send,
        R: Fn(&Ctx<'_>, &In) -> Out + Sync,
    {
        let mut out = Vec::with_capacity(items.len());
        let split = match self.plan(window, items.len()) {
            Plan::Inline => None,
            Plan::Parallel(chunk, est) => Some((chunk, est)),
            Plan::Probe(hint) => self.probe(items, &mut out, &run, hint),
        };
        match split {
            Some((chunk, est)) => self.parallel(items, &mut out, chunk.max(1), est, &run),
            None => out.extend(items[out.len()..].iter().map(|x| run(self, x))),
        }
        out
    }

    /// [`map_indexed_with`](Self::map_indexed_with) over exclusive borrows: `run` gets
    /// `&mut items[i]` and `result[i]` is its return value.
    ///
    /// Each item is handed to exactly one call (or to none, when another item panicked), so
    /// the `&mut` never aliases; items share nothing through this call, which is what keeps
    /// a window's result independent of the schedule. `T: Send` because the item may be
    /// mutated on another worker.
    pub fn map_mut_with<T, Out, R>(&self, window: Window, items: &mut [T], run: R) -> Vec<Out>
    where
        T: Send,
        Out: Send,
        R: Fn(&Ctx<'_>, &mut T) -> Out + Sync,
    {
        let ptrs: Vec<ItemPtr<T>> = items.iter_mut().map(|x| ItemPtr(x as *mut T)).collect();
        // SAFETY: `ptrs[i]` points to `items[i]`, which stays exclusively borrowed for this
        // call; the window runs each element of `ptrs` at most once, so no two `&mut` to the
        // same item exist at once.
        self.map_indexed_with(window, &ptrs, |ctx, p| run(ctx, unsafe { &mut *p.0 }))
    }

    /// [`map_mut_with`](Self::map_mut_with) without hints or a nested context.
    pub fn map_mut<T, Out>(&self, items: &mut [T], run: impl Fn(&mut T) -> Out + Sync) -> Vec<Out>
    where
        T: Send,
        Out: Send,
    {
        self.map_mut_with(Window::new(), items, |_, x| run(x))
    }

    fn plan(&self, w: Window, n: usize) -> Plan {
        let sh = &*self.local.shared;
        let t = &sh.tuning;
        if n < 2 {
            return Plan::Inline;
        }
        let even = n.div_ceil(4 * sh.workers);
        match t.phase {
            PhaseMode::Inline => return Plan::Inline,
            PhaseMode::Parallel => return Plan::Parallel(w.chunk.unwrap_or(even), None),
            PhaseMode::Mixed => {
                return match self.local.rand_below(3) {
                    0 => Plan::Inline,
                    1 => Plan::Parallel(w.chunk.unwrap_or(even), None),
                    _ => Plan::Parallel(1 + self.local.rand_below(n), None),
                };
            }
            PhaseMode::Auto => {}
        }
        if sh.workers == 1 {
            return Plan::Inline;
        }
        match (w.strategy, w.item_ns) {
            (Some(Strategy::Inline), _) => Plan::Inline,
            // Forced parallel: the caller asked for helpers, so they are not rationed.
            (Some(Strategy::Parallel), ns) => {
                Plan::Parallel(w.chunk.unwrap_or_else(|| ns.map_or(even, |ns| t.chunk_for(ns, n))), None)
            }
            (None, Some(ns)) if ns.saturating_mul(n as u64) < t.inline_below_ns => Plan::Inline,
            (None, Some(ns)) => {
                Plan::Parallel(w.chunk.unwrap_or_else(|| t.chunk_for(ns, n)), Some(ns.saturating_mul(n as u64)))
            }
            (None, None) => Plan::Probe(w.chunk),
        }
    }

    /// Maps items inline in blocks that double the count done, timing them. Once the prefix
    /// took long enough to extrapolate, returns a chunk size and the estimated rest if the rest
    /// is worth splitting.
    ///
    /// The rate is the fastest of the blocks holding at least an eighth of the prefix (the
    /// last block always qualifies): on a busy machine a block that lost its core to another
    /// process would otherwise make a short window look long, and a split window costs more
    /// CPU than an inline one.
    fn probe<In, Out, R>(
        &self,
        items: &[In],
        out: &mut Vec<Out>,
        run: &R,
        hint: Option<usize>,
    ) -> Option<(usize, Option<u64>)>
    where
        R: Fn(&Ctx<'_>, &In) -> Out,
    {
        let t = &self.local.shared.tuning;
        let n = items.len();
        let probe_ns = t.chunk_target_ns / 4;
        let start = Instant::now();
        // (items, ns) of each block so far; blocks double, so 64 always suffice.
        let mut blocks = [(0u64, 0u64); 64];
        let mut nblocks = 0;
        let mut lap = start;
        while out.len() < n {
            let from = out.len();
            let done = (2 * from).clamp(1, n);
            out.extend(items[from..done].iter().map(|x| run(self, x)));
            let now = Instant::now();
            blocks[nblocks] = ((done - from) as u64, (now - lap).as_nanos() as u64);
            nblocks += 1;
            lap = now;
            if ((now - start).as_nanos() as u64) < probe_ns {
                continue;
            }
            // The block with the lowest ns per item, compared by cross-multiplying.
            let (bi, bns) = blocks[..nblocks]
                .iter()
                .copied()
                .filter(|&(k, _)| 8 * k >= done as u64)
                .min_by(|a, b| (a.1 as u128 * b.0 as u128).cmp(&(b.1 as u128 * a.0 as u128)))
                .expect("the last block holds half the prefix");
            let rest = bns as u128 * (n - done) as u128 / bi as u128;
            if rest < t.inline_below_ns as u128 {
                return None;
            }
            let chunk = hint.unwrap_or_else(|| t.chunk_for(bns / bi, n));
            return Some((chunk, Some(rest.min(u64::MAX as u128) as u64)));
        }
        None
    }

    /// Publishes `items[out.len()..]` as a window, takes part in it and waits for it.
    ///
    /// With an estimate `est` of that work (automatic strategy only), it wakes only as many
    /// parked helpers as get [`PoolConfig::helper_share`](crate::PoolConfig::helper_share)
    /// each, and runs inline when that is none; workers already awake still join.
    fn parallel<In, Out, R>(&self, items: &[In], out: &mut Vec<Out>, size: usize, est: Option<u64>, run: &R)
    where
        In: Sync,
        Out: Send,
        R: Fn(&Ctx<'_>, &In) -> Out + Sync,
    {
        let sh = &*self.local.shared;
        let n = items.len();
        let base = out.len();
        let chunks = self.chunking(base, n - base, size);
        let mut wake = sh.wake_count(self.local, chunks.count.saturating_sub(1));
        if let Some(est) = est.filter(|_| !self.local.chaos()) {
            wake = wake.min((est / sh.tuning.helper_share_ns).saturating_sub(1) as usize);
            if wake == 0 {
                out.extend(items[base..].iter().map(|x| run(self, x)));
                return;
            }
        }
        let slot_idx = if chunks.count > 1 { sh.reserve_window(self.local) } else { None };
        // One chunk, or every slot taken by other windows: run inline, same result.
        let Some(si) = slot_idx else {
            out.extend(items[base..].iter().map(|x| run(self, x)));
            return;
        };
        let count = chunks.count;
        let job = WindowJob {
            header: Header::new(exec_window::<In, Out, R>, count),
            items,
            out: out.as_mut_ptr(),
            chunks,
            run,
            family: self.family,
            poisoned: AtomicBool::new(false),
            failed: Mutex::new(Failed::default()),
        };
        let jp = (&raw const job).cast::<Header>();
        let slot = &sh.windows[si];
        let abort = AbortOnUnwind;
        slot.publish(jp.cast(), self.family, self.local.idx);
        sh.live.fetch_or(1 << si, SeqCst);
        sh.wake(wake);
        while let Some(p) = job.header.claim() {
            // SAFETY: the job is alive and `p` was claimed.
            unsafe { Header::exec(jp, p, self.local) };
        }
        sh.live.fetch_and(!(1 << si), SeqCst);
        if !slot.close() {
            sh.wait_drained(slot, self.local, self.family);
        }
        slot.free();
        mem::forget(abort);
        let failed = job.failed.into_inner().unwrap_or_else(PoisonError::into_inner);
        if let Some(payload) = failed.payload {
            for c in (0..count).filter(|c| !failed.chunks.contains(c)) {
                let r = job.chunks.range(c, n);
                // SAFETY: chunk `c` completed, so exactly these slots hold initialized outputs.
                unsafe { ptr::drop_in_place(ptr::slice_from_raw_parts_mut(out.as_mut_ptr().add(r.start), r.len())) };
            }
            panic::resume_unwind(payload);
        }
        // SAFETY: every chunk completed, so `base..n` is initialized.
        unsafe { out.set_len(n) };
    }

    /// Chunks of `size` items over `base..base + m`; random boundaries and claim order in chaos
    /// mode.
    fn chunking(&self, base: usize, m: usize, size: usize) -> Chunks {
        if m < 2 {
            return Chunks { base, size: 1, bounds: None, order: None, count: m };
        }
        if !self.local.chaos() {
            return Chunks { base, size, bounds: None, order: None, count: m.div_ceil(size) };
        }
        self.local.with_rng(|rng| {
            let want = 1 + rng.below(m.min(4 * self.workers() + 4));
            let mut bounds: Vec<usize> = (1..want).map(|_| 1 + rng.below(m - 1)).collect();
            bounds.extend([0, m]);
            bounds.sort_unstable();
            bounds.dedup();
            let count = bounds.len() - 1;
            let mut order: Vec<u32> = (0..count as u32).collect();
            rng.shuffle(&mut order);
            Chunks { base, size, bounds: Some(bounds), order: Some(order), count }
        })
    }
}

struct Chunks {
    base: usize,
    size: usize,
    /// Chaos mode: chunk `c` is `base + bounds[c]..base + bounds[c + 1]`.
    bounds: Option<Vec<usize>>,
    /// Chaos mode: the chunk handed out for the `i`-th claim.
    order: Option<Vec<u32>>,
    count: usize,
}

impl Chunks {
    fn range(&self, chunk: usize, n: usize) -> Range<usize> {
        match &self.bounds {
            Some(b) => self.base + b[chunk]..self.base + b[chunk + 1],
            None => {
                let start = self.base + chunk * self.size;
                start..(start + self.size).min(n)
            }
        }
    }

    fn chunk_of(&self, piece: usize) -> usize {
        self.order.as_ref().map_or(piece, |o| o[piece] as usize)
    }
}

#[derive(Default)]
struct Failed {
    payload: Option<Box<dyn Any + Send>>,
    /// Chunks that panicked or were skipped; their outputs are not initialized.
    chunks: Vec<usize>,
}

#[repr(C)]
struct WindowJob<'a, In, Out, R> {
    header: Header,
    items: &'a [In],
    out: *mut Out,
    chunks: Chunks,
    run: &'a R,
    family: u64,
    poisoned: AtomicBool,
    failed: Mutex<Failed>,
}

impl<In, Out, R> WindowJob<'_, In, Out, R> {
    fn fail(&self, chunk: usize, payload: Option<Box<dyn Any + Send>>) {
        let mut f = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
        f.chunks.push(chunk);
        if f.payload.is_none() {
            f.payload = payload;
        }
    }
}

unsafe fn exec_window<In, Out, R>(h: *const Header, piece: usize, local: &WorkerLocal)
where
    In: Sync,
    Out: Send,
    R: Fn(&Ctx<'_>, &In) -> Out + Sync,
{
    // SAFETY: `h` points to a live WindowJob<In, Out, R> (see `Header::exec`).
    let job = unsafe { &*h.cast::<WindowJob<'_, In, Out, R>>() };
    local.chaos_point();
    let chunk = job.chunks.chunk_of(piece);
    if job.poisoned.load(Relaxed) {
        job.fail(chunk, None);
        return;
    }
    let range = job.chunks.range(chunk, job.items.len());
    let ctx = Ctx::new(local, job.family);
    let res = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: chunks are disjoint and each is claimed once, so only this call writes these
        // slots; they lie within the output's capacity.
        let mut written = Written { at: unsafe { job.out.add(range.start) }, len: 0 };
        for item in &job.items[range.clone()] {
            let v = (job.run)(&ctx, item);
            unsafe { written.at.add(written.len).write(v) };
            written.len += 1;
        }
        mem::forget(written);
    }));
    StatCells::add(&local.shared.stats[local.idx].chunks, 1);
    if let Err(p) = res {
        job.poisoned.store(true, Relaxed);
        job.fail(chunk, Some(p));
    }
}

/// One item of a [`Ctx::map_mut_with`] window.
struct ItemPtr<T>(*mut T);

// SAFETY: an `ItemPtr` is only dereferenced by the single call that runs its item, which then
// holds the only reference to a `T: Send`.
unsafe impl<T: Send> Sync for ItemPtr<T> {}
unsafe impl<T: Send> Send for ItemPtr<T> {}

/// Drops the outputs a chunk wrote before one of its items panicked.
struct Written<T> {
    at: *mut T,
    len: usize,
}

impl<T> Drop for Written<T> {
    fn drop(&mut self) {
        // SAFETY: the first `len` slots were written by this chunk and not yet handed out.
        unsafe { ptr::drop_in_place(ptr::slice_from_raw_parts_mut(self.at, self.len)) };
    }
}
