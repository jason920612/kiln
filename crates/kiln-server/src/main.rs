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
    let shutdown = to_sim.clone();
    let shared = Arc::new(kiln_net::Shared::new(net_config, to_sim));

    let sim = std::thread::Builder::new().name("sim".into()).spawn(move || kiln_sim::run(sim_config, sim_rx))?;

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("net").enable_all().build()?;
    runtime.block_on(async {
        tokio::select! {
            r = kiln_net::listen(shared) => r,
            _ = tokio::signal::ctrl_c() => Ok(()),
        }
    })?;
    tracing::info!("stopping");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if shutdown.send(kiln_link::ToSim::Shutdown { done: done_tx }).is_ok() {
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(60));
    }
    let _ = sim.join();
    Ok(())
}
