//! A guest built against the frozen 1.0 WIT runs on the 1.1 host (`docs/plugin-api.md` §10).
//!
//! `tests/fixtures/compat10` is a plugin written straight against `kiln:api@1.0.0` (the WIT as
//! of the freeze, not the SDK, which is 1.1), checked in as `compat10.wasm`. 1.1 added an
//! interface (`world-read`), a record (`move-event`) and a case of `observed`
//! (`player-moved`): this test loads the old binary on the current host and uses every kind of
//! call the fixture makes (hooks of both instances, imports of the always-linked interfaces
//! and of `chat`, a variant list in `on-observe`).

use kiln_plugin_host::{Actor, ChatOutcome, GlobalValue, Manifest, PluginRuntime, Registries, RuntimeConfig, SpawnReason, Verdict};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const STONE: u32 = 1;

fn fixture() -> (Manifest, Vec<u8>) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compat10");
    let manifest = Manifest::parse(&std::fs::read_to_string(dir.join("plugin.toml")).unwrap()).unwrap();
    (manifest, std::fs::read(dir.join("compat10.wasm")).unwrap())
}

fn runtime() -> PluginRuntime {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let registries = Arc::new(Registries::new(
        s(&["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]),
        s(&["minecraft:air", "minecraft:stone"]),
        s(&["minecraft:air", "minecraft:stone"]),
        s(&["minecraft:cow"]),
    ));
    let cfg = RuntimeConfig {
        registries,
        call_budget: Duration::from_millis(500),
        tick_budget: Duration::from_secs(3600),
        player_events_per_second: 0,
        ..RuntimeConfig::default()
    };
    let mut rt = PluginRuntime::new(vec![fixture()], cfg).unwrap();
    assert_eq!(rt.ids(), ["compat10"], "the 1.0 guest links against the 1.1 host");
    rt.sync_regions(0, [1]);
    rt
}

fn text(spans: &[kiln_plugin_host::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn a_1_0_guest_loads_and_its_hooks_run() {
    let mut rt = runtime();
    let a = Actor::new(0xa11ce, "Alice", false);
    // The global instance: `init` returned a command, and it answers.
    assert_eq!(rt.commands().len(), 1);
    assert_eq!(rt.commands()[0].name, "compat10");
    assert!(text(&rt.run_command(0, Some(&a), "compat10", "x")).starts_with("compat10:x:"));
    // The region instance: a cancellable event, with the denial message crossing back.
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.block_break(&a, [0, 64, 0], STONE), Verdict::Allow);
    match r.block_break(&a, [0, 300, 0], STONE) {
        Verdict::Deny(Some(m)) => assert_eq!(text(&m), "compat10: too high"),
        other => panic!("expected a denial, got {other:?}"),
    }
    match r.chat(&a, "hello") {
        ChatOutcome::Rewrite(spans) => assert_eq!(text(&spans), "[1.0] hello"),
        other => panic!("expected a rewrite, got {other:?}"),
    }
    assert_eq!(rt.stat("traps"), 0, "{:?}", rt.stat_values());
}

#[test]
fn a_1_0_guest_hears_the_observed_kinds_it_knew() {
    let mut rt = runtime();
    let a = Actor::new(0xa11ce, "Alice", false);
    let r = rt.region_mut(0, 1).unwrap();
    r.observe_block(true, &a, [0, 64, 0], STONE);
    r.observe_block(true, &a, [1, 64, 0], STONE);
    r.observe_block(false, &a, [2, 64, 0], STONE);
    // 1.1 events the manifest did not ask for are not sent: the guest's `observed` has no such case.
    r.observe_move(&a, [0, 64, 0], [1, 64, 0]);
    r.observe_spawn(&a, [0, 64, 0], SpawnReason::Join);
    r.flush_observed();
    rt.begin_tick();
    assert_eq!(rt.global_value("compat10", "compat10.broken"), Some(GlobalValue::Int(2)));
    assert_eq!(rt.stat("traps"), 0, "{:?}", rt.stat_values());
}
