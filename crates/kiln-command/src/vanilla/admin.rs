//! Server administration: `whitelist`, `ban`, `ban-ip`, `banlist`, `pardon`, `pardon-ip`
//! (vanilla `WhitelistCommand`, `BanPlayerCommands`, `BanIpCommands`, `BanListCommands`,
//! `PardonCommand`, `PardonIpCommand`) over the shared [`AccessLists`], and `save-all`,
//! `save-on`, `save-off`, `defaultgamemode`, `setidletimeout`,
//! `version`, and the profilers `debug`, `perf` and `jfr` (which Kiln does not have: they
//! answer as a server that is not profiling).

use super::{LEVEL_ADMINS, LEVEL_GAMEMASTERS, LEVEL_OWNERS, resolve_profiles};
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, Profile};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use kiln_link::access::{AccessLists, BanInfo, IpBan, NameAndId, SharedAccess, UserBan};

type Result<T> = std::result::Result<T, CommandError>;

fn lists<S: Host>(s: &S) -> Result<SharedAccess> {
    s.access().ok_or_else(|| CommandError::new(Text::literal("This server keeps no player lists")))
}

fn with_lists<S: Host, T>(s: &S, f: impl FnOnce(&mut AccessLists) -> T) -> Result<T> {
    let shared = lists(s)?;
    let mut guard = shared.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(f(&mut guard))
}

fn name_and_id(p: &Profile) -> NameAndId {
    NameAndId { uuid: p.uuid, name: p.name.clone() }
}

/// `CommandSourceStack.getTextName`: the executing entity's name, else the source's.
fn text_name<S: Host>(s: &S) -> String {
    match s.source_entity() {
        Some(e) => e.name(),
        None => s.stack().name.to_plain(),
    }
}

/// `BanListEntry.getReasonMessage`.
fn reason_message(ban: &BanInfo) -> Text {
    match &ban.reason {
        Some(r) => Text::literal(r.clone()),
        None => tr!("multiplayer.disconnect.banned.reason.default"),
    }
}

/// Guava `InetAddresses.isInetAddress` for what a `word` argument can hold: an IPv4 dotted
/// quad of decimal octets without leading zeros (IPv6 needs `:`, which a word cannot hold).
pub fn is_inet_address(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && !(p.len() > 1 && p.starts_with('0'))
                && p.parse::<u16>().is_ok_and(|v| v <= 255)
        })
}

/// `MinecraftServer.kickUnlistedPlayers`: with `enforce-whitelist`, players off the list leave.
fn kick_unlisted<S: Host>(s: &mut S) -> Result<()> {
    let (enforce, on) = with_lists(s, |l| (l.enforce_whitelist, l.use_whitelist))?;
    if !enforce || !on {
        return Ok(());
    }
    for p in s.players() {
        let user = NameAndId { uuid: p.uuid(), name: p.name() };
        if !with_lists(s, |l| l.is_whitelisted(&user))? {
            s.kick(&p, tr!("multiplayer.disconnect.not_whitelisted"));
        }
    }
    Ok(())
}

