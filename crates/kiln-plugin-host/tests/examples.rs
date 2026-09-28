//! The example plugins against the host: spawn protection (fail-closed, cell-scoped claims),
//! the chat formatter (cancellable rewrite), the counter (player state from observe batches
//! plus a global atomic add), failure policies, write buffering, persistence and the cost of
//! a cancellable call.

use kiln_plugin_host::{Actor, CellKey, ChatOutcome, GlobalValue, PluginRuntime, RuntimeConfig, Verdict, examples};
use std::time::{Duration, Instant};

const SPAWN: [i32; 3] = [8, -60, 8];

/// Tests run in parallel with compilations; a generous budget keeps preemption from turning
/// into timeouts (the timeout test sets the real default).
fn runtime(plugins: &[(&str, &str)], data_dir: Option<std::path::PathBuf>) -> PluginRuntime {
    runtime_with(plugins, data_dir, Duration::from_millis(200))
}

fn runtime_with(plugins: &[(&str, &str)], data_dir: Option<std::path::PathBuf>, call_budget: Duration) -> PluginRuntime {
    let loaded = plugins.iter().map(|(id, extra)| examples::load(id, extra).unwrap()).collect();
    let cfg = RuntimeConfig { data_dir, spawn: SPAWN, call_budget, ..RuntimeConfig::default() };
    let mut rt = PluginRuntime::new(loaded, cfg).unwrap();
    assert_eq!(rt.ids().len(), plugins.len(), "every plugin loaded");
    rt.sync_regions(0, [1]);
    rt
}

fn alice() -> Actor<'static> {
    Actor { uuid: 0xa11ce, name: "Alice", operator: false }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("kiln-plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn spawn_protection_denies_near_spawn_and_records_claims_per_cell() {
    let mut rt = runtime(&[("spawn-protection", "")], None);
    let a = alice();
    let r = rt.region_mut(0, 1).unwrap();
    let near = [SPAWN[0] + 5, SPAWN[1], SPAWN[2] - 16];
    match r.block_break(&a, "minecraft:overworld", near, "minecraft:grass_block") {
        Verdict::Deny(Some(msg)) => assert!(msg[0].text.contains("protected")),
        v => panic!("expected a denial with a message, got {v:?}"),
    }
    assert!(matches!(r.block_place(&a, "minecraft:overworld", near, [near[0], near[1] - 1, near[2]], "minecraft:stone"), Verdict::Deny(_)));
    // Outside the square, another level, or an operator: allowed.
    assert_eq!(r.block_break(&a, "minecraft:overworld", [SPAWN[0] + 17, SPAWN[1], SPAWN[2]], "minecraft:stone"), Verdict::Allow);
    assert_eq!(r.block_break(&a, "minecraft:the_nether", near, "minecraft:netherrack"), Verdict::Allow);
    let op = Actor { operator: true, ..a };
    assert_eq!(r.block_break(&op, "minecraft:overworld", near, "minecraft:stone"), Verdict::Allow);
    // The claim and the denial count live in the cell.
    let cell = CellKey::of_block(0, near[0], near[2]);
    let claim = rt.cell_value(cell, "spawn-protection", "claim").expect("claim recorded");
    assert_eq!(claim, [SPAWN[0], SPAWN[2], 16].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(2i64.to_le_bytes().to_vec()));
}

