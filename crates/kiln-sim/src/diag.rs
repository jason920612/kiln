//! Finer phase timing for profiling (`KILN_PHASE_DETAIL=1`): named parts of the tick's phases,
//! summed over the tick and reported with the phases (sim_load prints them). Off, a part costs a
//! clock read.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::time::{Duration, Instant};

static SUMS: Mutex<Vec<(&'static str, Duration)>> = Mutex::new(Vec::new());
/// 0: not looked up yet, 1: off, 2: on.
static STATE: AtomicU8 = AtomicU8::new(0);

fn on() -> bool {
    match STATE.load(Relaxed) {
        0 => {
            let on = std::env::var_os("KILN_PHASE_DETAIL").is_some();
            STATE.store(if on { 2 } else { 1 }, Relaxed);
            on
        }
        s => s == 2,
    }
}

/// Adds `d` to the part `name`.
pub(crate) fn add(name: &'static str, d: Duration) {
    if on() {
        let mut s = SUMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match s.iter_mut().find(|(n, _)| *n == name) {
            Some((_, t)) => *t += d,
            None => s.push((name, d)),
        }
    }
}

/// Adds the time from `start` to now to the part `name`; returns now.
pub(crate) fn lap(name: &'static str, start: Instant) -> Instant {
    let now = Instant::now();
    add(name, now - start);
    now
}

/// The parts timed since the last call.
pub(crate) fn take() -> Vec<(&'static str, Duration)> {
    if !on() {
        return Vec::new();
    }
    std::mem::take(&mut *SUMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}
