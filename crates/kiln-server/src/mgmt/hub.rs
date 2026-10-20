//! Notifications (`JsonRpcNotificationService`): the connections listen at a hub, the server and the
//! simulation raise what happened, the heartbeat is a timer.

use super::exec;
use super::json::J;
use kiln_link::mgmt::{Notification, Notifier, RuleValue};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Where notifications go: one queue per connection.
#[derive(Default)]
pub struct Hub {
    listeners: Mutex<Vec<(u32, Sender<Notification>)>>,
}

impl Hub {
    /// A connection starts listening; what happens from now on reaches its queue.
    pub fn listen(&self, client: u32) -> Receiver<Notification> {
        let (tx, rx) = channel();
        self.listeners.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((client, tx));
        rx
    }

    pub fn forget(&self, client: u32) {
        self.listeners.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(c, _)| *c != client);
    }

    pub fn clear(&self) {
        self.listeners.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    }

    pub fn count(&self) -> usize {
        self.listeners.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len()
    }
}

impl Notifier for Hub {
    fn notify(&self, n: Notification) {
        let mut listeners = self.listeners.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        listeners.retain(|(_, tx)| tx.send(n.clone()).is_ok());
    }
}

/// `OutgoingRpcMethod.Notification` as the connection writes it: the method name and the one parameter, if any.
pub fn notification_json(n: &Notification, info: &kiln_link::ServerInfo) -> J {
    let (method, param): (&str, Option<J>) = match n {
        Notification::ServerStarted => ("server/started", None),
        Notification::ServerShuttingDown => ("server/stopping", None),
        Notification::ServerSaveStarted => ("server/saving", None),
        Notification::ServerSaveCompleted => ("server/saved", None),
        Notification::ServerActivity => ("server/activity", None),
        Notification::WorldUpgradeStarted => ("world/upgrade_started", None),
        Notification::WorldUpgradeProgress(p) => ("world/upgrade_progress", Some(J::Num(format!("{p:?}")))),
        Notification::WorldUpgradeFinished => ("world/upgrade_finished", None),
        Notification::WorldUpgradeFailed(reason) => ("world/upgrade_failed", Some(J::str(reason.clone()))),
        Notification::PlayerJoined(u) => ("players/joined", Some(exec::player_json(u))),
        Notification::PlayerLeft(u) => ("players/left", Some(exec::player_json(u))),
        Notification::PlayerOped(e) => ("operators/added", Some(exec::op_json(e))),
        Notification::PlayerDeoped(e) => ("operators/removed", Some(exec::op_json(e))),
        Notification::PlayerAddedToAllowlist(u) => ("allowlist/added", Some(exec::player_json(u))),
        Notification::PlayerRemovedFromAllowlist(u) => ("allowlist/removed", Some(exec::player_json(u))),
        Notification::IpBanned(b) => ("ip_bans/added", Some(exec::ip_ban_json(b))),
        Notification::IpUnbanned(ip) => ("ip_bans/removed", Some(J::str(ip.clone()))),
        Notification::PlayerBanned(b) => ("bans/added", Some(exec::user_ban_json(b))),
        Notification::PlayerUnbanned(u) => ("bans/removed", Some(exec::player_json(u))),
        Notification::GameRuleChanged { key, value } => ("gamerules/updated", Some(exec::rule_json(key, *value))),
        Notification::StatusHeartbeat => ("server/status", Some(exec::state_json(info))),
    };
    let mut members = vec![("jsonrpc", J::str("2.0")), ("method", J::str(format!("minecraft:notification/{method}")))];
    if let Some(p) = param {
        members.push(("params", J::Arr(vec![p])));
    }
    J::obj(members)
}

/// `ManagementServer.scheduleHeartbeat`: a status notification every so many seconds.
pub struct Heartbeat {
    state: Mutex<HeartbeatState>,
    wake: Condvar,
}

struct HeartbeatState {
    /// Seconds between beats; zero or less is off.
    interval: i64,
    /// Bumped whenever the schedule changes, so a waiting timer starts over.
    generation: u64,
    stopped: bool,
}

impl Heartbeat {
    /// Starts the timer thread; it ends with [`Heartbeat::stop`].
    pub fn start(notifier: Arc<dyn Notifier>, interval: i64) -> Arc<Heartbeat> {
        let hb = Arc::new(Heartbeat { state: Mutex::new(HeartbeatState { interval, generation: 0, stopped: false }), wake: Condvar::new() });
        let timer = hb.clone();
        let _ = std::thread::Builder::new().name("Management heartbeat".into()).spawn(move || timer.run(&*notifier));
        hb
    }

    fn run(&self, notifier: &dyn Notifier) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if state.stopped {
                return;
            }
            if state.interval <= 0 {
                state = self.wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
            let generation = state.generation;
            let wait = Duration::from_secs(state.interval as u64);
            let (guard, timeout) = self.wake.wait_timeout(state, wait).unwrap_or_else(std::sync::PoisonError::into_inner);
            state = guard;
            if timeout.timed_out() && state.generation == generation && !state.stopped && state.interval > 0 {
                notifier.notify(Notification::StatusHeartbeat);
            }
        }
    }

    /// A new schedule (`scheduleHeartbeat`): the old one is cancelled and, for a positive interval, the
    /// first beat comes after one interval. True when it was applied.
    pub fn schedule(&self, seconds: i64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stopped {
            return false;
        }
        state.interval = seconds;
        state.generation += 1;
        self.wake.notify_all();
        true
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stopped = true;
        self.wake.notify_all();
    }
}
