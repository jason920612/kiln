//! The 1.0 API against the sample plugins: fail-closed claims that reach across cells, the
//! shop's atomic purchases and locked menu, the homes' warm-up, the scoreboard HUD, events
//! plugins raise to each other, block edits bound to a cell, and what each plugin hears.

use kiln_plugin_host::{
    Actor, CellKey, ClickKind, ContainerClick, Effect, EffectKind, GlobalValue, ItemRef, ObserveKinds, PlayerAt, PlayerInfo, PluginRuntime, Registries,
    RuntimeConfig, SpawnReason, Verdict, World, examples,
};
use std::sync::Arc;
use std::time::Duration;

const ALICE: u128 = 0xa11ce;
const BOB: u128 = 0xb0b;
const GOLD: u32 = 1;
const STICK: u32 = 2;

fn registries() -> Arc<Registries> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Arc::new(
        Registries::new(
            s(&["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]),
            s(&["minecraft:air", "minecraft:stone"]),
            s(&["minecraft:air", "minecraft:gold_block", "minecraft:stick", "minecraft:stone"]),
            s(&["minecraft:cow", "minecraft:zombie"]),
        )
        .with_damage_types(s(&["minecraft:generic", "minecraft:player_attack", "minecraft:fall"])),
    )
}

fn runtime(plugins: &[(&str, &str)]) -> PluginRuntime {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let loaded = plugins.iter().map(|(id, extra)| examples::load(id, extra).unwrap()).collect();
    let cfg = RuntimeConfig {
        registries: registries(),
        call_budget: Duration::from_millis(200),
        tick_budget: Duration::from_secs(3600),
        player_events_per_second: 0,
        ..RuntimeConfig::default()
    };
    let mut rt = PluginRuntime::new(loaded, cfg).unwrap();
    assert_eq!(rt.ids().len(), plugins.len(), "every plugin loaded");
    rt.sync_regions(0, [1, 2]);
    rt
}

/// Players with their levels and regions; cells at x < 128 are region 1's, the rest region 2's.
#[derive(Default)]
struct Layout(Vec<PlayerAt>);

impl Layout {
    fn with(mut self, uuid: u128, name: &str, region: u64, pos: [f64; 3]) -> Layout {
        self.0.push(PlayerAt {
            uuid,
            level: 0,
            region,
            name: name.into(),
            operator: false,
            info: PlayerInfo { pos, health: 20.0, ..PlayerInfo::default() },
        });
        self
    }
}

impl World for Layout {
    fn player(&self, uuid: u128) -> Option<PlayerAt> {
        self.0.iter().find(|p| p.uuid == uuid).cloned()
    }
    fn owner(&self, _: u32, x: i32, _: i32) -> Option<u64> {
        Some(if x < 128 { 1 } else { 2 })
    }
}

fn alice() -> Actor<'static> {
    Actor::new(ALICE, "Alice", false)
}

fn bob() -> Actor<'static> {
    Actor::new(BOB, "Bob", false)
}

