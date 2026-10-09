//! The example plugins against the host: spawn protection (fail-closed, cell-scoped claims,
//! a permission filter), the chat formatter (cancellable rewrite), the counter (player state
//! from observe batches plus a global atomic add), the heartbeat (tasks, results, hot reload,
//! the host's clock and random stream), petting (entity-scoped state), failure policies,
//! filters, rate limits, strict mode, write buffering, persistence, the `.cwasm` cache and the
//! cost of a cancellable call.

use kiln_plugin_host::{
    Actor, CellKey, ChatOutcome, EntityData, EntityRef, ExecMode, GlobalValue, PlayerAt, PluginRuntime, Registries, RuntimeConfig,
    Verdict, World, examples,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SPAWN: [i32; 3] = [8, -60, 8];

const STONE: u32 = 1;
const GRASS: u32 = 2;
const DIRT: u32 = 3;
const NETHERRACK: u32 = 4;
const OAK_LOG: u32 = 5;
const COW: u32 = 0;
const ZOMBIE: u32 = 1;

fn registries() -> Arc<Registries> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let r = Registries::new(
        s(&["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]),
        s(&["minecraft:air", "minecraft:stone", "minecraft:grass_block", "minecraft:dirt", "minecraft:netherrack", "minecraft:oak_log"]),
        s(&["minecraft:air", "minecraft:stone", "minecraft:water_bucket"]),
        s(&["minecraft:cow", "minecraft:zombie"]),
    );
    Arc::new(r.with_tags(Arc::new(|kind, tag| match (kind, tag) {
        (kiln_plugin_host::RegistryKind::Block, "minecraft:logs") => Some(vec![OAK_LOG]),
        _ => None,
    })))
}

fn config(call_budget: Duration) -> RuntimeConfig {
    RuntimeConfig { spawn: SPAWN, call_budget, registries: registries(), ..RuntimeConfig::default() }
}

/// Tests run in parallel with compilations; a generous budget keeps preemption from turning
/// into timeouts (the timeout test sets the real default).
fn runtime(plugins: &[(&str, &str)], data_dir: Option<std::path::PathBuf>) -> PluginRuntime {
    runtime_cfg(plugins, RuntimeConfig { data_dir, ..config(Duration::from_millis(200)) })
}

fn runtime_cfg(plugins: &[(&str, &str)], cfg: RuntimeConfig) -> PluginRuntime {
    // Plugin warnings (traps, timeouts) in the test output.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let loaded = plugins.iter().map(|(id, extra)| examples::load(id, extra).unwrap()).collect();
    let mut rt = PluginRuntime::new(loaded, cfg).unwrap();
    assert_eq!(rt.ids().len(), plugins.len(), "every plugin loaded");
    rt.sync_regions(0, [1]);
    rt
}

fn alice() -> Actor<'static> {
    Actor::new(0xa11ce, "Alice", false)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kiln-plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn calls(rt: &PluginRuntime) -> u64 {
    rt.stat("calls")
}

fn stat(rt: &PluginRuntime, name: &str) -> u64 {
    rt.stat(name)
}

fn text(spans: &[kiln_plugin_host::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

fn texts(rt: &mut PluginRuntime) -> Vec<String> {
    rt.take_messages().iter().map(|m| text(&m.text)).collect()
}

#[test]
fn spawn_protection_denies_near_spawn_and_records_claims_per_cell() {
    let mut rt = runtime(&[("spawn-protection", "")], None);
    rt.sync_regions(1, [2]);
    let a = alice();
    let r = rt.region_mut(0, 1).unwrap();
    let near = [SPAWN[0] + 5, SPAWN[1], SPAWN[2] - 16];
    match r.block_break(&a, near, GRASS) {
        Verdict::Deny(Some(msg)) => assert!(msg[0].text.contains("protected")),
        v => panic!("expected a denial with a message, got {v:?}"),
    }
    assert!(matches!(r.block_place(&a, near, [near[0], near[1] - 1, near[2]], Some(1)), Verdict::Deny(_)));
    // Outside the square: allowed.
    assert_eq!(r.block_break(&a, [SPAWN[0] + 17, SPAWN[1], SPAWN[2]], STONE), Verdict::Allow);
    // An operator: the manifest's bypass-permission lets it through without a call.
    let before = calls(&rt);
    let op = Actor { operator: true, ..a };
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&op, near, STONE), Verdict::Allow);
    assert_eq!(calls(&rt), before, "operators are filtered on the host");
    // Another level.
    assert_eq!(rt.region_mut(1, 2).unwrap().block_break(&a, near, NETHERRACK), Verdict::Allow);
    // The claim and the denial count live in the cell.
    let cell = CellKey::of_block(0, near[0], near[2]);
    let claim = rt.cell_value(cell, "spawn-protection", "claim").expect("claim recorded");
    assert_eq!(claim, [SPAWN[0], SPAWN[2], 16].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(2i64.to_le_bytes().to_vec()));
}

#[test]
fn chat_formatter_rewrites_and_cancels() {
    let mut rt = runtime(&[("chat-format", "prefix = \"* \"")], None);
    let r = rt.region_mut(0, 1).unwrap();
    match r.chat(&alice(), "hello there") {
        ChatOutcome::Rewrite(spans) => {
            assert_eq!(text(&spans), "* [Alice] \u{bb} hello there");
            assert_eq!(spans[2].color.as_deref(), Some("gold"));
        }
        other => panic!("expected a rewrite, got {other:?}"),
    }
    assert_eq!(r.chat(&alice(), "Griefing tips"), ChatOutcome::Cancel);
}

#[test]
fn counter_counts_per_player_and_globally() {
    let mut rt = runtime(&[("counter", "")], None);
    assert_eq!(rt.commands().len(), 1);
    assert_eq!(rt.commands()[0].name, "broken");
    let (a, b) = (alice(), Actor::new(0xb0b, "Bob", false));
    rt.sync_regions(0, [1, 2]);
    for _ in 0..3 {
        rt.region_mut(0, 1).unwrap().observe_block(true, &a, [0, 0, 0], STONE);
    }
    rt.region_mut(0, 2).unwrap().observe_block(true, &b, [900, 0, 0], DIRT);
    rt.region_mut(0, 2).unwrap().observe_block(false, &b, [900, 0, 0], DIRT);
    rt.region_mut(0, 1).unwrap().flush_observed();
    rt.region_mut(0, 2).unwrap().flush_observed();
    assert_eq!(rt.player_value(a.uuid, "counter", "broken"), Some(3i64.to_le_bytes().to_vec()));
    assert_eq!(rt.player_value(b.uuid, "counter", "broken"), Some(1i64.to_le_bytes().to_vec()));
    // The global adds apply in B0.
    assert_eq!(rt.global_value("counter", "broken-total"), None);
    rt.begin_tick();
    assert_eq!(rt.global_value("counter", "broken-total"), Some(GlobalValue::Int(4)));
    let reply = text(&rt.run_command(0, Some(&a), "broken", ""));
    assert_eq!(reply, "You broke 3 blocks; everyone: 4");
}

#[test]
fn fail_closed_keeps_denying_through_traps_timeouts_and_demotion() {
    // Handlers trap at y = 70 and spin at y = 71 (after writing the cell's denial count).
    let mut rt = runtime(&[("spawn-protection", "chaos = \"trap\"\nchaos_y = 70")], None);
    let a = alice();
    let near = |y| [SPAWN[0], y, SPAWN[2]];
    let cell = CellKey::of_block(0, SPAWN[0], SPAWN[2]);
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.block_break(&a, near(0), STONE), Verdict::Deny(Some(_))));
    // A trap denies without a message and leaves no write behind.
    assert_eq!(r.block_break(&a, near(70), STONE), Verdict::Deny(None));
    // Far from spawn a trap still denies (fail-closed), and the instance came back.
    assert_eq!(r.block_break(&a, [5000, 70, 0], STONE), Verdict::Deny(None));
    assert_eq!(r.block_break(&a, [5000, 0, 0], STONE), Verdict::Allow);
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(1i64.to_le_bytes().to_vec()), "trapped calls wrote nothing");
    assert_eq!((stat(&rt, "traps"), stat(&rt, "timeouts")), (2, 0), "calls {}", calls(&rt));
    assert!(!rt.is_demoted(0), "traps are not strikes");

    let mut rt = runtime_cfg(&[("spawn-protection", "chaos = \"spin\"\nchaos_y = 71")], config(RuntimeConfig::default().call_budget));
    let r = rt.region_mut(0, 1).unwrap();
    for i in 0..3 {
        let start = Instant::now();
        assert_eq!(r.block_break(&a, near(71), STONE), Verdict::Deny(None), "timeout {i}");
        let took = start.elapsed();
        println!("timed-out call {i}: {took:?}");
        // About 1 ms on an idle machine (Windows wakes the epoch thread about every 1 ms);
        // the bound only allows for a loaded test machine (the whole suite in parallel with
        // other builds has starved the epoch thread for over a second).
        assert!(took < Duration::from_secs(5), "the epoch deadline stopped the loop ({took:?})");
    }
    assert!(rt.is_demoted(0), "three strikes demote");
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), None, "timed-out calls wrote nothing");
    // Demoted: no more calls, but fail-closed still denies, even where it would allow.
    let before = calls(&rt);
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&a, [5000, 0, 0], STONE), Verdict::Deny(None));
    assert_eq!(calls(&rt), before, "a demoted plugin is not called for cancellable events");
}

