//! Who may join: the whitelist, player bans and IP bans (vanilla `UserWhiteList`,
//! `UserBanList`, `IpBanList`, kept in `whitelist.json`, `banned-players.json` and
//! `banned-ips.json`). The network layer checks them at login (`PlayerList.canPlayerLogin`);
//! the simulation's commands change them. Both share one [`SharedAccess`].
//!
//! Vanilla keeps each list in a `HashMap` keyed by the UUID or IP string and lists entries
//! in the map's iteration order; [`JavaHashOrder`] reproduces that order so `/banlist` and
//! `/whitelist list` print entries as vanilla does.

use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub type SharedAccess = Arc<RwLock<AccessLists>>;

/// `BanListEntry.DATE_FORMAT` pattern `yyyy-MM-dd HH:mm:ss Z`, in UTC.
pub fn format_date(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let secs = unix_seconds.rem_euclid(86_400);
    // Civil-from-days (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} +0000", secs / 3600, secs / 60 % 60, secs % 60)
}

fn now_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    format_date(secs)
}

/// `String.hashCode`.
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// A `HashMap<String, V>`'s iteration order: by bucket (`(h ^ h >>> 16) & (capacity - 1)`),
/// then by insertion within a bucket. The table starts at 16 buckets, doubles past a load
/// of 0.75 and never shrinks.
#[derive(Debug, Clone)]
pub struct JavaHashOrder<V> {
    entries: Vec<(String, V)>,
    capacity: usize,
}

impl<V> Default for JavaHashOrder<V> {
    fn default() -> Self {
        Self { entries: Vec::new(), capacity: 0 }
    }
}

impl<V> JavaHashOrder<V> {
    fn bucket(&self, key: &str) -> usize {
        let h = java_string_hash(key) as u32;
        ((h ^ (h >> 16)) as usize) & (self.capacity - 1)
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// `put`: replaces the value in place, or appends.
    pub fn put(&mut self, key: String, value: V) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
            return;
        }
        if self.capacity == 0 {
            self.capacity = 16;
        }
        self.entries.push((key, value));
        if self.entries.len() > self.capacity * 3 / 4 {
            self.capacity *= 2;
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(i).1)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `values()` in iteration order.
    pub fn values(&self) -> Vec<&V> {
        let mut order: Vec<(usize, usize)> =
            self.entries.iter().enumerate().map(|(i, (k, _))| (self.bucket(k), i)).collect();
        order.sort_unstable();
        order.into_iter().map(|(_, i)| &self.entries[i].1).collect()
    }
}

/// `BanListEntry`: when, by whom and why; `expires` is `None` for "forever".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanInfo {
    pub created: String,
    pub source: String,
    pub expires: Option<String>,
    /// `None`: "Banned by an operator." (`multiplayer.disconnect.banned.reason.default`).
    pub reason: Option<String>,
}

impl BanInfo {
    /// A ban made now (`new BanListEntry(user, null, source, null, reason)`).
    pub fn now(source: Option<&str>, reason: Option<String>) -> Self {
        Self { created: now_date(), source: source.unwrap_or("(Unknown)").to_owned(), expires: None, reason }
    }

    fn write(&self, o: &mut Map<String, Value>) {
        o.insert("created".into(), self.created.clone().into());
        o.insert("source".into(), self.source.clone().into());
        o.insert("expires".into(), self.expires.clone().unwrap_or_else(|| "forever".into()).into());
        if let Some(r) = &self.reason {
            o.insert("reason".into(), r.clone().into());
        }
    }

    fn read(o: &Map<String, Value>) -> Self {
        let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
        Self {
            created: s("created").unwrap_or_else(now_date),
            source: s("source").unwrap_or_else(|| "(Unknown)".into()),
            expires: s("expires").filter(|e| e != "forever"),
            reason: s("reason"),
        }
    }
}

/// A player (`NameAndId`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameAndId {
    pub uuid: Uuid,
    pub name: String,
}

/// An operator (`ServerOpListEntry`): `ops.json` lists the player with a permission level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpEntry {
    pub user: NameAndId,
    /// 1..=4 (`op-permission-level`, 4 for operators made with `/op`).
    pub level: u8,
    pub bypasses_player_limit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBan {
    pub user: NameAndId,
    pub ban: BanInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpBan {
    pub ip: String,
    pub ban: BanInfo,
}

/// Why a login is refused (`canPlayerLogin`), for the disconnect message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// `multiplayer.disconnect.banned.reason` (+ `.expiration`).
    Banned(BanInfo),
    /// `multiplayer.disconnect.not_whitelisted`.
    NotWhitelisted,
    /// `multiplayer.disconnect.banned_ip.reason` (+ `.expiration`).
    IpBanned(BanInfo),
}

