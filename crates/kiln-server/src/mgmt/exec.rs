//! The methods: what `net.minecraft.server.jsonrpc.methods` and `internalapi` do for each request.
//! Parameters are decoded and players looked up on the connection's thread; what touches the running
//! game (kicks, operators, settings, game rules, saving) runs on the simulation thread.

use super::codec::{self, Errs, PlayerDto, R};
use super::json::J;
use super::{Ctx, Fail, jhash};
use kiln_link::access::{AccessLists, BanInfo, IpBan, NameAndId, OpEntry, UserBan};
use kiln_link::mgmt::{MgmtText, RuleValue, Setting, SettingValue};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use uuid::Uuid;

const SOURCE: &str = "Management server";

fn invalid(e: String) -> Fail {
    Fail::InvalidParams(e)
}

fn lists(ctx: &Ctx) -> std::sync::RwLockReadGuard<'_, AccessLists> {
    ctx.access.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn lists_mut(ctx: &Ctx) -> std::sync::RwLockWriteGuard<'_, AccessLists> {
    ctx.access.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- results ----

pub fn player_json(u: &NameAndId) -> J {
    J::obj(vec![("id", J::str(u.uuid.to_string())), ("name", J::str(u.name.clone()))])
}

fn dto_json(p: &PlayerDto) -> J {
    let mut m = Vec::new();
    if let Some(id) = p.id {
        m.push(("id", J::str(id.to_string())));
    }
    if let Some(n) = &p.name {
        m.push(("name", J::str(n.clone())));
    }
    J::obj(m)
}

fn expires_json(b: &BanInfo) -> Option<J> {
    let secs = b.expires.as_deref().and_then(kiln_link::access::parse_date)?;
    Some(J::str(codec::instant_string(secs, u32::from(b.expires_ms) * 1_000_000)))
}

pub fn user_ban_json(b: &UserBan) -> J {
    let mut m = vec![("player", player_json(&b.user))];
    if let Some(r) = &b.ban.reason {
        m.push(("reason", J::str(r.clone())));
    }
    m.push(("source", J::str(b.ban.source.clone())));
    if let Some(e) = expires_json(&b.ban) {
        m.push(("expires", e));
    }
    J::obj(m)
}

pub fn ip_ban_json(b: &IpBan) -> J {
    let mut m = vec![("ip", J::str(b.ip.clone()))];
    if let Some(r) = &b.ban.reason {
        m.push(("reason", J::str(r.clone())));
    }
    m.push(("source", J::str(b.ban.source.clone())));
    if let Some(e) = expires_json(&b.ban) {
        m.push(("expires", e));
    }
    J::obj(m)
}

pub fn op_json(e: &OpEntry) -> J {
    J::obj(vec![
        ("player", player_json(&e.user)),
        ("permissionLevel", J::int(i64::from(e.level))),
        ("bypassesPlayerLimit", J::Bool(e.bypasses_player_limit)),
    ])
}

pub fn rule_json(key: &str, value: RuleValue) -> J {
    let (ty, v) = match value {
        RuleValue::Bool(b) => ("boolean", J::Bool(b)),
        RuleValue::Int(i) => ("integer", J::int(i64::from(i))),
    };
    J::obj(vec![("type", J::str(ty)), ("value", v), ("key", J::str(key))])
}

/// `ServerStateService.status`: `started`, the players when there are any, the version.
pub fn state_json(ctx_info: &kiln_link::ServerInfo) -> J {
    let mut m = vec![("started", J::Bool(ctx_info.ready.load(Ordering::Relaxed)))];
    let players: Vec<J> = if ctx_info.ready.load(Ordering::Relaxed) {
        ctx_info.players().iter().map(|p| player_json(&NameAndId { uuid: p.uuid, name: p.name.clone() })).collect()
    } else {
        Vec::new()
    };
    if !players.is_empty() {
        m.push(("players", J::Arr(players)));
    }
    m.push((
        "version",
        J::obj(vec![("name", J::str(kiln_data::version::NAME)), ("protocol", J::int(i64::from(kiln_data::version::PROTOCOL)))]),
    ));
    J::obj(m)
}

fn players_json(l: &AccessLists) -> J {
    J::Arr(l.whitelist.values().into_iter().map(player_json).collect())
}

fn bans_json(l: &AccessLists) -> J {
    J::Arr(l.bans.values().into_iter().map(user_ban_json).collect())
}

fn ip_bans_json(l: &AccessLists) -> J {
    J::Arr(l.ip_bans.values().into_iter().map(ip_ban_json).collect())
}

fn ops_json(l: &AccessLists) -> J {
    J::Arr(l.op_list.values().into_iter().map(op_json).collect())
}

// ---- decoding ----

type Instant = (i64, u32);

struct UserBanDto {
    player: PlayerDto,
    reason: Option<String>,
    source: Option<String>,
    expires: Option<Instant>,
}

fn user_ban_dto(j: &J) -> R<UserBanDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let player = errs.take(codec::required(j, "player").and_then(codec::player_dto));
    let reason = j.get("reason").and_then(|v| errs.take(codec::string(v)));
    let source = j.get("source").and_then(|v| errs.take(codec::string(v)));
    let expires = j.get("expires").and_then(|v| errs.take(codec::instant(v)));
    errs.finish()?;
    Ok(UserBanDto { player: player.expect("checked"), reason, source, expires })
}

struct IpBanDto {
    player: Option<PlayerDto>,
    ip: Option<String>,
    reason: Option<String>,
    source: Option<String>,
    expires: Option<Instant>,
}

fn incoming_ip_ban_dto(j: &J) -> R<IpBanDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let player = j.get("player").and_then(|v| errs.take(codec::player_dto(v)));
    let ip = j.get("ip").and_then(|v| errs.take(codec::string(v)));
    let reason = j.get("reason").and_then(|v| errs.take(codec::string(v)));
    let source = j.get("source").and_then(|v| errs.take(codec::string(v)));
    let expires = j.get("expires").and_then(|v| errs.take(codec::instant(v)));
    errs.finish()?;
    Ok(IpBanDto { player, ip, reason, source, expires })
}

