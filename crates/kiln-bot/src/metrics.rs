//! Counters shared by all bots and the reports built from them.
//!
//! Bots count traffic locally and flush it here once per tick, so the hot path touches no
//! shared cache lines.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

macro_rules! traffic {
    ($($field:ident),* $(,)?) => {
        /// Traffic counters; per bot until flushed, then totals.
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
        pub struct Traffic {
            $(pub $field: u64,)*
        }

        #[derive(Debug, Default)]
        struct SharedTraffic {
            $($field: AtomicU64,)*
        }

        impl SharedTraffic {
            fn add(&self, t: &Traffic) {
                $(if t.$field != 0 {
                    self.$field.fetch_add(t.$field, Relaxed);
                })*
            }

            fn load(&self) -> Traffic {
                Traffic { $($field: self.$field.load(Relaxed),)* }
            }
        }

        impl Traffic {
            /// Adds another set of counters.
            pub fn merge(&mut self, o: &Traffic) {
                $(self.$field += o.$field;)*
            }

            fn since(&self, earlier: &Traffic) -> Traffic {
                Traffic { $($field: self.$field - earlier.$field,)* }
            }
        }
    };
}

traffic!(
    rx_packets,
    rx_bytes,
    tx_packets,
    tx_bytes,
    chunks,
    teleports,
    chat_sent,
    chat_received,
    connected_nanos,
    // Survival bots.
    decode_errors,
    commands,
    own_teleports,
    dig_started,
    dig_done,
    dig_rejected,
    placed,
    place_rejected,
    place_wrong_state,
    deaths,
    respawns,
    containers_opened,
    items_moved,
    eaten,
    attacks,
    levers,
    walk_ticks,
    stall_ticks,
    walked_dm,
    teleport_resends,
    blocks_seen_changed,
);

#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub launched: AtomicU64,
    pub connected: AtomicU64,
    pub online: AtomicU64,
    pub joined: AtomicU64,
    pub failed: AtomicU64,
    pub dropped: AtomicU64,
    /// Survival bots that reached their site and started to play.
    pub arrived: AtomicU64,
    traffic: SharedTraffic,
    reasons: Mutex<BTreeMap<String, u64>>,
    join_ms: Mutex<Vec<f64>>,
    /// Shared centre of the movement scripts: configured, or where the first bot spawned.
    pub origin: OnceLock<[f64; 2]>,
    /// Chunk Data arrival latency: from the chunk entering the view until it arrived.
    pub chunk_latency: Hist,
    /// From a bot's chunk position changing (walking over a chunk border) until every chunk
    /// of its new view had arrived.
    pub area_ready_walk: Hist,
    /// The same after a teleport (the view jumped).
    pub area_ready_teleport: Hist,
    /// From the first Chunk Batch Start after joining until the whole first view had arrived.
    pub area_ready_join: Hist,
    /// Distinct decode problems seen, by text, with how often.
    pub problems: Mutex<BTreeMap<String, u64>>,
}

/// 1 ms buckets up to 10 s, then 100 ms buckets up to 20 minutes.
const HIST_FINE: usize = 10_000;
const HIST_MS: usize = HIST_FINE + 12_000 + 1;

fn bucket(ms: usize) -> usize {
    if ms < HIST_FINE { ms } else { (HIST_FINE + (ms - HIST_FINE) / 100).min(HIST_MS - 1) }
}

fn bucket_ms(i: usize) -> f64 {
    if i < HIST_FINE { i as f64 } else { (HIST_FINE + (i - HIST_FINE) * 100) as f64 }
}

/// Milliseconds histogram (see [`bucket`]); values past twenty minutes land in the last bucket.
pub struct Hist {
    buckets: Box<[AtomicU64]>,
    sum_us: AtomicU64,
}

impl Default for Hist {
    fn default() -> Self {
        Self { buckets: (0..HIST_MS).map(|_| AtomicU64::new(0)).collect(), sum_us: AtomicU64::new(0) }
    }
}

impl std::fmt::Debug for Hist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hist({} samples)", self.summary().n)
    }
}