/// Strict mode: the budget is fuel, so a runaway handler stops after the same amount of work
/// on every run, however loaded the machine is, and the plugin's clock and random stream come
/// from the tick and the seed.
#[test]
fn strict_mode_budgets_are_fuel_and_the_environment_is_seeded() {
    let strict = |seed| RuntimeConfig { mode: ExecMode::Strict, seed, call_fuel: 200_000, ..config(Duration::from_secs(1)) };
    let a = alice();
    let near = |y| [SPAWN[0], y, SPAWN[2]];
    let mut rt = runtime_cfg(&[("spawn-protection", "chaos = \"spin\"\nchaos_y = 71")], strict(0));
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.block_break(&a, near(0), STONE), Verdict::Deny(Some(_))), "normal calls fit the budget");
    for _ in 0..3 {
        assert_eq!(r.block_break(&a, near(71), STONE), Verdict::Deny(None));
    }
    assert_eq!(stat(&rt, "timeouts"), 3);
    assert!(rt.is_demoted(0));

    let roll = |seed: u64, ticks: usize| {
        let mut rt = runtime_cfg(&[("heartbeat", "")], strict(seed));
        for _ in 0..ticks {
            rt.begin_tick();
        }
        text(&rt.run_command(0, Some(&a), "beat", "roll"))
    };
    let first = roll(7, 3);
    assert_eq!(first, roll(7, 3), "same seed, same tick: same roll and time");
    assert!(first.contains(&format!("at {} ms (tick 3)", 1_600_000_000_000u64 + 150)), "{first}");
    assert_ne!(roll(7, 4), first, "the tick changes the roll and the clock");
    assert_ne!(roll(8, 3), first, "so does the seed");
}