pub fn whitelist<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn change<S: Host>(c: &CommandContext<S>, s: &mut S, add: bool) -> Result<i32> {
        let profiles = resolve_profiles(c.game_profile("targets"), s)?;
        let mut changed = 0;
        for p in &profiles {
            let user = name_and_id(p);
            let key = user.uuid.to_string();
            let done = with_lists(s, |l| {
                if l.whitelist.contains(&key) == add {
                    return false;
                }
                if add {
                    l.whitelist.put(key.clone(), user.clone());
                } else {
                    l.whitelist.remove(&key);
                }
                l.save_whitelist();
                true
            })?;
            if done {
                let k = if add { "commands.whitelist.add.success" } else { "commands.whitelist.remove.success" };
                s.send_success(tr!(k, Text::literal(p.name.clone())), true);
                changed += 1;
            }
        }
        if changed == 0 {
            let k = if add { "commands.whitelist.add.failed" } else { "commands.whitelist.remove.failed" };
            return Err(CommandError::new(tr!(k)));
        }
        if !add {
            kick_unlisted(s)?;
        }
        Ok(changed)
    }
    fn switch<S: Host>(s: &mut S, on: bool) -> Result<i32> {
        if with_lists(s, |l| l.use_whitelist)? == on {
            let k = if on { "commands.whitelist.alreadyOn" } else { "commands.whitelist.alreadyOff" };
            return Err(CommandError::new(tr!(k)));
        }
        with_lists(s, |l| l.use_whitelist = on)?;
        s.send_success(tr!(if on { "commands.whitelist.enabled" } else { "commands.whitelist.disabled" }), true);
        if on {
            kick_unlisted(s)?;
        }
        Ok(1)
    }
    d.register(
        literal("whitelist")
            .requires(LEVEL_ADMINS)
            .then(literal("on").executes(|_, s: &mut S| switch(s, true)))
            .then(literal("off").executes(|_, s: &mut S| switch(s, false)))
            .then(literal("list").executes(|_, s: &mut S| {
                let names: Vec<String> = with_lists(s, |l| l.whitelist.values().into_iter().map(|u| u.name.clone()).collect())?;
                if names.is_empty() {
                    s.send_success(tr!("commands.whitelist.none"), false);
                } else {
                    s.send_success(tr!("commands.whitelist.list", names.len() as i32, names.join(", ")), false);
                }
                Ok(names.len() as i32)
            }))
            .then(
                literal("add").then(
                    argument("targets", ArgumentType::GameProfile)
                        .suggests_server(|_, s: &S, b| {
                            let listed: Vec<String> =
                                s.access().map(|a| a.read().map(|l| l.whitelist.values().iter().map(|u| u.name.clone()).collect()).unwrap_or_default()).unwrap_or_default();
                            let names: Vec<String> = s.players().iter().map(SelectorTarget::name).filter(|n| !listed.contains(n)).collect();
                            b.suggest_matching(names.iter().map(String::as_str));
                        })
                        .executes(|c, s: &mut S| change(c, s, true)),
                ),
            )
            .then(
                literal("remove").then(
                    argument("targets", ArgumentType::GameProfile)
                        .suggests_server(|_, s: &S, b| {
                            let names: Vec<String> =
                                s.access().map(|a| a.read().map(|l| l.whitelist.values().iter().map(|u| u.name.clone()).collect()).unwrap_or_default()).unwrap_or_default();
                            b.suggest_matching(names.iter().map(String::as_str));
                        })
                        .executes(|c, s: &mut S| change(c, s, false)),
                ),
            )
            .then(literal("reload").executes(|_, s: &mut S| {
                // `PlayerList.reloadWhiteList` does nothing in 26.3.
                s.send_success(tr!("commands.whitelist.reloaded"), true);
                kick_unlisted(s)?;
                Ok(1)
            })),
    );
}

pub fn ban<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
        let profiles = resolve_profiles(c.game_profile("targets"), s)?;
        let reason = match c.get("reason") {
            Some(_) => Some(c.message("reason").resolve(s)?.to_plain()),
            None => None,
        };
        let source = text_name(s);
        let mut banned = 0;
        for p in &profiles {
            let user = name_and_id(p);
            let key = user.uuid.to_string();
            let entry = UserBan { user, ban: BanInfo::now(Some(&source), reason.clone()) };
            let message = reason_message(&entry.ban);
            let added = with_lists(s, |l| {
                if l.is_banned(&entry.user.uuid) {
                    return false;
                }
                l.bans.put(key.clone(), entry.clone());
                l.save_bans();
                true
            })?;
            if added {
                banned += 1;
                s.send_success(tr!("commands.ban.success", Text::literal(p.name.clone()), message), true);
                if let Some(player) = s.players().into_iter().find(|e| e.uuid() == p.uuid) {
                    s.kick(&player, tr!("multiplayer.disconnect.banned"));
                }
            }
        }
        if banned == 0 {
            return Err(CommandError::new(tr!("commands.ban.failed")));
        }
        Ok(banned)
    }
    d.register(
        literal("ban").requires(LEVEL_ADMINS).then(
            argument("targets", ArgumentType::GameProfile)
                .executes(run)
                .then(argument("reason", ArgumentType::Message).executes(run)),
        ),
    );
}

pub fn ban_ip<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
        let target = c.string("target").to_owned();
        let reason = match c.get("reason") {
            Some(_) => Some(c.message("reason").resolve(s)?.to_plain()),
            None => None,
        };
        let ip = if is_inet_address(&target) {
            target
        } else {
            // `PlayerList.getPlayerByName`.
            let player = s.players().into_iter().find(|p| p.name().eq_ignore_ascii_case(&target));
            match player.and_then(|p| s.player_ip(&p)) {
                Some(ip) => ip,
                None => return Err(CommandError::new(tr!("commands.banip.invalid"))),
            }
        };
        if with_lists(s, |l| l.is_ip_banned(&ip))? {
            return Err(CommandError::new(tr!("commands.banip.failed")));
        }
        let players: Vec<S::Entity> = s.players().into_iter().filter(|p| s.player_ip(p).as_deref() == Some(ip.as_str())).collect();
        let entry = IpBan { ip: ip.clone(), ban: BanInfo::now(Some(&text_name(s)), reason) };
        let message = reason_message(&entry.ban);
        with_lists(s, |l| {
            l.ip_bans.put(ip.clone(), entry);
            l.save_ip_bans();
        })?;
        s.send_success(tr!("commands.banip.success", ip.as_str(), message), true);
        if !players.is_empty() {
            let names = Text::join(players.iter().map(SelectorTarget::display_name));
            s.send_success(tr!("commands.banip.info", players.len() as i32, names), true);
        }
        for p in &players {
            s.kick(p, tr!("multiplayer.disconnect.ip_banned"));
        }
        Ok(players.len() as i32)
    }
    d.register(
        literal("ban-ip").requires(LEVEL_ADMINS).then(
            argument("target", ArgumentType::word())
                .executes(run)
                .then(argument("reason", ArgumentType::Message).executes(run)),
        ),
    );
}

