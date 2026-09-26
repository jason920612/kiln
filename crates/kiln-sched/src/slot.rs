//! Publication slots: how a job living on its owner's stack is shared with helpers.
//!
//! The owner reserves a free slot, publishes a pointer to its job, and later closes the slot.
//! Helpers enter only while the slot is live and hold a reference for as long as they touch
//! the job. The owner keeps the job alive until every helper has left (`drained`), so a
//! helper never sees a dangling pointer. The last helper to leave a closed slot learns which
//! worker to unpark. The slot itself belongs to the pool and outlives every job.
//!
//! A helper may race with the slot being retired and republished; the reference count makes
//! it enter whichever job is published at that moment, and it reads the job pointer only
//! after entering, so it always sees a job that stays alive while it is inside.

use crate::sync::Ordering::{AcqRel, Acquire, Relaxed, Release};
use crate::sync::{AtomicPtr, AtomicU64, AtomicUsize};

const LIVE: usize = 1;
const RESERVED: usize = 2;
const REF: usize = 4;

pub(crate) struct Slot {
    /// `refs * REF | RESERVED | LIVE`; zero when free.
    state: AtomicUsize,
    job: AtomicPtr<()>,
    family: AtomicU64,
    owner: AtomicUsize,
}

pub(crate) struct Entered {
    pub job: *const (),
    pub family: u64,
}

impl Slot {
    pub fn new() -> Self {
        Slot {
            state: AtomicUsize::new(0),
            job: AtomicPtr::new(std::ptr::null_mut()),
            family: AtomicU64::new(0),
            owner: AtomicUsize::new(0),
        }
    }

    pub fn try_reserve(&self) -> bool {
        self.state.compare_exchange(0, RESERVED, Acquire, Relaxed).is_ok()
    }

    /// Makes `job` visible to helpers. The caller must hold the reservation and keep the job
    /// alive until `drained` returns true after `close`.
    pub fn publish(&self, job: *const (), family: u64, owner: usize) {
        self.job.store(job.cast_mut(), Relaxed);
        self.family.store(family, Relaxed);
        self.owner.store(owner, Relaxed);
        self.state.store(RESERVED | LIVE, Release);
    }

    /// The published family, possibly stale; only for skipping slots cheaply.
    pub fn family_hint(&self) -> u64 {
        self.family.load(Relaxed)
    }

    pub fn enter(&self) -> Option<Entered> {
        let mut s = self.state.load(Relaxed);
        while s & LIVE != 0 {
            match self.state.compare_exchange_weak(s, s + REF, Acquire, Relaxed) {
                Ok(_) => {
                    return Some(Entered { job: self.job.load(Relaxed), family: self.family.load(Relaxed) });
                }
                Err(cur) => s = cur,
            }
        }
        None
    }

    /// Leaves an entered slot. Returns the owner to unpark when it is waiting for this helper.
    pub fn leave(&self) -> Option<usize> {
        let owner = self.owner.load(Relaxed);
        let prev = self.state.fetch_sub(REF, Release);
        (prev == RESERVED + REF).then_some(owner)
    }

    /// Stops new helpers from entering; returns whether none is inside.
    pub fn close(&self) -> bool {
        self.state.fetch_and(!LIVE, AcqRel) & !LIVE == RESERVED
    }

    /// Whether a closed slot has no helper left; the job may then be dropped.
    pub fn drained(&self) -> bool {
        self.state.load(Acquire) == RESERVED
    }

    pub fn free(&self) {
        self.state.store(0, Release);
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::Slot;
    use loom::cell::UnsafeCell;
    use loom::sync::Arc;
    use loom::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use loom::thread;

    struct Job {
        next: AtomicUsize,
        out: [UnsafeCell<u32>; 2],
    }

    impl Job {
        fn new() -> Self {
            Job { next: AtomicUsize::new(0), out: [UnsafeCell::new(0), UnsafeCell::new(0)] }
        }

        fn work(&self) {
            loop {
                let i = self.next.fetch_add(1, Relaxed);
                if i >= self.out.len() {
                    return;
                }
                self.out[i].with_mut(|v| unsafe { *v += 1 });
            }
        }

        /// Reads every output non-atomically: loom flags a race if a helper's write is not
        /// ordered before the owner's drain check.
        fn check(&self) {
            for o in &self.out {
                assert_eq!(o.with(|v| unsafe { *v }), 1);
            }
        }
    }

    fn help(slot: &Slot, jobs: &[Arc<Job>], owner: &thread::Thread) {
        if let Some(e) = slot.enter() {
            let job = &jobs[e.family as usize];
            assert!(std::ptr::eq(e.job, Arc::as_ptr(job).cast()));
            job.work();
            if let Some(o) = slot.leave() {
                assert_eq!(o, 0);
                owner.unpark();
            }
        }
    }

    fn own(slot: &Slot, job: &Job) {
        job.work();
        if !slot.close() {
            while !slot.drained() {
                thread::park();
            }
        }
        job.check();
        slot.free();
    }

    /// One owner, one helper: the owner never finishes while the helper is inside, and it sees
    /// every write the helper made.
    #[test]
    fn owner_waits_for_helper() {
        loom::model(|| {
            let slot = Arc::new(Slot::new());
            let jobs = Arc::new([Arc::new(Job::new())]);
            assert!(slot.try_reserve());
            slot.publish(Arc::as_ptr(&jobs[0]).cast(), 0, 0);
            let owner = thread::current();
            let h = {
                let (slot, jobs) = (slot.clone(), jobs.clone());
                thread::spawn(move || help(&slot, &jobs[..], &owner))
            };
            own(&slot, &jobs[0]);
            h.join().unwrap();
        });
    }

    /// The slot is retired and republished while a helper may still be racing to enter the
    /// first job; the helper enters at most one of them and both complete exactly once.
    #[test]
    fn republish_while_helper_races() {
        loom::model(|| {
            let slot = Arc::new(Slot::new());
            let jobs = Arc::new([Arc::new(Job::new()), Arc::new(Job::new())]);
            let owner = thread::current();
            assert!(slot.try_reserve());
            slot.publish(Arc::as_ptr(&jobs[0]).cast(), 0, 0);
            let h = {
                let (slot, jobs) = (slot.clone(), jobs.clone());
                thread::spawn(move || help(&slot, &jobs[..], &owner))
            };
            own(&slot, &jobs[0]);
            assert!(slot.try_reserve());
            slot.publish(Arc::as_ptr(&jobs[1]).cast(), 1, 0);
            own(&slot, &jobs[1]);
            h.join().unwrap();
        });
    }
}
