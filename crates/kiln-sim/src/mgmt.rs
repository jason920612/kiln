//! The simulation's face for the management server (`MinecraftApi`'s services): players, the
//! operator list, server settings, game rules, saving and stopping. The server's methods run as
//! [`kiln_link::ToSim::Admin`] closures between ticks and reach this through
//! [`kiln_link::AdminHost::mgmt`].

use crate::Sim;
use kiln_command::selector::SelectorWorld;
use kiln_command::{Difficulty, GameMode, GameRuleValue, Host, Text};
use kiln_link::access::{NameAndId, OpEntry, SharedAccess};
use kiln_link::mgmt::{MgmtHost, MgmtText, Notification, OnlinePlayer, RuleValue, Setting, SettingValue};
use kiln_proto::packets;
use std::sync::atomic::Ordering;
use uuid::Uuid;

fn text_of(message: &MgmtText) -> Text {
    match message {
        MgmtText::Literal(s) => Text::literal(s.clone()),
        MgmtText::Translatable { key, args } => {
            Text::translate(key.clone(), args.iter().map(|a| kiln_command::text::Arg::Str(a.clone())).collect())
        }
    }
}

fn game_rule_value(v: RuleValue) -> GameRuleValue {
    match v {
        RuleValue::Bool(b) => GameRuleValue::Bool(b),
        RuleValue::Int(i) => GameRuleValue::Int(i),
    }
}

fn rule_value(v: GameRuleValue) -> RuleValue {
    match v {
        GameRuleValue::Bool(b) => RuleValue::Bool(b),
        GameRuleValue::Int(i) => RuleValue::Int(i),
    }
}

impl Sim {
    /// `MinecraftServer.enforceGameTypeForPlayers`: with `force-gamemode`, everybody plays the default mode.
    fn enforce_game_mode(&mut self) {
        if !self.config.server.force_gamemode {
            return;
        }
        let mode = GameMode::ALL[usize::from(self.default_game_mode().min(3))];
        for p in SelectorWorld::players(self) {
            Host::set_game_mode(self, &p, mode);
        }
    }
}

impl MgmtHost for Sim {
    fn access(&self) -> SharedAccess {
        self.config.access.clone()
    }

    fn online_players(&self) -> Vec<OnlinePlayer> {
        let mut v: Vec<_> = self.players.iter().collect();
        v.sort_by_key(|(c, _)| **c); // join order
        v.into_iter().map(|(_, p)| OnlinePlayer { user: NameAndId { uuid: p.uuid, name: p.name.clone() }, address: p.address }).collect()
    }

    fn kick(&mut self, user: Uuid, reason: &MgmtText) {
        let Some((&conn, p)) = self.players.iter_mut().find(|(_, p)| p.uuid == user) else { return };
        p.flush();
        p.sink.disconnect(packets::play_disconnect_text(text_of(reason).to_nbt()));
        self.leave(conn);
    }

    fn system_message(&mut self, text: &MgmtText, overlay: bool, to: Option<&[Uuid]>) {
        let pkt = packets::system_chat(text_of(text).to_nbt(), overlay);
        let mut conns: Vec<_> = self.players.iter().filter(|(_, p)| to.is_none_or(|u| u.contains(&p.uuid))).map(|(c, _)| *c).collect();
        conns.sort_unstable();
        for c in conns {
            if let Some(p) = self.players.get_mut(&c) {
                p.send(pkt.clone());
            }
        }
    }

    fn op(&mut self, user: &NameAndId, level: Option<u8>, bypasses_player_limit: Option<bool>) {
        let level = level.unwrap_or(self.config.server.op_permission_level);
        {
            let mut access = self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            let bypass = bypasses_player_limit
                .unwrap_or_else(|| access.op_list.get(&user.uuid.to_string()).is_some_and(|e| e.bypasses_player_limit));
            access.op_add(OpEntry { user: user.clone(), level, bypasses_player_limit: bypass });
        }
        self.commands.ops.insert(user.name.clone());
        self.commands.op_levels.insert(user.name.clone(), level);
        self.ops_changed(&user.name);
    }