/// Count, mean, percentiles and maximum of a [`Hist`], in milliseconds.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Latency {
    pub n: u64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Hist {
    pub fn add(&self, d: Duration) {
        self.buckets[bucket(d.as_millis() as usize)].fetch_add(1, Relaxed);
        self.sum_us.fetch_add(d.as_micros() as u64, Relaxed);
    }

    pub fn summary(&self) -> Latency {
        let counts: Vec<u64> = self.buckets.iter().map(|b| b.load(Relaxed)).collect();
        let n: u64 = counts.iter().sum();
        if n == 0 {
            return Latency::default();
        }
        let at = |q: f64| {
            let want = ((n as f64 * q).ceil() as u64).max(1);
            let mut seen = 0;
            for (ms, c) in counts.iter().enumerate() {
                seen += c;
                if seen >= want {
                    return bucket_ms(ms);
                }
            }
            bucket_ms(HIST_MS - 1)
        };
        Latency {
            n,
            mean_ms: self.sum_us.load(Relaxed) as f64 / n as f64 / 1e3,
            p50_ms: at(0.5),
            p90_ms: at(0.9),
            p99_ms: at(0.99),
            max_ms: bucket_ms(counts.iter().rposition(|c| *c > 0).unwrap_or(0)),
        }
    }
}

impl Shared {
    /// Notes a problem (a packet the bot could not decode, ...) once per distinct text.
    pub fn problem(&self, text: String) {
        let mut p = self.problems.lock().unwrap();
        if p.len() < 50 || p.contains_key(&text) {
            *p.entry(text).or_default() += 1;
        }
    }
}

/// Counter values at one instant.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Snapshot {
    at: Instant,
    launched: u64,
    connected: u64,
    online: u64,
    joined: u64,
    failed: u64,
    dropped: u64,
    arrived: u64,
    traffic: Traffic,
}

impl Shared {
    /// Adds a bot's local counters to the totals and resets them.
    pub fn flush(&self, t: &mut Traffic) {
        self.traffic.add(t);
        *t = Traffic::default();
    }

    pub fn record_join(&self, latency: Duration) {
        self.joined.fetch_add(1, Relaxed);
        self.online.fetch_add(1, Relaxed);
        self.join_ms.lock().unwrap().push(latency.as_secs_f64() * 1e3);
    }

    /// A bot's connection ended; `reason` is `None` when the run stopped it.
    pub fn record_end(&self, joined: bool, reason: Option<String>) {
        if joined {
            self.online.fetch_sub(1, Relaxed);
        }
        if let Some(reason) = reason {
            let counter = if joined { &self.dropped } else { &self.failed };
            counter.fetch_add(1, Relaxed);
            *self.reasons.lock().unwrap().entry(reason).or_default() += 1;
        }
    }

    /// Final counters: connection counts as of the stop, traffic including the bots' last flush.
    pub fn final_snapshot(&self, at_stop: &Snapshot) -> Snapshot {
        let now = self.snapshot();
        Snapshot { failed: now.failed, dropped: now.dropped, traffic: now.traffic, ..*at_stop }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            at: Instant::now(),
            launched: self.launched.load(Relaxed),
            connected: self.connected.load(Relaxed),
            online: self.online.load(Relaxed),
            joined: self.joined.load(Relaxed),
            failed: self.failed.load(Relaxed),
            dropped: self.dropped.load(Relaxed),
            arrived: self.arrived.load(Relaxed),
            traffic: self.traffic.load(),
        }
    }

    fn reasons(&self) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self.reasons.lock().unwrap().iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    fn join_percentiles(&self) -> [Option<f64>; 3] {
        let mut v = self.join_ms.lock().unwrap().clone();
        v.sort_by(f64::total_cmp);
        [percentile(&v, 0.50), percentile(&v, 0.99), v.last().copied()]
    }
}

/// Nearest-rank percentile of sorted values.
fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.max(1) - 1).copied()
}

/// State of a run. Counts are cumulative; rates cover `window_secs` before `elapsed_secs`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub elapsed_secs: f64,
    pub window_secs: f64,
    /// Bots started so far.
    pub launched: u64,
    /// Bots with an open connection.
    pub connected: u64,
    /// Bots that joined and are still connected.
    pub online: u64,
    /// Bots that reached the world (first chunk batch after spawning).
    pub joined: u64,
    /// Connections that ended before joining.
    pub failed: u64,
    /// Connections that ended after joining, before the run stopped.
    pub dropped: u64,
    /// Survival bots at their site, playing.
    pub arrived: u64,
    /// Why connections ended, most frequent first, prefixed with the protocol state.
    pub disconnect_reasons: Vec<(String, u64)>,
    pub traffic: Traffic,
    pub rx_packets_per_sec: f64,
    pub rx_bytes_per_sec: f64,
    /// Rates divided by connected bot-seconds in the window.
    pub rx_packets_per_sec_per_bot: f64,
    pub rx_bytes_per_sec_per_bot: f64,
    pub tx_packets_per_sec: f64,
    /// From starting to connect until the first chunk batch after spawning.
    pub join_ms_p50: Option<f64>,
    pub join_ms_p99: Option<f64>,
    pub join_ms_max: Option<f64>,
    /// Survival bots: chunk arrival latency and the time until a new view was complete.
    pub chunk_latency: Latency,
    pub area_ready_walk: Latency,
    pub area_ready_teleport: Latency,
    pub area_ready_join: Latency,
    pub problems: Vec<(String, u64)>,
}