/// `IpBanDto`: the `ip` is required.
fn ip_ban_dto(j: &J) -> R<IpBanDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let ip = errs.take(codec::required(j, "ip").and_then(codec::string));
    let reason = j.get("reason").and_then(|v| errs.take(codec::string(v)));
    let source = j.get("source").and_then(|v| errs.take(codec::string(v)));
    let expires = j.get("expires").and_then(|v| errs.take(codec::instant(v)));
    errs.finish()?;
    Ok(IpBanDto { player: None, ip, reason, source, expires })
}

struct OpDto {
    player: PlayerDto,
    level: Option<u8>,
    bypass: Option<bool>,
}

fn op_dto(j: &J) -> R<OpDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let player = errs.take(codec::required(j, "player").and_then(codec::player_dto));
    // (`PermissionLevel.INT_CODEC` clamps.)
    let level = j.get("permissionLevel").and_then(|v| errs.take(codec::integer(v))).map(|l| l.clamp(0, 4) as u8);
    let bypass = j.get("bypassesPlayerLimit").and_then(|v| errs.take(codec::boolean(v)));
    errs.finish()?;
    Ok(OpDto { player: player.expect("checked"), level, bypass })
}

struct KickDto {
    player: PlayerDto,
    message: Option<codec::Message>,
}

fn kick_dto(j: &J) -> R<KickDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let player = errs.take(codec::required(j, "player").and_then(codec::player_dto));
    let message = j.get("message").and_then(|v| errs.take(codec::message(v)));
    errs.finish()?;
    Ok(KickDto { player: player.expect("checked"), message })
}

struct SystemMessageDto {
    message: codec::Message,
    overlay: bool,
    receiving: Option<Vec<PlayerDto>>,
}

fn system_message_dto(j: &J) -> R<SystemMessageDto> {
    codec::object(j)?;
    let mut errs = Errs::default();
    let message = errs.take(codec::required(j, "message").and_then(codec::message));
    let overlay = errs.take(codec::required(j, "overlay").and_then(codec::boolean));
    // (`lenientOptionalFieldOf`.)
    let receiving = j.get("receivingPlayers").and_then(|v| codec::list(v, codec::player_dto).ok());
    errs.finish()?;
    Ok(SystemMessageDto { message: message.expect("checked"), overlay: overlay.expect("checked"), receiving })
}

fn permission_level_name(level: u8) -> &'static str {
    ["ALL", "MODERATORS", "GAMEMASTERS", "ADMINS", "OWNERS"][usize::from(level.min(4))]
}

const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];
const GAME_TYPES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

fn enum_index(j: &J, names: &[&str]) -> R<usize> {
    let s = codec::string(j)?;
    names.iter().position(|n| *n == s).ok_or_else(|| format!("Unknown element name:{s}"))
}

/// `Identifier.CODEC`/`tryParse` for a game rule key: namespace and path, `minecraft` by default.
fn identifier(s: &str) -> R<String> {
    let valid_ns = |ns: &str| ns.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    let valid_path = |p: &str| p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-' | '/'));
    let (ns, path) = match s.split_once(':') {
        Some((ns, path)) if !ns.is_empty() => (ns, path),
        Some((_, path)) => ("minecraft", path),
        None => ("minecraft", s),
    };
    if !valid_ns(ns) {
        return Err(format!("Not a valid resource location: {s} Non [a-z0-9_.-] character in namespace of identifier: {s}"));
    }
    if !valid_path(path) {
        return Err(format!("Not a valid resource location: {s} Non [a-z0-9/._-] character in path of location: {ns}:{path}"));
    }
    Ok(format!("{ns}:{path}"))
}

/// Guava `InetAddresses.isInetAddress`.
pub fn is_inet_address(s: &str) -> bool {
    let v4 = {
        let parts: Vec<&str> = s.split('.').collect();
        parts.len() == 4
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.len() <= 3
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && !(p.len() > 1 && p.starts_with('0'))
                    && p.parse::<u16>().is_ok_and(|v| v <= 255)
            })
    };
    v4 || (s.contains(':') && !s.contains('%') && s.parse::<std::net::Ipv6Addr>().is_ok())
}

// ---- players ----

impl Ctx {
    /// `MinecraftPlayerListService.getUser`: who a request means. Players online and the names the lists
    /// know, then (an online-mode server) the session service, else (offline mode) the offline profile.
    fn resolve(&self, dto: &PlayerDto) -> Option<NameAndId> {
        if let Some(id) = dto.id {
            return self.user_by_id(id);
        }
        let name = dto.name.as_deref()?;
        self.user_by_name(name)
    }

    fn remember(&self, u: &NameAndId) {
        if let Ok(mut c) = self.users.lock() {
            c.insert(u.uuid, u.clone());
        }
    }

    fn user_by_id(&self, id: Uuid) -> Option<NameAndId> {
        if let Some(p) = self.info.players().iter().find(|p| p.uuid == id) {
            return Some(NameAndId { uuid: id, name: p.name.clone() });
        }
        {
            let l = lists(self);
            let key = id.to_string();
            if let Some(u) = l.whitelist.get(&key) {
                return Some(u.clone());
            }
            if let Some(e) = l.op_list.get(&key) {
                return Some(e.user.clone());
            }
            if let Some(b) = l.bans.get(&key) {
                return Some(b.user.clone());
            }
        }
        if let Some(u) = self.users.lock().ok().and_then(|c| c.get(&id).cloned()) {
            return Some(u);
        }
        let found = self.lookup.as_ref()?.by_id(id).ok().flatten()?;
        let u = NameAndId { uuid: found.id, name: found.name };
        self.remember(&u);
        Some(u)
    }