#[test]
fn fail_open_carries_on_after_a_trap() {
    // The ledger's chat subscription is fail-open: a trapping `paytrap` passes the chat on.
    let mut rt = runtime(&[("ledger", ""), ("chat-format", "")], None);
    let a = alice();
    rt.player_joined(&a);
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.chat(&a, "paytrap 5 someone"), ChatOutcome::Rewrite(_)), "later plugins still run");
    assert_eq!(rt.player_value(a.uuid, "ledger", "balance"), Some(100i64.to_le_bytes().to_vec()), "the trapped debit did not stick");
    rt.begin_tick();
    assert_eq!(rt.global_value("ledger", "escrow:someone"), None, "the trapped credit did not stick");
}

#[test]
fn consecutive_calls_get_fresh_handles() {
    // Every call gets a new serial in its handles; consecutive calls work with their own.
    let mut rt = runtime(&[("ledger", "")], None);
    let a = alice();
    rt.player_joined(&a);
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.chat(&a, "pay 1 x"), ChatOutcome::Cancel);
    assert_eq!(r.chat(&a, "pay 1 x"), ChatOutcome::Cancel);
    assert_eq!(rt.player_value(a.uuid, "ledger", "balance"), Some(98i64.to_le_bytes().to_vec()));
}

/// Host-side filters: events outside a subscription's filter never reach the plugin.
#[test]
fn manifest_filters_keep_events_on_the_host() {
    let (mut manifest, wasm) = examples::load("spawn-protection", "").unwrap();
    for s in &mut manifest.subscriptions {
        if s.event == kiln_plugin_host::EventKind::BlockBreak {
            s.filter.blocks = vec!["#minecraft:logs".into(), "minecraft:grass_block".into()];
            s.filter.area = Some(kiln_plugin_host::Area { level: Some("minecraft:overworld".into()), center: None, radius: 32 });
        }
    }
    let mut rt = PluginRuntime::new(vec![(manifest, wasm)], config(Duration::from_millis(200))).unwrap();
    rt.sync_regions(0, [1]);
    let a = alice();
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.block_break(&a, SPAWN, OAK_LOG), Verdict::Deny(_)), "a log (by tag) near spawn reaches the plugin");
    assert!(matches!(r.block_break(&a, SPAWN, GRASS), Verdict::Deny(_)), "so does grass (by key)");
    let before = calls(&rt);
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&a, SPAWN, STONE), Verdict::Allow, "stone is outside the block filter");
    assert_eq!(r.block_break(&a, [SPAWN[0] + 40, SPAWN[1], SPAWN[2]], OAK_LOG), Verdict::Allow, "outside the area");
    assert_eq!(calls(&rt), before, "filtered events cost no call");
}

