use anyhow::Result;
use kiln_server::eula;
use kiln_server::launch::{self, Options};
use kiln_server::properties::{self, ServerProperties};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// A variable of the environment, if set to something (the harnesses' settings).
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // The harnesses (KILN_* variables and no server.properties) keep Kiln's own log format; a server with a properties
    // file logs like vanilla's, to the console and to logs/latest.log.
    if kiln_server::launch::harness_env(env) && !Path::new("server.properties").exists() {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else if let Err(e) = kiln_server::logging::init_server_log(filter) {
        eprintln!("cannot write logs/latest.log: {e}");
        tracing_subscriber::fmt().with_env_filter(EnvFilter::new("info")).init();
    }

    // `kiln world ...`: world tools, without starting the server.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "world") {
        return world_tool(&args[1..]);
    }
    let options = match Options::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if options.help {
        eprint!("{}", launch::HELP);
        return Ok(());
    }
    if let Some(pid_file) = &options.pid_file {
        std::fs::write(pid_file, std::process::id().to_string())?;
    }

    if !(kiln_server::launch::harness_env(env) && !Path::new("server.properties").exists()) {
        info!("Starting minecraft server version {}", kiln_data::version::NAME);
        info!("Loading properties");
    }
    // `Main.main`: server.properties is read and written back (normalized) before anything else.
    let props_path = Path::new("server.properties");
    let legacy = launch::harness_env(env) && !props_path.exists();
    let props = if legacy {
        ServerProperties::parse(launch::harness_defaults(), properties::generate_secret_key)?
    } else {
        let props = ServerProperties::load(props_path)?;
        props.store(props_path);
        props
    };
    if !legacy {
        let eula_path = Path::new("eula.txt");
        let agreed = eula::has_agreed(eula_path);
        if options.init_settings {
            let abs = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            info!("Initialized '{}' and '{}'", abs(props_path).display(), abs(eula_path).display());
            return Ok(());
        }
        if !agreed {
            info!("You need to agree to the EULA in order to run the server. Go to eula.txt for more info.");
            return Ok(());
        }
    }

    // The console prints in the vanilla language, which the jar of the user's own server has.
    match kiln_server::jar::find_language(env) {
        Some(json) => {
            kiln_sim::set_console_language(&json);
        }
        None => warn!("no vanilla language file (en_us.json) found: the console shows translation keys. Put the vanilla server.jar in the working directory, or set KILN_JAR or KILN_LANG"),
    }

    let level_name = options.world.clone().unwrap_or_else(|| props.level_name.clone());
    let world_dir = (!legacy).then(|| options.universe.clone().unwrap_or_else(|| ".".into()).join(&level_name));
    let defaults = launch::kiln_defaults(&props, &options, world_dir.as_deref());
    // A variable that is set wins over the properties file.
    let var = |k: &str| env(k).or_else(|| defaults.get(k).filter(|v| !v.trim().is_empty()).cloned());
    let number = |k: &str| var(k).and_then(|v| v.trim().parse::<i64>().ok());
    // (Single-threaded still: the simulation reads these two on its own.)
    for k in ["KILN_INITIAL_PACKS", "KILN_INITIAL_DISABLED_PACKS"] {
        if std::env::var_os(k).is_none()
            && let Some(v) = defaults.get(k)
        {
            // SAFETY: no other thread exists yet.
            unsafe { std::env::set_var(k, v) };
        }
    }

    let port = number("KILN_PORT").filter(|p| (0..=65535).contains(p)).unwrap_or(25565) as u16;
    let bind_ip: std::net::IpAddr = match props.server_ip.trim() {
        "" => [0, 0, 0, 0].into(),
        host => match host.parse() {
            Ok(ip) => ip,
            Err(_) => std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
                .ok()
                .and_then(|mut a| a.next())
                .map(|a| a.ip())
                .ok_or_else(|| anyhow::anyhow!("server-ip {host:?} is not an address"))?,
        },
    };
    if !legacy {
        info!("Starting Minecraft server on {}:{port}", if props.server_ip.trim().is_empty() { "*" } else { props.server_ip.trim() });
    }
    let max_players = number("KILN_MAX_PLAYERS").unwrap_or(20).max(0) as usize;
    let view_distance = number("KILN_VIEW_DISTANCE").unwrap_or(10).clamp(2, 32) as u8;
    let simulation_distance = number("KILN_SIMULATION_DISTANCE").unwrap_or(10).clamp(2, 32) as u8;

    let login = kiln_net::LoginConfig::from_vars(var)?;
    let lobby = kiln_net::LobbyConfig::from_vars(var)?;
    let enforce_secure_chat = props.enforce_secure_profile && login.online_mode;
    let mut net_config = kiln_net::Config::new((bind_ip, port).into(), &props.motd, max_players, view_distance);
    net_config.simulation_distance = simulation_distance;
    net_config.compression_threshold = usize::try_from(props.network_compression_threshold).ok();
    net_config.enforces_secure_chat = enforce_secure_chat;
    net_config.rate_limit = props.rate_limit.max(0) as u32;
    // `server-icon.png`, else the world's `icon.png`; it has to be a 64x64 PNG.
    let icon = [Path::new("server-icon.png").to_path_buf(), world_dir.clone().unwrap_or_default().join("icon.png")]
        .into_iter()
        .find(|p| p.is_file());
    if let Some(icon) = icon {
        match std::fs::read(&icon).map_err(|e| e.to_string()).and_then(|b| kiln_net::status::favicon_from_png(&b)) {
            Ok(f) => net_config.favicon = Some(f),
            Err(e) => tracing::error!("Couldn't load server icon: {e}"),
        }
    }

    let info = Arc::new(kiln_link::ServerInfo::new(&props.motd, max_players as i32, &level_name));
    info.hide_online_players.store(props.hide_online_players, std::sync::atomic::Ordering::Relaxed);
    info.enable_status.store(props.enable_status, std::sync::atomic::Ordering::Relaxed);

    let (to_sim, sim_rx) = crossbeam_channel::unbounded();
    let shutdown = to_sim.clone();
    // whitelist.json, banned-players.json and banned-ips.json in the working directory, as
    // vanilla keeps them.
    let mut access = kiln_link::access::AccessLists::new(Some(std::env::current_dir()?));
    access.use_whitelist = var("KILN_WHITELIST").is_some_and(|v| v == "true");
    access.enforce_whitelist = var("KILN_ENFORCE_WHITELIST").is_some_and(|v| v == "true");
    let access = access.shared();
    let shared = Arc::new(
        kiln_net::Shared::with_login(net_config, login, to_sim)
            .with_lobby(lobby)
            .with_access(access.clone())
            .with_info(info.clone()),
    );
    let mut sim_config = kiln_sim::SimConfig::new(max_players, view_distance, var("KILN_WORLD").map(Into::into));
    sim_config.simulation_distance = simulation_distance;
    sim_config.access = access;
    sim_config.info = info;
    sim_config.broadcast_console_to_ops = props.broadcast_console_to_ops;
    sim_config.broadcast_rcon_to_ops = props.broadcast_rcon_to_ops;
    sim_config.server = kiln_sim::ServerSettings {
        default_game_mode: (!legacy).then_some(props.gamemode),
        force_gamemode: props.force_gamemode,
        hardcore: props.hardcore,
        spawn_protection: props.spawn_protection,
        allow_flight: props.allow_flight,
        player_idle_timeout: props.player_idle_timeout,
        pause_when_empty_seconds: props.pause_when_empty_seconds,
        max_tick_time_ms: props.max_tick_time,
        entity_broadcast_range_percentage: props.entity_broadcast_range_percentage,
        max_world_size: props.max_world_size,
        max_chained_neighbor_updates: props.max_chained_neighbor_updates,
        op_permission_level: props.op_permission_level,
        function_permission_level: props.function_permission_level,
        enforce_secure_profile: enforce_secure_chat,
        command_spam_threshold_seconds: props.command_spam_threshold_seconds,
        chat_spam_threshold_seconds: props.chat_spam_threshold_seconds,
        log_ips: props.log_ips,
        generate_structures: props.generate_structures,
        announce_advancements: props.announce_player_achievements,
    };
    sim_config.data_sync = shared.data_sync.clone();
    sim_config.online_mode = shared.authenticates();
    // `fetchprofile` looks accounts up through the session service in online mode
    // (KILN_PROFILE_LOOKUP=true|false overrides); offline servers make no such requests.
    let lookup = std::env::var("KILN_PROFILE_LOOKUP").map_or(shared.authenticates(), |v| v == "true");
    if lookup {
        sim_config.profile_lookup = Some(kiln_net::SessionProfiles::new());
    }
    sim_config.replies = Some(shutdown.clone());
    sim_config.require_resource_pack = shared.resource_pack_required();
    // KILN_TICK_THREADS: tick pool size; KILN_REGIONS=unified: one region (vanilla profile).
    if let Some(n) = number("KILN_TICK_THREADS") {
        sim_config.pool.workers = n as usize;
    }
    sim_config.unified_regions = env("KILN_REGIONS").is_some_and(|v| v == "unified");
    // KILN_ENTITY_TICKING=serial|islands|tiles (default serial); KILN_LOCATOR_INTERVAL=n (1).
    if let Some(t) = env("KILN_ENTITY_TICKING").and_then(|v| kiln_sim::EntityTicking::parse(&v)) {
        sim_config.entity_ticking = t;
    }
    if let Some(n) = number("KILN_LOCATOR_INTERVAL") {
        sim_config.locator_interval = n as u32;
    }
    // KILN_TICK_WINDOWS=inline: every phase window inline (A/B measurements of the windows).
    if env("KILN_TICK_WINDOWS").is_some_and(|v| v == "inline") {
        sim_config.pool.phase = kiln_sched::PhaseMode::Inline;
    }
    // KILN_SCHEDULE=independent: regions too slow for the tick leave the lockstep (not
    // deterministic; lockstep is the default).
    if env("KILN_SCHEDULE").is_some_and(|v| v == "independent") {
        sim_config.schedule = kiln_sim::ScheduleMode::Independent;
    }
    // KILN_WORLD_FORMAT=native: a new world is stored in Kiln's native format (an existing
    // world keeps its format; convert one with `kiln world convert`).
    if env("KILN_WORLD_FORMAT").is_some_and(|v| v == "native") {
        sim_config.world_format = kiln_storage::WorldFormat::Native;
    }
    // KILN_PLUGINS_DIR: WASM plugins (`<dir>/<plugin>/plugin.toml` + `plugin.wasm`);
    // KILN_PLUGIN_BUDGET_US: time budget of each cancellable plugin call (default 500);
    // KILN_PLUGIN_MODE=strict: fuel budgets (KILN_PLUGIN_FUEL per call), a tick clock and
    // seeded random streams, for replays.
    if let Some(dir) = std::env::var_os("KILN_PLUGINS_DIR") {
        let mut plugins = kiln_sim::PluginSettings::new(dir);
        if let Some(us) = number("KILN_PLUGIN_BUDGET_US") {
            plugins.call_budget = std::time::Duration::from_micros(us as u64);
        }
        if env("KILN_PLUGIN_MODE").is_some_and(|v| v == "strict") {
            plugins.mode = kiln_sim::PluginMode::Strict;
        }
        if let Some(fuel) = number("KILN_PLUGIN_FUEL") {
            plugins.call_fuel = fuel as u64;
        }
        sim_config.plugins = Some(plugins);
    }
    // KILN_ENABLE_COMMAND_BLOCK=true: run command blocks (vanilla 26.3 has a game rule for this
    // and its server.properties no longer has `enable-command-block`).
    sim_config.enable_command_block = env("KILN_ENABLE_COMMAND_BLOCK").is_some_and(|v| v == "true");
    // KILN_DIFFICULTY=peaceful|easy|normal|hard (or 0..3): the server's difficulty, applied over
    // the world's saved one at every start (vanilla's `difficulty` property).
    sim_config.difficulty = var("KILN_DIFFICULTY").and_then(|v| {
        ["peaceful", "easy", "normal", "hard"]
            .iter()
            .position(|d| d.eq_ignore_ascii_case(v.trim()))
            .map(|i| i as u8)
            .or_else(|| v.trim().parse::<u8>().ok().filter(|d| *d <= 3))
    });
    // KILN_GENERATOR=noise: vanilla overworld terrain (KILN_SEED, KILN_DATAPACK = the data
    // generator output, default work/generated). Without the variable a server with a properties
    // file generates `level-type=minecraft:normal` worlds this way when it finds the data.
    let explicit_datapack = std::env::var_os("KILN_DATAPACK").map(std::path::PathBuf::from);
    let datapack = explicit_datapack
        .iter()
        .cloned()
        .chain(["generated", "work/generated"].map(std::path::PathBuf::from))
        .find(|p| p.join("data").is_dir())
        .or(explicit_datapack)
        .unwrap_or_else(|| "work/generated".into());
    let noise = match env("KILN_GENERATOR").as_deref() {
        Some(g) => g == "noise",
        None if legacy => false,
        None => match props.level_type.as_str() {
            "minecraft:flat" | "flat" => false,
            other => {
                if other != "minecraft:normal" && other != "normal" && other != "default" {
                    warn!("level-type {other} is not supported, generating minecraft:normal terrain");
                }
                let found = datapack.join("data").is_dir();
                if !found {
                    warn!(
                        "no vanilla world generation data in {} (see docs/wp86.md): generating a superflat world",
                        datapack.display()
                    );
                }
                found
            }
        },
    };
    if noise {
        // Without KILN_SEED the world's own seed (`world_gen_settings.dat`) generates; a new world takes `level-seed`.
        let saved = sim_config.world.as_deref().and_then(kiln_storage::read_seed);
        let wanted = launch::parse_seed(&props.level_seed);
        let seed = number("KILN_SEED").or(saved).or(wanted).unwrap_or_else(random_seed);
        // KILN_GEN_THREADS: chunk generation threads per dimension (as many as tick, on CPU
        // time nothing else wants).
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = number("KILN_GEN_THREADS").map_or_else(|| kiln_sim::default_workers(cores).max(2), |n| n as usize);
        sim_config.noise = Some(kiln_sim::NoiseConfig { seed, datapack, threads });
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

    // `DedicatedServer.initServer`: the watchdog of `max-tick-time`.
    let _watchdog = kiln_server::watchdog::start(shared.info.clone(), props.max_tick_time, 50);

    // `DedicatedServer.initServer`: Query, then RCON, once the server is up.
    let host_ip = match props.server_ip.trim() {
        "" | "0.0.0.0" => kiln_server::query::local_host_address().unwrap_or_else(|| "0.0.0.0".to_owned()),
        ip => ip.to_owned(),
    };
    let mut query = None;
    if props.enable_query {
        info!("Starting GS4 status listener");
        let facts = Arc::new(kiln_server::query::LiveFacts { info: shared.info.clone(), port: i32::from(port), host_ip });
        let random = Arc::new(|| {
            use rand_core::RngCore;
            (rand_core::OsRng.next_u32() >> 8) as i32
        });
        query = kiln_server::query::QueryServer::start(&props.server_ip, props.query_port, facts, random);
    }
    let mut rcon = None;
    if props.enable_rcon {
        info!("Starting remote control listener");
        let to_sim = shutdown.clone();
        let runner: kiln_server::rcon::Runner = Arc::new(move |command: &str| {
            let (tx, rx) = std::sync::mpsc::channel();
            let command = command.to_owned();
            let work = Box::new(move |host: &mut dyn kiln_link::AdminHost| {
                let _ = tx.send(host.rcon_command(&command));
            });
            if to_sim.send(kiln_link::ToSim::Admin(work)).is_err() {
                return String::new();
            }
            rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap_or_default()
        });
        rcon = kiln_server::rcon::RconServer::start(&props.server_ip, props.rcon_port, &props.rcon_password, runner);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("net").enable_all().build()?;
    let sim_stopped = runtime.block_on(async {
        tokio::select! {
            r = kiln_net::listen(shared) => r.map(|_| false),
            _ = tokio::signal::ctrl_c() => Ok(false),
            _ = sim_done_rx => Ok(true),
        }
    })?;
    // `DedicatedServer.onServerExit`: the listeners stop with the server.
    let stop_listeners = |rcon: &mut Option<kiln_server::rcon::RconServer>, query: &mut Option<kiln_server::query::QueryServer>| {
        if let Some(r) = rcon.as_mut() {
            r.stop();
        }
        if let Some(q) = query.as_mut() {
            q.stop();
        }
    };
    if sim_stopped {
        stop_listeners(&mut rcon, &mut query);
        let _ = sim.join();
        return Ok(());
    }
    tracing::info!("stopping");
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    if shutdown.send(kiln_link::ToSim::Shutdown { done: done_tx }).is_ok() {
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(60));
    }
    stop_listeners(&mut rcon, &mut query);
    let _ = sim.join();
    Ok(())
}

