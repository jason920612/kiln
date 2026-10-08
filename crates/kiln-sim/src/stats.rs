//! Tick duration statistics (MSPT percentiles) over a fixed window.

use std::fmt;
use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;

/// `KILN_TICK_TRACE=<file>`: one line per tick, `<unix ms> <players> <tick micros>`, so a
/// benchmark can compute exact percentiles over the stretch of a run it cares about.
static TRACE: Mutex<Option<std::io::BufWriter<std::fs::File>>> = Mutex::new(None);
static TRACE_INIT: std::sync::Once = std::sync::Once::new();

/// Appends a tick to the trace file, if one is configured.
pub fn trace(micros: u64, players: usize) {
    TRACE_INIT.call_once(|| {
        if let Some(path) = std::env::var_os("KILN_TICK_TRACE") {
            match std::fs::File::create(&path) {
                Ok(f) => *TRACE.lock().unwrap() = Some(std::io::BufWriter::new(f)),
                Err(e) => tracing::warn!("cannot write the tick trace {}: {e}", path.to_string_lossy()),
            }
        }
    });
    if let Some(w) = TRACE.lock().unwrap().as_mut() {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let _ = writeln!(w, "{ms} {players} {micros}");
    }
}

/// Writes out what the trace file has buffered.
pub fn flush_trace() {
    if let Some(w) = TRACE.lock().unwrap().as_mut() {
        let _ = w.flush();
    }
}

const WINDOW_TICKS: usize = 600; // 30 s at 20 TPS

#[derive(Default)]
pub struct TickStats {
    micros: Vec<u32>,
    /// Accumulated time per named tick phase over the window.
    phases: Vec<(&'static str, Duration)>,
    /// The same, since the last `reset_totals` (not cleared by a completed window).
    totals: Vec<(&'static str, Duration)>,
}

pub struct Report {
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Mean milliseconds per tick for each phase.
    pub phases: Vec<(&'static str, f64)>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mspt mean {:.3} p50 {:.3} p99 {:.3} max {:.3}",
            self.mean_ms, self.p50_ms, self.p99_ms, self.max_ms
        )?;
        for (name, ms) in &self.phases {
            write!(f, " | {name} {ms:.3}")?;
        }
        Ok(())
    }
}

impl TickStats {
    /// Adds time spent in a phase this tick.
    pub fn phase(&mut self, name: &'static str, d: Duration) {
        match self.totals.iter_mut().find(|(n, _)| *n == name) {
            Some((_, t)) => *t += d,
            None => self.totals.push((name, d)),
        }
        match self.phases.iter_mut().find(|(n, _)| *n == name) {
            Some((_, t)) => *t += d,
            None => self.phases.push((name, d)),
        }
    }

    /// Time per phase since the last reset, in the order the phases first appeared.
    pub fn totals(&self) -> &[(&'static str, Duration)] {
        &self.totals
    }

    pub fn reset_totals(&mut self) {
        self.totals.clear();
    }

    /// Records one tick; returns a report when a window completes.
    pub fn record(&mut self, d: Duration) -> Option<Report> {
        self.micros.push(d.as_micros().min(u32::MAX as u128) as u32);
        if self.micros.len() < WINDOW_TICKS {
            return None;
        }
        let mut v = std::mem::take(&mut self.micros);
        v.sort_unstable();
        // Nearest-rank percentile: the smallest value with at least q of the samples at or below it.
        let at = |q: f64| v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1] as f64 / 1000.0;
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64 / 1000.0;
        let ticks = v.len() as f64;
        let phases = std::mem::take(&mut self.phases)
            .into_iter()
            .map(|(n, d)| (n, d.as_secs_f64() * 1e3 / ticks))
            .collect();
        Some(Report {
            mean_ms: mean,
            p50_ms: at(0.5),
            p99_ms: at(0.99),
            max_ms: *v.last().unwrap() as f64 / 1000.0,
            phases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_over_a_window() {
        let mut s = TickStats::default();
        let mut report = None;
        for i in 0..WINDOW_TICKS {
            // 7 slow ticks out of 600 (> 1%) put p99 on a slow tick.
            report = s.record(Duration::from_micros(if i < 593 { 1000 } else { 20_000 }));
        }
        let r = report.unwrap();
        assert_eq!(r.p50_ms, 1.0);
        assert_eq!(r.p99_ms, 20.0);
        assert_eq!(r.max_ms, 20.0);
    }
}