/// Per-player token buckets: a player who floods events runs out alone; the policy applies
/// without a call or a strike, and the bucket refills with ticks.
#[test]
fn player_buckets_limit_one_player_only() {
    let cfg = RuntimeConfig { player_burst: 4, player_events_per_second: 20, ..config(Duration::from_millis(200)) };
    let mut rt = runtime_cfg(&[("spawn-protection", "")], cfg);
    let (a, b) = (alice(), Actor::new(0xb0b, "Bob", false));
    let far = [SPAWN[0] + 500, 0, SPAWN[2]];
    let r = rt.region_mut(0, 1).unwrap();
    for i in 0..4 {
        assert_eq!(r.block_break(&a, far, STONE), Verdict::Allow, "event {i} within the burst");
    }
    assert_eq!(r.block_break(&a, far, STONE), Verdict::Deny(None), "fail-closed once the bucket is empty");
    assert_eq!(r.block_break(&b, far, STONE), Verdict::Allow, "another player is not affected");
    assert_eq!(stat(&rt, "rate-limited"), 1);
    assert!(!rt.is_demoted(0), "no strike");
    rt.begin_tick();
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&a, far, STONE), Verdict::Allow, "one event per tick refills");
    assert_eq!(r.block_break(&a, far, STONE), Verdict::Deny(None));
}

/// A world with one player online in region 1 of the overworld.
struct OneRegion(Vec<u128>);