impl Refusal {
    /// The disconnect reason as a JSON text component.
    pub fn to_json(&self) -> Value {
        let reason = |b: &BanInfo| match &b.reason {
            Some(r) => serde_json::json!({ "text": r }),
            None => serde_json::json!({ "translate": "multiplayer.disconnect.banned.reason.default" }),
        };
        let with_expiry = |key: &str, exp_key: &str, b: &BanInfo| {
            let mut c = serde_json::json!({ "translate": key, "with": [reason(b)] });
            if let Some(e) = &b.expires {
                c["extra"] = serde_json::json!([{ "translate": exp_key, "with": [e] }]);
            }
            c
        };
        match self {
            Refusal::Banned(b) => {
                with_expiry("multiplayer.disconnect.banned.reason", "multiplayer.disconnect.banned.expiration", b)
            }
            Refusal::NotWhitelisted => serde_json::json!({ "translate": "multiplayer.disconnect.not_whitelisted" }),
            Refusal::IpBanned(b) => {
                with_expiry("multiplayer.disconnect.banned_ip.reason", "multiplayer.disconnect.banned_ip.expiration", b)
            }
        }
    }
}

/// The three lists, the whitelist switches and the operators (who bypass the whitelist).
#[derive(Debug, Default)]
pub struct AccessLists {
    /// Where the JSON files live (the server directory); `None` keeps them in memory.
    dir: Option<PathBuf>,
    pub whitelist: JavaHashOrder<NameAndId>,
    pub bans: JavaHashOrder<UserBan>,
    pub ip_bans: JavaHashOrder<IpBan>,
    /// `ops.json`: the operators with their permission levels (the names are in `ops` too).
    pub op_list: JavaHashOrder<OpEntry>,
    /// `white-list`.
    pub use_whitelist: bool,
    /// `enforce-whitelist`: turning the whitelist on or changing it kicks unlisted players.
    pub enforce_whitelist: bool,
    /// Operator names, mirrored by the simulation (`prefix*` matches a prefix).
    pub ops: HashSet<String>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Parses `yyyy-MM-dd HH:mm:ss Z` into Unix seconds (for `hasExpired`).
fn parse_date(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once(' ')?;
    let (time, zone) = rest.split_once(' ')?;
    let d: Vec<i64> = date.split('-').map(str::parse).collect::<Result<_, _>>().ok()?;
    let t: Vec<i64> = time.split(':').map(str::parse).collect::<Result<_, _>>().ok()?;
    if d.len() != 3 || t.len() != 3 || zone.len() != 5 {
        return None;
    }
    let (y, m, day) = (d[0] - i64::from(d[1] <= 2), d[1], d[2]);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let offset = sign * (zone[1..3].parse::<i64>().ok()? * 3600 + zone[3..5].parse::<i64>().ok()? * 60);
    Some(days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2] - offset)
}

fn expired(b: &BanInfo) -> bool {
    b.expires.as_deref().and_then(parse_date).is_some_and(|e| e < now_secs())
}

impl AccessLists {
    /// Lists stored in `dir` (loaded now if the files exist), or in memory.
    pub fn new(dir: Option<PathBuf>) -> Self {
        let mut lists = Self { dir, ..Self::default() };
        lists.load();
        lists
    }

    pub fn shared(self) -> SharedAccess {
        Arc::new(RwLock::new(self))
    }