impl Report {
    pub(crate) fn new(shared: &Shared, start: Instant, from: &Snapshot, to: &Snapshot) -> Self {
        let window = to.at.saturating_duration_since(from.at).as_secs_f64().max(1e-9);
        let delta = to.traffic.since(&from.traffic);
        let bot_secs = (delta.connected_nanos as f64 / 1e9).max(1e-9);
        let [p50, p99, max] = shared.join_percentiles();
        Self {
            elapsed_secs: to.at.saturating_duration_since(start).as_secs_f64(),
            window_secs: window,
            launched: to.launched,
            connected: to.connected,
            online: to.online,
            joined: to.joined,
            failed: to.failed,
            dropped: to.dropped,
            arrived: to.arrived,
            disconnect_reasons: shared.reasons(),
            traffic: to.traffic,
            rx_packets_per_sec: delta.rx_packets as f64 / window,
            rx_bytes_per_sec: delta.rx_bytes as f64 / window,
            rx_packets_per_sec_per_bot: delta.rx_packets as f64 / bot_secs,
            rx_bytes_per_sec_per_bot: delta.rx_bytes as f64 / bot_secs,
            tx_packets_per_sec: delta.tx_packets as f64 / window,
            join_ms_p50: p50,
            join_ms_p99: p99,
            join_ms_max: max,
            chunk_latency: shared.chunk_latency.summary(),
            area_ready_walk: shared.area_ready_walk.summary(),
            area_ready_teleport: shared.area_ready_teleport.summary(),
            area_ready_join: shared.area_ready_join.summary(),
            problems: shared.problems.lock().unwrap().iter().map(|(k, v)| (k.clone(), *v)).collect(),
        }
    }

