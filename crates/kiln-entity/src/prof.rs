//! Opt-in cost counters: `cargo run --release -p kiln-sim --features prof --example sim_load ...`
//! prints, per named scope, how often it ran and the inclusive wall time it took (scopes nest,
//! so a parent's time includes its children's). Without the `prof` feature a scope is not even
//! created: `prof!` compiles to nothing.

/// Times the rest of the enclosing block under `name` (a `&'static str`), optionally with a tag
/// that tells apart scopes of one name (`prof!("start", b.name())`).
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
    pub fn report(_ticks: u64) {}

    pub fn reset() {}
}

#[cfg(feature = "prof")]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    type Key = (&'static str, &'static str);
    type Counters = Arc<Mutex<HashMap<Key, (u64, u64)>>>;

    static ALL: Mutex<Vec<Counters>> = Mutex::new(Vec::new());

    thread_local! {
        static LOCAL: RefCell<Option<Counters>> = const { RefCell::new(None) };
    }

    fn add(key: Key, nanos: u64) {
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            let c = l.get_or_insert_with(|| {
                let c: Counters = Arc::default();
                ALL.lock().unwrap().push(c.clone());
                c
            });
            let mut m = c.lock().unwrap();
            let e = m.entry(key).or_default();
            e.0 += 1;
            e.1 += nanos;
        });
    }

    pub struct Scope(Key, Instant);

    pub fn scope(tag: &'static str, name: &'static str) -> Scope {
        Scope((tag, name), Instant::now())
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            add(self.0, self.1.elapsed().as_nanos() as u64);
        }
    }

    pub fn reset() {
        for c in ALL.lock().unwrap().iter() {
            c.lock().unwrap().clear();
        }
    }

    /// Prints the scopes by inclusive time, per tick of `ticks` measured ticks.
    pub fn report(ticks: u64) {
        let mut total: HashMap<Key, (u64, u64)> = HashMap::new();
        for c in ALL.lock().unwrap().iter() {
            for (k, v) in c.lock().unwrap().iter() {
                let e = total.entry(*k).or_default();
                e.0 += v.0;
                e.1 += v.1;
            }
        }
        let mut rows: Vec<_> = total.into_iter().collect();
        rows.sort_by_key(|(_, (_, n))| std::cmp::Reverse(*n));
        let t = ticks.max(1) as f64;
        println!("{:<52} {:>10} {:>10} {:>10}", "scope (inclusive)", "calls/tick", "us/call", "ms/tick");
        for ((tag, name), (n, nanos)) in rows.iter().take(70) {
            println!("{:<52} {:>10.1} {:>10.2} {:>10.3}", format!("{tag} {name}"), *n as f64 / t, *nanos as f64 / *n as f64 / 1e3, *nanos as f64 / t / 1e6);
        }
    }
}

pub use imp::{report, reset};
#[cfg(feature = "prof")]
pub use imp::{Scope, scope};
