//! Headless load-test bots for Minecraft Java Edition 26.3 (protocol 777).
//!
//! Each bot is one tokio task that owns its socket: handshake, offline login, configuration,
//! then play, where a 20 Hz client tick moves it according to a [`Behavior`] and sends
//! Client Tick End, while keep-alives, teleports and chunk batches are answered as they arrive.
//! Only packets a bot acts on are decoded; the rest are counted by id.
//!
//! Where the vanilla server polices clients, bots follow the 26.3 client: at most one position
//! per client tick, no movement before Player Loaded, and teleports confirmed by Accept
//! Teleportation alone. They join, walk and chat on a vanilla 26.3 server without corrections.

pub mod behavior;
mod bot;
mod metrics;
mod physics;
pub mod proto;
pub mod survival;
pub mod text;
pub mod wire;
mod world;

pub use behavior::Behavior;
pub use metrics::{Latency, Report, Traffic};
pub use survival::Role;

pub(crate) use bot::{Out, unix_millis};

use anyhow::{Context, Result, bail, ensure};
use metrics::Shared;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinSet;

#[derive(Debug, Clone)]
pub struct Config {
    /// Server address, `host:port`.
    pub addr: String,
    pub count: usize,
    /// Joins started per second.
    pub rate: f64,
    pub behavior: Behavior,
    /// Length of the run, counted from the first join.
    pub duration: Duration,
    /// View distance sent in Client Information.
    pub view_distance: u8,
    /// Bot `i` is named `<name_prefix><i>`.
    pub name_prefix: String,
    /// Radius of the movement script; `None` uses the behavior's default.
    pub radius: Option<f64>,
    /// Walking speed in blocks per second.
    pub speed: f64,
    /// Shared centre (x, z) of the movement scripts; `None` uses the first bot's spawn point.
    pub center: Option<[f64; 2]>,
    /// Each bot sends a chat message this often; `None` disables chat.
    pub chat_interval: Option<Duration>,
    /// Chunks per tick requested in Chunk Batch Received.
    pub chunks_per_tick: f32,
    /// Bots that have not reached the world after this long give up.
    pub join_timeout: Duration,
    /// How often `run_with` reports progress.
    pub report_interval: Duration,
    /// Seed for the movement scripts and chat salts.
    pub seed: u64,
    /// Bots are dealt round-robin into this many groups, each with its own centre on a grid.
    pub groups: usize,
    /// Distance between neighbouring group centres.
    pub group_spacing: f64,
    /// Bots whose group centre is far from where they spawned `/tp` there (they must be ops).
    pub teleport_to_group: bool,
    /// Survival: roles dealt to bots in turn (bot `i` gets `roles[i % len]`).
    pub roles: Vec<Role>,
    /// Survival: bots `[k * group_size, (k + 1) * group_size)` share a site; `None` deals
    /// the bots round-robin into `groups`.
    pub group_size: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:25565".into(),
            count: 1,
            rate: 50.0,
            behavior: Behavior::Idle,
            duration: Duration::from_secs(60),
            view_distance: 2,
            name_prefix: "Bot".into(),
            radius: None,
            speed: behavior::WALK_SPEED,
            center: None,
            chat_interval: None,
            chunks_per_tick: 64.0,
            join_timeout: Duration::from_secs(60),
            report_interval: Duration::from_secs(5),
            seed: 1,
            groups: 1,
            group_spacing: 48.0,
            teleport_to_group: false,
            roles: vec![Role::Explorer, Role::Miner, Role::Builder, Role::Redstone],
            group_size: None,
        }
    }
}

impl Config {
    /// The group bot `i` belongs to, and how many groups there are.
    pub fn group_of(&self, i: usize) -> (usize, usize) {
        match self.group_size {
            Some(n) => (i / n.max(1), self.count.div_ceil(n.max(1)).max(1)),
            None => (i % self.groups, self.groups),
        }
    }

    /// Offset of group `g`'s centre from the shared centre: groups fill a square grid.
    pub fn group_offset(&self, g: usize) -> [f64; 2] {
        let cols = (self.groups as f64).sqrt().ceil().max(1.0) as usize;
        let rows = self.groups.div_ceil(cols);
        let at = |i: usize, n: usize| (i as f64 - (n - 1) as f64 / 2.0) * self.group_spacing;
        [at(g % cols, cols), at(g / cols, rows)]
    }

    /// Like [`Config::group_offset`] for a given number of groups.
    pub fn group_offset_in(&self, g: usize, groups: usize) -> [f64; 2] {
        let cols = (groups as f64).sqrt().ceil().max(1.0) as usize;
        let rows = groups.div_ceil(cols);
        let at = |i: usize, n: usize| (i as f64 - (n - 1) as f64 / 2.0) * self.group_spacing;
        [at(g % cols, cols), at(g / cols, rows)]
    }