    /// One line for periodic progress output.
    pub fn summary_line(&self) -> String {
        format!(
            "[{:6.1}s] bots {} conn {} online {} joined {} arrived {} failed {} dropped {} | rx {}/s {}/s, per bot {}/s {}/s \
             | tx {}/s | chunks {} | join p50 {} p99 {}",
            self.elapsed_secs,
            self.launched,
            self.connected,
            self.online,
            self.joined,
            self.arrived,
            self.failed,
            self.dropped,
            count(self.rx_packets_per_sec),
            bytes(self.rx_bytes_per_sec),
            count(self.rx_packets_per_sec_per_bot),
            bytes(self.rx_bytes_per_sec_per_bot),
            count(self.tx_packets_per_sec),
            self.traffic.chunks,
            ms(self.join_ms_p50),
            ms(self.join_ms_p99),
        )
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = &self.traffic;
        writeln!(f, "kiln-bot report after {:.1} s (rates over {:.1} s)", self.elapsed_secs, self.window_secs)?;
        writeln!(
            f,
            "  bots       launched {}, joined {}, failed {}, dropped {}; at stop: connected {}, online {}",
            self.launched, self.joined, self.failed, self.dropped, self.connected, self.online
        )?;
        writeln!(
            f,
            "  join       p50 {}, p99 {}, max {} (connect to first chunk batch after spawning)",
            ms(self.join_ms_p50),
            ms(self.join_ms_p99),
            ms(self.join_ms_max)
        )?;
        writeln!(
            f,
            "  received   {} packets, {}: {} packets/s, {}/s; per bot {} packets/s, {}/s",
            t.rx_packets,
            bytes(t.rx_bytes as f64),
            count(self.rx_packets_per_sec),
            bytes(self.rx_bytes_per_sec),
            count(self.rx_packets_per_sec_per_bot),
            bytes(self.rx_bytes_per_sec_per_bot)
        )?;
        writeln!(
            f,
            "  sent       {} packets, {}: {} packets/s",
            t.tx_packets,
            bytes(t.tx_bytes as f64),
            count(self.tx_packets_per_sec)
        )?;
        writeln!(f, "  chunks     {}", t.chunks)?;
        if t.walk_ticks + t.stall_ticks > 0 || self.chunk_latency.n > 0 {
            let lat = |name: &str, l: &Latency| {
                format!(
                    "  {name:<19} n {} mean {:.0} p50 {:.0} p90 {:.0} p99 {:.0} max {:.0} ms
",
                    l.n, l.mean_ms, l.p50_ms, l.p90_ms, l.p99_ms, l.max_ms
                )
            };
            write!(f, "{}", lat("chunk arrival", &self.chunk_latency))?;
            write!(f, "{}", lat("view ready (walk)", &self.area_ready_walk))?;
            write!(f, "{}", lat("view ready (tp)", &self.area_ready_teleport))?;
            write!(f, "{}", lat("view ready (join)", &self.area_ready_join))?;
            writeln!(
                f,
                "  walking    {} ticks moving ({:.0} blocks), {} ticks waiting for chunks",
                t.walk_ticks, t.walked_dm as f64 / 10.0, t.stall_ticks
            )?;
            writeln!(
                f,
                "  survival   dig {} started / {} done / {} rejected; placed {} (rejected {}, wrong state {}); deaths {};                  commands {}; containers {} ({} stacks moved); eaten {}; attacks {}; levers {}; decode errors {}",
                t.dig_started,
                t.dig_done,
                t.dig_rejected,
                t.placed,
                t.place_rejected,
                t.place_wrong_state,
                t.deaths,
                t.commands,
                t.containers_opened,
                t.items_moved,
                t.eaten,
                t.attacks,
                t.levers,
                t.decode_errors
            )?;
            for (p, n) in &self.problems {
                writeln!(f, "  problem    {n:>6}  {p}")?;
            }
        }
        writeln!(
            f,
            "  teleports  {} after spawning (server corrections); {} we asked for; {} repeats of one",
            t.teleports, t.own_teleports, t.teleport_resends
        )?;
        writeln!(f, "  chat       sent {}, received {}", t.chat_sent, t.chat_received)?;
        if self.disconnect_reasons.is_empty() {
            writeln!(f, "  ended      none before the run stopped")?;
        } else {
            writeln!(f, "  ended      (count, reason)")?;
            for (reason, n) in &self.disconnect_reasons {
                writeln!(f, "    {n:>6}  {reason}")?;
            }
        }
        Ok(())
    }
}

fn count(v: f64) -> String {
    match v {
        v if v >= 1e6 => format!("{:.2}M", v / 1e6),
        v if v >= 1e4 => format!("{:.1}k", v / 1e3),
        v if v >= 100.0 => format!("{v:.0}"),
        v => format!("{v:.1}"),
    }
}

fn bytes(v: f64) -> String {
    match v {
        v if v >= 1e9 => format!("{:.2} GB", v / 1e9),
        v if v >= 1e6 => format!("{:.2} MB", v / 1e6),
        v if v >= 1e3 => format!("{:.1} kB", v / 1e3),
        v => format!("{v:.0} B"),
    }
}

fn ms(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{v:.1} ms"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.50), Some(50.0));
        assert_eq!(percentile(&v, 0.99), Some(99.0));
        assert_eq!(percentile(&[7.0], 0.99), Some(7.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn report_rates_cover_the_window() {
        let shared = Shared::default();
        let start = Instant::now();
        let from = shared.snapshot();
        let mut t = Traffic { rx_packets: 100, rx_bytes: 10_000, connected_nanos: 4_000_000_000, ..Traffic::default() };
        shared.flush(&mut t);
        assert_eq!(t, Traffic::default());
        shared.record_join(Duration::from_millis(20));
        shared.record_end(true, Some("play: Timed out".into()));
        shared.record_end(false, Some("login: The server is full.".into()));
        shared.record_end(false, Some("login: The server is full.".into()));
        let mut to = shared.snapshot();
        to.at = from.at + Duration::from_secs(2);

        let r = Report::new(&shared, start, &from, &to);
        assert_eq!(r.rx_packets_per_sec, 50.0);
        assert_eq!(r.rx_bytes_per_sec_per_bot, 2_500.0); // 10 kB over 4 bot-seconds
        assert_eq!((r.joined, r.online, r.failed, r.dropped), (1, 0, 2, 1));
        assert_eq!(
            r.disconnect_reasons,
            [("login: The server is full.".to_string(), 2), ("play: Timed out".to_string(), 1)]
        );
        assert_eq!(r.join_ms_p50, Some(20.0));
    }
}
