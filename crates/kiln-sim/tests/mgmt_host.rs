//! wp86: the simulation's side of the management server (`kiln_link::mgmt::MgmtHost`): who is online, kicks, operators,
//! settings, game rules, saving, and the notifications the simulation and the player lists raise, as vanilla 26.3's do.

use kiln_link::access::NameAndId;
use kiln_link::mgmt::{MgmtHost, MgmtText, Notification, Notifier, RuleValue, Setting, SettingValue};
use kiln_link::{ConnId, ToSim};
use kiln_sim::testing::{self, Client, SinkStats};
use kiln_sim::{Sim, SimConfig};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen(Mutex<Vec<Notification>>);

impl Notifier for Seen {
    fn notify(&self, n: Notification) {
        self.0.lock().unwrap().push(n);
    }
}

struct Joined {
    stats: Arc<SinkStats>,
    client: Client,
}

fn join(sim: &mut Sim, conn: ConnId, name: &str) -> Joined {
    let (msg, stats) = testing::join(conn, name, 2);
    assert!(sim.step([msg]));
    Joined { client: Client::new(conn, stats.clone()), stats }
}

fn tick(sim: &mut Sim, players: &mut [&mut Joined], n: usize) {
    for _ in 0..n {
        let mut inbox = Vec::new();
        for p in players.iter_mut() {
            p.client.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
}

/// Runs `f` on the simulation between ticks, as the management server's requests do.
fn admin<T: Send + 'static>(sim: &mut Sim, f: impl FnOnce(&mut dyn MgmtHost) -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let work: Box<dyn FnOnce(&mut dyn kiln_link::AdminHost) + Send> = Box::new(move |h| {
        let _ = tx.send(f(h.mgmt().expect("the simulation has a management face")));
    });
    assert!(sim.step([ToSim::Admin(work)]));
    rx.try_recv().expect("answered in the same tick")
}

fn setup() -> (Sim, Arc<Seen>, Joined, Joined) {
    let config = SimConfig::new(4, 2, None);
    ACCESS.with(|a| *a.borrow_mut() = Some(config.access.clone()));
    let seen = Arc::new(Seen::default());
    config.info.notifier.set(Some(seen.clone()));
    config.access.write().unwrap().notifier = config.info.notifier.clone();
    let mut sim = Sim::new(config);
    let mut a = join(&mut sim, 1, "Alice");
    let mut b = join(&mut sim, 2, "Bob");
    tick(&mut sim, &mut [&mut a, &mut b], 20);
    (sim, seen, a, b)
}

thread_local! {
    static ACCESS: std::cell::RefCell<Option<kiln_link::access::SharedAccess>> = const { std::cell::RefCell::new(None) };
}

fn access() -> kiln_link::access::SharedAccess {
    ACCESS.with(|a| a.borrow().clone().expect("set up"))
}

/// The player of this name who is online (offline-mode uuids are what the simulation made of the names).
fn user(sim: &mut Sim, name: &str) -> NameAndId {
    let name = name.to_owned();
    admin(sim, move |h| h.online_players().into_iter().find(|p| p.user.name == name).expect("online").user)
}

#[test]
fn joins_and_leaves_are_announced_and_the_players_listed_in_join_order() {
    let (mut sim, seen, a, _b) = setup();
    assert_eq!(seen.0.lock().unwrap().iter().filter(|n| matches!(n, Notification::PlayerJoined(_))).count(), 2);
    let online = admin(&mut sim, |h| h.online_players());
    assert_eq!(online.iter().map(|p| p.user.name.as_str()).collect::<Vec<_>>(), ["Alice", "Bob"]);
    assert_eq!(online[0].user.uuid, user(&mut sim, "Alice").uuid);
    // A player who disconnects is announced as having left.
    let alice = user(&mut sim, "Alice");
    assert!(sim.step([ToSim::Leave(1)]));
    assert!(seen.0.lock().unwrap().contains(&Notification::PlayerLeft(alice)));
    drop(a);
}

#[test]
fn a_kick_tells_the_player_and_removes_it_at_once() {
    let (mut sim, seen, a, _b) = setup();
    let alice = user(&mut sim, "Alice");
    let uuid = alice.uuid;
    admin(&mut sim, move |h| h.kick(uuid, &MgmtText::Literal("Out you go".into())));
    assert!(a.stats.disconnected.load(std::sync::atomic::Ordering::SeqCst), "the connection was told to close");
    assert!(seen.0.lock().unwrap().contains(&Notification::PlayerLeft(alice)));
    let online = admin(&mut sim, |h| h.online_players());
    assert_eq!(online.len(), 1);
    // The close that follows is no second leave.
    assert!(sim.step([ToSim::Leave(1)]));
    let lefts = seen.0.lock().unwrap().iter().filter(|n| matches!(n, Notification::PlayerLeft(_))).count();
    assert_eq!(lefts, 1);
}