#[test]
fn chat_formatter_rewrites_and_cancels() {
    let mut rt = runtime(&[("chat-format", "")], None);
    let r = rt.region_mut(0, 1).unwrap();
    match r.chat(&alice(), "hello there") {
        ChatOutcome::Rewrite(spans) => {
            let text: String = spans.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(text, "[Alice] \u{bb} hello there");
            assert_eq!(spans[1].color.as_deref(), Some("gold"));
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
    let (a, b) = (alice(), Actor { uuid: 0xb0b, name: "Bob", operator: false });
    rt.sync_regions(0, [1, 2]);
    for _ in 0..3 {
        rt.region_mut(0, 1).unwrap().observe_block(true, &a, "minecraft:overworld", [0, 0, 0], "minecraft:stone");
    }
    rt.region_mut(0, 2).unwrap().observe_block(true, &b, "minecraft:overworld", [900, 0, 0], "minecraft:dirt");
    rt.region_mut(0, 2).unwrap().observe_block(false, &b, "minecraft:overworld", [900, 0, 0], "minecraft:dirt");
    rt.region_mut(0, 1).unwrap().flush_observed();
    rt.region_mut(0, 2).unwrap().flush_observed();
    assert_eq!(rt.player_value(a.uuid, "counter", "broken"), Some(3i64.to_le_bytes().to_vec()));
    assert_eq!(rt.player_value(b.uuid, "counter", "broken"), Some(1i64.to_le_bytes().to_vec()));
    // The global adds apply in B0.
    assert_eq!(rt.global_value("counter", "broken-total"), None);
    rt.begin_tick();
    assert_eq!(rt.global_value("counter", "broken-total"), Some(GlobalValue::Int(4)));
    let reply: String = rt.run_command(0, Some(&a), "broken", "").iter().map(|s| s.text.clone()).collect();
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
    assert!(matches!(r.block_break(&a, "minecraft:overworld", near(0), "minecraft:stone"), Verdict::Deny(Some(_))));
    // A trap denies without a message and leaves no write behind.
    assert_eq!(r.block_break(&a, "minecraft:overworld", near(70), "minecraft:stone"), Verdict::Deny(None));
    // Far from spawn a trap still denies (fail-closed), and the instance came back.
    assert_eq!(r.block_break(&a, "minecraft:overworld", [5000, 70, 0], "minecraft:stone"), Verdict::Deny(None));
    assert_eq!(r.block_break(&a, "minecraft:overworld", [5000, 0, 0], "minecraft:stone"), Verdict::Allow);
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(1i64.to_le_bytes().to_vec()), "trapped calls wrote nothing");
    let st = rt.stats();
    let load = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!((load(&st.traps), load(&st.timeouts)), (2, 0), "calls {}", load(&st.calls));
    assert!(!rt.is_demoted(0), "traps are not strikes");

    let mut rt = runtime_with(&[("spawn-protection", "chaos = \"spin\"\nchaos_y = 71")], None, RuntimeConfig::default().call_budget);
    let r = rt.region_mut(0, 1).unwrap();
    for i in 0..3 {
        let start = Instant::now();
        assert_eq!(r.block_break(&a, "minecraft:overworld", near(71), "minecraft:stone"), Verdict::Deny(None), "timeout {i}");
        let took = start.elapsed();
        println!("timed-out call {i}: {took:?}");
        // About 1 ms on an idle machine (Windows wakes the epoch thread about every 1 ms);
        // the bound only allows for a loaded test machine.
        assert!(took < Duration::from_secs(1), "the epoch deadline stopped the loop ({took:?})");
    }
    assert!(rt.is_demoted(0), "three strikes demote");
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), None, "timed-out calls wrote nothing");
    // Demoted: no more calls, but fail-closed still denies, even where it would allow.
    let calls = rt_calls(&rt);
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&a, "minecraft:overworld", [5000, 0, 0], "minecraft:stone"), Verdict::Deny(None));
    assert_eq!(rt_calls(&rt), calls, "a demoted plugin is not called for cancellable events");
}

fn rt_calls(rt: &PluginRuntime) -> u64 {
    rt.stats().calls.load(std::sync::atomic::Ordering::Relaxed)
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

#[test]
fn namespaces_persist_across_restarts() {
    let dir = temp_dir("persist");
    let a = alice();
    let cell = CellKey::of_block(0, SPAWN[0], SPAWN[2]);
    {
        let mut rt = runtime(&[("counter", ""), ("spawn-protection", "")], Some(dir.clone()));
        rt.player_joined(&a);
        let r = rt.region_mut(0, 1).unwrap();
        r.observe_block(true, &a, "minecraft:overworld", [0, 0, 0], "minecraft:stone");
        r.flush_observed();
        assert!(matches!(r.block_break(&a, "minecraft:overworld", SPAWN, "minecraft:stone"), Verdict::Deny(_)));
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
    assert_eq!(welcome[0].text.iter().map(|s| s.text.as_str()).collect::<String>(), "Welcome back! You have broken 1 blocks.");
    assert_eq!(rt.global_value("counter", "broken-total"), Some(GlobalValue::Int(1)));
    // Cell data loads with its sidecar when a handler first reaches the cell.
    let r = rt.region_mut(0, 1).unwrap();
    assert!(matches!(r.block_break(&a, "minecraft:overworld", SPAWN, "minecraft:stone"), Verdict::Deny(_)));
    assert_eq!(rt.cell_value(cell, "spawn-protection", "denied"), Some(2i64.to_le_bytes().to_vec()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cost of a cancellable call through the whole path (frame, deadline, lifting, guest,
/// commit): `cargo test -p kiln-plugin-host --release --test examples -- --nocapture call_overhead`.
#[test]
fn call_overhead() {
    let mut rt = runtime(&[("spawn-protection", ""), ("chat-format", "")], None);
    let a = alice();
    let r = rt.region_mut(0, 1).unwrap();
    let far = [SPAWN[0] + 1000, 0, SPAWN[2]];
    let n = 20_000u32;
    let measure = |name: &str, f: &mut dyn FnMut()| {
        for _ in 0..1000 {
            f();
        }
        let start = Instant::now();
        for _ in 0..n {
            f();
        }
        let per = start.elapsed() / n;
        println!("{name}: {} ns per call", per.as_nanos());
        per
    };
    let allow = measure("block-break, allowed (no state access)", &mut || {
        assert_eq!(r.block_break(&a, "minecraft:overworld", far, "minecraft:stone"), Verdict::Allow);
    });
    measure("block-break, denied (cell get/put, message)", &mut || {
        assert!(matches!(r.block_break(&a, "minecraft:overworld", SPAWN, "minecraft:stone"), Verdict::Deny(_)));
    });
    measure("chat rewrite", &mut || {
        assert!(matches!(r.chat(&a, "hello"), ChatOutcome::Rewrite(_)));
    });
    let start = Instant::now();
    for i in 0..50u64 {
        rt.sync_regions(0, [1, 100 + i]);
    }
    println!("region instance set (2 plugins): {:?} per new region", start.elapsed() / 50);
    // Generous bound for unoptimized builds; the release number is what gets reported.
    assert!(allow < Duration::from_micros(200));
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
}