    fn user_by_name(&self, name: &str) -> Option<NameAndId> {
        if let Some(p) = self.info.players().iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Some(NameAndId { uuid: p.uuid, name: p.name.clone() });
        }
        {
            let l = lists(self);
            if let Some(u) = l.whitelist.values().into_iter().find(|u| u.name.eq_ignore_ascii_case(name)) {
                return Some(u.clone());
            }
            if let Some(e) = l.op_list.values().into_iter().find(|e| e.user.name.eq_ignore_ascii_case(name)) {
                return Some(e.user.clone());
            }
            if let Some(b) = l.bans.values().into_iter().find(|b| b.user.name.eq_ignore_ascii_case(name)) {
                return Some(b.user.clone());
            }
        }
        if let Some(u) = self.users.lock().ok().and_then(|c| c.values().find(|u| u.name.eq_ignore_ascii_case(name)).cloned()) {
            return Some(u);
        }
        if let Some(lookup) = &self.lookup
            && let Ok(Some(found)) = lookup.by_name(name)
        {
            let u = NameAndId { uuid: found.id, name: found.name };
            self.remember(&u);
            return Some(u);
        }
        if self.online_mode {
            return None;
        }
        // `NameAndId.createOffline`.
        let lower = name.to_ascii_lowercase();
        Some(NameAndId { uuid: super::offline_uuid(&lower), name: lower })
    }

    fn resolve_all(&self, dtos: &[PlayerDto]) -> Vec<Option<NameAndId>> {
        dtos.iter().map(|d| self.resolve(d)).collect()
    }

    /// An online player by id, else by name (`getServerPlayer`).
    fn online(&self, dto: &PlayerDto) -> Option<Uuid> {
        let players = self.info.players();
        match (dto.id, &dto.name) {
            (Some(id), _) => players.iter().find(|p| p.uuid == id).map(|p| p.uuid),
            (None, Some(n)) => players.iter().find(|p| p.name.eq_ignore_ascii_case(n)).map(|p| p.uuid),
            _ => None,
        }
    }
}

fn translatable(key: &str) -> MgmtText {
    MgmtText::Translatable { key: key.to_owned(), args: Vec::new() }
}

// ---- the methods ----