fn text(spans: &[kiln_plugin_host::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

fn denial(v: &Verdict) -> String {
    match v {
        Verdict::Deny(Some(m)) => text(m),
        Verdict::Deny(None) => String::new(),
        Verdict::Allow => panic!("expected a denial"),
    }
}

fn messages(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match &e.kind {
            EffectKind::Message { text: t, .. } => Some(text(t)),
            _ => None,
        })
        .collect()
}

/// A claim next to a cell border reaches into the neighbour: that cell's own region records
/// its copy a tick later, from the note in the global snapshot, and every decision is made
/// from the cell data of the region that asks.
#[test]
fn claims_protect_their_land_across_cell_borders() {
    let mut rt = runtime(&[("claims", "")]);
    let world = Layout::default().with(ALICE, "Alice", 1, [127.0, 64.0, 5.0]).with(BOB, "Bob", 2, [130.0, 64.0, 5.0]);
    let (a, b) = (alice(), bob());
    // Alice puts down a gold block at x = 127, the last column of cell 0: the claim spans x 119 to 135.
    let v = rt.region_mut(0, 1).unwrap().block_place(&a, [127, 64, 5], [127, 63, 5], Some(GOLD));
    assert_eq!(v, Verdict::Allow);
    assert_eq!(messages(&rt.take_effects()), ["Claimed 17x17 blocks."]);
    let home = CellKey::of_block(0, 127, 5);
    let next = CellKey::of_block(0, 130, 5);
    assert_eq!(rt.cell_value(home, "claims", "claims").map(|c| c.len()), Some(32), "recorded in its own cell");
    assert_eq!(rt.cell_value(next, "claims", "claims"), None, "not yet in the neighbour");
    // Bob digs inside it in the home cell: refused, with the owner's name unknown (offline list).
    rt.set_online(vec![kiln_plugin_host::OnlinePlayer { uuid: ALICE, name: "Alice".into(), level: 0 }]);
    let v = rt.region_mut(0, 1).unwrap().block_break(&b, [125, 64, 5], 1);
    assert_eq!(denial(&v), "This land is claimed by Alice");
    assert_eq!(rt.region_mut(0, 1).unwrap().block_break(&a, [125, 64, 5], 1), Verdict::Allow, "the owner may");
    assert_eq!(rt.region_mut(0, 1).unwrap().block_break(&b, [100, 64, 5], 1), Verdict::Allow, "outside the claim");
    // Next tick the neighbour's region has its copy.
    rt.begin_tick_in(&world);
    assert_eq!(rt.cell_value(next, "claims", "claims").map(|c| c.len()), Some(32), "copied by the region owning the neighbour");
    let v = rt.region_mut(0, 2).unwrap().block_break(&b, [131, 64, 5], 1);
    assert!(denial(&v).starts_with("This land is claimed"), "{v:?}");
    assert_eq!(rt.region_mut(0, 2).unwrap().block_break(&a, [131, 64, 5], 1), Verdict::Allow);
    // Using a block (a chest) is a placement event with an empty hand: refused as well.
    assert!(rt.region_mut(0, 2).unwrap().block_place(&b, [131, 64, 5], [131, 63, 5], None) != Verdict::Allow);
    // Operators are filtered on the host: the plugin is not even called.
    let calls = rt.stat("calls");
    let op = Actor { operator: true, ..b };
    assert_eq!(rt.region_mut(0, 2).unwrap().block_break(&op, [131, 64, 5], 1), Verdict::Allow);
    assert_eq!(rt.stat("calls"), calls);
    // Players do not fight on claimed land, but do elsewhere.
    let v = rt.region_mut(0, 1).unwrap().player_damage(&a, Some(&b), [120, 64, 5], 1, 4.0);
    assert!(denial(&v).starts_with("No fighting"));
    assert_eq!(rt.region_mut(0, 1).unwrap().player_damage(&a, Some(&b), [10, 64, 5], 1, 4.0), Verdict::Allow);
    assert_eq!(rt.region_mut(0, 1).unwrap().player_damage(&a, None, [120, 64, 5], 2, 4.0), Verdict::Allow, "falls hurt anyone");
    // A fourth claim is refused: three each.
    for i in 0..2 {
        let v = rt.region_mut(0, 1).unwrap().block_place(&a, [60 - i * 30, 64, 5], [60, 63, 5], Some(GOLD));
        assert_eq!(v, Verdict::Allow);
    }
    let v = rt.region_mut(0, 1).unwrap().block_place(&a, [10, 64, 50], [10, 63, 50], Some(GOLD));
    assert_eq!(denial(&v), "You can have 3 claims.");
}

/// Every protection the claims plugin subscribes to is fail-closed: a trap, a timeout or a
/// spent budget denies (the host applies the policy; spawn-protection's chaos tests exercise
/// the failures themselves).
#[test]
fn claims_subscribe_fail_closed() {
    let (manifest, _) = examples::load("claims", "").unwrap();
    for kind in [
        kiln_plugin_host::EventKind::BlockBreak,
        kiln_plugin_host::EventKind::BlockPlace,
        kiln_plugin_host::EventKind::EntityInteract,
        kiln_plugin_host::EventKind::EntityAttack,
        kiln_plugin_host::EventKind::PlayerDamage,
    ] {
        assert_eq!(manifest.subscription(kind).unwrap().policy, kiln_plugin_host::FailPolicy::Closed, "{kind:?}");
    }
}

/// A purchase is an atomic `try-add` whose answer comes a tick later in the region holding the
/// player; the menu is the plugin's own, and the wand's tag reaches only its plugin.
#[test]
fn shop_sells_with_atomic_money_and_a_locked_menu() {
    let mut rt = runtime(&[("shop", "")]);
    let world = Layout::default().with(ALICE, "Alice", 1, [5.0, 64.0, 5.0]);
    let a = alice();
    rt.player_joined(&a);
    let key = format!("bal:{}", uuid_string(ALICE));
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(100)), "starting money");
    rt.player_left(&a);
    rt.player_joined(&a);
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(100)), "only once per player");
    // /shop opens the menu (an effect).
    let reply = rt.run_command(0, Some(&a), "shop", "");
    assert!(reply.is_empty());
    let effects = rt.take_effects();
    let menu = effects
        .iter()
        .find_map(|e| match &e.kind {
            EffectKind::OpenMenu { who, menu } => Some((*who, menu.clone())),
            _ => None,
        })
        .expect("the menu opens");
    assert_eq!(menu.0, ALICE);
    assert_eq!((menu.1.id.as_str(), menu.1.rows, menu.1.items.len()), ("shop:main", 3, 5));
    assert_eq!(text(&menu.1.title), "Shop");
    // Clicking the iron sword (slot 12, 40 coins): the money moves a tick later.
    let click = |slot: i32| ContainerClick {
        menu: Some("shop:main"),
        container: "minecraft:generic_9x3",
        slot,
        button: 0,
        kind: ClickKind::Left,
        clicked: None,
    };
    let r = rt.region_mut(0, 1).unwrap();
    assert_eq!(r.container_click(&a, &click(12)), Verdict::Allow);
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(100)), "not yet");
    rt.begin_tick_in(&world);
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(60)));
    let effects = rt.take_effects();
    assert!(effects.iter().any(|e| matches!(&e.kind, EffectKind::Give { who, item } if *who == ALICE && item.item == "minecraft:iron_sword")), "{effects:?}");
    assert!(messages(&effects).contains(&"Bought Iron sword. Balance: 60".to_owned()));
    // The diamond costs 100: refused, nothing taken.
    rt.region_mut(0, 1).unwrap().container_click(&a, &click(14));
    rt.begin_tick_in(&world);
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(60)));
    let effects = rt.take_effects();
    assert_eq!(messages(&effects), ["Not enough money."]);
    assert!(!effects.iter().any(|e| matches!(e.kind, EffectKind::Give { .. })));
    // The wand carries the plugin's tag, a glint and a lore line.
    rt.region_mut(0, 1).unwrap().container_click(&a, &click(16));
    rt.begin_tick_in(&world);
    let effects = rt.take_effects();
    let wand = effects
        .iter()
        .find_map(|e| match &e.kind {
            EffectKind::Give { item, .. } => Some(item.clone()),
            _ => None,
        })
        .expect("the wand");
    assert_eq!((wand.item.as_str(), wand.tag.as_deref(), wand.glint, wand.lore.len()), ("minecraft:stick", Some("shop:wand"), true, 1));
    assert_eq!(rt.global_value("shop", &key), Some(GlobalValue::Int(35)));
    // The tag reaches the shop only: its wand zaps, a stick tagged by another plugin does not reach it.
    let calls = rt.stat("calls");
    let zap = rt.region_mut(0, 1).unwrap().item_use(&a, ItemRef { item: STICK, count: 1, tag: Some("shop:wand") }, false, None);
    assert!(matches!(zap, Verdict::Deny(None)));
    assert_eq!(messages(&rt.take_effects()), ["Zap! (Alice)"]);
    let other = rt.region_mut(0, 1).unwrap().item_use(&a, ItemRef { item: STICK, count: 1, tag: Some("other:wand") }, false, None);
    let plain = rt.region_mut(0, 1).unwrap().item_use(&a, ItemRef { item: STICK, count: 1, tag: None }, false, None);
    assert_eq!((other, plain), (Verdict::Allow, Verdict::Allow));
    assert_eq!(rt.stat("calls"), calls + 1, "only the wand was delivered");
    // Clicks in other menus and in vanilla containers do not reach the shop.
    let calls = rt.stat("calls");
    let r = rt.region_mut(0, 1).unwrap();
    let foreign = ContainerClick { menu: Some("other:menu"), ..click(12) };
    let vanilla = ContainerClick { menu: None, container: "minecraft:inventory", ..click(12) };
    assert_eq!((r.container_click(&a, &foreign), r.container_click(&a, &vanilla)), (Verdict::Allow, Verdict::Allow));
    assert_eq!(rt.stat("calls"), calls);
}

fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Two buyers in two regions racing for the same coins: exactly as many purchases succeed as
/// the money allows, whatever the order the regions called in.
#[test]
fn racing_purchases_never_overdraw() {
    let mut rt = runtime(&[("shop", "")]);
    let world = Layout::default().with(ALICE, "Alice", 1, [5.0, 64.0, 5.0]).with(BOB, "Bob", 2, [200.0, 64.0, 5.0]);
    let (a, b) = (alice(), bob());
    rt.player_joined(&a);
    rt.player_joined(&b);
    let click = |slot: i32| ContainerClick {
        menu: Some("shop:main"),
        container: "minecraft:generic_9x3",
        slot,
        button: 0,
        kind: ClickKind::Left,
        clicked: None,
    };
    // Each has 100: three 40-coin swords from each (6 attempts, 240 coins wanted).
    for _ in 0..3 {
        rt.region_mut(0, 2).unwrap().container_click(&b, &click(12));
        rt.region_mut(0, 1).unwrap().container_click(&a, &click(12));
    }
    rt.begin_tick_in(&world);
    let effects = rt.take_effects();
    let gave = |who: u128| effects.iter().filter(|e| matches!(&e.kind, EffectKind::Give { who: w, .. } if *w == who)).count();
    assert_eq!((gave(ALICE), gave(BOB)), (2, 2), "100 coins buy two 40-coin swords, not three");
    for who in [ALICE, BOB] {
        assert_eq!(rt.global_value("shop", &format!("bal:{}", uuid_string(who))), Some(GlobalValue::Int(20)));
    }
}

