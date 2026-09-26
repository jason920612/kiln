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
    let net_config = kiln_net::Config {
        bind: ([0, 0, 0, 0], port).into(),
        motd: "A Kiln server".into(),
        max_players: 100,
        view_distance: 10,
        simulation_distance: 10,
        compression_threshold: Some(256),
    };
    let sim_config = kiln_sim::SimConfig {
        max_players: net_config.max_players,
        view_distance: net_config.view_distance,
        simulation_distance: net_config.simulation_distance,
        world: std::env::var_os("KILN_WORLD").map(Into::into),
    };

    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    let shared = Arc::new(kiln_net::Shared::new(net_config, to_sim));

    std::thread::Builder::new().name("sim".into()).spawn(move || kiln_sim::run(sim_config, sim_rx))?;

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("net")
        .enable_all()
        .build()?
        .block_on(kiln_net::listen(shared))
}