    fn deop(&mut self, user: &NameAndId) {
        let removed = self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner).op_remove(user).is_some();
        if removed {
            self.commands.ops.remove(&user.name);
            self.commands.op_levels.remove(&user.name);
            self.ops_changed(&user.name);
        }
    }

    fn deop_all(&mut self) {
        let names: Vec<String> = {
            let mut access = self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            let names = access.op_list.values().into_iter().map(|e| e.user.name.clone()).collect();
            access.op_clear();
            names
        };
        for n in &names {
            self.commands.ops.remove(n);
            self.commands.op_levels.remove(n);
        }
        for n in &names {
            self.ops_changed(n);
        }
    }

    fn kick_unlisted(&mut self) {
        let (enforce, on) = {
            let l = self.config.access.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            (l.enforce_whitelist, l.use_whitelist)
        };
        if !enforce || !on {
            return;
        }
        let unlisted: Vec<Uuid> = {
            let l = self.config.access.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.players
                .values()
                .filter(|p| !l.is_whitelisted(&NameAndId { uuid: p.uuid, name: p.name.clone() }))
                .map(|p| p.uuid)
                .collect()
        };
        for u in unlisted {
            MgmtHost::kick(self, u, &MgmtText::Translatable { key: "multiplayer.disconnect.not_whitelisted".into(), args: vec![] });
        }
    }

    fn setting(&self, setting: Setting) -> SettingValue {
        let access = || self.config.access.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        match setting {
            Setting::Autosave => SettingValue::Bool(self.commands.auto_save),
            Setting::Difficulty => SettingValue::Int(self.commands.difficulty as i32),
            Setting::EnforceAllowlist => SettingValue::Bool(access().enforce_whitelist),
            Setting::UseAllowlist => SettingValue::Bool(access().use_whitelist),
            Setting::MaxPlayers => SettingValue::Int(self.config.info.max_players()),
            Setting::PauseWhenEmptySeconds => SettingValue::Int(self.config.server.pause_when_empty_seconds),
            Setting::PlayerIdleTimeout => SettingValue::Int(self.commands.idle_timeout),
            Setting::AllowFlight => SettingValue::Bool(self.config.server.allow_flight),
            Setting::Motd => SettingValue::Str(self.config.info.motd()),
            Setting::SpawnProtection => SettingValue::Int(self.config.server.spawn_protection),
            Setting::ForceGameMode => SettingValue::Bool(self.config.server.force_gamemode),
            Setting::GameMode => SettingValue::Int(i32::from(self.default_game_mode())),
            Setting::ViewDistance => SettingValue::Int(i32::from(self.config.view_distance)),
            Setting::SimulationDistance => SettingValue::Int(i32::from(self.config.simulation_distance)),
            Setting::AcceptTransfers => SettingValue::Bool(self.config.info.accepts_transfers.load(Ordering::Relaxed)),
            Setting::OperatorPermissionLevel => SettingValue::Int(i32::from(self.config.server.op_permission_level)),
            Setting::HideOnlinePlayers => SettingValue::Bool(self.config.info.hide_online_players.load(Ordering::Relaxed)),
            Setting::StatusReplies => SettingValue::Bool(self.config.info.enable_status.load(Ordering::Relaxed)),
            Setting::EntityBroadcastRange => SettingValue::Int(self.config.server.entity_broadcast_range_percentage),
        }
    }

    fn set_setting(&mut self, setting: Setting, value: SettingValue) {
        let (b, i) = match &value {
            SettingValue::Bool(b) => (*b, 0),
            SettingValue::Int(i) => (false, *i),
            SettingValue::Str(_) => (false, 0),
        };
        match setting {
            Setting::Autosave => self.commands.auto_save = b,
            Setting::Difficulty => Host::set_difficulty(self, Difficulty::ALL[i.clamp(0, 3) as usize]),
            Setting::EnforceAllowlist => {
                self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner).enforce_whitelist = b;
            }
            Setting::UseAllowlist => {
                self.config.access.write().unwrap_or_else(std::sync::PoisonError::into_inner).use_whitelist = b;
            }
            Setting::MaxPlayers => {
                self.config.info.max_players.store(i, Ordering::Relaxed);
                self.config.max_players = i.max(0) as usize;
            }
            Setting::PauseWhenEmptySeconds => self.config.server.pause_when_empty_seconds = i,
            Setting::PlayerIdleTimeout => self.commands.idle_timeout = i.max(0),
            Setting::AllowFlight => self.config.server.allow_flight = b,
            Setting::Motd => {
                if let (SettingValue::Str(s), Ok(mut motd)) = (&value, self.config.info.motd.write()) {
                    *motd = s.clone();
                }
            }
            Setting::SpawnProtection => self.config.server.spawn_protection = i,
            Setting::ForceGameMode => {
                self.config.server.force_gamemode = b;
                self.enforce_game_mode();
            }
            Setting::GameMode => {
                let mode = i.clamp(0, 3) as u8;
                self.commands.default_game_mode = Some(mode);
                if let Some(s) = self.storage.as_mut() {
                    s.level.set_game_type(mode);
                }
                self.enforce_game_mode();
            }
            Setting::ViewDistance => self.config.view_distance = i.clamp(2, 32) as u8,
            Setting::SimulationDistance => self.config.simulation_distance = i.clamp(2, 32) as u8,
            Setting::AcceptTransfers => self.config.info.accepts_transfers.store(b, Ordering::Relaxed),
            Setting::OperatorPermissionLevel => self.config.server.op_permission_level = i.clamp(0, 4) as u8,
            Setting::HideOnlinePlayers => self.config.info.hide_online_players.store(b, Ordering::Relaxed),
            Setting::StatusReplies => self.config.info.enable_status.store(b, Ordering::Relaxed),
            Setting::EntityBroadcastRange => {
                self.config.server.entity_broadcast_range_percentage = i;
                crate::entities::set_broadcast_range_percentage(i);
            }
        }
    }

    fn game_rules(&self) -> Vec<(String, RuleValue, Option<(i32, i32)>)> {
        kiln_command::vanilla::gamerules::rules()
            .iter()
            .map(|&r| (r.to_owned(), rule_value(Host::game_rule(self, r)), kiln_command::vanilla::gamerules::int_range(r)))
            .collect()
    }

    fn game_rule(&self, key: &str) -> Option<(String, RuleValue, Option<(i32, i32)>)> {
        let id = if key.contains(':') { key.to_owned() } else { format!("minecraft:{key}") };
        let &rule = kiln_command::vanilla::gamerules::rules().iter().find(|r| **r == id)?;
        Some((rule.to_owned(), rule_value(Host::game_rule(self, rule)), kiln_command::vanilla::gamerules::int_range(rule)))
    }

    fn set_game_rule(&mut self, key: &str, value: RuleValue) {
        Host::set_game_rule(self, key, game_rule_value(value));
    }

    fn save_everything(&mut self, _flush: bool) -> bool {
        // At once, not at the next tick: the notifications follow the reply as vanilla's do.
        self.start_save();
        self.continue_save();
        true
    }

    fn plain(&self, text: &MgmtText) -> String {
        text_of(text).to_plain()
    }

    fn halt(&mut self) {
        Host::stop(self);
    }
}

impl Sim {
    /// Tells the players concerned that their operator level changed (`sendPlayerPermissionLevel`).
    fn ops_changed(&mut self, name: &str) {
        self.sync_ops();
        let conn = self.players.iter().find(|(_, p)| p.name == name).map(|(c, _)| *c);
        if let Some(c) = conn {
            self.send_command_tree(c);
        }
    }

    /// Raises a notification for the management server.
    pub(crate) fn notify(&self, n: Notification) {
        self.config.info.notifier.notify(n);
    }
}