/// `/home` arms a warm-up that teleports a player who stood still and was not hurt.
#[test]
fn homes_teleport_after_a_warm_up_unless_moved_or_hurt() {
    let mut rt = runtime(&[("homes", "warmup = 2")]);
    let at = [10.0, 64.0, 10.0];
    let a = Actor::new(ALICE, "Alice", false).with_info(PlayerInfo { pos: at, level: 0, rot: [90.0, 0.0], ..PlayerInfo::default() });
    let world = Layout::default().with(ALICE, "Alice", 1, at);
    assert_eq!(text(&rt.run_command(0, Some(&a), "sethome", "base")), "Home base set.");
    assert_eq!(text(&rt.run_command(0, Some(&a), "homes", "")), "Homes: base");
    assert!(text(&rt.run_command(0, Some(&a), "home", "nowhere")).starts_with("No home called"));
    // Armed and left alone: the teleport follows the warm-up.
    assert!(text(&rt.run_command(0, Some(&a), "home", "base")).starts_with("Teleporting to base"));
    rt.begin_tick_in(&world);
    assert!(rt.take_effects().iter().all(|e| !matches!(e.kind, EffectKind::Teleport { .. })), "not before the warm-up");
    rt.begin_tick_in(&world);
    let effects = rt.take_effects();
    let tp = effects.iter().find_map(|e| match &e.kind {
        EffectKind::Teleport { who, level, pos, rot } => Some((*who, *level, *pos, *rot)),
        _ => None,
    });
    assert_eq!(tp, Some((ALICE, 0, at, [90.0, 0.0])));
    // Hurt during the warm-up: disarmed.
    rt.run_command(0, Some(&a), "home", "base");
    assert_eq!(rt.region_mut(0, 1).unwrap().player_damage(&a, None, [10, 64, 10], 2, 3.0), Verdict::Allow);
    rt.begin_tick_in(&world);
    rt.begin_tick_in(&world);
    assert!(rt.take_effects().iter().all(|e| !matches!(e.kind, EffectKind::Teleport { .. })), "damage cancelled it");
    // Moved during the warm-up: cancelled with a notice.
    rt.run_command(0, Some(&a), "home", "base");
    let moved = Layout::default().with(ALICE, "Alice", 1, [20.0, 64.0, 10.0]);
    rt.begin_tick_in(&moved);
    rt.begin_tick_in(&moved);
    let effects = rt.take_effects();
    assert!(effects.iter().all(|e| !matches!(e.kind, EffectKind::Teleport { .. })));
    assert!(messages(&effects).iter().any(|m| m.contains("You moved")), "{:?}", messages(&effects));
    // Deleting a home.
    assert_eq!(text(&rt.run_command(0, Some(&a), "delhome", "base")), "Home base removed.");
    assert_eq!(text(&rt.run_command(0, Some(&a), "homes", "")), "You have no homes. /sethome sets one.");
}

