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
        max_players: std::env::var("KILN_MAX_PLAYERS").ok().and_then(|v| v.parse().ok()).unwrap_or(100),
        view_distance: 10,
        simulation_distance: 10,
        compression_threshold: Some(256),
    };
    let max_players = net_config.max_players;
    let view_distance = net_config.view_distance;
    let simulation_distance = net_config.simulation_distance;

    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    let shutdown = to_sim.clone();
    let shared = Arc::new(kiln_net::Shared::new(net_config, to_sim));
    let mut sim_config = kiln_sim::SimConfig::new(max_players, view_distance, std::env::var_os("KILN_WORLD").map(Into::into));
    sim_config.simulation_distance = simulation_distance;
    sim_config.online_mode = shared.authenticates();
    sim_config.require_resource_pack = shared.resource_pack_required();
    // KILN_TICK_THREADS: tick pool size; KILN_REGIONS=unified: one region (vanilla profile).
    if let Some(n) = std::env::var("KILN_TICK_THREADS").ok().and_then(|v| v.parse().ok()) {
        sim_config.pool.workers = n;
    }
    sim_config.unified_regions = std::env::var("KILN_REGIONS").is_ok_and(|v| v == "unified");
    // KILN_TICK_WINDOWS=inline: every phase window inline (A/B measurements of the windows).
    if std::env::var("KILN_TICK_WINDOWS").is_ok_and(|v| v == "inline") {
        sim_config.pool.phase = kiln_sched::PhaseMode::Inline;
    }
    // KILN_SCHEDULE=independent: regions too slow for the tick leave the lockstep (not
    // deterministic; lockstep is the default).
    if std::env::var("KILN_SCHEDULE").is_ok_and(|v| v == "independent") {
        sim_config.schedule = kiln_sim::ScheduleMode::Independent;
    }
    // KILN_PLUGINS_DIR: WASM plugins (`<dir>/<plugin>/plugin.toml` + `plugin.wasm`);
    // KILN_PLUGIN_BUDGET_US: time budget of each cancellable plugin call (default 500).
    if let Some(dir) = std::env::var_os("KILN_PLUGINS_DIR") {
        let mut plugins = kiln_sim::PluginSettings::new(dir);
        if let Some(us) = std::env::var("KILN_PLUGIN_BUDGET_US").ok().and_then(|v| v.parse().ok()) {
            plugins.call_budget = std::time::Duration::from_micros(us);
        }
        sim_config.plugins = Some(plugins);
    }
    // KILN_GENERATOR=noise: vanilla overworld terrain (KILN_SEED, KILN_DATAPACK = the data
    // generator output, default work/generated).
    if std::env::var("KILN_GENERATOR").is_ok_and(|v| v == "noise") {
        let seed = std::env::var("KILN_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let datapack = std::env::var_os("KILN_DATAPACK").map_or_else(|| "work/generated".into(), Into::into);
        sim_config.noise = Some(kiln_sim::NoiseConfig { seed, datapack, threads: 3 });
    }

    // The simulation also ends on its own after /stop; that ends the process.
    let (sim_done_tx, sim_done_rx) = tokio::sync::oneshot::channel::<()>();
    let sim = std::thread::Builder::new().name("sim".into()).spawn(move || {
        kiln_sim::run(sim_config, sim_rx);
        let _ = sim_done_tx.send(());
    })?;

    // Console: each line on stdin is a command run at permission level 4.
    let console = shutdown.clone();
    std::thread::Builder::new().name("console".into()).spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            let line = line.trim();
            if !line.is_empty() && console.send(kiln_link::ToSim::Console(line.to_owned())).is_err() {
                break;
            }
        }
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("net").enable_all().build()?;
    let sim_stopped = runtime.block_on(async {
        tokio::select! {
            r = kiln_net::listen(shared) => r.map(|_| false),
            _ = tokio::signal::ctrl_c() => Ok(false),
            _ = sim_done_rx => Ok(true),
        }
    })?;
    if sim_stopped {
        let _ = sim.join();
        return Ok(());
    }
    tracing::info!("stopping");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if shutdown.send(kiln_link::ToSim::Shutdown { done: done_tx }).is_ok() {
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(60));
    }
    let _ = sim.join();
    Ok(())
}