    fn validate(&self) -> Result<()> {
        ensure!(!self.roles.is_empty(), "at least one role is needed");
        ensure!(self.group_size != Some(0), "group size must be positive");
        ensure!(self.rate.is_finite() && self.rate > 0.0, "rate must be positive");
        ensure!(self.speed.is_finite() && self.speed >= 0.0, "speed must be non-negative");
        ensure!(self.radius.is_none_or(|r| r.is_finite() && r >= 0.0), "radius must be non-negative");
        ensure!(self.chat_interval.is_none_or(|d| !d.is_zero()), "chat interval must be positive");
        ensure!(!self.report_interval.is_zero(), "report interval must be positive");
        ensure!(self.groups >= 1, "groups must be at least 1");
        ensure!(self.group_spacing.is_finite(), "group spacing must be finite");
        ensure!(
            self.name_prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "name prefix may only contain letters, digits and _"
        );
        let longest = self.name_prefix.len() + self.count.saturating_sub(1).to_string().len();
        ensure!(longest <= 16, "names would be {longest} characters long; the limit is 16");
        Ok(())
    }
}

/// Runs the bots for `config.duration` and returns the final report.
pub async fn run(config: Config) -> Result<Report> {
    run_with(config, |_| {}, std::future::pending()).await
}

/// Like [`run`], calling `progress` every `report_interval` with rates over that interval,
/// and stopping early when `shutdown` completes.
pub async fn run_with(
    config: Config,
    mut progress: impl FnMut(&Report),
    shutdown: impl Future<Output = ()>,
) -> Result<Report> {
    config.validate()?;
    let target = Arc::new(bot::Target::resolve(&config.addr).await?);
    let shared = Arc::new(Shared::default());
    if let Some(c) = config.center {
        let _ = shared.origin.set(c);
    }
    let cfg = Arc::new(config);
    let (stop_tx, stop_rx) = watch::channel(false);

    let start = Instant::now();
    let launch_at = |i: usize| tokio::time::Instant::from_std(start + Duration::from_secs_f64(i as f64 / cfg.rate));
    let zero = shared.snapshot();
    let mut last = zero;
    let mut report_tick = tokio::time::interval_at((start + cfg.report_interval).into(), cfg.report_interval);
    let end = tokio::time::sleep_until((start + cfg.duration).into());
    tokio::pin!(end, shutdown);
    let mut bots = JoinSet::new();
    let mut launched = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(launch_at(launched)), if launched < cfg.count => {
                let now = tokio::time::Instant::now();
                while launched < cfg.count && launch_at(launched) <= now {
                    let bot = bot::run(launched, cfg.clone(), target.clone(), shared.clone(), stop_rx.clone());
                    bots.spawn(bot);
                    launched += 1;
                }
            }
            _ = report_tick.tick() => {
                let snap = shared.snapshot();
                progress(&Report::new(&shared, start, &last, &snap));
                last = snap;
            }
            Some(_) = bots.join_next(), if !bots.is_empty() => {}
            _ = &mut end => break,
            _ = &mut shutdown => break,
        }
    }

    let at_stop = shared.snapshot();
    let _ = stop_tx.send(true);
    let drained =
        tokio::time::timeout(Duration::from_secs(5), async { while bots.join_next().await.is_some() {} }).await;
    if drained.is_err() {
        bots.shutdown().await;
    }
    let end = shared.final_snapshot(&at_stop);
    Ok(Report::new(&shared, start, &zero, &end))
}

/// Parses `x,z`.
pub fn parse_center(s: &str) -> Result<[f64; 2]> {
    let Some((x, z)) = s.split_once(',') else { bail!("expected x,z") };
    let parse = |v: &str| v.trim().parse::<f64>().with_context(|| format!("bad coordinate {v:?}"));
    Ok([parse(x)?, parse(z)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_names_over_sixteen_characters() {
        // 11 + 6 digits (LoadTestBot999999) is too long; 11 + 5 is fine.
        let cfg = Config { name_prefix: "LoadTestBot".into(), count: 1_000_000, ..Config::default() };
        assert!(cfg.validate().is_err());
        let cfg = Config { name_prefix: "LoadTestBot".into(), count: 100_000, ..Config::default() };
        assert!(cfg.validate().is_ok());
        let cfg = Config { name_prefix: "bad name".into(), ..Config::default() };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn parses_center() {
        assert_eq!(parse_center("100, -20.5").unwrap(), [100.0, -20.5]);
        assert!(parse_center("100").is_err());
    }

    #[test]
    fn groups_form_a_centred_grid() {
        let c = Config { groups: 4, group_spacing: 100.0, ..Config::default() };
        let offsets: Vec<_> = (0..4).map(|g| c.group_offset(g)).collect();
        assert_eq!(offsets, [[-50.0, -50.0], [50.0, -50.0], [-50.0, 50.0], [50.0, 50.0]]);
        assert_eq!(Config::default().group_offset(0), [0.0, 0.0]);
        let c = Config { groups: 20, ..Config::default() };
        let mut all: Vec<_> = (0..20).map(|g| c.group_offset(g)).collect();
        all.dedup();
        assert_eq!(all.len(), 20);
    }
}
