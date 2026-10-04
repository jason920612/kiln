//! `kiln-bot`: runs load-test bots against a Minecraft Java 26.3 server and prints metrics.

use anyhow::Result;
use clap::Parser;
use kiln_bot::{Behavior, Config, Role};
use std::time::Duration;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Headless load-test bots for Minecraft Java Edition 26.3 (offline mode).
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Server address.
    #[arg(long, default_value = "127.0.0.1:25565")]
    addr: String,
    /// Number of bots.
    #[arg(long, default_value_t = 10)]
    count: usize,
    /// Joins started per second.
    #[arg(long, default_value_t = 50.0)]
    rate: f64,
    #[arg(long, value_enum, default_value_t = Behavior::Idle)]
    behavior: Behavior,
    /// Run time in seconds, counted from the first join.
    #[arg(long, default_value_t = 60.0)]
    duration: f64,
    /// View distance sent in Client Information.
    #[arg(long, default_value_t = 2)]
    view_distance: u8,
    /// Bot i is named <prefix><i>.
    #[arg(long, default_value = "Bot")]
    name_prefix: String,
    /// Radius of the behavior [default: walk 64, circle 16, crowd 6, spread 256]. Survival bots
    /// ignore it; their group sites are --group-spacing apart around --center.
    #[arg(long)]
    radius: Option<f64>,
    /// Seconds between chat messages per bot [default: no chat].
    #[arg(long)]
    chat_interval: Option<f64>,
    /// Shared centre "x,z" of the behaviors [default: where the first bot spawned].
    #[arg(long, value_parser = kiln_bot::parse_center)]
    center: Option<[f64; 2]>,
    /// Walking speed in blocks per second.
    #[arg(long, default_value_t = kiln_bot::behavior::WALK_SPEED)]
    speed: f64,
    /// Seed for the behaviors.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Deal the bots into this many groups with their own centres on a grid.
    #[arg(long, default_value_t = 1)]
    groups: usize,
    /// Blocks between neighbouring group centres.
    #[arg(long, default_value_t = 48.0)]
    group_spacing: f64,
    /// Survival roles dealt to the bots in turn (explorer, miner, builder, redstone).
    #[arg(long, value_delimiter = ',')]
    roles: Option<Vec<Role>>,
    /// Survival: bots [k*size, (k+1)*size) share a site (overrides --groups).
    #[arg(long)]
    group_size: Option<usize>,
    /// Teleport bots to their group centre with /tp (the server must make them operators).
    #[arg(long)]
    teleport_to_group: bool,
    /// Seconds between progress lines.
    #[arg(long, default_value_t = 5.0)]
    report_interval: f64,
    /// Tokio worker threads [default: one per core].
    #[arg(long)]
    threads: Option<usize>,
    /// Print the final report as JSON.
    #[arg(long)]
    json: bool,
}

fn secs(s: f64, what: &str) -> Result<Duration> {
    Duration::try_from_secs_f64(s).map_err(|_| anyhow::anyhow!("invalid {what}: {s}"))
}

fn main() -> Result<()> {
    let a = Args::parse();
    let config = Config {
        addr: a.addr,
        count: a.count,
        rate: a.rate,
        behavior: a.behavior,
        duration: secs(a.duration, "duration")?,
        view_distance: a.view_distance,
        name_prefix: a.name_prefix,
        radius: a.radius,
        speed: a.speed,
        center: a.center,
        chat_interval: a.chat_interval.map(|s| secs(s, "chat interval")).transpose()?,
        report_interval: secs(a.report_interval, "report interval")?,
        seed: a.seed,
        groups: a.groups,
        group_spacing: a.group_spacing,
        teleport_to_group: a.teleport_to_group,
        group_size: a.group_size,
        roles: a.roles.unwrap_or_else(|| Config::default().roles),
        ..Config::default()
    };

    let mut rt = tokio::runtime::Builder::new_multi_thread();
    if let Some(n) = a.threads {
        rt.worker_threads(n);
    }
    let rt = rt.thread_name("bot").enable_all().build()?;
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let report = rt.block_on(kiln_bot::run_with(config, |r| println!("{}", r.summary_line()), ctrl_c))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{report}");
    }
    Ok(())
}
