//! Survival benchmark: a real `kiln` server on vanilla noise terrain, survival bots (explorers,
//! miners, builders, redstone engineers) in small groups spread over the world, and the numbers
//! that matter read back from both sides.
//!
//! ```text
//! cargo build --release -p kiln-server -p kiln-bot
//! cargo run --release -p kiln-bot --example survival_bench -- --count 200 --measure 180 \
//!     --datapack work/generated --world scratch/world
//! ```
//!
//! With `--ssh user@host` the server runs on that machine (started over ssh, its console on the
//! ssh session's standard input and output) and the bots connect to it over the network from
//! this one, as real remote players would, without taking the server's CPU. `--server`,
//! `--datapack` and `--world` are then paths on the remote machine (relative to `--remote-dir`),
//! and the tick trace, world sizes and the server's CPU are read there:
//!
//! ```text
//! survival_bench --ssh test@192.168.1.169 --remote-dir /home/test/wt/wp47-survival-perf \
//!     --server target/release/kiln --datapack /home/test/kiln/work/generated \
//!     --world /home/test/bench/world --bot target/release/kiln-bot.exe --count 200
//! ```
//!
//! A run starts the server (`--phase gen` on an empty world directory, `--phase reload` on a copy
//! of a saved one, `both` does the two in a row), lets the bots join and walk to their sites,
//! then measures a stretch of `--measure` seconds with all of them in play:
//!
//! * server tick time (mean, p50, p99, max, share over 50 ms) from the server's per-tick trace
//!   (`KILN_TICK_TRACE`), and the phases that make it up from its 30 s reports,
//! * chunk generation (chunks per second, how busy the generation threads were, request to
//!   delivery), loading from disk (count, time per chunk) and installing, from the server's
//!   `chunk totals` lines,
//! * the bots' side: chunk arrival latency, time until a new view was complete (walking,
//!   teleporting, joining), time explorers waited for chunks, corrections, disconnects, decode
//!   errors, digging and placing success,
//! * CPU of the server and of the bot process, and of the machine.
//!
//! Nothing here is vanilla data: the datapack directory is the one the server already needs.

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser, Clone)]
struct Args {
    /// The `kiln` server binary.
    #[arg(long, default_value = "target/release/kiln.exe")]
    server: PathBuf,
    /// The `kiln-bot` binary.
    #[arg(long, default_value = "target/release/kiln-bot.exe")]
    bot: PathBuf,
    /// The data generator output (`KILN_DATAPACK`).
    #[arg(long, default_value = "work/generated")]
    datapack: PathBuf,
    /// World directory: created (gen) or copied from (reload).
    #[arg(long)]
    world: PathBuf,
    /// `gen`: a new world; `reload`: a copy of the saved one; `both`: gen, then reload.
    #[arg(long, default_value = "both")]
    phase: String,
    /// `anvil` or `native` (`KILN_WORLD_FORMAT`).
    #[arg(long, default_value = "anvil")]
    format: String,
    #[arg(long, default_value_t = 50)]
    count: usize,
    /// Bots per group site.
    #[arg(long, default_value_t = 10)]
    group_size: usize,
    /// Blocks between group sites: far enough apart that groups stay in their own regions
    /// (cells of 128 blocks, regions merge across gaps of two cells or less).
    #[arg(long, default_value_t = 1536.0)]
    spacing: f64,
    /// How far explorers roam from their group's site (0: no limit).
    #[arg(long, default_value_t = 320.0)]
    roam: f64,
    #[arg(long, default_value_t = 25584)]
    port: u16,
    #[arg(long, default_value_t = 4242)]
    seed: u64,
    #[arg(long, default_value_t = 10)]
    view_distance: u8,
    #[arg(long, default_value_t = 10)]
    simulation_distance: u8,
    /// Joins per second.
    #[arg(long, default_value_t = 10.0)]
    rate: f64,
    /// Seconds after everyone is in before the measured stretch starts.
    #[arg(long, default_value_t = 60.0)]
    warmup: f64,
    /// Seconds measured.
    #[arg(long, default_value_t = 180.0)]
    measure: f64,
    /// Survival roles, comma separated, dealt to the bots in turn.
    #[arg(long, default_value = "explorer,miner,builder,redstone,explorer")]
    roles: String,
    /// Seconds between chat messages per bot.
    #[arg(long, default_value_t = 120.0)]
    chat_interval: f64,
    /// Seconds into the measured stretch at which the server is told `save-all` (an autosave
    /// comes every 5 minutes of play; this puts one inside the measurement).
    #[arg(long)]
    save_all_at: Option<f64>,
    /// Passed to the server: `KILN_PHASE_DETAIL=1`.
    #[arg(long)]
    phase_detail: bool,
    /// Passed to the server: `KILN_SLOW_PRINT=<ms>`.
    #[arg(long)]
    slow_print: Option<u32>,
    /// Extra `KEY=VALUE` environment for the server.
    #[arg(long)]
    env: Vec<String>,
    /// Where logs and the JSON result go.
    #[arg(long, default_value = "bench-out")]
    out: PathBuf,
    /// Tag for file names.
    #[arg(long, default_value = "run")]
    tag: String,
    /// Windows priority class of the server and bot processes: `normal`, `above-normal` or `high`.
    /// Other programs on the machine (builds, browsers) then take less of the benchmark's CPU.
    #[arg(long, default_value = "normal")]
    priority: String,
    /// Run the server on this ssh destination (`user@host`) instead of locally.
    #[arg(long)]
    ssh: Option<String>,
    /// Working directory of the server on the remote machine.
    #[arg(long, default_value = ".")]
    remote_dir: String,
    /// Address the bots connect to (host or host:port) [default: 127.0.0.1; with --ssh, a
    /// tunnel through ssh to the server's port].
    #[arg(long)]
    connect: Option<String>,
    /// With --ssh and no --connect: the local port of the ssh tunnel (`ssh -L`) to the server,
    /// for a server machine whose game port cannot be reached directly (a VM behind NAT).
    #[arg(long, default_value_t = 35565)]
    tunnel_port: u16,
}