pub fn call(ctx: &Ctx, client: u32, name: &str, param: Option<&J>) -> Result<J, Fail> {
    let p = || param.expect("a method with a parameter");
    let list = |f: fn(&J) -> R<PlayerDto>| codec::list(p(), f).map_err(invalid);
    match name {
        // allowlist
        "allowlist" => Ok(players_json(&lists(ctx))),
        "allowlist/add" => {
            let dtos = list(codec::player_dto)?;
            for u in ctx.resolve_all(&dtos).into_iter().flatten() {
                ctx.log(client, &format!("Add player '{}' to allowlist", u.name));
                lists_mut(ctx).whitelist_add(u);
            }
            Ok(players_json(&lists(ctx)))
        }
        "allowlist/clear" => {
            ctx.log(client, "Clear allowlist");
            lists_mut(ctx).whitelist_clear();
            Ok(players_json(&lists(ctx)))
        }
        "allowlist/remove" => {
            let dtos = list(codec::player_dto)?;
            for u in ctx.resolve_all(&dtos).into_iter().flatten() {
                ctx.log(client, &format!("Remove player '{}' from allowlist", u.name));
                lists_mut(ctx).whitelist_remove(&u);
            }
            ctx.log(client, "Kick unlisted players");
            ctx.on_sim(|h| h.kick_unlisted())?;
            Ok(players_json(&lists(ctx)))
        }
        "allowlist/set" => {
            let dtos = list(codec::player_dto)?;
            let wanted: Vec<NameAndId> = ctx.resolve_all(&dtos).into_iter().flatten().collect();
            let current: Vec<NameAndId> = lists(ctx).whitelist.values().into_iter().cloned().collect();
            // (`Collectors.toSet()` on both sides; removals and additions follow the order of those sets.)
            let wanted = user_set(wanted);
            let current = user_set(current);
            for u in current.iter().filter(|u| !wanted.contains(u)) {
                ctx.log(client, &format!("Remove player '{}' from allowlist", u.name));
                lists_mut(ctx).whitelist_remove(u);
            }
            for u in wanted.iter().filter(|u| !current.contains(u)) {
                ctx.log(client, &format!("Add player '{}' to allowlist", u.name));
                lists_mut(ctx).whitelist_add(u.clone());
            }
            ctx.log(client, "Kick unlisted players");
            ctx.on_sim(|h| h.kick_unlisted())?;
            Ok(players_json(&lists(ctx)))
        }
        // bans
        "bans" => Ok(bans_json(&lists(ctx))),
        "bans/add" => {
            let dtos = codec::list(p(), user_ban_dto).map_err(invalid)?;
            let users = ctx.resolve_all(&dtos.iter().map(|d| d.player.clone()).collect::<Vec<_>>());
            for (dto, user) in dtos.iter().zip(users) {
                let Some(user) = user else { continue };
                add_ban(ctx, client, &user, dto)?;
            }
            Ok(bans_json(&lists(ctx)))
        }
        "bans/clear" => {
            lists_mut(ctx).ban_clear();
            Ok(bans_json(&lists(ctx)))
        }
        "bans/remove" => {
            let dtos = list(codec::player_dto)?;
            for u in ctx.resolve_all(&dtos).into_iter().flatten() {
                ctx.log(client, &format!("Remove player '{}' from banlist", u.name));
                lists_mut(ctx).ban_remove(&u);
            }
            Ok(bans_json(&lists(ctx)))
        }
        "bans/set" => {
            let dtos = codec::list(p(), user_ban_dto).map_err(invalid)?;
            let users = ctx.resolve_all(&dtos.iter().map(|d| d.player.clone()).collect::<Vec<_>>());
            let wanted: Vec<(BanKey, (NameAndId, &UserBanDto))> = users
                .into_iter()
                .zip(&dtos)
                .filter_map(|(u, d)| u.map(|u| (BanKey::user(&u, d.reason.clone(), d.source.as_deref().unwrap_or(SOURCE), d.expires), (u, d))))
                .collect();
            let current: Vec<(BanKey, UserBan)> = lists(ctx).bans.values().into_iter().map(|b| (BanKey::of_user_ban(b), b.clone())).collect();
            let (wanted, current) = (ban_set(wanted), ban_set(current));
            for (_, b) in current.iter().filter(|(k, _)| !wanted.iter().any(|(w, _)| w == k)) {
                ctx.log(client, &format!("Remove player '{}' from banlist", b.user.name));
                lists_mut(ctx).ban_remove(&b.user);
            }
            for (_, (u, d)) in wanted.iter().filter(|(k, _)| !current.iter().any(|(c, _)| c == k)) {
                add_ban(ctx, client, u, d)?;
            }
            Ok(bans_json(&lists(ctx)))
        }
        // ip bans
        "ip_bans" => Ok(ip_bans_json(&lists(ctx))),
        "ip_bans/add" => {
            let dtos = codec::list(p(), incoming_ip_ban_dto).map_err(invalid)?;
            for dto in &dtos {
                let kicked = ban_ip(ctx, client, dto)?;
                for u in kicked {
                    ctx.on_sim(move |h| h.kick(u, &translatable("multiplayer.disconnect.ip_banned"), false))?;
                }
            }
            Ok(ip_bans_json(&lists(ctx)))
        }
        "ip_bans/clear" => {
            ctx.log(client, "Clear ip ban list");
            lists_mut(ctx).ip_ban_clear();
            Ok(ip_bans_json(&lists(ctx)))
        }
        "ip_bans/remove" => {
            let ips = codec::list(p(), codec::string).map_err(invalid)?;
            for ip in ips {
                ctx.log(client, &format!("Remove ip '{ip}' from ban list"));
                lists_mut(ctx).ip_ban_remove(&ip);
            }
            Ok(ip_bans_json(&lists(ctx)))
        }
        "ip_bans/set" => {
            let dtos = codec::list(p(), ip_ban_dto).map_err(invalid)?;
            let wanted: Vec<(BanKey, IpBan)> = dtos
                .iter()
                .filter(|d| d.ip.as_deref().is_some_and(is_inet_address))
                .map(|d| {
                    let ban = ban_info(d.source.as_deref(), d.reason.clone(), d.expires);
                    let ip = d.ip.clone().unwrap_or_default();
                    (BanKey::ip(&ip, d.reason.clone(), d.source.as_deref().unwrap_or(SOURCE), d.expires), IpBan { ip, ban })
                })
                .collect();
            let current: Vec<(BanKey, IpBan)> = lists(ctx).ip_bans.values().into_iter().map(|b| (BanKey::of_ip_ban(b), b.clone())).collect();
            let (wanted, current) = (ban_set(wanted), ban_set(current));
            for (_, b) in current.iter().filter(|(k, _)| !wanted.iter().any(|(w, _)| w == k)) {
                ctx.log(client, &format!("Remove ip '{}' from ban list", b.ip));
                lists_mut(ctx).ip_ban_remove(&b.ip);
            }
            let mut added = Vec::new();
            for (_, b) in wanted.iter().filter(|(k, _)| !current.iter().any(|(c, _)| c == k)) {
                ctx.log(client, &format!("Add ip '{}' to ban list", b.ip));
                lists_mut(ctx).ip_ban_add(b.clone());
                added.push(b.ip.clone());
            }
            for ip in added {
                for u in ctx.players_at(&ip)? {
                    ctx.on_sim(move |h| h.kick(u, &translatable("multiplayer.disconnect.ip_banned"), false))?;
                }
            }
            Ok(ip_bans_json(&lists(ctx)))
        }
        // players
        "players" => Ok(J::Arr(
            ctx.info.players().iter().map(|p| player_json(&NameAndId { uuid: p.uuid, name: p.name.clone() })).collect(),
        )),
        "players/kick" => {
            let dtos = codec::list(p(), kick_dto).map_err(invalid)?;
            let mut kicked = Vec::new();
            for dto in &dtos {
                let Some(uuid) = ctx.online(&dto.player) else { continue };
                let reason = dto
                    .message
                    .as_ref()
                    .and_then(codec::message_text)
                    .unwrap_or_else(|| translatable("multiplayer.disconnect.kicked"));
                let name = ctx.info.players().iter().find(|p| p.uuid == uuid).map(|p| p.name.clone()).unwrap_or_default();
                ctx.log(client, &format!("Remove player '{name}'"));
                ctx.on_sim(move |h| h.kick(uuid, &reason, true))?;
                kicked.push(dto_json(&dto.player));
            }
            Ok(J::Arr(kicked))
        }
        // operators
        "operators" => Ok(ops_json(&lists(ctx))),
        "operators/add" => {
            let dtos = codec::list(p(), op_dto).map_err(invalid)?;
            let users = ctx.resolve_all(&dtos.iter().map(|d| d.player.clone()).collect::<Vec<_>>());
            for (d, u) in dtos.iter().zip(users) {
                let Some(u) = u else { continue };
                op(ctx, client, &u, d.level, d.bypass)?;
            }
            Ok(ops_json(&lists(ctx)))
        }
        "operators/clear" => {
            ctx.log(client, "Clear operator list");
            ctx.on_sim(|h| h.deop_all())?;
            Ok(ops_json(&lists(ctx)))
        }
        "operators/remove" => {
            let dtos = list(codec::player_dto)?;
            for u in ctx.resolve_all(&dtos).into_iter().flatten() {
                ctx.log(client, &format!("Deop '{}'", u.name));
                ctx.on_sim(move |h| h.deop(&u))?;
            }
            Ok(ops_json(&lists(ctx)))
        }
        "operators/set" => {
            let dtos = codec::list(p(), op_dto).map_err(invalid)?;
            let users = ctx.resolve_all(&dtos.iter().map(|d| d.player.clone()).collect::<Vec<_>>());
            let wanted: Vec<(NameAndId, Option<u8>, Option<bool>)> =
                users.into_iter().zip(&dtos).filter_map(|(u, d)| u.map(|u| (u, d.level, d.bypass))).collect();
            let current: Vec<OpEntry> = lists(ctx).op_list.values().into_iter().cloned().collect();
            let key = |u: &NameAndId, l: Option<u8>, b: Option<bool>| (u.uuid, u.name.clone(), l, b);
            let want: HashSet<_> = wanted.iter().map(|(u, l, b)| key(u, *l, *b)).collect();
            let have: HashSet<_> = current.iter().map(|e| key(&e.user, Some(e.level), Some(e.bypasses_player_limit))).collect();
            for e in current.iter().filter(|e| !want.contains(&key(&e.user, Some(e.level), Some(e.bypasses_player_limit)))) {
                ctx.log(client, &format!("Deop '{}'", e.user.name));
                let u = e.user.clone();
                ctx.on_sim(move |h| h.deop(&u))?;
            }
            for (u, l, b) in &wanted {
                if !have.contains(&key(u, *l, *b)) {
                    op(ctx, client, u, *l, *b)?;
                }
            }
            Ok(ops_json(&lists(ctx)))
        }
        // server
        "server/status" => Ok(state_json(&ctx.info)),
        "server/save" => {
            let flush = codec::boolean(p()).map_err(invalid)?;
            ctx.log(client, &format!("Save everything. SuppressLogs: true, flush: {flush}, force: true"));
            Ok(J::Bool(ctx.on_sim(move |h| h.save_everything(flush))?))
        }
        "server/stop" => {
            ctx.log(client, "Halt server. WaitForShutdown: false");
            ctx.on_sim_detached(|h| h.halt());
            Ok(J::Bool(true))
        }
        "server/system_message" => {
            let dto = system_message_dto(p()).map_err(invalid)?;
            let Some(text) = codec::message_text(&dto.message) else { return Ok(J::Bool(false)) };
            match &dto.receiving {
                Some(players) => {
                    if players.is_empty() {
                        return Ok(J::Bool(false));
                    }
                    let ids: Vec<Uuid> = players.iter().filter_map(|d| ctx.online(d)).collect();
                    ctx.on_sim(move |h| h.system_message(&text, dto.overlay, Some(&ids)))?;
                }
                None => {
                    let overlay = dto.overlay;
                    let shown = ctx.on_sim({
                        let text = text.clone();
                        move |h| {
                            let plain = h.plain(&text);
                            h.system_message(&text, overlay, None);
                            plain
                        }
                    })?;
                    ctx.log(client, &format!("Broadcast system message (overlay: {overlay}): '{shown}'"));
                }
            }
            Ok(J::Bool(true))
        }
        // game rules
        "gamerules" => {
            let rules = ctx.on_sim(|h| h.game_rules())?;
            Ok(J::Arr(rules.iter().map(|(k, v, _)| rule_json(k, *v)).collect()))
        }
        "gamerules/update" => {
            let j = p();
            codec::object(j).map_err(invalid)?;
            let key = codec::required(j, "key").and_then(codec::string).and_then(|s| identifier(&s)).map_err(invalid)?;
            let known = {
                let k = key.clone();
                ctx.on_sim(move |h| h.game_rule(&k))?
            };
            let Some((id, old, range)) = known else {
                return Err(invalid(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:game_rule]: {key}")));
            };
            let value = codec::required(j, "value").map_err(invalid)?;
            let new = match old {
                RuleValue::Bool(_) => RuleValue::Bool(codec::boolean(value).map_err(invalid)?),
                RuleValue::Int(_) => {
                    let v = codec::integer(value).map_err(invalid)?;
                    let (lo, hi) = range.unwrap_or((i32::MIN, i32::MAX));
                    if v < lo || v > hi {
                        return Err(invalid(format!("Value {v} outside of range [{lo}:{hi}]")));
                    }
                    RuleValue::Int(v)
                }
            };
            let shown = |v: RuleValue| match v {
                RuleValue::Bool(b) => b.to_string(),
                RuleValue::Int(i) => i.to_string(),
            };
            ctx.log(client, &format!("Game rule '{}' updated from '{}' to '{}'", id.strip_prefix("minecraft:").unwrap_or(&id), shown(old), shown(new)));
            let k = id.clone();
            ctx.on_sim(move |h| h.set_game_rule(&k, new))?;
            Ok(rule_json(&id, new))
        }
        _ => settings(ctx, client, name, param),
    }
}

/// `PlayerList.op` for a user (and the log of `MinecraftOperatorListServiceImpl.op`).
fn op(ctx: &Ctx, client: u32, user: &NameAndId, level: Option<u8>, bypass: Option<bool>) -> Result<(), Fail> {
    ctx.log(client, &format!("Op '{}'", user.name));
    let user = user.clone();
    ctx.on_sim(move |h| h.op(&user, level, bypass))
}

fn ban_info(source: Option<&str>, reason: Option<String>, expires: Option<Instant>) -> BanInfo {
    let mut b = BanInfo::now(Some(source.unwrap_or(SOURCE)), reason);
    b.expires = expires.map(|(secs, _)| kiln_link::access::format_date(secs));
    b.expires_ms = expires.map_or(0, |(_, nanos)| (nanos / 1_000_000) as u16);
    b
}

fn add_ban(ctx: &Ctx, client: u32, user: &NameAndId, dto: &UserBanDto) -> Result<(), Fail> {
    let ban = ban_info(dto.source.as_deref(), dto.reason.clone(), dto.expires);
    let shown = ban.reason.clone().unwrap_or_else(|| "Banned by an operator.".to_owned());
    ctx.log(client, &format!("Add player '{}' to banlist. Reason: '{shown}'", user.name));
    lists_mut(ctx).ban_add(UserBan { user: user.clone(), ban });
    let uuid = user.uuid;
    ctx.on_sim(move |h| h.kick(uuid, &translatable("multiplayer.disconnect.banned"), false))
}

/// `IpBanlistService.banIp`: the address of the request, or of the player it names; the players who
/// have that address (to be kicked).
fn ban_ip(ctx: &Ctx, client: u32, dto: &IpBanDto) -> Result<Vec<Uuid>, Fail> {
    let ip = match dto.ip.as_deref().filter(|ip| is_inet_address(ip)) {
        Some(ip) => ip.to_owned(),
        None => {
            let Some(p) = dto.player.as_ref() else { return Ok(Vec::new()) };
            let Some(uuid) = ctx.online(p) else { return Ok(Vec::new()) };
            match ctx.on_sim(move |h| h.online_players().into_iter().find(|o| o.user.uuid == uuid).and_then(|o| o.address))? {
                Some(a) => a.to_string(),
                None => return Ok(Vec::new()),
            }
        }
    };
    ctx.log(client, &format!("Add ip '{ip}' to ban list"));
    lists_mut(ctx).ip_ban_add(IpBan { ip: ip.clone(), ban: ban_info(dto.source.as_deref(), dto.reason.clone(), dto.expires) });
    ctx.players_at(&ip)
}

impl Ctx {
    /// The players online who connected from `ip`.
    fn players_at(&self, ip: &str) -> Result<Vec<Uuid>, Fail> {
        let ip = ip.to_owned();
        self.on_sim(move |h| h.online_players().into_iter().filter(|o| o.address.is_some_and(|a| a.to_string() == ip)).map(|o| o.user.uuid).collect())
    }
}

/// An entry of a ban list as the `set` methods compare them: a record of the player or address, reason,
/// source and expiry, with the hash Java gives it.
#[derive(PartialEq, Eq, Clone)]
struct BanKey {
    who: (Uuid, String),
    reason: Option<String>,
    source: String,
    expires: Option<Instant>,
    hash: i32,
}

impl BanKey {
    fn user(u: &NameAndId, reason: Option<String>, source: &str, expires: Option<Instant>) -> BanKey {
        let hash = jhash::record_hash(&[
            jhash::name_and_id_hash(&u.uuid, &u.name),
            jhash::string_hash(reason.as_deref()),
            jhash::string_hash(Some(source)),
            jhash::instant_hash(expires),
        ]);
        BanKey { who: (u.uuid, u.name.clone()), reason, source: source.to_owned(), expires, hash }
    }

    fn ip(ip: &str, reason: Option<String>, source: &str, expires: Option<Instant>) -> BanKey {
        let hash = jhash::record_hash(&[
            jhash::string_hash(Some(ip)),
            jhash::string_hash(reason.as_deref()),
            jhash::string_hash(Some(source)),
            jhash::instant_hash(expires),
        ]);
        BanKey { who: (Uuid::nil(), ip.to_owned()), reason, source: source.to_owned(), expires, hash }
    }

    fn expiry(b: &BanInfo) -> Option<Instant> {
        b.expires.as_deref().and_then(kiln_link::access::parse_date).map(|s| (s, u32::from(b.expires_ms) * 1_000_000))
    }

    fn of_user_ban(b: &UserBan) -> BanKey {
        BanKey::user(&b.user, b.ban.reason.clone(), &b.ban.source, BanKey::expiry(&b.ban))
    }

    fn of_ip_ban(b: &IpBan) -> BanKey {
        BanKey::ip(&b.ip, b.ban.reason.clone(), &b.ban.source, BanKey::expiry(&b.ban))
    }
}

/// `Collectors.toSet()` of ban records: the distinct ones, in the order a `HashSet` iterates them.
fn ban_set<T>(items: Vec<(BanKey, T)>) -> Vec<(BanKey, T)> {
    let mut distinct: Vec<(BanKey, T)> = Vec::new();
    for item in items {
        if !distinct.iter().any(|(k, _)| *k == item.0) {
            distinct.push(item);
        }
    }
    jhash::set_order(distinct.into_iter().map(|(k, t)| (k.hash, (k, t))).collect())
}

/// `Collectors.toSet()` of `NameAndId`s.
fn user_set(users: Vec<NameAndId>) -> Vec<NameAndId> {
    let mut distinct: Vec<NameAndId> = Vec::new();
    for u in users {
        if !distinct.contains(&u) {
            distinct.push(u);
        }
    }
    jhash::set_order(distinct.into_iter().map(|u| (jhash::name_and_id_hash(&u.uuid, &u.name), u)).collect())
}

// ---- server settings ----

struct SettingDef {
    method: &'static str,
    setting: Setting,
    /// `{old}`/`{new}` stand for the values.
    log: &'static str,
}

const SETTINGS: &[SettingDef] = &[
    SettingDef { method: "autosave", setting: Setting::Autosave, log: "Update autosave from {old} to {new}" },
    SettingDef { method: "difficulty", setting: Setting::Difficulty, log: "Update difficulty from '{old}' to '{new}'" },
    SettingDef { method: "enforce_allowlist", setting: Setting::EnforceAllowlist, log: "Update enforce allowlist from {old} to {new}" },
    SettingDef { method: "use_allowlist", setting: Setting::UseAllowlist, log: "Update using allowlist from {old} to {new}" },
    SettingDef { method: "max_players", setting: Setting::MaxPlayers, log: "Update max players from {old} to {new}" },
    SettingDef { method: "pause_when_empty_seconds", setting: Setting::PauseWhenEmptySeconds, log: "Update pause when empty from {old} seconds to {new} seconds" },
    SettingDef { method: "player_idle_timeout", setting: Setting::PlayerIdleTimeout, log: "Update player idle timeout from {old} minutes to {new} minutes" },
    SettingDef { method: "allow_flight", setting: Setting::AllowFlight, log: "Update allow flight from {old} to {new}" },
    SettingDef { method: "motd", setting: Setting::Motd, log: "Update MOTD from '{old}' to '{new}'" },
    SettingDef { method: "spawn_protection_radius", setting: Setting::SpawnProtection, log: "Update spawn protection radius from {old} to {new}" },
    SettingDef { method: "force_game_mode", setting: Setting::ForceGameMode, log: "Update force game mode from {old} to {new}" },
    SettingDef { method: "game_mode", setting: Setting::GameMode, log: "Update game mode from '{old}' to '{new}'" },
    SettingDef { method: "view_distance", setting: Setting::ViewDistance, log: "Update view distance from {old} to {new}" },
    SettingDef { method: "simulation_distance", setting: Setting::SimulationDistance, log: "Update simulation distance from {old} to {new}" },
    SettingDef { method: "accept_transfers", setting: Setting::AcceptTransfers, log: "Update accepts transfers from {old} to {new}" },
    SettingDef { method: "operator_user_permission_level", setting: Setting::OperatorPermissionLevel, log: "Update operator user permission level from {old} to {new}" },
    SettingDef { method: "hide_online_players", setting: Setting::HideOnlinePlayers, log: "Update hides online players from {old} to {new}" },
    SettingDef { method: "status_replies", setting: Setting::StatusReplies, log: "Update replies to status from {old} to {new}" },
    SettingDef { method: "entity_broadcast_range", setting: Setting::EntityBroadcastRange, log: "Update entity broadcast range percentage from {old}% to {new}%" },
];

/// The value of a setting as the server properties hold it.
fn prop_get(p: &crate::properties::ServerProperties, s: Setting) -> SettingValue {
    match s {
        Setting::MaxPlayers => SettingValue::Int(p.max_players),
        Setting::PauseWhenEmptySeconds => SettingValue::Int(p.pause_when_empty_seconds),
        Setting::PlayerIdleTimeout => SettingValue::Int(p.player_idle_timeout),
        Setting::AllowFlight => SettingValue::Bool(p.allow_flight),
        Setting::Motd => SettingValue::Str(p.motd.clone()),
        Setting::SpawnProtection => SettingValue::Int(p.spawn_protection),
        Setting::ForceGameMode => SettingValue::Bool(p.force_gamemode),
        Setting::GameMode => SettingValue::Int(i32::from(p.gamemode)),
        Setting::ViewDistance => SettingValue::Int(p.view_distance),
        Setting::SimulationDistance => SettingValue::Int(p.simulation_distance),
        Setting::AcceptTransfers => SettingValue::Bool(p.accepts_transfers),
        Setting::OperatorPermissionLevel => SettingValue::Int(i32::from(p.op_permission_level)),
        Setting::HideOnlinePlayers => SettingValue::Bool(p.hide_online_players),
        Setting::StatusReplies => SettingValue::Bool(p.enable_status),
        Setting::EntityBroadcastRange => SettingValue::Int(p.entity_broadcast_range_percentage),
        Setting::Difficulty => SettingValue::Int(i32::from(p.difficulty)),
        Setting::UseAllowlist => SettingValue::Bool(p.white_list),
        Setting::EnforceAllowlist => SettingValue::Bool(p.enforce_whitelist),
        Setting::Autosave => SettingValue::Bool(true),
    }
}

/// `Settings.MutableValue.update`: the property takes the value its parser would make of it (the
/// entity broadcast range is clamped to 10..=1000, the operator level to 0..=4); the properties file follows.
fn prop_set(p: &mut crate::properties::ServerProperties, s: Setting, v: &SettingValue) -> SettingValue {
    let (b, i) = match v {
        SettingValue::Bool(b) => (*b, 0),
        SettingValue::Int(i) => (false, *i),
        SettingValue::Str(_) => (false, 0),
    };
    let (key, text, mapped) = match s {
        Setting::MaxPlayers => {
            p.max_players = i;
            ("max-players", i.to_string(), SettingValue::Int(i))
        }
        Setting::PauseWhenEmptySeconds => {
            p.pause_when_empty_seconds = i;
            ("pause-when-empty-seconds", i.to_string(), SettingValue::Int(i))
        }
        Setting::PlayerIdleTimeout => {
            p.player_idle_timeout = i;
            ("player-idle-timeout", i.to_string(), SettingValue::Int(i))
        }
        Setting::AllowFlight => {
            p.allow_flight = b;
            ("allow-flight", b.to_string(), SettingValue::Bool(b))
        }
        Setting::Motd => {
            let m = if let SettingValue::Str(m) = v { m.clone() } else { String::new() };
            p.motd = m.clone();
            ("motd", m.clone(), SettingValue::Str(m))
        }
        Setting::SpawnProtection => {
            p.spawn_protection = i;
            ("spawn-protection", i.to_string(), SettingValue::Int(i))
        }
        Setting::ForceGameMode => {
            p.force_gamemode = b;
            ("force-gamemode", b.to_string(), SettingValue::Bool(b))
        }
        Setting::GameMode => {
            p.gamemode = i.clamp(0, 3) as u8;
            ("gamemode", GAME_TYPES[usize::from(p.gamemode)].to_owned(), SettingValue::Int(i))
        }
        Setting::ViewDistance => {
            p.view_distance = i;
            ("view-distance", i.to_string(), SettingValue::Int(i))
        }
        Setting::SimulationDistance => {
            p.simulation_distance = i;
            ("simulation-distance", i.to_string(), SettingValue::Int(i))
        }
        Setting::AcceptTransfers => {
            p.accepts_transfers = b;
            ("accepts-transfers", b.to_string(), SettingValue::Bool(b))
        }
        Setting::OperatorPermissionLevel => {
            p.op_permission_level = i.clamp(0, 4) as u8;
            ("op-permission-level", p.op_permission_level.to_string(), SettingValue::Int(i32::from(p.op_permission_level)))
        }
        Setting::HideOnlinePlayers => {
            p.hide_online_players = b;
            ("hide-online-players", b.to_string(), SettingValue::Bool(b))
        }
        Setting::StatusReplies => {
            p.enable_status = b;
            ("enable-status", b.to_string(), SettingValue::Bool(b))
        }
        Setting::EntityBroadcastRange => {
            p.entity_broadcast_range_percentage = i.clamp(10, 1000);
            ("entity-broadcast-range-percentage", p.entity_broadcast_range_percentage.to_string(), SettingValue::Int(p.entity_broadcast_range_percentage))
        }
        Setting::Difficulty => {
            p.difficulty = i.clamp(0, 3) as u8;
            ("difficulty", DIFFICULTIES[usize::from(p.difficulty)].to_owned(), SettingValue::Int(i))
        }
        Setting::UseAllowlist => {
            p.white_list = b;
            ("white-list", b.to_string(), SettingValue::Bool(b))
        }
        Setting::EnforceAllowlist => {
            p.enforce_whitelist = b;
            ("enforce-whitelist", b.to_string(), SettingValue::Bool(b))
        }
        Setting::Autosave => return v.clone(),
    };
    p.set_raw(key, text);
    mapped
}

fn setting_json(s: Setting, v: &SettingValue) -> J {
    match (s, v) {
        (Setting::Difficulty, SettingValue::Int(i)) => J::str(DIFFICULTIES[(*i).clamp(0, 3) as usize]),
        (Setting::GameMode, SettingValue::Int(i)) => J::str(GAME_TYPES[(*i).clamp(0, 3) as usize]),
        (_, SettingValue::Bool(b)) => J::Bool(*b),
        (_, SettingValue::Int(i)) => J::int(i64::from(*i)),
        (_, SettingValue::Str(s)) => J::str(s.clone()),
    }
}

/// The value as the log shows it (`Difficulty.toString` is the constant's name, the permission level
/// a `LevelBasedPermissionSet`).
fn setting_log(s: Setting, v: &SettingValue, is_old: bool) -> String {
    match (s, v) {
        (Setting::Difficulty, SettingValue::Int(i)) => DIFFICULTIES[(*i).clamp(0, 3) as usize].to_ascii_uppercase(),
        (Setting::GameMode, SettingValue::Int(i)) => GAME_TYPES[(*i).clamp(0, 3) as usize].to_ascii_uppercase(),
        (Setting::OperatorPermissionLevel, SettingValue::Int(i)) => {
            let name = permission_level_name((*i).clamp(0, 4) as u8);
            if is_old { format!("permission level: {name}") } else { name.to_owned() }
        }
        (_, SettingValue::Bool(b)) => b.to_string(),
        (_, SettingValue::Int(i)) => i.to_string(),
        (_, SettingValue::Str(s)) => s.clone(),
    }
}

/// `minecraft:serversettings/...` and its `/set`.
fn settings(ctx: &Ctx, client: u32, name: &str, param: Option<&J>) -> Result<J, Fail> {
    let rest = name.strip_prefix("serversettings/").ok_or_else(|| Fail::MethodNotFound(name.to_owned()))?;
    let (method, set) = match rest.strip_suffix("/set") {
        Some(m) => (m, true),
        None => (rest, false),
    };
    if method == "status_heartbeat_interval" {
        return heartbeat_setting(ctx, client, set, param);
    }
    let def = SETTINGS.iter().find(|d| d.method == method).ok_or_else(|| Fail::MethodNotFound(name.to_owned()))?;
    let live = matches!(def.setting, Setting::Autosave | Setting::Difficulty | Setting::EnforceAllowlist | Setting::UseAllowlist);
    let current = |ctx: &Ctx| -> Result<SettingValue, Fail> {
        if live {
            let s = def.setting;
            ctx.on_sim(move |h| h.setting(s))
        } else {
            Ok(prop_get(&ctx.settings.props.lock().unwrap_or_else(std::sync::PoisonError::into_inner), def.setting))
        }
    };
    if !set {
        return Ok(setting_json(def.setting, &current(ctx)?));
    }
    let p = param.expect("a setter has a parameter");
    let requested = match def.setting {
        Setting::Autosave
        | Setting::EnforceAllowlist
        | Setting::UseAllowlist
        | Setting::AllowFlight
        | Setting::ForceGameMode
        | Setting::AcceptTransfers
        | Setting::HideOnlinePlayers
        | Setting::StatusReplies => SettingValue::Bool(codec::boolean(p).map_err(invalid)?),
        Setting::Motd => SettingValue::Str(codec::string(p).map_err(invalid)?),
        Setting::Difficulty => SettingValue::Int(enum_index(p, &DIFFICULTIES).map_err(invalid)? as i32),
        Setting::GameMode => SettingValue::Int(enum_index(p, &GAME_TYPES).map_err(invalid)? as i32),
        // (`PermissionLevel.INT_CODEC` clamps.)
        Setting::OperatorPermissionLevel => SettingValue::Int(codec::integer(p).map_err(invalid)?.clamp(0, 4)),
        _ => SettingValue::Int(codec::integer(p).map_err(invalid)?),
    };
    let old = current(ctx)?;
    let text = def.log.replace("{old}", &setting_log(def.setting, &old, true)).replace("{new}", &setting_log(def.setting, &requested, false));
    ctx.log(client, &text);
    let mapped = {
        let mut props = ctx.settings.props.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mapped = prop_set(&mut props, def.setting, &requested);
        ctx.settings.save(&props);
        mapped
    };
    let s = def.setting;
    let kicks = matches!(s, Setting::EnforceAllowlist | Setting::UseAllowlist);
    let after = ctx.on_sim(move |h| {
        h.set_setting(s, mapped);
        if kicks {
            h.kick_unlisted();
        }
        h.setting(s)
    })?;
    // (A live setting answers with what the game says; the others with the property.)
    let shown = if live { after } else { current(ctx)? };
    Ok(setting_json(def.setting, &shown))
}

fn heartbeat_setting(ctx: &Ctx, client: u32, set: bool, param: Option<&J>) -> Result<J, Fail> {
    let get = |ctx: &Ctx| ctx.settings.props.lock().unwrap_or_else(std::sync::PoisonError::into_inner).status_heartbeat_interval;
    if set {
        let seconds = codec::integer(param.expect("a setter has a parameter")).map_err(invalid)?;
        let old = get(ctx);
        if ctx.heartbeat.schedule(i64::from(seconds)) {
            {
                let mut props = ctx.settings.props.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                props.status_heartbeat_interval = seconds;
                props.set_raw("status-heartbeat-interval", seconds.to_string());
                ctx.settings.save(&props);
            }
            // (Vanilla logs the old value after the new one was stored.)
            let _ = old;
            ctx.log(client, &format!("Updated status heartbeat interval from {seconds} to {seconds}"));
        } else {
            ctx.log(
                client,
                &format!("The heartbeat interval was not updated to the new value of {seconds} seconds as the json rpc server is null or a pre-existing heartbeat failed to get canceled"),
            );
        }
    }
    Ok(J::int(i64::from(get(ctx))))
}