pub fn banlist<S: Host + 'static>(d: &mut Dispatcher<S>) {
    /// Display name, source and reason of each entry.
    type Entry = (Text, String, Text);
    fn show<S: Host>(s: &mut S, players: bool, ips: bool) -> Result<i32> {
        let entries: Vec<Entry> = with_lists(s, |l| {
            let mut v: Vec<Entry> = Vec::new();
            if players {
                v.extend(l.bans.values().into_iter().map(|b| (Text::literal(b.user.name.clone()), b.ban.source.clone(), reason_message(&b.ban))));
            }
            if ips {
                v.extend(l.ip_bans.values().into_iter().map(|b| (Text::literal(b.ip.clone()), b.ban.source.clone(), reason_message(&b.ban))));
            }
            v
        })?;
        if entries.is_empty() {
            s.send_success(tr!("commands.banlist.none"), false);
        } else {
            s.send_success(tr!("commands.banlist.list", entries.len() as i32), false);
            for (name, source, reason) in &entries {
                s.send_success(tr!("commands.banlist.entry", name.clone(), source.as_str(), reason.clone()), false);
            }
        }
        Ok(entries.len() as i32)
    }
    d.register(
        literal("banlist")
            .requires(LEVEL_ADMINS)
            .executes(|_, s: &mut S| show(s, true, true))
            .then(literal("ips").executes(|_, s: &mut S| show(s, false, true)))
            .then(literal("players").executes(|_, s: &mut S| show(s, true, false))),
    );
}

pub fn pardon<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("pardon").requires(LEVEL_ADMINS).then(
            argument("targets", ArgumentType::GameProfile)
                .suggests_server(|_, s: &S, b| {
                    let names: Vec<String> =
                        s.access().map(|a| a.read().map(|l| l.bans.values().iter().map(|b| b.user.name.clone()).collect()).unwrap_or_default()).unwrap_or_default();
                    b.suggest_matching(names.iter().map(String::as_str));
                })
                .executes(|c, s: &mut S| {
                    let profiles = resolve_profiles(c.game_profile("targets"), s)?;
                    let mut n = 0;
                    for p in &profiles {
                        let removed = with_lists(s, |l| {
                            if !l.is_banned(&p.uuid) {
                                return false;
                            }
                            l.bans.remove(&p.uuid.to_string());
                            l.save_bans();
                            true
                        })?;
                        if removed {
                            n += 1;
                            s.send_success(tr!("commands.pardon.success", Text::literal(p.name.clone())), true);
                        }
                    }
                    if n == 0 {
                        return Err(CommandError::new(tr!("commands.pardon.failed")));
                    }
                    Ok(n)
                }),
        ),
    );
}

pub fn pardon_ip<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("pardon-ip").requires(LEVEL_ADMINS).then(
            argument("target", ArgumentType::word())
                .suggests_server(|_, s: &S, b| {
                    let ips: Vec<String> =
                        s.access().map(|a| a.read().map(|l| l.ip_bans.values().iter().map(|b| b.ip.clone()).collect()).unwrap_or_default()).unwrap_or_default();
                    b.suggest_matching(ips.iter().map(String::as_str));
                })
                .executes(|c, s: &mut S| {
                    let ip = c.string("target").to_owned();
                    if !is_inet_address(&ip) {
                        return Err(CommandError::new(tr!("commands.pardonip.invalid")));
                    }
                    let removed = with_lists(s, |l| {
                        if !l.is_ip_banned(&ip) {
                            return false;
                        }
                        l.ip_bans.remove(&ip);
                        l.save_ip_bans();
                        true
                    })?;
                    if !removed {
                        return Err(CommandError::new(tr!("commands.pardonip.failed")));
                    }
                    s.send_success(tr!("commands.pardonip.success", ip.as_str()), true);
                    Ok(1)
                }),
        ),
    );
}