impl World for OneRegion {
    fn player(&self, uuid: u128) -> Option<PlayerAt> {
        self.0.contains(&uuid).then(|| PlayerAt { uuid, level: 0, region: 1, name: "Alice".into(), operator: false, info: Default::default() })
    }
    fn owner(&self, _: u32, _: i32, _: i32) -> Option<u64> {
        Some(1)
    }
}

/// Hot reload with tasks in flight: the global instance's memory crosses as the state blob,
/// player data stays in its namespace, tasks of the old generation are cancelled with a
/// notice and scheduled again by the new one, so each runs exactly once, as the new version.
#[test]
fn reload_keeps_state_and_reschedules_tasks_in_flight() {
    let mut rt = runtime(&[("heartbeat", "")], None);
    let world = OneRegion(vec![alice().uuid]);
    let a = alice();
    rt.player_joined(&a);
    assert_eq!(text(&rt.run_command(0, Some(&a), "beat", "2")), "[v1] scheduled in 2 ticks");
    assert_eq!(text(&rt.run_command(0, Some(&a), "beat", "8")), "[v1] scheduled in 8 ticks");
    for _ in 0..3 {
        rt.begin_tick_in(&world);
    }
    // Tick 2: the first ping and beat ran (in v1); tick 3: its result came back.
    assert_eq!(texts(&mut rt), ["[v1] ping 1", "[v1] 1 pings so far"]);
    assert_eq!(text(&rt.run_command(0, Some(&a), "beat", "")), "[v1] 1 beats");
    assert_eq!(rt.pending_tasks(), 3, "the welcome, the second ping and the second beat");
    let instantiations = stat(&rt, "instantiations");

    let (mut manifest, wasm) = examples::load("heartbeat", "").unwrap();
    manifest.config.insert("label".into(), "v2".into());
    let r = rt.reload("heartbeat", manifest, &wasm).unwrap();
    assert_eq!((r.generation, r.blob, r.cancelled_tasks, r.region_instances), (1, Some(8), 3, 1));
    assert_eq!(rt.generation(0), 1);
    assert_eq!(stat(&rt, "instantiations"), instantiations + 2, "a new global and region instance");
    assert_eq!(text(&rt.run_command(0, Some(&a), "beat", "")), "[v2] 1 beats", "the blob carried the count");
    assert_eq!(rt.pending_tasks(), 3, "rescheduled by the new generation");

    let mut heard = Vec::new();
    for _ in 0..30 {
        rt.begin_tick_in(&world);
        heard.extend(texts(&mut rt));
    }
    assert_eq!(heard, ["[v2] ping 2".to_owned(), "[v2] 2 pings so far".into(), "[v2] welcome, Alice! Your roll: ".to_owned() + &roll(&rt, a.uuid)]);
    assert_eq!(text(&rt.run_command(0, Some(&a), "beat", "")), "[v2] 2 beats", "every beat ran exactly once");
    assert_eq!(rt.player_value(a.uuid, "heartbeat", "pings"), Some(2i64.to_le_bytes().to_vec()), "player data kept");
    assert_eq!(rt.pending_tasks(), 0);
}

fn roll(rt: &PluginRuntime, uuid: u128) -> String {
    let v = rt.player_value(uuid, "heartbeat", "roll").expect("roll");
    i64::from_le_bytes(v.try_into().unwrap()).to_string()
}

#[test]
fn tasks_of_players_who_left_are_cancelled_with_a_notice() {
    let mut rt = runtime(&[("heartbeat", "")], None);
    let a = alice();
    rt.player_joined(&a);
    rt.run_command(0, Some(&a), "beat", "1");
    rt.begin_tick_in(&OneRegion(Vec::new()));
    assert_eq!(stat(&rt, "tasks-cancelled"), 1, "the ping");
    assert_eq!(stat(&rt, "tasks-run"), 1, "the beat");
    assert_eq!(rt.pending_tasks(), 1, "the welcome is not due yet");
}