/// `kiln world convert --to native|anvil <world> <new world> [--threads N]`: converts a world
/// between Anvil and the native format into a new directory (the original is untouched).
/// `kiln world compare <world> <world>`: compares two Anvil worlds chunk by chunk.
fn world_tool(args: &[String]) -> Result<()> {
    const USAGE: &str = "usage: kiln world convert --to native|anvil <world> <new world> [--threads N]\n       kiln world compare <anvil world> <anvil world>";
    let mut to = None;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut paths = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--to" => {
                to = match it.next().map(String::as_str) {
                    Some("native") => Some(kiln_storage::WorldFormat::Native),
                    Some("anvil") => Some(kiln_storage::WorldFormat::Anvil),
                    _ => anyhow::bail!(USAGE),
                }
            }
            "--threads" => threads = it.next().and_then(|n| n.parse().ok()).ok_or_else(|| anyhow::anyhow!(USAGE))?,
            p => paths.push(std::path::PathBuf::from(p)),
        }
    }
    match (args.first().map(String::as_str), &paths[..]) {
        (Some("convert"), [src, dst]) => {
            let to = to.ok_or_else(|| anyhow::anyhow!(USAGE))?;
            let report = kiln_storage::native::convert::convert_world(src, dst, to, threads)?;
            println!("{report}");
            Ok(())
        }
        (Some("compare"), [a, b]) => {
            let (chunks, diffs) = kiln_storage::native::convert::compare_worlds(a, b)?;
            for d in &diffs {
                println!("{d}");
            }
            println!("{chunks} chunks compared, {} differences", diffs.len());
            if diffs.is_empty() { Ok(()) } else { anyhow::bail!("the worlds differ") }
        }
        _ => anyhow::bail!(USAGE),
    }
}

/// `RandomSource.create().nextLong()`: the seed of a world nobody gave one.
fn random_seed() -> i64 {
    use rand_core::RngCore;
    rand_core::OsRng.next_u64() as i64
}