    fn read_file(&self, name: &str) -> Vec<Map<String, Value>> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(text) = std::fs::read_to_string(dir.join(name)) else { return Vec::new() };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => a.into_iter().filter_map(|v| if let Value::Object(o) = v { Some(o) } else { None }).collect(),
            _ => Vec::new(),
        }
    }

    fn write_file(&self, name: &str, entries: Vec<Map<String, Value>>) {
        let Some(dir) = &self.dir else { return };
        let json = Value::Array(entries.into_iter().map(Value::Object).collect());
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        if let Err(e) = std::fs::write(dir.join(name), text) {
            eprintln!("could not save {name}: {e}");
        }
    }

    fn user(o: &Map<String, Value>) -> Option<NameAndId> {
        let uuid = o.get("uuid")?.as_str()?.parse().ok()?;
        let name = o.get("name")?.as_str()?.to_owned();
        Some(NameAndId { uuid, name })
    }

    fn write_user(u: &NameAndId, o: &mut Map<String, Value>) {
        o.insert("uuid".into(), u.uuid.to_string().into());
        o.insert("name".into(), u.name.clone().into());
    }

    fn load(&mut self) {
        for o in self.read_file("whitelist.json") {
            if let Some(u) = Self::user(&o) {
                self.whitelist.put(u.uuid.to_string(), u);
            }
        }
        for o in self.read_file("ops.json") {
            if let Some(user) = Self::user(&o) {
                let level = o.get("level").and_then(Value::as_i64).map_or(4, |l| l.clamp(0, 4) as u8);
                let bypasses_player_limit = o.get("bypassesPlayerLimit").and_then(Value::as_bool).unwrap_or(false);
                self.op_list.put(user.uuid.to_string(), OpEntry { user, level, bypasses_player_limit });
            }
        }
        for o in self.read_file("banned-players.json") {
            if let Some(u) = Self::user(&o) {
                self.bans.put(u.uuid.to_string(), UserBan { user: u, ban: BanInfo::read(&o) });
            }
        }
        for o in self.read_file("banned-ips.json") {
            if let Some(ip) = o.get("ip").and_then(Value::as_str) {
                self.ip_bans.put(ip.to_owned(), IpBan { ip: ip.to_owned(), ban: BanInfo::read(&o) });
            }
        }
    }

    pub fn save_whitelist(&self) {
        let entries = self
            .whitelist
            .values()
            .into_iter()
            .map(|u| {
                let mut o = Map::new();
                Self::write_user(u, &mut o);
                o
            })
            .collect();
        self.write_file("whitelist.json", entries);
    }

    /// Writes `ops.json` (vanilla's `ServerOpList`: uuid, name, level, bypassesPlayerLimit).
    pub fn save_ops(&self) {
        let entries = self
            .op_list
            .values()
            .into_iter()
            .map(|e| {
                let mut o = Map::new();
                Self::write_user(&e.user, &mut o);
                o.insert("level".into(), e.level.into());
                o.insert("bypassesPlayerLimit".into(), e.bypasses_player_limit.into());
                o
            })
            .collect();
        self.write_file("ops.json", entries);
    }

    pub fn save_bans(&self) {
        let entries = self
            .bans
            .values()
            .into_iter()
            .map(|b| {
                let mut o = Map::new();
                Self::write_user(&b.user, &mut o);
                b.ban.write(&mut o);
                o
            })
            .collect();
        self.write_file("banned-players.json", entries);
    }

    pub fn save_ip_bans(&self) {
        let entries = self
            .ip_bans
            .values()
            .into_iter()
            .map(|b| {
                let mut o = Map::new();
                o.insert("ip".into(), b.ip.clone().into());
                b.ban.write(&mut o);
                o
            })
            .collect();
        self.write_file("banned-ips.json", entries);
    }

    pub fn is_op(&self, name: &str) -> bool {
        self.ops.contains(name) || self.ops.iter().any(|o| o.strip_suffix('*').is_some_and(|p| name.starts_with(p)))
    }

    /// `PlayerList.isWhiteListed`.
    pub fn is_whitelisted(&self, user: &NameAndId) -> bool {
        !self.use_whitelist || self.is_op(&user.name) || self.whitelist.contains(&user.uuid.to_string())
    }

    /// `UserBanList.isBanned` (expired bans are dropped first, as `get` does).
    pub fn is_banned(&mut self, uuid: &Uuid) -> bool {
        self.drop_expired();
        self.bans.contains(&uuid.to_string())
    }

    pub fn is_ip_banned(&mut self, ip: &str) -> bool {
        self.drop_expired();
        self.ip_bans.contains(ip)
    }

    fn drop_expired(&mut self) {
        let users: Vec<String> =
            self.bans.values().into_iter().filter(|b| expired(&b.ban)).map(|b| b.user.uuid.to_string()).collect();
        for k in &users {
            self.bans.remove(k);
        }
        let ips: Vec<String> = self.ip_bans.values().into_iter().filter(|b| expired(&b.ban)).map(|b| b.ip.clone()).collect();
        for k in &ips {
            self.ip_bans.remove(k);
        }
    }

    /// `PlayerList.canPlayerLogin` without the capacity check (the network layer does that).
    pub fn can_login(&mut self, user: &NameAndId, ip: Option<&str>) -> Result<(), Refusal> {
        self.drop_expired();
        if let Some(b) = self.bans.get(&user.uuid.to_string()) {
            return Err(Refusal::Banned(b.ban.clone()));
        }
        if !self.is_whitelisted(user) {
            return Err(Refusal::NotWhitelisted);
        }
        if let Some(b) = ip.and_then(|ip| self.ip_bans.get(ip)) {
            return Err(Refusal::IpBanned(b.ban.clone()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hash_matches_string_hash_code() {
        assert_eq!(java_string_hash(""), 0);
        assert_eq!(java_string_hash("hello"), 99_162_322);
        assert_eq!(java_string_hash("10.0.0.1"), 511552166);
    }

    #[test]
    fn hash_order_by_bucket_then_insertion() {
        let mut m = JavaHashOrder::default();
        for k in ["b", "a", "c"] {
            m.put(k.to_owned(), k);
        }
        // "a" = 97, "b" = 98, "c" = 99: buckets 1, 2, 3 of 16.
        assert_eq!(m.values(), vec![&"a", &"b", &"c"]);
        // 20 keys grow the table to 32 buckets.
        for i in 0..20 {
            m.put(format!("k{i}"), "x");
        }
        assert_eq!(m.capacity, 32);
    }

    #[test]
    fn ops_json_round_trip() {
        let dir = std::env::temp_dir().join(format!("kiln-access-ops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ops.json"),
            r#"[{"uuid":"00000000-0000-0000-0000-000000000007","name":"Alex","level":2,"bypassesPlayerLimit":true}]"#,
        )
        .unwrap();
        let mut a = AccessLists::new(Some(dir.clone()));
        let alex = a.op_list.get("00000000-0000-0000-0000-000000000007").unwrap().clone();
        assert_eq!((alex.level, alex.bypasses_player_limit), (2, true));
        let steve = NameAndId { uuid: Uuid::from_u128(8), name: "Steve".into() };
        a.op_list.put(steve.uuid.to_string(), OpEntry { user: steve, level: 4, bypasses_player_limit: false });
        a.save_ops();
        let back = AccessLists::new(Some(dir.clone()));
        assert_eq!(back.op_list.len(), 2);
        assert_eq!(back.op_list.get("00000000-0000-0000-0000-000000000008").map(|e| e.level), Some(4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(format_date(0), "1970-01-01 00:00:00 +0000");
        assert_eq!(format_date(1_790_000_000), "2026-09-21 14:13:20 +0000");
        assert_eq!(parse_date("2026-09-21 14:13:20 +0000"), Some(1_790_000_000));
        assert_eq!(parse_date("2026-09-21 22:13:20 +0800"), Some(1_790_000_000));
    }

    #[test]
    fn login_checks_in_vanilla_order() {
        let mut a = AccessLists::new(None);
        let user = NameAndId { uuid: Uuid::from_u128(7), name: "Alex".into() };
        assert_eq!(a.can_login(&user, Some("1.2.3.4")), Ok(()));
        a.use_whitelist = true;
        assert_eq!(a.can_login(&user, Some("1.2.3.4")), Err(Refusal::NotWhitelisted));
        a.ops.insert("Alex".into());
        assert_eq!(a.can_login(&user, Some("1.2.3.4")), Ok(()));
        let ban = BanInfo::now(Some("Server"), None);
        a.ip_bans.put("1.2.3.4".into(), IpBan { ip: "1.2.3.4".into(), ban: ban.clone() });
        assert_eq!(a.can_login(&user, Some("1.2.3.4")), Err(Refusal::IpBanned(ban.clone())));
        a.bans.put(user.uuid.to_string(), UserBan { user: user.clone(), ban: ban.clone() });
        assert_eq!(a.can_login(&user, Some("1.2.3.4")), Err(Refusal::Banned(ban)));
        let json = Refusal::NotWhitelisted.to_json().to_string();
        assert_eq!(json, r#"{"translate":"multiplayer.disconnect.not_whitelisted"}"#);
    }
}