/// Where the server runs: here, or on another machine reached over ssh.
#[derive(Clone)]
enum Host {
    Local,
    Ssh { dest: String, dir: String },
}

/// `s` quoted for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Host {
    fn of(a: &Args) -> Host {
        match &a.ssh {
            Some(dest) => Host::Ssh { dest: dest.clone(), dir: a.remote_dir.clone() },
            None => Host::Local,
        }
    }

    fn ssh(dest: &str) -> Command {
        let mut c = Command::new("ssh");
        c.args(["-o", "BatchMode=yes", "-o", "ServerAliveInterval=15", dest]);
        c
    }

    /// Runs a shell script on the remote machine (in its working directory); its output.
    fn sh(&self, script: &str) -> Result<String> {
        let Host::Ssh { dest, dir } = self else { bail!("not a remote host") };
        let out = Self::ssh(dest).arg(format!("cd {} && {script}", sh_quote(dir))).stdin(Stdio::null()).output()?;
        if !out.status.success() {
            bail!("ssh {script:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn read(&self, path: &Path) -> String {
        match self {
            Host::Local => std::fs::read_to_string(path).unwrap_or_default(),
            Host::Ssh { .. } => self.sh(&format!("cat {} 2>/dev/null || true", sh_quote(&path.to_string_lossy()))).unwrap_or_default(),
        }
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        match self {
            Host::Local => {
                if path.exists() {
                    std::fs::remove_dir_all(path)?;
                }
                Ok(())
            }
            Host::Ssh { .. } => self.sh(&format!("rm -rf {}", sh_quote(&path.to_string_lossy()))).map(drop),
        }
    }

    /// Copies a world without the players' saved state.
    fn copy_world(&self, src: &Path, dst: &Path) -> Result<()> {
        match self {
            Host::Local => copy_dir(src, dst, &["playerdata", "players"]),
            Host::Ssh { .. } => {
                let (src, dst) = (sh_quote(&src.to_string_lossy()), sh_quote(&dst.to_string_lossy()));
                self.sh(&format!(
                    "rm -rf {dst} && mkdir -p {dst} && (cd {src} && tar cf - --exclude=./playerdata --exclude=./players .) | (cd {dst} && tar xf -)"
                ))
                .map(drop)
            }
        }
    }

    fn dir_size(&self, path: &Path) -> u64 {
        match self {
            Host::Local => dir_size(path),
            Host::Ssh { .. } => self
                .sh(&format!("du -sb {} 2>/dev/null | cut -f1", sh_quote(&path.to_string_lossy())))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
        }
    }

    fn cores(&self) -> f64 {
        match self {
            Host::Local => logical_cores(),
            Host::Ssh { .. } => self.sh("nproc").ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1.0),
        }
    }

    /// CPU seconds the server process has used, and (busy, total) CPU seconds of its machine.
    fn server_cpu(&self, server: &Server) -> (Option<f64>, Option<(f64, f64)>) {
        match self {
            Host::Local => (cpu::process(&server.child), cpu::system()),
            Host::Ssh { .. } => {
                let Some(pid) = server.remote_pid() else { return (None, None) };
                let Ok(out) = self.sh(&format!("cat /proc/{pid}/stat; head -1 /proc/stat")) else { return (None, None) };
                let mut lines = out.lines();
                // Clock ticks of 10 ms (USER_HZ).
                let process = lines.next().and_then(|l| {
                    let rest: Vec<&str> = l.rsplit_once(')')?.1.split_whitespace().collect();
                    Some((rest.get(11)?.parse::<f64>().ok()? + rest.get(12)?.parse::<f64>().ok()?) / 100.0)
                });
                let system = lines.next().and_then(|l| {
                    let v: Vec<f64> = l.split_whitespace().skip(1).filter_map(|w| w.parse().ok()).collect();
                    let total: f64 = v.iter().take(8).sum();
                    let idle = v.get(3)? + v.get(4)?;
                    Some(((total - idle) / 100.0, total / 100.0))
                });
                (process, system)
            }
        }
    }
}

