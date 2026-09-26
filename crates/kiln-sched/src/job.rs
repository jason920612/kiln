//! The type-erased part of a published job: a claim counter over its pieces.

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;

use crate::pool::WorkerLocal;

/// First field of every published job (`#[repr(C)]`), so a `*const Header` derived from a
/// pointer to the whole job can be cast back by `exec`.
#[repr(C)]
pub(crate) struct Header {
    exec: unsafe fn(*const Header, usize, &WorkerLocal),
    next: AtomicUsize,
    count: usize,
}

impl Header {
    pub fn new(exec: unsafe fn(*const Header, usize, &WorkerLocal), count: usize) -> Self {
        Header { exec, next: AtomicUsize::new(0), count }
    }

    /// Claims the next unclaimed piece; every piece is handed out exactly once.
    pub fn claim(&self) -> Option<usize> {
        if self.next.load(Relaxed) >= self.count {
            return None;
        }
        let i = self.next.fetch_add(1, Relaxed);
        (i < self.count).then_some(i)
    }

    /// Runs a claimed piece.
    ///
    /// # Safety
    /// `this` must point to a live job of the type `exec` was instantiated for, with
    /// provenance over the whole job, and `piece` must have come from `claim`.
    pub unsafe fn exec(this: *const Header, piece: usize, local: &WorkerLocal) {
        unsafe { ((*this).exec)(this, piece, local) }
    }
}

/// Held while helpers may reference a job on this stack. Pool code between publishing and
/// draining does not panic (user panics are caught), so reaching this drop is a bug; aborting
/// is the only way to avoid a use-after-free in the helpers.
pub(crate) struct AbortOnUnwind;

impl Drop for AbortOnUnwind {
    fn drop(&mut self) {
        eprintln!("kiln-sched: unwinding while a published job is still shared; aborting");
        std::process::abort();
    }
}
