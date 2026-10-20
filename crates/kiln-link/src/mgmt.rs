//! What the management server (`net.minecraft.server.jsonrpc`) shares with the simulation: the
//! notifications the simulation and the player lists raise (`NotificationService`), and the
//! face the simulation lends to the server's methods (`MinecraftApi` and its services).

use crate::access::{IpBan, NameAndId, OpEntry, SharedAccess, UserBan};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

/// A game rule's value (`GameRule<Boolean>` or `GameRule<Integer>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleValue {
    Bool(bool),
    Int(i32),
}

/// `NotificationService`: what happened on the server, for the connected management clients.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    ServerStarted,
    ServerShuttingDown,
    ServerSaveStarted,
    ServerSaveCompleted,
    ServerActivity,
    WorldUpgradeStarted,
    WorldUpgradeProgress(f32),
    WorldUpgradeFinished,
    WorldUpgradeFailed(String),
    PlayerJoined(NameAndId),
    PlayerLeft(NameAndId),
    PlayerOped(OpEntry),
    PlayerDeoped(OpEntry),
    PlayerAddedToAllowlist(NameAndId),
    PlayerRemovedFromAllowlist(NameAndId),
    IpBanned(IpBan),
    IpUnbanned(String),
    PlayerBanned(UserBan),
    PlayerUnbanned(NameAndId),
    GameRuleChanged { key: String, value: RuleValue },
    StatusHeartbeat,
}

/// Receives the notifications; the management server's connections are its listeners. Calls come
/// from any thread (the simulation's, mostly) and must not block.
pub trait Notifier: Send + Sync {
    fn notify(&self, notification: Notification);
}

/// A message of the management protocol (`Message`): literal text or a translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MgmtText {
    Literal(String),
    Translatable { key: String, args: Vec<String> },
}

/// The server settings the management server reads and writes (`MinecraftServerSettingsService`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Autosave,
    Difficulty,
    EnforceAllowlist,
    UseAllowlist,
    MaxPlayers,
    PauseWhenEmptySeconds,
    PlayerIdleTimeout,
    AllowFlight,
    Motd,
    SpawnProtection,
    ForceGameMode,
    GameMode,
    ViewDistance,
    SimulationDistance,
    AcceptTransfers,
    OperatorPermissionLevel,
    HideOnlinePlayers,
    StatusReplies,
    EntityBroadcastRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Bool(bool),
    /// Also the ids of the difficulty (0 peaceful .. 3 hard) and the game mode (0 survival .. 3 spectator).
    Int(i32),
    Str(String),
}

/// A player online.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlinePlayer {
    pub user: NameAndId,
    pub address: Option<IpAddr>,
}

/// The simulation's side of the management methods; the closures sent with
/// [`crate::ToSim::Admin`] call it between ticks.
pub trait MgmtHost {
    /// The player lists and their files (whitelist.json, banned-players.json, banned-ips.json, ops.json).
    fn access(&self) -> SharedAccess;
    /// The players online, in join order.
    fn online_players(&self) -> Vec<OnlinePlayer>;
    /// `ServerGamePacketListenerImpl.disconnect(reason)`; the player leaves the player list.
    fn kick(&mut self, user: Uuid, reason: &MgmtText);
    /// `ServerPlayer.sendSystemMessage` / `sendOverlayMessage` to the players (all of them if `None`).
    fn system_message(&mut self, text: &MgmtText, overlay: bool, to: Option<&[Uuid]>);
    /// `PlayerList.op`: the level defaults to `op-permission-level`.
    fn op(&mut self, user: &NameAndId, level: Option<u8>, bypasses_player_limit: Option<bool>);
    fn deop(&mut self, user: &NameAndId);
    fn deop_all(&mut self);
    /// `MinecraftServer.kickUnlistedPlayers`.
    fn kick_unlisted(&mut self);
    fn setting(&self, setting: Setting) -> SettingValue;
    /// Stores the value (already mapped as the property's parser would) and applies it.
    fn set_setting(&mut self, setting: Setting, value: SettingValue);
    /// Every game rule with its value, and the integer range of the integer ones.
    fn game_rules(&self) -> Vec<(String, RuleValue, Option<(i32, i32)>)>;
    /// The game rule `key` (`minecraft:` default namespace) if there is one: its id, value and integer range.
    fn game_rule(&self, key: &str) -> Option<(String, RuleValue, Option<(i32, i32)>)>;
    /// `GameRules.set`; raises [`Notification::GameRuleChanged`].
    fn set_game_rule(&mut self, key: &str, value: RuleValue);
    /// `DedicatedServer.saveEverything(suppressLogs, flush, force)`.
    fn save_everything(&mut self, flush: bool) -> bool;
    /// The text of a message as the server shows it (its language).
    fn plain(&self, text: &MgmtText) -> String;
    /// `DedicatedServer.halt(false)`: the server stops.
    fn halt(&mut self);
}

/// What threads outside the simulation share about the management server's notifications.
#[derive(Default)]
pub struct NotifierSlot {
    notifier: std::sync::RwLock<Option<Arc<dyn Notifier>>>,
    last_activity_ms: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for NotifierSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotifierSlot")
    }
}

impl NotifierSlot {
    pub fn set(&self, notifier: Option<Arc<dyn Notifier>>) {
        if let Ok(mut slot) = self.notifier.write() {
            *slot = notifier;
        }
    }

    pub fn notify(&self, notification: Notification) {
        let notifier = self.notifier.read().ok().and_then(|n| n.clone());
        if let Some(n) = notifier {
            n.notify(notification);
        }
    }
}

impl NotifierSlot {
    /// `ServerActivityMonitor.reportLoginActivity`: a login worth telling the management clients about,
    /// at most one notification every 30 seconds.
    pub fn activity(&self) {
        use std::sync::atomic::Ordering;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let last = self.last_activity_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 30_000 && self.last_activity_ms.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            self.notify(Notification::ServerActivity);
        }
    }
}