/// Process creation flag for a Windows priority class.
#[cfg(windows)]
fn priority_flag(name: &str) -> u32 {
    match name {
        "above-normal" => 0x0000_8000,
        "high" => 0x0000_0080,
        _ => 0x0000_0020,
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

// ---- process times ------------------------------------------------------------------------------

#[cfg(windows)]
mod cpu {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn secs(self) -> f64 {
            (((self.high as u64) << 32) | self.low as u64) as f64 / 1e7
        }
    }

    unsafe extern "system" {
        fn GetProcessTimes(h: *mut std::ffi::c_void, c: *mut FileTime, e: *mut FileTime, k: *mut FileTime, u: *mut FileTime) -> i32;
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }

    /// CPU seconds (kernel + user) the process has used.
    pub fn process(child: &Child) -> Option<f64> {
        let mut t = [FileTime::default(); 4];
        // SAFETY: the handle belongs to a live `Child`; the out pointers are valid.
        let ok = unsafe { GetProcessTimes(child.as_raw_handle(), &mut t[0], &mut t[1], &mut t[2], &mut t[3]) };
        (ok != 0).then(|| t[2].secs() + t[3].secs())
    }

    /// (busy, total) CPU seconds of all cores since boot.
    pub fn system() -> Option<(f64, f64)> {
        let mut t = [FileTime::default(); 3];
        // SAFETY: valid out pointers.
        let ok = unsafe { GetSystemTimes(&mut t[0], &mut t[1], &mut t[2]) };
        // Kernel time includes idle time.
        (ok != 0).then(|| (t[1].secs() + t[2].secs() - t[0].secs(), t[1].secs() + t[2].secs()))
    }
}

#[cfg(not(windows))]
mod cpu {
    use std::process::Child;
    pub fn process(_: &Child) -> Option<f64> {
        None
    }
    pub fn system() -> Option<(f64, f64)> {
        None
    }
}

// ---- server process ---------------------------------------------------------------------------------

struct Server {
    child: Child,
    lines: Arc<Mutex<Vec<(u128, String)>>>,
    host: Host,
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' && it.peek() == Some(&'[') {
            for c in it.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Server {
    fn start(a: &Args, world: &Path, trace: &Path, log: &Path, format: &str) -> Result<Server> {
        let mut env: Vec<(String, String)> = vec![
            ("KILN_PORT".into(), a.port.to_string()),
            ("KILN_GENERATOR".into(), "noise".into()),
            ("KILN_SEED".into(), a.seed.to_string()),
            ("KILN_DATAPACK".into(), a.datapack.to_string_lossy().into_owned()),
            ("KILN_OPS".into(), "SB*".into()),
            ("KILN_MAX_PLAYERS".into(), (a.count + 20).to_string()),
            ("KILN_WORLD".into(), world.to_string_lossy().into_owned()),
            ("KILN_VIEW_DISTANCE".into(), a.view_distance.to_string()),
            ("KILN_SIMULATION_DISTANCE".into(), a.simulation_distance.to_string()),
            ("KILN_TICK_TRACE".into(), trace.to_string_lossy().into_owned()),
            ("RUST_LOG".into(), "info".into()),
        ];
        if format == "native" {
            env.push(("KILN_WORLD_FORMAT".into(), "native".into()));
        }
        if a.phase_detail {
            env.push(("KILN_PHASE_DETAIL".into(), "1".into()));
        }
        if let Some(ms) = a.slow_print {
            env.push(("KILN_SLOW_PRINT".into(), ms.to_string()));
        }
        for kv in &a.env {
            let (k, v) = kv.split_once('=').context("--env wants KEY=VALUE")?;
            env.push((k.into(), v.into()));
        }
        let host = Host::of(a);
        let mut cmd = match &host {
            Host::Local => {
                let mut cmd = Command::new(&a.server);
                cmd.envs(env.iter().map(|(k, v)| (k, v)));
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(priority_flag(&a.priority));
                }
                cmd
            }
            Host::Ssh { dest, dir } => {
                // The shell becomes the server (`exec`), so the pid it prints is the server's.
                let vars: Vec<String> = env.iter().map(|(k, v)| format!("{k}={}", sh_quote(v))).collect();
                let trace_dir = trace.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into());
                let script = format!(
                    "cd {} && mkdir -p {} && echo KILN_PID=$$ && exec env {} {}",
                    sh_quote(dir),
                    sh_quote(&trace_dir),
                    vars.join(" "),
                    sh_quote(&a.server.to_string_lossy())
                );
                let mut cmd = Host::ssh(dest);
                cmd.arg(script);
                cmd
            }
        };
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", a.server.display()))?;
        let lines = Arc::new(Mutex::new(Vec::new()));
        let file = Arc::new(Mutex::new(std::fs::File::create(log)?));
        for stream in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let (lines, file) = (lines.clone(), file.clone());
            std::thread::spawn(move || {
                for l in BufReader::new(stream).lines().map_while(Result::ok) {
                    let l = strip_ansi(&l);
                    let _ = writeln!(file.lock().unwrap(), "{l}");
                    lines.lock().unwrap().push((now_ms(), l));
                }
            });
        }
        Ok(Server { child, lines, host })
    }

    /// The server's pid on the remote machine.
    fn remote_pid(&self) -> Option<u32> {
        self.lines.lock().unwrap().iter().find_map(|(_, l)| l.strip_prefix("KILN_PID=")?.trim().parse().ok())
    }

    fn command(&mut self, c: &str) {
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = writeln!(stdin, "{c}");
            let _ = stdin.flush();
        }
    }

    fn wait_for(&self, what: &str, timeout: Duration) -> Result<()> {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.lines.lock().unwrap().iter().any(|(_, l)| l.contains(what)) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("the server did not log {what:?} within {timeout:?}")
    }

    fn snapshot(&self) -> Vec<(u128, String)> {
        self.lines.lock().unwrap().clone()
    }

    /// `stop`, and wait for the process to save and exit.
    fn stop(&mut self, wait: Duration) -> bool {
        self.command("stop");
        let end = Instant::now() + wait;
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                std::thread::sleep(Duration::from_millis(300));
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let (Host::Ssh { .. }, Some(pid)) = (&self.host, self.remote_pid()) {
            let _ = self.host.sh(&format!("kill -9 {pid}"));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        false
    }
}

