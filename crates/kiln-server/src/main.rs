mod net;
mod packets;
mod sim;
mod world;

use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let port: u16 = std::env::var("KILN_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(25565);
    let config = net::Config {
        bind: ([0, 0, 0, 0], port).into(),
        motd: "A Kiln server".into(),
        max_players: 100,
        view_distance: 10,
        simulation_distance: 10,
        compression_threshold: Some(256),
    };

    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    let shared = Arc::new(net::Shared::new(config, to_sim));

    let sim_shared = shared.clone();
    std::thread::Builder::new().name("sim".into()).spawn(move || sim::run(sim_shared, sim_rx))?;

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("net")
        .enable_all()
        .build()?
        .block_on(net::listen(shared))
}