/// Entity-scoped state travels with the entity (the embedder stores it in the entity's NBT):
/// it is the same data whichever region the entity is in; the entity-type filter keeps other
/// entities from the plugin.
#[test]
fn entity_data_follows_the_entity() {
    let mut rt = runtime(&[("petting", "")], None);
    rt.sync_regions(0, [1, 2]);
    let a = alice();
    let mut data = EntityData::new();
    let cow = |rt: &mut PluginRuntime, region: u64, data: &mut EntityData| {
        let mut e = EntityRef { uuid: 77, kind: COW, pos: [0.5, 64.0, 0.5], data };
        rt.region_mut(0, region).unwrap().entity_interact(&a, &mut e)
    };
    assert_eq!(cow(&mut rt, 1, &mut data), Verdict::Allow);
    assert_eq!(cow(&mut rt, 2, &mut data), Verdict::Allow);
    assert_eq!(data["petting"]["pets"], 2i64.to_le_bytes());
    assert_eq!(texts(&mut rt), ["You petted this cow once.", "You petted this cow 2 times."]);
    let before = calls(&rt);
    let mut other = EntityData::new();
    let mut zombie = EntityRef { uuid: 78, kind: ZOMBIE, pos: [0.5, 64.0, 0.5], data: &mut other };
    assert_eq!(rt.region_mut(0, 1).unwrap().entity_interact(&a, &mut zombie), Verdict::Allow);
    assert_eq!(calls(&rt), before, "filtered by entity type");
    assert!(other.is_empty());
}