// ---- analysis helpers -----------------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct Stats {
    n: usize,
    mean: f64,
    p50: f64,
    p99: f64,
    max: f64,
    over_50ms: f64,
}

fn stats(mut v: Vec<f64>) -> Stats {
    if v.is_empty() {
        return Stats::default();
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1];
    Stats {
        n: v.len(),
        mean: v.iter().sum::<f64>() / v.len() as f64,
        p50: at(0.5),
        p99: at(0.99),
        max: *v.last().unwrap(),
        over_50ms: v.iter().filter(|x| **x > 50.0).count() as f64 / v.len() as f64,
    }
}

fn stats_json(s: Stats) -> Value {
    json!({"ticks": s.n, "mean_ms": s.mean, "p50_ms": s.p50, "p99_ms": s.p99, "max_ms": s.max, "share_over_50ms": s.over_50ms})
}

/// `key=value` pairs of a `chunk totals:` line.
fn kv(line: &str) -> BTreeMap<String, f64> {
    line.split_whitespace().filter_map(|w| w.split_once('=')).filter_map(|(k, v)| Some((k.to_string(), v.parse().ok()?))).collect()
}

struct Report {
    unix_ms: u128,
    players: usize,
    regions: usize,
    chunks: usize,
    mspt: [f64; 4],
    phases: Vec<(String, f64)>,
}

/// A 30 s tick report line: `N players, R regions, C chunks | mspt mean ... | phase x | ...`.
fn parse_report(ms: u128, line: &str) -> Option<Report> {
    let (head, rest) = line.split_once(" | ")?;
    let msg = head.rsplit_once("kiln_sim: ").map_or(head, |x| x.1);
    let words: Vec<&str> = msg.split_whitespace().collect();
    if words.len() < 6 || words[1] != "players," {
        return None;
    }
    let players = words[0].parse().ok()?;
    let regions = words[2].parse().ok()?;
    let chunks = words[4].parse().ok()?;
    let mut parts = rest.split(" | ");
    let m: Vec<f64> = parts.next()?.split_whitespace().filter_map(|w| w.parse().ok()).collect();
    if m.len() < 4 {
        return None;
    }
    let phases = parts
        .filter_map(|p| {
            let (name, v) = p.rsplit_once(' ')?;
            Some((name.to_string(), v.parse().ok()?))
        })
        .collect();
    Some(Report { unix_ms: ms, players, regions, chunks, mspt: [m[0], m[1], m[2], m[3]], phases })
}

fn copy_dir(src: &Path, dst: &Path, skip: &[&str]) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let to = dst.join(&name);
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to, skip)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

// ---- one run ----------------------------------------------------------------------------------------------