pub fn save<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn save_all<S: Host>(s: &mut S, flush: bool) -> Result<i32> {
        s.send_success(tr!("commands.save.saving"), false);
        if !s.save_all(flush) {
            return Err(CommandError::new(tr!("commands.save.failed")));
        }
        s.send_success(tr!("commands.save.success"), true);
        Ok(1)
    }
    d.register(
        literal("save-all")
            .requires(LEVEL_OWNERS)
            .executes(|_, s: &mut S| save_all(s, false))
            .then(literal("flush").executes(|_, s: &mut S| save_all(s, true))),
    );
    for (name, on) in [("save-on", true), ("save-off", false)] {
        d.register(literal(name).requires(LEVEL_OWNERS).executes(move |_, s: &mut S| {
            if !s.set_auto_save(on) {
                return Err(CommandError::new(tr!(if on { "commands.save.alreadyOn" } else { "commands.save.alreadyOff" })));
            }
            s.send_success(tr!(if on { "commands.save.enabled" } else { "commands.save.disabled" }), true);
            Ok(1)
        }));
    }
}

pub fn defaultgamemode<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("defaultgamemode").requires(LEVEL_GAMEMASTERS).then(argument("gamemode", ArgumentType::GameMode).executes(
            |c, s: &mut S| {
                let mode = c.game_mode("gamemode");
                let changed = s.set_default_game_mode(mode);
                s.send_success(tr!("commands.defaultgamemode.success", mode.display_name()), true);
                Ok(changed)
            },
        )),
    );
}

pub fn setidletimeout<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("setidletimeout").requires(LEVEL_ADMINS).then(argument("minutes", ArgumentType::integer_min(0)).executes(
            |c, s: &mut S| {
                let minutes = c.integer("minutes");
                s.set_idle_timeout(minutes);
                if minutes > 0 {
                    s.send_success(tr!("commands.setidletimeout.success", minutes), true);
                } else {
                    s.send_success(tr!("commands.setidletimeout.success.disabled"), true);
                }
                Ok(minutes)
            },
        )),
    );
}

/// `version`: the server's version as `SharedConstants.getCurrentVersion` describes it.
pub fn version<S: Host + 'static>(d: &mut Dispatcher<S>) {
    use kiln_data::version::{NAME, PROTOCOL, WORLD_VERSION};
    d.register(literal("version").requires(LEVEL_GAMEMASTERS).executes(|_, s: &mut S| {
        let lines = [
            tr!("commands.version.header"),
            tr!("commands.version.id", NAME),
            tr!("commands.version.name", NAME),
            tr!("commands.version.data", WORLD_VERSION),
            tr!("commands.version.series", "main"),
            tr!("commands.version.protocol", PROTOCOL, format!("0x{PROTOCOL:X}")),
            // The vanilla jar's build time; vanilla prints it in the JVM's time zone.
            tr!("commands.version.build_time", "Tue Sep 15 11:20:48 UTC 2026"),
            tr!("commands.version.pack.resource", "97.1"),
            tr!("commands.version.pack.data", "121.0"),
            tr!("commands.version.stable.yes"),
        ];
        for line in lines {
            s.send_success(line, false);
        }
        Ok(0)
    }));
}

/// The profilers Kiln does not have: `debug`, `perf` and `jfr` start nothing, and stopping
/// answers as vanilla does when no profiler runs.
pub fn profilers<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let unsupported = |_: &CommandContext<S>, _: &mut S| -> Result<i32> {
        Err(CommandError::new(Text::literal("Kiln has no profiler; use /kiln tick")))
    };
    d.register(
        literal("debug")
            .requires(LEVEL_ADMINS)
            .then(literal("start").executes(unsupported))
            .then(literal("stop").executes(|_, _: &mut S| Err(CommandError::new(tr!("commands.debug.notRunning")))))
            .then(literal("function").requires(LEVEL_ADMINS).then(argument("name", ArgumentType::Function).executes(unsupported))),
    );
    d.register(
        literal("perf")
            .requires(LEVEL_OWNERS)
            .then(literal("start").executes(unsupported))
            .then(literal("stop").executes(|_, _: &mut S| Err(CommandError::new(tr!("commands.perf.notRunning"))))),
    );
    d.register(
        literal("jfr")
            .requires(LEVEL_OWNERS)
            .then(literal("start").executes(unsupported))
            .then(literal("stop").executes(|_, _: &mut S| {
                Err(CommandError::new(tr!("commands.jfr.dump.failed", "Not currently profiling")))
            })),
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn inet_addresses_as_guava() {
        use super::is_inet_address as ok;
        assert!(ok("10.0.0.1") && ok("255.255.255.255") && ok("0.0.0.0"));
        assert!(!ok("10.0.0.300") && !ok("10.0.0") && !ok("010.0.0.1") && !ok("a.b.c.d") && !ok("1.2.3.4.5") && !ok(""));
    }
}
