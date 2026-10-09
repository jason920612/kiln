//! The `async-tasks` world (WASI 0.3 component-model async): a plugin's second component runs
//! jobs off the tick (HTTP to a granted host, timers in server ticks, a small key-value store),
//! and a hot reload interrupts the jobs in flight and lets the new generation submit them again.

use kiln_plugin_host::{Actor, ExecMode, GlobalValue, PlayerAt, PlayerInfo, PluginRuntime, RuntimeConfig, World, examples};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ALICE: u128 = 0xa11ce;

struct Alice;

impl World for Alice {
    fn player(&self, uuid: u128) -> Option<PlayerAt> {
        (uuid == ALICE).then(|| PlayerAt { uuid, level: 0, region: 1, name: "Alice".into(), operator: false, info: PlayerInfo::default() })
    }
    fn owner(&self, _: u32, _: i32, _: i32) -> Option<u64> {
        Some(1)
    }
}

fn alice() -> Actor<'static> {
    Actor::new(ALICE, "Alice", false)
}

fn runtime(mode: ExecMode, data_dir: Option<std::path::PathBuf>) -> PluginRuntime {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let loaded = vec![examples::load("webhook", "").unwrap()];
    let cfg = RuntimeConfig { mode, data_dir, call_budget: Duration::from_millis(500), ..RuntimeConfig::default() };
    let mut rt = PluginRuntime::new(loaded, cfg).unwrap();
    assert_eq!(rt.ids(), ["webhook"], "the plugin loaded");
    rt.sync_regions(0, [1]);
    rt
}

fn texts(rt: &mut PluginRuntime) -> Vec<String> {
    rt.take_messages().iter().map(|m| m.text.iter().map(|s| s.text.as_str()).collect()).collect()
}

/// Runs ticks until the plugin has told the player something that matches, or time runs out.
fn until(rt: &mut PluginRuntime, what: &str, mut seen: impl FnMut(&str) -> bool) -> (Vec<String>, u32) {
    let start = Instant::now();
    let mut all = Vec::new();
    let mut ticks = 0;
    while start.elapsed() < Duration::from_secs(20) {
        rt.begin_tick_in(&Alice);
        ticks += 1;
        all.extend(texts(rt));
        if all.iter().any(|t| seen(t)) {
            return (all, ticks);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no message for `{what}` within 20 s; got {all:?}");
}

/// A small HTTP server on a free port: answers every request with `hello`, records the request
/// lines and bodies.
fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let want = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if body.len() >= want {
                        seen.lock().unwrap().push(format!("{}|{}", head.lines().next().unwrap_or(""), body));
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        }
    });
    (port, log)
}

#[test]
fn jobs_fetch_post_sleep_and_remember_off_the_tick() {
    let data = std::env::temp_dir().join(format!("kiln-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let mut rt = runtime(ExecMode::Ordered, Some(data.clone()));
    let a = alice();
    rt.player_joined(&a);
    let (port, log) = serve();
    let say = |rt: &mut PluginRuntime, args: &str| -> String { rt.run_command(0, Some(&a), "web", args).iter().map(|s| s.text.as_str()).collect() };

    // A GET to the granted host: the game side returns at once, the answer comes later.
    assert_eq!(say(&mut rt, &format!("fetch http://127.0.0.1:{port}/status")), "Job 1 (fetch) started.");
    let (got, _) = until(&mut rt, "fetch", |t| t.contains("200 hello"));
    assert!(got.iter().any(|t| t == "[web] 200 hello"), "{got:?}");
    assert_eq!(log.lock().unwrap()[0], "GET /status HTTP/1.1|");
    // A POST with a body.
    say(&mut rt, &format!("post http://127.0.0.1:{port}/hook payload text"));
    until(&mut rt, "post", |t| t == "[web] 200");
    assert_eq!(log.lock().unwrap()[1], "POST /hook HTTP/1.1|payload text");
    // A host the plugin has no capability for is denied.
    say(&mut rt, "fetch http://example.com/");
    let (got, _) = until(&mut rt, "denied", |t| t.contains("failed"));
    assert!(got.iter().any(|t| t.contains("Denied") && t.contains("example.com")), "{got:?}");

    // A timer counts server ticks: three ticks of sleep are not over after one.
    say(&mut rt, "sleep 3");
    let (_, ticks) = until(&mut rt, "sleep", |t| t == "[web] slept 3");
    assert!((3..=6).contains(&ticks), "the sleep took {ticks} ticks");

    // Storage: written, read back, counted, and kept across a restart.
    say(&mut rt, "remember colour=teal");
    until(&mut rt, "remember", |t| t == "[web] remembered");
    say(&mut rt, "recall colour");
    until(&mut rt, "recall", |t| t == "[web] teal");
    for i in 1..=3 {
        say(&mut rt, "count");
        until(&mut rt, "count", |t| t == format!("[web] {i}"));
    }
    drop(rt);
    let mut rt = runtime(ExecMode::Ordered, Some(data.clone()));
    rt.player_joined(&a);
    say(&mut rt, "recall colour");
    until(&mut rt, "recall after a restart", |t| t == "[web] teal");
    say(&mut rt, "count");
    until(&mut rt, "count after a restart", |t| t == "[web] 4");
    let _ = std::fs::remove_dir_all(&data);
}

/// A reload drops the tasks instance with its jobs in flight. The plugin hears of each as a
/// cancelled task (reason reload) and submits it again; the job then runs in the new
/// generation, and its answer arrives exactly once.
#[test]
fn a_reload_interrupts_jobs_and_the_new_generation_submits_them_again() {
    let mut rt = runtime(ExecMode::Ordered, None);
    let a = alice();
    rt.player_joined(&a);
    let say = |rt: &mut PluginRuntime, args: &str| -> String { rt.run_command(0, Some(&a), "web", args).iter().map(|s| s.text.as_str()).collect() };
    assert_eq!(say(&mut rt, "sleep 30"), "Job 1 (sleep) started.");
    for _ in 0..5 {
        rt.begin_tick_in(&Alice);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(texts(&mut rt).is_empty(), "still sleeping");
    let (manifest, wasm) = examples::load("webhook", "").unwrap();
    let r = rt.reload("webhook", manifest, &wasm).unwrap();
    assert_eq!((r.generation, r.cancelled_tasks), (1, 1), "the job in flight was interrupted");
    assert_eq!(rt.global_value("webhook", "interrupted"), Some(GlobalValue::Int(1)), "the new generation was told");
    // Not done after the 25 ticks the first run had left: the job started over.
    for _ in 0..26 {
        rt.begin_tick_in(&Alice);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(texts(&mut rt).is_empty(), "restarted, not resumed");
    let (got, _) = until(&mut rt, "the restarted sleep", |t| t == "[web] slept 30");
    assert_eq!(got.iter().filter(|t| t.contains("slept")).count(), 1, "answered once: {got:?}");
    // And once more, quietly: nothing else arrives.
    for _ in 0..40 {
        rt.begin_tick_in(&Alice);
    }
    assert!(texts(&mut rt).is_empty());
}

/// Strict mode keeps the outside world away: jobs fail at once, deterministically.
#[test]
fn strict_mode_has_no_async_tasks() {
    let mut rt = runtime(ExecMode::Strict, None);
    let a = alice();
    rt.player_joined(&a);
    rt.run_command(0, Some(&a), "web", "fetch http://127.0.0.1:1/");
    rt.begin_tick_in(&Alice);
    let got = texts(&mut rt);
    assert_eq!(got, ["[web] failed: async tasks are not available in strict mode"]);
}
