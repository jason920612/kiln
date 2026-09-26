//! Tick duration statistics (MSPT percentiles) over a fixed window.

use std::fmt;
use std::time::Duration;

const WINDOW_TICKS: usize = 600; // 30 s at 20 TPS

#[derive(Default)]
pub struct TickStats {
    micros: Vec<u32>,
}

pub struct Report {
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mspt mean {:.3} p50 {:.3} p99 {:.3} max {:.3}",
            self.mean_ms, self.p50_ms, self.p99_ms, self.max_ms
        )
    }
}

impl TickStats {
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
        Some(Report { mean_ms: mean, p50_ms: at(0.5), p99_ms: at(0.99), max_ms: *v.last().unwrap() as f64 / 1000.0 })
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