#[test]
fn namespaces_persist_across_restarts() {
    let dir = temp_dir("persist");
    let a = alice();
    let cell = CellKey::of_block(0, SPAWN[0], SPAWN[2]);
    {
        let mut rt = runtime(&[("counter", ""), ("spawn-protection", "")], Some(dir.clone()));
        rt.player_joined(&a);
        let r = rt.region_mut(0, 1).unwrap();
        r.observe_block(true, &a, [0, 0, 0], STONE);
        r.flush_observed();
        assert!(matches!(r.block_break(&a, SPAWN, STONE), Verdict::Deny(_)));
        rt.begin_tick();
        rt.player_left(&a);
        assert!(!rt.player_loaded(a.uuid));
        rt.save();
    }
    assert!(dir.join("players").join(format!("{}.bin", uuid::Uuid::from_u128(a.uuid).hyphenated())).is_file());
    assert!(dir.join("cells/minecraft/overworld/r.0.0.bin").is_file());
    assert!(dir.join("global.bin").is_file());
    let mut rt = runtime(&[("counter", ""), ("spawn-protection", "")], Some(dir.clone()));
    rt.player_joined(&a);
    assert_eq!(rt.player_value(a.uuid, "counter", "broken"), Some(1i64.to_le_bytes().to_vec()));
    let welcome = rt.take_messages();
    assert_eq!(welcome.len(), 1);
    assert_eq!(welcome[0].to, Some(a.uuid));
    assert_eq!(text(&welcome[0].text), "Welcome back! You have broken 1 blocks.");
    assert_eq!(rt.global_value("counter", "broken-total"), Some(GlobalValue::Int(1)));
    // Cell data loads with its sidecar when a handler first reaches the cell.
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.block_break(&a, SPAWN, STONE), Verdict::Deny(_)));
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(2i64.to_le_bytes().to_vec()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `.cwasm` cache: the second start deserializes instead of compiling.
#[test]
fn compiled_components_are_cached() {
    let dir = temp_dir("cache");
    let ids = [("chat-format", ""), ("counter", ""), ("spawn-protection", "")];
    let start = |n| {
        let t = Instant::now();
        let rt = runtime_cfg(&ids, RuntimeConfig { cache_dir: Some(dir.clone()), ..config(Duration::from_millis(200)) });
        println!("start {n} with the cache: {:?}", t.elapsed());
        rt
    };
    let first = start(1);
    assert_eq!(stat(&first, "cache-hits"), 0);
    let second = start(2);
    assert_eq!(stat(&second, "cache-hits"), 3);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3, "one file per component");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cost of a cancellable call through the whole path (bucket, filters, frame, deadline,
/// lifting, guest, commit):
/// `cargo test -p kiln-plugin-host --release --test examples -- --nocapture call_overhead`.
#[test]
fn call_overhead() {
    // No per-player or per-tick limits: every call runs within one tick here.
    let cfg = |mode| RuntimeConfig {
        mode,
        player_events_per_second: 0,
        tick_budget: Duration::from_secs(3600),
        tick_fuel: u64::MAX,
        ..config(Duration::from_millis(200))
    };
    for mode in [ExecMode::Ordered, ExecMode::Strict] {
        let mut rt = runtime_cfg(&[("spawn-protection", ""), ("chat-format", "")], cfg(mode));
        let a = alice();
        let r = rt.region_mut(0, 1).unwrap();
        let far = [SPAWN[0] + 1000, 0, SPAWN[2]];
        let n = 20_000u32;
        let measure = |name: &str, f: &mut dyn FnMut()| {
            // The best of 20 batches: other processes only ever add time.
            for _ in 0..1000 {
                f();
            }
            let mut per = Duration::MAX;
            for _ in 0..20 {
                let start = Instant::now();
                for _ in 0..n / 20 {
                    f();
                }
                per = per.min(start.elapsed() / (n / 20));
            }
            println!("{mode:?} {name}: {} ns per call", per.as_nanos());
            per
        };
        let allow = measure("block-break, allowed (one cell read)", &mut || {
            assert_eq!(r.block_break(&a, far, STONE), Verdict::Allow);
        });
        measure("block-break, denied (cell get/put, message)", &mut || {
            assert!(matches!(r.block_break(&a, SPAWN, STONE), Verdict::Deny(_)));
        });
        measure("chat rewrite", &mut || {
            assert!(matches!(r.chat(&a, "hello"), ChatOutcome::Rewrite(_)));
        });
        let op = Actor { operator: true, ..a };
        measure("block-break, filtered on the host", &mut || {
            assert_eq!(r.block_break(&op, SPAWN, STONE), Verdict::Allow);
        });
        let start = Instant::now();
        for i in 0..50u64 {
            rt.sync_regions(0, [1, 100 + i]);
        }
        println!("{mode:?} region instance set (2 plugins): {:?} per new region", start.elapsed() / 50);
        // Generous bound for unoptimized builds; the release number is what gets reported.
        assert!(allow < Duration::from_micros(200));
    }
}

#[test]
fn only_granted_capabilities_are_linked() {
    // The counter imports `kiln:api/chat`; without `player.message` it cannot be linked.
    let (mut manifest, wasm) = examples::load("counter", "").unwrap();
    manifest.capabilities.retain(|c| *c != kiln_plugin_host::Capability::PlayerMessage);
    let rt = PluginRuntime::new(vec![(manifest, wasm.clone())], RuntimeConfig::default()).unwrap();
    assert!(rt.ids().is_empty(), "the plugin was refused");
    // Without `command.register` it loads, but its command is not registered.
    let (mut manifest, _) = examples::load("counter", "").unwrap();
    manifest.capabilities.retain(|c| *c != kiln_plugin_host::Capability::CommandRegister);
    let rt = PluginRuntime::new(vec![(manifest, wasm)], RuntimeConfig::default()).unwrap();
    assert_eq!(rt.ids(), ["counter"]);
    assert!(rt.commands().is_empty());
    // The heartbeat imports the scheduler: refused without `scheduler`.
    let (mut manifest, wasm) = examples::load("heartbeat", "").unwrap();
    manifest.capabilities.retain(|c| *c != kiln_plugin_host::Capability::Scheduler);
    assert!(PluginRuntime::new(vec![(manifest, wasm)], RuntimeConfig::default()).unwrap().ids().is_empty());
}