/// The HUD refreshes every second from a task that follows the player, counts deaths in the
/// player's own namespace and kills in a global counter, and survives a hot reload.
#[test]
fn the_scoreboard_hud_counts_and_survives_a_reload() {
    let mut rt = runtime(&[("scoreboard-hud", "")]);
    let world = Layout::default().with(ALICE, "Alice", 1, [5.0, 64.0, 5.0]).with(BOB, "Bob", 1, [6.0, 64.0, 5.0]);
    let (a, b) = (alice(), bob());
    rt.set_online(vec![
        kiln_plugin_host::OnlinePlayer { uuid: ALICE, name: "Alice".into(), level: 0 },
        kiln_plugin_host::OnlinePlayer { uuid: BOB, name: "Bob".into(), level: 0 },
    ]);
    rt.player_joined(&a);
    rt.player_joined(&b);
    rt.begin_tick_in(&world);
    let effects = rt.take_effects();
    let sidebar = effects.iter().find_map(|e| match &e.kind {
        EffectKind::Sidebar { to, title, lines } if *to == ALICE => Some((text(title), lines.iter().map(|l| text(l)).collect::<Vec<_>>())),
        _ => None,
    });
    assert_eq!(sidebar, Some(("Kiln".to_owned(), ["Deaths: 0", "Kills: 0", "Online: 2", "", "Health: 20"].map(String::from).to_vec())));
    assert!(effects.iter().any(|e| matches!(&e.kind, EffectKind::Bossbar { to, id, progress, .. } if *to == ALICE && id == "scoreboard-hud:health" && *progress == 1.0)));
    // Bob dies, killed by Alice: Bob's deaths in Bob's namespace, Alice's kills in the global counter.
    let r = rt.region_mut(0, 1).unwrap();
    r.observe_death(&b, [6, 64, 5], 1, Some(ALICE));
    r.flush_observed();
    assert_eq!(rt.player_value(BOB, "scoreboard-hud", "deaths"), Some(1i64.to_le_bytes().to_vec()));
    // Not delivered to a plugin that did not ask for deaths (the counter observes blocks only).
    // The refresh a second later shows it.
    for _ in 0..20 {
        rt.begin_tick_in(&world);
    }
    let effects = rt.take_effects();
    let lines = effects.iter().rev().find_map(|e| match &e.kind {
        EffectKind::Sidebar { to, lines, .. } if *to == ALICE => Some(lines.iter().map(|l| text(l)).collect::<Vec<_>>()),
        _ => None,
    });
    assert_eq!(lines.map(|l| l[1].clone()), Some("Kills: 1".to_owned()));
    // A reload cancels the tasks; the new generation schedules them again with their delay.
    let before = rt.pending_tasks();
    assert_eq!(before, 2, "one refresh per player");
    let (manifest, wasm) = examples::load("scoreboard-hud", "").unwrap();
    let r = rt.reload("scoreboard-hud", manifest, &wasm).unwrap();
    assert_eq!(r.cancelled_tasks, 2);
    assert_eq!(rt.pending_tasks(), 2, "scheduled again");
    for _ in 0..25 {
        rt.begin_tick_in(&world);
    }
    let refreshed = rt.take_effects().iter().filter(|e| matches!(e.kind, EffectKind::Sidebar { .. })).count();
    assert!(refreshed >= 2, "the HUD kept refreshing after the reload ({refreshed})");
    // Spawn events: the HUD greets with a title.
    let r = rt.region_mut(0, 1).unwrap();
    r.observe_spawn(&a, [5, 64, 5], SpawnReason::Join);
    r.flush_observed();
    assert!(rt.take_effects().iter().any(|e| matches!(&e.kind, EffectKind::Title { to, .. } if *to == ALICE)));
}