#[test]
fn operators_are_added_with_their_levels_and_announced() {
    let (mut sim, seen, _a, _b) = setup();
    let alice = user(&mut sim, "Alice");
    let who = alice.clone();
    admin(&mut sim, move |h| h.op(&who, Some(2), Some(true)));
    let entry = access().read().unwrap().op_list.get(&alice.uuid.to_string()).cloned().unwrap();
    assert_eq!((entry.level, entry.bypasses_player_limit), (2, true));
    // Without a level the setting `op-permission-level` decides; the bypass flag stays what it was.
    let who = alice.clone();
    admin(&mut sim, move |h| h.op(&who, None, None));
    let entry = access().read().unwrap().op_list.get(&alice.uuid.to_string()).cloned().unwrap();
    assert_eq!((entry.level, entry.bypasses_player_limit), (4, true));
    let who = alice.clone();
    admin(&mut sim, move |h| h.deop(&who));
    assert!(access().read().unwrap().op_list.is_empty());
    let got = seen.0.lock().unwrap().clone();
    assert!(matches!(got.iter().find(|n| matches!(n, Notification::PlayerOped(_))), Some(Notification::PlayerOped(e)) if e.level == 2));
    assert!(got.iter().any(|n| matches!(n, Notification::PlayerDeoped(e) if e.user == alice)));
}

#[test]
fn game_rules_are_listed_set_and_announced_for_commands_too() {
    let (mut sim, seen, _a, _b) = setup();
    let rules = admin(&mut sim, |h| h.game_rules());
    let keep = rules.iter().find(|r| r.0 == "minecraft:keep_inventory").expect("a rule");
    assert_eq!((keep.1, keep.2), (RuleValue::Bool(false), None));
    let forks = rules.iter().find(|r| r.0 == "minecraft:max_command_forks").unwrap();
    assert_eq!((forks.1, forks.2), (RuleValue::Int(65536), Some((0, i32::MAX))));
    assert!(admin(&mut sim, |h| h.game_rule("keep_inventory")).is_some());
    assert!(admin(&mut sim, |h| h.game_rule("nope")).is_none());
    admin(&mut sim, |h| h.set_game_rule("minecraft:keep_inventory", RuleValue::Bool(true)));
    // The command raises it as well.
    sim.console_now("gamerule max_command_forks 5");
    let got = seen.0.lock().unwrap().clone();
    assert!(got.contains(&Notification::GameRuleChanged { key: "minecraft:keep_inventory".into(), value: RuleValue::Bool(true) }));
    assert!(got.contains(&Notification::GameRuleChanged { key: "minecraft:max_command_forks".into(), value: RuleValue::Int(5) }));
}

#[test]
fn settings_are_read_and_applied() {
    let (mut sim, _seen, _a, _b) = setup();
    admin(&mut sim, |h| h.set_setting(Setting::Motd, SettingValue::Str("hello".into())));
    admin(&mut sim, |h| h.set_setting(Setting::MaxPlayers, SettingValue::Int(7)));
    admin(&mut sim, |h| h.set_setting(Setting::Difficulty, SettingValue::Int(3)));
    admin(&mut sim, |h| h.set_setting(Setting::Autosave, SettingValue::Bool(false)));
    admin(&mut sim, |h| h.set_setting(Setting::UseAllowlist, SettingValue::Bool(true)));
    let read = |sim: &mut Sim, s: Setting| admin(sim, move |h| h.setting(s));
    assert_eq!(read(&mut sim, Setting::Motd), SettingValue::Str("hello".into()));
    assert_eq!(read(&mut sim, Setting::MaxPlayers), SettingValue::Int(7));
    assert_eq!(read(&mut sim, Setting::Difficulty), SettingValue::Int(3));
    assert_eq!(read(&mut sim, Setting::Autosave), SettingValue::Bool(false));
    assert_eq!(read(&mut sim, Setting::UseAllowlist), SettingValue::Bool(true));
    assert!(access().read().unwrap().use_whitelist);
}

#[test]
fn enforcing_the_allowlist_kicks_players_who_are_not_on_it() {
    let (mut sim, _seen, _a, _b) = setup();
    access().write().unwrap().whitelist_add(user(&mut sim, "Alice"));
    admin(&mut sim, |h| {
        h.set_setting(Setting::UseAllowlist, SettingValue::Bool(true));
        h.set_setting(Setting::EnforceAllowlist, SettingValue::Bool(true));
        h.kick_unlisted();
    });
    let online = admin(&mut sim, |h| h.online_players());
    assert_eq!(online.iter().map(|p| p.user.name.as_str()).collect::<Vec<_>>(), ["Alice"]);
}

#[test]
fn saving_announces_itself_and_a_system_message_reaches_the_named_players() {
    let (mut sim, seen, a, b) = setup();
    assert!(admin(&mut sim, |h| h.save_everything(true)));
    let got = seen.0.lock().unwrap().clone();
    assert!(got.contains(&Notification::ServerSaveStarted));
    let alice = user(&mut sim, "Alice").uuid;
    let before = (a.stats.packets.load(std::sync::atomic::Ordering::SeqCst), b.stats.packets.load(std::sync::atomic::Ordering::SeqCst));
    admin(&mut sim, move |h| h.system_message(&MgmtText::Literal("psst".into()), false, Some(&[alice])));
    let mut a = a;
    let mut b = b;
    tick(&mut sim, &mut [&mut a, &mut b], 2);
    let after = (a.stats.packets.load(std::sync::atomic::Ordering::SeqCst), b.stats.packets.load(std::sync::atomic::Ordering::SeqCst));
    assert!(after.0 > before.0, "Alice was told");
    assert!(admin(&mut sim, |h| h.plain(&MgmtText::Literal("x".into()))) == "x");
}