fn run(a: &Args, world: &Path, format: &str, label: &str) -> Result<Value> {
    std::fs::create_dir_all(&a.out)?;
    // The trace is written by the server, on its machine: next to the world there.
    let trace = match &a.ssh {
        Some(_) => PathBuf::from(format!("{}-{}-{label}.ticks", world.display(), a.tag)),
        None => a.out.join(format!("{}-{label}.ticks", a.tag)),
    };
    let log = a.out.join(format!("{}-{label}.server.log", a.tag));
    let bot_log = a.out.join(format!("{}-{label}.bots.log", a.tag));
    eprintln!("[{label}] starting the server on {} ({format}, {} bots)", world.display(), a.count);
    let mut server = Server::start(a, world, &trace, &log, format)?;
    server.wait_for("Loaded", Duration::from_secs(120)).ok();
    server.wait_for("advancements", Duration::from_secs(120))?;
    std::thread::sleep(Duration::from_secs(1));
    server.command("defaultgamemode survival");
    std::thread::sleep(Duration::from_millis(500));

    // Everyone joins and walks to a site; once 95% have arrived and the warmup is over, the
    // measured stretch starts. The bots run until told to stop.
    let join_secs = a.count as f64 / a.rate;
    let host = Host::of(a);
    // The bots' way to the server: directly, or through an ssh tunnel to its machine.
    let mut tunnel = None;
    let connect = match (&a.connect, &a.ssh) {
        (Some(c), _) if c.contains(':') => c.clone(),
        (Some(c), _) => format!("{c}:{}", a.port),
        (None, None) => format!("127.0.0.1:{}", a.port),
        (None, Some(dest)) => {
            let mut t = Host::ssh(dest);
            t.args(["-N", "-o", "ExitOnForwardFailure=yes", "-c", "aes128-gcm@openssh.com", "-L"])
                .arg(format!("127.0.0.1:{}:127.0.0.1:{}", a.tunnel_port, a.port))
                .stdin(Stdio::null());
            tunnel = Some(t.spawn().context("starting the ssh tunnel")?);
            let addr: std::net::SocketAddr = ([127, 0, 0, 1], a.tunnel_port).into();
            let end = Instant::now() + Duration::from_secs(20);
            while std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err() {
                if Instant::now() > end {
                    bail!("the ssh tunnel to the server did not open");
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            addr.to_string()
        }
    };
    let mut bot_cmd = Command::new(&a.bot);
    bot_cmd
        .args(["--addr", &connect, "--count", &a.count.to_string(), "--rate", &a.rate.to_string()])
        .args(["--behavior", "survival", "--duration", "86400", "--stdin-control", "--name-prefix", "SB"])
        .args(["--group-size", &a.group_size.to_string(), "--group-spacing", &a.spacing.to_string()])
        .args(if a.roam > 0.0 { vec!["--radius".to_string(), a.roam.to_string()] } else { Vec::new() })
        .args(["--view-distance", &a.view_distance.to_string(), "--roles", &a.roles])
        .args(["--chat-interval", &a.chat_interval.to_string(), "--seed", &a.seed.to_string()])
        .args(["--report-interval", "10", "--json", "--center", "0,0", "--join-timeout", "900"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        bot_cmd.creation_flags(priority_flag(&a.priority));
    }
    let bots_started = now_ms();
    let mut bots = bot_cmd.spawn().with_context(|| format!("starting {}", a.bot.display()))?;
    let (out_buf, err_buf) = (Arc::new(Mutex::new(String::new())), Arc::new(Mutex::new(String::new())));
    for (stream, buf) in [
        (Box::new(bots.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>, out_buf.clone()),
        (Box::new(bots.stderr.take().unwrap()), err_buf.clone()),
    ] {
        std::thread::spawn(move || {
            for l in BufReader::new(stream).lines().map_while(Result::ok) {
                let mut b = buf.lock().unwrap();
                b.push_str(&l);
                b.push('\n');
            }
        });
    }

    // Wait for the arrival, then sample CPU while the measured stretch runs.
    let arrive_limit = bots_started + ((join_secs + 900.0) * 1000.0) as u128;
    let mut arrived_at: Option<u128> = None;
    let mut measure_from = u128::MAX;
    let mut measure_to = u128::MAX;
    let mut at_from: Option<(f64, f64, Option<(f64, f64)>)> = None;
    let mut at_to: Option<(f64, f64, Option<(f64, f64)>)> = None;
    let mut next_note = Instant::now();
    let mut stop_sent = false;
    let mut save_sent = false;
    loop {
        let t = now_ms();
        if arrived_at.is_none() {
            let arrived = out_buf.lock().unwrap().lines().rev().find_map(|l| {
                let (_, rest) = l.split_once(" arrived ")?;
                rest.split_whitespace().next()?.parse::<usize>().ok()
            });
            if arrived.is_some_and(|n| n as f64 >= a.count as f64 * 0.95) || t >= arrive_limit {
                arrived_at = Some(t);
                measure_from = t + (a.warmup * 1000.0) as u128;
                measure_to = measure_from + (a.measure * 1000.0) as u128;
                eprintln!("[{label}] {} bots arrived after {:.0} s; measuring from +{:.0} s", arrived.unwrap_or(0), (t - bots_started) as f64 / 1000.0, (measure_from - bots_started) as f64 / 1000.0);
            }
        }
        if let Some(at) = a.save_all_at
            && !save_sent
            && t >= measure_from.saturating_add((at * 1000.0) as u128)
        {
            save_sent = true;
            server.command("save-all");
        }
        if !stop_sent && t >= measure_to.saturating_add(2000) {
            stop_sent = true;
            if let Some(stdin) = bots.stdin.as_mut() {
                let _ = writeln!(stdin, "stop");
                let _ = stdin.flush();
            }
        }
        if at_from.is_none() && t >= measure_from {
            let (sp, sys) = host.server_cpu(&server);
            at_from = Some((sp.unwrap_or(0.0), cpu::process(&bots).unwrap_or(0.0), sys));
        }
        if at_to.is_none() && t >= measure_to {
            let (sp, sys) = host.server_cpu(&server);
            at_to = Some((sp.unwrap_or(0.0), cpu::process(&bots).unwrap_or(0.0), sys));
        }
        if let Ok(Some(_)) = bots.try_wait() {
            break;
        }
        if Instant::now() >= next_note {
            next_note += Duration::from_secs(30);
            let lines = server.snapshot();
            let last = lines.iter().rev().find_map(|(t, l)| parse_report(*t, l));
            let phase = if arrived_at.is_none() { "joining" } else if t < measure_from { "warmup" } else if t < measure_to { "measuring" } else { "wind-down" };
            eprintln!(
                "[{label}] +{:.0}s {phase}{}",
                (t - bots_started) as f64 / 1000.0,
                last.map_or(String::new(), |r| format!(": {} players, {} chunks, mspt {:.2}/{:.2}/{:.2}/{:.2}", r.players, r.chunks, r.mspt[0], r.mspt[1], r.mspt[2], r.mspt[3]))
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = bots.wait();
    if let Some(mut t) = tunnel {
        let _ = t.kill();
        let _ = t.wait();
    }
    std::thread::sleep(Duration::from_secs(1));
    let stopped_clean = server.stop(Duration::from_secs(180));
    let bot_out = out_buf.lock().unwrap().clone();
    std::fs::write(&bot_log, format!("{bot_out}\n--- stderr ---\n{}", err_buf.lock().unwrap()))?;

    // Bot report: the JSON block after the progress lines.
    let report: Value = bot_out
        .find("\n{\n")
        .and_then(|i| serde_json::from_str(&bot_out[i + 1..]).ok())
        .unwrap_or(Value::Null);

    // Tick trace: ticks of the measured stretch (and of the whole run, for comparison).
    let text = host.read(&trace);
    let mut all = Vec::new();
    let mut measured = Vec::new();
    // Ticks that took over a second: they freeze every player, whatever the cause.
    let (mut long_ticks, mut long_ms, mut longest) = (0u32, 0.0f64, 0.0f64);
    let mut full_at = None;
    for l in text.lines() {
        let mut w = l.split_whitespace();
        let (Some(t), Some(p), Some(us)) = (w.next(), w.next(), w.next()) else { continue };
        let (Ok(t), Ok(p), Ok(us)) = (t.parse::<u128>(), p.parse::<usize>(), us.parse::<f64>()) else { continue };
        let ms = us / 1000.0;
        all.push(ms);
        if ms > 1000.0 {
            long_ticks += 1;
            long_ms += ms;
            longest = longest.max(ms);
        }
        if p as f64 >= a.count as f64 * 0.98 && full_at.is_none() {
            full_at = Some(t);
        }
        if t >= measure_from && t < measure_to {
            measured.push(ms);
        }
    }
    let lines = server.snapshot();
    let in_window = |t: u128| t >= measure_from && t < measure_to;
    let reports: Vec<Report> = lines.iter().filter_map(|(t, l)| parse_report(*t, l)).collect();
    // 30 s windows ending inside the measured stretch (the last one may straddle its start).
    let window: Vec<&Report> = reports.iter().filter(|r| r.unix_ms > measure_from + 15_000 && r.unix_ms <= measure_to + 15_000).collect();
    let mut phase_sums: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for r in &window {
        for (n, v) in &r.phases {
            let e = phase_sums.entry(n.clone()).or_default();
            e.0 += v;
            e.1 += 1;
        }
    }
    let mut phases: Vec<(String, f64)> = phase_sums.into_iter().map(|(n, (s, c))| (n, s / c.max(1) as f64)).collect();
    phases.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mean_of = |f: &dyn Fn(&Report) -> f64| if window.is_empty() { 0.0 } else { window.iter().map(|r| f(r)).sum::<f64>() / window.len() as f64 };

    // Chunk totals before and after the measured stretch.
    let totals: Vec<(u128, BTreeMap<String, f64>)> =
        lines.iter().filter(|(_, l)| l.contains("chunk totals:")).map(|(t, l)| (*t, kv(l))).collect();
    let before = totals.iter().rev().find(|(t, _)| *t <= measure_from + 1000).map(|x| x.1.clone()).unwrap_or_default();
    let after = totals.iter().rev().find(|(t, _)| *t <= measure_to + 1000).map(|x| x.1.clone()).unwrap_or_default();
    let last = totals.last().map(|x| x.1.clone()).unwrap_or_default();
    let d = |k: &str| after.get(k).copied().unwrap_or(0.0) - before.get(k).copied().unwrap_or(0.0);
    let secs = a.measure;
    let threads = after.get("gen_threads").copied().unwrap_or(3.0);
    let chunk = json!({
        "generated_per_s": d("gen_done") / secs,
        "gen_thread_utilisation": d("gen_busy_ms") / 1000.0 / (secs * threads.max(1.0)),
        "gen_busy_ms_per_chunk": if d("gen_done") > 0.0 { d("gen_busy_ms") / d("gen_done") } else { 0.0 },
        "disk_loaded_per_s": d("disk_hits") / secs,
        "disk_ms_per_chunk": if d("disk_hits") > 0.0 { d("disk_ms") / d("disk_hits") } else { 0.0 },
        "disk_max_ms": last.get("disk_max_ms").copied().unwrap_or(0.0),
        "disk_miss_lookups": d("disk_misses"),
        "installed_per_s": d("installed") / secs,
        "install_ms_per_chunk": if d("installed") > 0.0 { d("install_ms") / d("installed") } else { 0.0 },
        "install_max_ms": last.get("install_max_ms").copied().unwrap_or(0.0),
        "sync_loads_in_window": d("sync_loads"),
        "sync_ms_in_window": d("sync_ms"),
        "sync_loads_whole_run": last.get("sync_loads"),
        "sync_ms_whole_run": last.get("sync_ms"),
        "sync_max_ms_whole_run": last.get("sync_max_ms"),
        "ticks_over_1s_whole_run": { "count": long_ticks, "total_ms": long_ms, "longest_ms": longest },
        "limbo_whole_run": { "n": last.get("limbo_n"), "mean_ms": last.get("limbo_mean_ms"), "p99_ms": last.get("limbo_p99_ms"), "max_ms": last.get("limbo_max_ms") },
        "join_wait_whole_run": { "n": last.get("join_wait_n"), "mean_ms": last.get("join_wait_mean_ms"), "max_ms": last.get("join_wait_max_ms") },
        "server_rss_mb_max": totals.iter().filter_map(|(_, t)| t.get("rss_mb").copied()).fold(0.0, f64::max),
        "gen_held_last": last.get("gen_held"),
        "gen_request_to_ready_ms_whole_run": {
            "n": last.get("gen_latency_n"), "mean": last.get("gen_latency_mean_ms"), "p50": last.get("gen_latency_p50_ms"),
            "p99": last.get("gen_latency_p99_ms"), "max": last.get("gen_latency_max_ms"),
        },
        "totals_whole_run": {
            "generated": last.get("gen_done"), "disk_loaded": last.get("disk_hits"), "installed": last.get("installed"), "sync_loads": last.get("sync_loads"),
        },
    });

    let server_cores = host.cores();
    let cpu_json = match (at_from, at_to) {
        (Some(f), Some(t)) => {
            let sys = match (f.2, t.2) {
                (Some(f), Some(t)) if t.1 > f.1 => Some((t.0 - f.0) / (t.1 - f.1)),
                _ => None,
            };
            json!({
                "server_cores": (t.0 - f.0) / secs,
                "bots_cores": (t.1 - f.1) / secs,
                "server_machine_cores": server_cores,
                "server_machine_utilisation": sys,
                // On the server's machine, besides the server (and the bots, when they run there).
                "background_cores": sys.map(|u| {
                    let bots = if matches!(host, Host::Local) { (t.1 - f.1) / secs } else { 0.0 };
                    (u * server_cores - (t.0 - f.0) / secs - bots).max(0.0)
                }),
            })
        }
        _ => Value::Null,
    };
    let ok_window = measured.len() as f64 > a.measure * 20.0 * 0.5;
    let result = json!({
        "label": label,
        "format": format,
        "count": a.count,
        "view_distance": a.view_distance,
        "all_bots_in_after_s": full_at.map(|t| (t.saturating_sub(bots_started)) as f64 / 1000.0),
        "bots_arrived_after_s": arrived_at.map(|t| (t.saturating_sub(bots_started)) as f64 / 1000.0),
        "mspt_measured": stats_json(stats(measured)),
        "mspt_whole_run": stats_json(stats(all)),
        "mspt_window_reports": {
            "reports": window.len(),
            "mean": mean_of(&|r| r.mspt[0]), "p50": mean_of(&|r| r.mspt[1]), "p99": mean_of(&|r| r.mspt[2]), "max": window.iter().map(|r| r.mspt[3]).fold(0.0, f64::max),
            "players": mean_of(&|r| r.players as f64), "regions": mean_of(&|r| r.regions as f64), "loaded_chunks": mean_of(&|r| r.chunks as f64),
        },
        "phases_ms_per_tick": phases.iter().take(14).map(|(n, v)| json!({n: v})).collect::<Vec<_>>(),
        "chunks": chunk,
        "cpu": cpu_json,
        "bots": report,
        "server_stopped_cleanly": stopped_clean,
        "world_size_mb": host.dir_size(world) as f64 / 1e6,
        "server_host": a.ssh.clone().unwrap_or_else(|| "local".into()),
        "measured_window_complete": ok_window,
        "in_window_reports_seen": reports.iter().filter(|r| in_window(r.unix_ms)).count(),
    });
    print_summary(&result);
    std::fs::write(a.out.join(format!("{}-{label}.json", a.tag)), serde_json::to_string_pretty(&result)?)?;
    Ok(result)
}

fn logical_cores() -> f64 {
    std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64)
}

fn f(v: &Value, path: &str) -> f64 {
    path.split('.').try_fold(v, |v, k| v.get(k)).and_then(Value::as_f64).unwrap_or(0.0)
}

fn print_summary(r: &Value) {
    let m = &r["mspt_measured"];
    println!("== {} ({}, {} bots, view {}) ==", r["label"].as_str().unwrap_or(""), r["format"].as_str().unwrap_or(""), r["count"], r["view_distance"]);
    println!(
        "mspt   mean {:.2}  p50 {:.2}  p99 {:.2}  max {:.2}  over 50 ms {:.2}%  ({} ticks measured; whole run mean {:.2}, p99 {:.2}, max {:.2})",
        f(m, "mean_ms"), f(m, "p50_ms"), f(m, "p99_ms"), f(m, "max_ms"), f(m, "share_over_50ms") * 100.0, f(m, "ticks"),
        f(r, "mspt_whole_run.mean_ms"), f(r, "mspt_whole_run.p99_ms"), f(r, "mspt_whole_run.max_ms")
    );
    println!(
        "world  {:.0} players, {:.0} regions, {:.0} loaded chunks (mean over the measured reports)",
        f(r, "mspt_window_reports.players"), f(r, "mspt_window_reports.regions"), f(r, "mspt_window_reports.loaded_chunks")
    );
    let phases: Vec<String> = r["phases_ms_per_tick"]
        .as_array()
        .map(|a| a.iter().filter_map(|o| o.as_object()?.iter().next().map(|(k, v)| format!("{k} {:.2}", v.as_f64().unwrap_or(0.0)))).collect())
        .unwrap_or_default();
    println!("phases (ms per tick) {}", phases.join(" | "));
    let c = &r["chunks"];
    println!(
        "chunks generated {:.1}/s (gen threads {:.0}% busy, {:.1} ms of thread time per chunk); from disk {:.1}/s ({:.3} ms each, max {:.1} ms); installed {:.1}/s ({:.3} ms each, max {:.1} ms); sync loads {:.0} ({:.0} ms)",
        f(c, "generated_per_s"), f(c, "gen_thread_utilisation") * 100.0, f(c, "gen_busy_ms_per_chunk"), f(c, "disk_loaded_per_s"),
        f(c, "disk_ms_per_chunk"), f(c, "disk_max_ms"), f(c, "installed_per_s"), f(c, "install_ms_per_chunk"), f(c, "install_max_ms"),
        f(c, "sync_loads_in_window"), f(c, "sync_ms_in_window")
    );
    println!(
        "gen request->ready (whole run) mean {:.0} p50 {:.0} p99 {:.0} max {:.0} ms",
        f(c, "gen_request_to_ready_ms_whole_run.mean"), f(c, "gen_request_to_ready_ms_whole_run.p50"),
        f(c, "gen_request_to_ready_ms_whole_run.p99"), f(c, "gen_request_to_ready_ms_whole_run.max")
    );
    println!(
        "stalls whole run: {:.0} chunks generated on the tick thread ({:.1} s in all, longest {:.1} s); {:.0} ticks over 1 s ({:.1} s in all, longest {:.1} s)",
        f(c, "sync_loads_whole_run"), f(c, "sync_ms_whole_run") / 1000.0, f(c, "sync_max_ms_whole_run") / 1000.0,
        f(c, "ticks_over_1s_whole_run.count"), f(c, "ticks_over_1s_whole_run.total_ms") / 1000.0, f(c, "ticks_over_1s_whole_run.longest_ms") / 1000.0
    );
    println!(
        "waits whole run: teleported into terrain not made yet {:.0} times (waited mean {:.0}, p99 {:.0}, max {:.0} ms); joins waiting for their chunk {:.0} (mean {:.0}, max {:.0} ms)",
        f(c, "limbo_whole_run.n"), f(c, "limbo_whole_run.mean_ms"), f(c, "limbo_whole_run.p99_ms"), f(c, "limbo_whole_run.max_ms"),
        f(c, "join_wait_whole_run.n"), f(c, "join_wait_whole_run.mean_ms"), f(c, "join_wait_whole_run.max_ms")
    );
    println!(
        "memory server peak {:.0} MiB (reports every 30 s); generator holds {:.0} unfinished chunks at the end",
        f(c, "server_rss_mb_max"), f(c, "gen_held_last")
    );
    let b = &r["bots"];
    let lat = |name: &str, k: &str| {
        let l = &b[k];
        println!(
            "bots   {name:<22} n {:.0} mean {:.0} p50 {:.0} p90 {:.0} p99 {:.0} max {:.0} ms",
            f(l, "n"), f(l, "mean_ms"), f(l, "p50_ms"), f(l, "p90_ms"), f(l, "p99_ms"), f(l, "max_ms")
        );
    };
    lat("chunk arrival", "chunk_latency");
    lat("view ready (walk)", "area_ready_walk");
    lat("view ready (teleport)", "area_ready_teleport");
    lat("view ready (join)", "area_ready_join");
    let t = &b["traffic"];
    let (walk, stall) = (f(t, "walk_ticks"), f(t, "stall_ticks"));
    println!(
        "bots   joined {:.0}, failed {:.0}, dropped {:.0}; corrections {:.0} (+{:.0} repeats); decode errors {:.0}; waited for chunks {:.1}% of walking time; walked {:.0} blocks; deaths {:.0}",
        f(b, "joined"), f(b, "failed"), f(b, "dropped"), f(t, "teleports"), f(t, "teleport_resends"), f(t, "decode_errors"), stall / (walk + stall).max(1.0) * 100.0, f(t, "walked_dm") / 10.0, f(t, "deaths")
    );
    println!(
        "bots   dig {:.0} started / {:.0} done / {:.0} rejected; placed {:.0} (rejected {:.0}, wrong state {:.0}); containers {:.0}; commands {:.0}; chat {:.0}",
        f(t, "dig_started"), f(t, "dig_done"), f(t, "dig_rejected"), f(t, "placed"), f(t, "place_rejected"), f(t, "place_wrong_state"),
        f(t, "containers_opened"), f(t, "commands"), f(t, "chat_sent")
    );
    println!(
        "cpu    server {:.2} cores, bots {:.2} cores; server's machine {:.0}% busy (other programs about {:.1} cores of {:.0})",
        f(&r["cpu"], "server_cores"), f(&r["cpu"], "bots_cores"), f(&r["cpu"], "server_machine_utilisation") * 100.0,
        f(&r["cpu"], "background_cores"), f(&r["cpu"], "server_machine_cores")
    );
    if let Some(p) = b["problems"].as_array() {
        for x in p.iter().take(12) {
            println!("problem {} x {}", x[1], x[0].as_str().unwrap_or(""));
        }
    }
    if let Some(p) = b["disconnect_reasons"].as_array() {
        for x in p.iter().take(6) {
            println!("ended  {} x {}", x[1], x[0].as_str().unwrap_or(""));
        }
    }
}

fn main() -> Result<()> {
    let a = Args::parse();
    let mut results = Vec::new();
    let phases: Vec<&str> = match a.phase.as_str() {
        "both" => vec!["gen", "reload"],
        p @ ("gen" | "reload") => vec![p],
        p => bail!("--phase gen, reload or both, not {p}"),
    };
    let host = Host::of(&a);
    let reload_dir = PathBuf::from(format!("{}-reload", a.world.display()));
    for p in phases {
        match p {
            "gen" => {
                host.remove_dir(&a.world)?;
                results.push(run(&a, &a.world, &a.format, &format!("gen-{}", a.format))?);
            }
            _ => {
                host.remove_dir(&reload_dir)?;
                // The same chunks, without the bots' saved positions: they start from the spawn again.
                host.copy_world(&a.world, &reload_dir)?;
                results.push(run(&a, &reload_dir, &a.format, &format!("reload-{}", a.format))?);
            }
        }
    }
    std::fs::write(a.out.join(format!("{}-all.json", a.tag)), serde_json::to_string_pretty(&results)?)?;
    Ok(())
}