/// Observed deaths and spawns reach only plugins that subscribed to those kinds.
#[test]
fn observe_kinds_are_filtered_on_the_host() {
    let mut rt = runtime(&[("counter", "")]);
    let calls = rt.stat("calls");
    let r = rt.region_mut(0, 1).unwrap();
    r.observe_death(&alice(), [0, 64, 0], 1, None);
    r.observe_spawn(&alice(), [0, 64, 0], SpawnReason::Respawn);
    r.flush_observed();
    assert_eq!(rt.stat("calls"), calls, "the counter observes blocks only");
    assert!(!rt.region_mut(0, 1).unwrap().observing_kind(ObserveKinds::PLAYER_DIED));
}

/// Plugins ask each other before acting: the answer comes back at once, in the same context.
#[test]
fn raised_events_are_answered_by_the_other_plugins() {
    let mut rt = runtime(&[("arena", ""), ("gatekeeper", "")]);
    let a = alice();
    // Open: the player is moved into the arena.
    assert_eq!(text(&rt.run_command(0, Some(&a), "arena", "join")), "Welcome to the arena.");
    let effects = rt.take_effects();
    let kinds: Vec<&str> = effects
        .iter()
        .map(|e| match &e.kind {
            EffectKind::Clear { .. } => "clear",
            EffectKind::Give { .. } => "give",
            EffectKind::GameMode { mode: 2, .. } => "adventure",
            EffectKind::Teleport { .. } => "teleport",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["clear", "give", "adventure", "teleport"]);
    // The gatekeeper vetoes: nothing happens.
    let (mut manifest, wasm) = examples::load("gatekeeper", "").unwrap();
    manifest.config.insert("deny".into(), "arena:join".into());
    rt.reload("gatekeeper", manifest, &wasm).unwrap();
    assert_eq!(text(&rt.run_command(0, Some(&a), "arena", "join")), "The arena is closed.");
    assert!(rt.take_effects().is_empty(), "a vetoed join does nothing");
    // Events nobody subscribed to are not delivered: an unrelated name passes.
    let calls = rt.stat("calls");
    assert_eq!(text(&rt.run_command(0, Some(&a), "arena", "other")), "/arena join | out | reset");
    assert_eq!(rt.stat("calls"), calls + 1, "the command only");
}

/// The floor task runs in the region owning the arena's cell, asks the other plugins (a
/// region-context raise), and edits blocks only inside the cell it was given.
#[test]
fn block_edits_are_bound_to_the_cell_of_the_call() {
    let world = Layout::default();
    for (extra, expect) in [("x = 1000", Some(49)), ("x = 1022", None)] {
        let mut rt = runtime(&[("arena", extra), ("gatekeeper", "")]);
        let cell = CellKey::of_block(0, 1000, 1000);
        assert_eq!(cell, CellKey::of_block(0, 1005, 1000));
        let a = alice();
        assert_eq!(text(&rt.run_command(0, Some(&a), "arena", "reset")), "Rebuilding the arena floor.");
        rt.begin_tick_in(&world);
        let changes = rt.take_effects().into_iter().find_map(|e| match e.kind {
            EffectKind::SetBlocks { level, changes } => Some((level, changes)),
            _ => None,
        });
        match expect {
            Some(n) => {
                let (level, changes) = changes.expect("the floor");
                assert_eq!((level, changes.len()), (0, n));
                assert!(changes.iter().all(|c| c.pos[1] == 80 && (c.pos[0] - 1000).abs() <= 3 && c.state.starts_with("minecraft:")));
            }
            None => assert!(changes.is_none(), "a floor across the cell border is refused"),
        }
    }
    // A veto from the gatekeeper (a region-context event) keeps the floor as it is.
    let mut rt = runtime(&[("arena", ""), ("gatekeeper", "deny = \"arena:reset\"")]);
    rt.run_command(0, Some(&alice()), "arena", "reset");
    rt.begin_tick_in(&world);
    assert!(rt.take_effects().iter().all(|e| !matches!(e.kind, EffectKind::SetBlocks { .. })), "vetoed");
}

/// A plugin menu's id and a custom item's tag carry the plugin's id, so a plugin cannot
/// pretend to be another one's menu or items.
#[test]
fn ids_made_up_by_plugins_are_prefixed_with_their_owner() {
    let mut rt = runtime(&[("shop", "")]);
    let a = alice();
    rt.player_joined(&a);
    rt.run_command(0, Some(&a), "shop", "");
    let effects = rt.take_effects();
    assert!(effects.iter().all(|e| &*e.plugin_id == "shop"));
    let EffectKind::OpenMenu { menu, .. } = &effects.iter().find(|e| matches!(e.kind, EffectKind::OpenMenu { .. })).unwrap().kind else { unreachable!() };
    assert!(menu.id.starts_with("shop:"));
}

/// What the new calls cost (release builds; run with `--nocapture` for the numbers). Each
/// handler runs in both modes; the number is the best of 20 batches.
#[test]
fn call_costs_of_the_new_events() {
    use std::time::Instant;
    for mode in [kiln_plugin_host::ExecMode::Ordered, kiln_plugin_host::ExecMode::Strict] {
        let loaded = ["noop", "claims", "shop", "homes", "arena", "gatekeeper"].iter().map(|id| examples::load(id, "").unwrap()).collect();
        let cfg = RuntimeConfig {
            mode,
            registries: registries(),
            call_budget: Duration::from_millis(200),
            tick_budget: Duration::from_secs(3600),
            tick_fuel: u64::MAX,
            player_events_per_second: 0,
            ..RuntimeConfig::default()
        };
        let mut rt = PluginRuntime::new(loaded, cfg).unwrap();
        rt.sync_regions(0, [1]);
        rt.set_online(vec![kiln_plugin_host::OnlinePlayer { uuid: ALICE, name: "Alice".into(), level: 0 }]);
        let (a, b) = (alice(), bob());
        let n = 20_000u32;
        let measure = |rt: &mut PluginRuntime, name: &str, f: &mut dyn FnMut(&mut PluginRuntime)| {
            for _ in 0..1000 {
                f(rt);
            }
            rt.take_effects();
            let mut per = Duration::MAX;
            for _ in 0..20 {
                let start = Instant::now();
                for _ in 0..n / 20 {
                    f(rt);
                }
                per = per.min(start.elapsed() / (n / 20));
                rt.take_effects();
            }
            println!("{mode:?} {name}: {} ns per call", per.as_nanos());
        };
        // Alice claims x 0..16 (the claim block); the noop plugin allows every break at once.
        rt.region_mut(0, 1).unwrap().block_place(&a, [8, 64, 8], [8, 63, 8], Some(GOLD));
        measure(&mut rt, "block-break through noop + claims (outside any claim)", &mut |rt| {
            rt.region_mut(0, 1).unwrap().block_break(&b, [90, 64, 90], 1);
        });
        measure(&mut rt, "block-break denied by claims (claim read, owner name lookup, message)", &mut |rt| {
            let _ = rt.region_mut(0, 1).unwrap().block_break(&b, [8, 64, 8], 1);
        });
        let click = ContainerClick { menu: Some("shop:main"), container: "minecraft:generic_9x3", slot: 14, button: 0, kind: ClickKind::Left, clicked: None };
        rt.player_joined(&a);
        measure(&mut rt, "container click in the shop (atomic try-add + player data)", &mut |rt| {
            rt.region_mut(0, 1).unwrap().container_click(&a, &click);
        });
        measure(&mut rt, "item use of the wand (a chat effect, a denial)", &mut |rt| {
            rt.region_mut(0, 1).unwrap().item_use(&a, ItemRef { item: STICK, count: 1, tag: Some("shop:wand") }, false, None);
        });
        measure(&mut rt, "player damage heard by homes and claims (claim lookup, one state read)", &mut |rt| {
            rt.region_mut(0, 1).unwrap().player_damage(&a, Some(&b), [50, 64, 50], 1, 2.0);
        });
        let arena = rt.plugin_index("arena").unwrap();
        measure(&mut rt, "/arena other: a global command call", &mut |rt| {
            rt.run_command(arena, Some(&a), "arena", "other");
        });
        measure(&mut rt, "/arena join: global command, event raised to the gatekeeper, four effects", &mut |rt| {
            rt.run_command(arena, Some(&a), "arena", "join");
        });
    }
}
