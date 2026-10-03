//! Opt-in sampling profile: `cargo run --release -p kiln-sim --features prof --example sim_load ...`.
//!
//! `prof!` marks the rest of the enclosing block as a named scope; a sampler thread looks at what
//! each thread is inside every few microseconds and counts the innermost scope, so the report
//! (ms per tick, `report`) is the time spent in a scope and not in a scope nested in it. A scope
//! costs one thread-local store when entered and left; without the `prof` feature it is not even
//! compiled (`prof!` expands to nothing).

/// Marks the rest of the enclosing block as `name` (a `&'static str`), optionally with a tag that
/// tells apart scopes of one name (`prof!("start", b.name())`).
#[macro_export]
macro_rules! prof {
    ($name:expr) => {
        #[cfg(feature = "prof")]
        let _prof_scope = $crate::prof::scope("", $name);
    };
    ($tag:expr, $name:expr) => {
        #[cfg(feature = "prof")]
        let _prof_scope = $crate::prof::scope($tag, $name);
    };
}

#[cfg(not(feature = "prof"))]
mod imp {
    pub fn start() {}

    pub fn report(_ticks: u64) {}
}

#[cfg(feature = "prof")]
mod imp {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// What each thread is inside (0: nothing): the name's address, its length and its tag.
    static CELLS: Mutex<Vec<&'static AtomicU64>> = Mutex::new(Vec::new());

    thread_local! {
        static CELL: Cell<Option<&'static AtomicU64>> = const { Cell::new(None) };
    }

    const TAGS: [&str; 13] = ["", "mob", "brain", "start", "tick", "sensor", "gate child", "mv", "path", "util", "lvl", "jump", "col"];

    /// The index in `TAGS`: a match on the first byte and length, so a literal tag costs nothing.
    #[inline(always)]
    fn tag_code(tag: &str) -> u64 {
        match (tag.as_bytes().first(), tag.len()) {
            (Some(b'm'), 3) => 1,
            (Some(b'm'), 2) => 7,
            (Some(b'b'), _) => 2,
            (Some(b's'), 5) => 3,
            (Some(b't'), _) => 4,
            (Some(b's'), 6) => 5,
            (Some(b'g'), _) => 6,
            (Some(b'p'), _) => 8,
            (Some(b'u'), _) => 9,
            (Some(b'l'), _) => 10,
            (Some(b'j'), _) => 11,
            (Some(b'c'), _) => 12,
            _ => 0,
        }
    }

    #[inline(always)]
    fn pack(tag: &'static str, name: &'static str) -> u64 {
        (name.as_ptr() as u64 & 0xFFFF_FFFF_FFFF) | (name.len().min(255) as u64) << 48 | tag_code(tag) << 56
    }

    fn unpack(v: u64) -> (&'static str, String) {
        let (ptr, len, tag) = (v & 0xFFFF_FFFF_FFFF, (v >> 48) & 0xFF, (v >> 56) as usize);
        // SAFETY: only `pack` makes these, from a `&'static str`.
        let name = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr as *const u8, len as usize)) };
        (TAGS[tag], name.to_owned())
    }

    fn cell() -> &'static AtomicU64 {
        CELL.with(|c| match c.get() {
            Some(cell) => cell,
            None => {
                let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
                CELLS.lock().unwrap().push(cell);
                c.set(Some(cell));
                cell
            }
        })
    }

    pub struct Scope {
        cell: &'static AtomicU64,
        previous: u64,
    }

    #[inline(always)]
    pub fn scope(tag: &'static str, name: &'static str) -> Scope {
        let cell = cell();
        let previous = cell.load(Relaxed);
        cell.store(pack(tag, name), Relaxed);
        Scope { cell, previous }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            self.cell.store(self.previous, Relaxed);
        }
    }

    struct Sampler {
        stop: &'static AtomicBool,
        thread: std::thread::JoinHandle<(HashMap<u64, u64>, u64, Duration)>,
    }

    static SAMPLER: Mutex<Option<Sampler>> = Mutex::new(None);

    /// Starts counting (the measured part of a run).
    pub fn start() {
        static STOP: OnceLock<&'static AtomicBool> = OnceLock::new();
        let stop = *STOP.get_or_init(|| Box::leak(Box::new(AtomicBool::new(false))));
        stop.store(false, Relaxed);
        let thread = std::thread::spawn(move || {
            let mut counts: HashMap<u64, u64> = HashMap::new();
            let mut cells: Vec<&'static AtomicU64> = Vec::new();
            let (mut loops, began) = (0u64, Instant::now());
            while !stop.load(Relaxed) {
                if loops % 2048 == 0 {
                    cells = CELLS.lock().unwrap().clone();
                }
                for c in &cells {
                    let v = c.load(Relaxed);
                    if v != 0 {
                        *counts.entry(v).or_default() += 1;
                    }
                }
                loops += 1;
                let t = Instant::now();
                while t.elapsed() < Duration::from_micros(20) {
                    std::hint::spin_loop();
                }
            }
            (counts, loops, began.elapsed())
        });
        *SAMPLER.lock().unwrap() = Some(Sampler { stop, thread });
    }

    /// Stops counting and prints the scopes by the time spent directly in them, per tick of
    /// `ticks` measured ticks.
    pub fn report(ticks: u64) {
        let Some(s) = SAMPLER.lock().unwrap().take() else { return };
        s.stop.store(true, Relaxed);
        let (counts, loops, elapsed) = s.thread.join().unwrap();
        let per_sample_ms = elapsed.as_secs_f64() * 1e3 / loops.max(1) as f64;
        let t = ticks.max(1) as f64;
        let mut rows: Vec<_> = counts.into_iter().map(|(k, n)| (unpack(k), n)).collect();
        rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let total: u64 = rows.iter().map(|(_, n)| n).sum();
        println!("sampled {loops} times, {:.1} us apart; {:.3} ms/tick inside scopes", per_sample_ms * 1e3, total as f64 * per_sample_ms / t);
        println!("{:<56} {:>10}", "scope (time directly in it)", "ms/tick");
        for ((tag, name), n) in rows.iter().take(60) {
            println!("{:<56} {:>10.3}", format!("{tag} {name}"), *n as f64 * per_sample_ms / t);
        }
    }
}

pub use imp::{report, start};
#[cfg(feature = "prof")]
pub use imp::{Scope, scope};
