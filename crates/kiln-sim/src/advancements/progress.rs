//! A player's advancement progress (`PlayerAdvancements`): when each criterion was obtained,
//! which advancements the client sees (`AdvancementVisibilityEvaluator`), what changed since
//! the last Update Advancements, and `players/advancements/<uuid>.json`.

use super::Advancements;
use bytes::{Bytes, BytesMut};
use kiln_proto::WriteExt;
use std::collections::BTreeSet;
use std::sync::Arc;

/// `AdvancementProgress`: when each criterion was obtained (epoch milliseconds).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Progress {
    pub obtained: Vec<Option<i64>>,
}

impl Progress {
    fn new(n: usize) -> Progress {
        Progress { obtained: vec![None; n] }
    }

    /// `isDone`: every requirement group has an obtained criterion.
    pub fn is_done(&self, requirements: &[Vec<usize>]) -> bool {
        !requirements.is_empty() && requirements.iter().all(|g| g.iter().any(|&c| self.obtained.get(c).is_some_and(Option::is_some)))
    }

    /// `hasProgress`.
    pub fn has_progress(&self) -> bool {
        self.obtained.iter().any(Option::is_some)
    }
}

/// Rewards and announcements of advancements completed in region work, for the next serial
/// phase.
#[derive(Debug, Default)]
pub(crate) struct Completed {
    pub advancements: Vec<usize>,
}

pub(crate) struct PlayerAdvancements {
    pub data: Arc<Advancements>,
    /// By advancement index (`getOrStartProgress` for every advancement).
    pub progress: Vec<Progress>,
    visible: Vec<bool>,
    progress_changed: BTreeSet<usize>,
    roots_to_update: BTreeSet<usize>,
    first_packet: bool,
    /// Completed since the last serial upkeep (rewards, announcements).
    pub completed: Completed,
    /// The tab the client has open (`lastSelectedTab`).
    last_tab: Option<usize>,
}

impl PlayerAdvancements {
    pub fn new(data: Arc<Advancements>) -> PlayerAdvancements {
        let n = data.len();
        let progress = data.list.iter().map(|a| Progress::new(a.criteria.len())).collect();
        PlayerAdvancements {
            data,
            progress,
            visible: vec![false; n],
            progress_changed: BTreeSet::new(),
            roots_to_update: BTreeSet::new(),
            first_packet: true,
            completed: Completed::default(),
            last_tab: None,
        }
    }

    pub fn is_done(&self, i: usize) -> bool {
        self.progress[i].is_done(&self.data.list[i].requirements)
    }

    /// Whether criterion `c` of advancement `i` is obtained.
    pub fn criterion_done(&self, i: usize, c: usize) -> bool {
        self.progress[i].obtained.get(c).is_some_and(Option::is_some)
    }

    /// Whether a listener for criterion `c` of `i` is registered (`registerListeners`: the
    /// advancement is not done and neither is the criterion).
    pub fn listening(&self, i: usize, c: usize) -> bool {
        !self.criterion_done(i, c) && !self.is_done(i)
    }

    /// `award`: returns whether the criterion was newly obtained. A completed advancement is
    /// queued for its rewards and announcement.
    pub fn award(&mut self, i: usize, c: usize, now: i64) -> bool {
        let was_done = self.is_done(i);
        let Some(slot) = self.progress[i].obtained.get_mut(c) else { return false };
        if slot.is_some() {
            return false;
        }
        *slot = Some(now);
        self.progress_changed.insert(i);
        if !was_done && self.is_done(i) {
            self.completed.advancements.push(i);
            self.mark_for_visibility_update(i);
        }
        true
    }

    /// `revoke`.
    pub fn revoke(&mut self, i: usize, c: usize) -> bool {
        let was_done = self.is_done(i);
        let Some(slot) = self.progress[i].obtained.get_mut(c) else { return false };
        if slot.is_none() {
            return false;
        }
        *slot = None;
        self.progress_changed.insert(i);
        if was_done && !self.is_done(i) {
            self.mark_for_visibility_update(i);
        }
        true
    }

    fn mark_for_visibility_update(&mut self, i: usize) {
        self.roots_to_update.insert(self.data.root(i));
    }

    /// `setSelectedTab` (`ServerboundSeenAdvancementsPacket`): a root with a display becomes
    /// the selected tab; Select Advancements Tab tells the client when it changed.
    pub fn select_tab(&mut self, tab: Option<&str>) -> Option<Bytes> {
        let old = self.last_tab;
        self.last_tab = tab.and_then(|t| self.data.get(t)).filter(|&i| self.data.parent[i].is_none() && self.data.list[i].display.is_some());
        (old != self.last_tab).then(|| select_advancements_tab(self.last_tab.map(|i| self.data.list[i].id.as_str())))
    }

    /// `AdvancementVisibilityEvaluator.evaluateVisibility` from `root`.
    fn update_tree_visibility(&mut self, root: usize, added: &mut Vec<usize>, removed: &mut Vec<usize>) {
        #[derive(Clone, Copy, PartialEq)]
        enum Rule {
            Show,
            Hide,
            NoChange,
        }
        fn eval(pa: &mut PlayerAdvancements, n: usize, stack: &mut Vec<Rule>, added: &mut Vec<usize>, removed: &mut Vec<usize>) -> bool {
            let done = pa.is_done(n);
            let a = &pa.data.list[n];
            let rule = match &a.display {
                None => Rule::Hide,
                Some(_) if done => Rule::Show,
                Some(d) if d.hidden => Rule::Hide,
                Some(_) => Rule::NoChange,
            };
            let mut visible = done;
            stack.push(rule);
            for c in pa.data.children[n].clone() {
                visible |= eval(pa, c, stack, added, removed);
            }
            // `evaluateVisiblityForUnfinishedNode`: the nearest decisive rule within 2 levels.
            if !visible {
                for k in 0..=2 {
                    match stack[stack.len() - 1 - k] {
                        Rule::Show => {
                            visible = true;
                            break;
                        }
                        Rule::Hide => break,
                        Rule::NoChange => {}
                    }
                }
            }
            stack.pop();
            if visible {
                if !pa.visible[n] {
                    pa.visible[n] = true;
                    added.push(n);
                    pa.progress_changed.insert(n);
                }
            } else if pa.visible[n] {
                pa.visible[n] = false;
                removed.push(n);
            }
            visible
        }
        let mut stack = vec![Rule::NoChange; 3];
        eval(self, root, &mut stack, added, removed);
    }

    /// `flushDirty`: Update Advancements with the newly visible, hidden and changed
    /// advancements, if any (the first packet resets the client's tree).
    pub fn flush(&mut self, show_advancements: bool) -> Option<Bytes> {
        let mut packet = None;
        if self.first_packet || !self.roots_to_update.is_empty() || !self.progress_changed.is_empty() {
            let (mut added, mut removed) = (Vec::new(), Vec::new());
            for root in std::mem::take(&mut self.roots_to_update) {
                self.update_tree_visibility(root, &mut added, &mut removed);
            }
            let changed: Vec<usize> = std::mem::take(&mut self.progress_changed).into_iter().filter(|&i| self.visible[i]).collect();
            if !changed.is_empty() || !added.is_empty() || !removed.is_empty() {
                packet = Some(self.update_packet(&added, &removed, &changed, show_advancements));
            }
        }
        self.first_packet = false;
        packet
    }

    fn update_packet(&self, added: &[usize], removed: &[usize], changed: &[usize], show: bool) -> Bytes {
        let mut b = BytesMut::with_capacity(256 + added.len() * 128);
        b.put_varint(kiln_data::packets::play::clientbound::UPDATE_ADVANCEMENTS);
        bytes::BufMut::put_u8(&mut b, self.first_packet as u8);
        b.put_varint(added.len() as i32);
        for &i in added {
            b.extend_from_slice(&self.data.encoded[i]);
        }
        b.put_varint(removed.len() as i32);
        for &i in removed {
            b.put_string(&self.data.list[i].id);
        }
        b.put_varint(changed.len() as i32);
        for &i in changed {
            let a = &self.data.list[i];
            b.put_string(&a.id);
            // `AdvancementProgress.STREAM_CODEC`: every criterion of the requirements.
            b.put_varint(a.criteria.len() as i32);
            for (c, (name, _)) in a.criteria.iter().enumerate() {
                b.put_string(name);
                match self.progress[i].obtained[c] {
                    Some(t) => {
                        bytes::BufMut::put_u8(&mut b, 1);
                        bytes::BufMut::put_i64(&mut b, t);
                    }
                    None => bytes::BufMut::put_u8(&mut b, 0),
                }
            }
        }
        bytes::BufMut::put_u8(&mut b, show as u8);
        b.freeze()
    }

    /// `PlayerAdvancements.Data` as JSON (Gson's pretty printing), with the data version.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        for (i, p) in self.progress.iter().enumerate() {
            if !p.has_progress() {
                continue;
            }
            let a = &self.data.list[i];
            out.push_str(&format!("  {}: {{\n    \"criteria\": {{\n", quote(&a.id)));
            let done: Vec<String> = a
                .criteria
                .iter()
                .zip(&p.obtained)
                .filter_map(|((name, _), t)| t.map(|t| format!("      {}: {}", quote(name), quote(&format_time(t)))))
                .collect();
            out.push_str(&done.join(",\n"));
            out.push_str(&format!("\n    }},\n    \"done\": {}\n  }},\n", p.is_done(&a.requirements)));
        }
        out.push_str(&format!("  \"DataVersion\": {}\n}}", kiln_storage::anvil::DATA_VERSION));
        out
    }

    /// Applies saved progress (`applyFrom`): unknown advancements and criteria are ignored.
    pub fn load_json(&mut self, text: &str) -> Result<(), String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let Some(map) = root.as_object() else { return Err("not an object".into()) };
        // `Data.forEach`: in order of first progress.
        let mut entries: Vec<(usize, Progress)> = Vec::new();
        for (id, v) in map {
            if id == "DataVersion" {
                continue;
            }
            let key = if id.contains(':') { id.clone() } else { format!("minecraft:{id}") };
            let Some(i) = self.data.get(&key) else {
                tracing::warn!("Ignored advancement '{id}' in progress file - it doesn't exist anymore?");
                continue;
            };
            let a = &self.data.list[i];
            let mut p = Progress::new(a.criteria.len());
            if let Some(criteria) = v.get("criteria").and_then(|c| c.as_object()) {
                for (name, t) in criteria {
                    let (Some(c), Some(t)) = (a.criterion_index(name), t.as_str().and_then(parse_time)) else { continue };
                    p.obtained[c] = Some(t);
                }
            }
            entries.push((i, p));
        }
        entries.sort_by_key(|(_, p)| p.obtained.iter().flatten().min().copied().unwrap_or(i64::MAX));
        for (i, p) in entries {
            self.progress[i] = p;
            self.progress_changed.insert(i);
            self.mark_for_visibility_update(i);
        }
        Ok(())
    }
}

/// `ClientboundSelectAdvancementsTabPacket`.
pub(crate) fn select_advancements_tab(tab: Option<&str>) -> Bytes {
    let mut b = BytesMut::with_capacity(48);
    b.put_varint(kiln_data::packets::play::clientbound::SELECT_ADVANCEMENTS_TAB);
    match tab {
        Some(t) => {
            bytes::BufMut::put_u8(&mut b, 1);
            b.put_string(t);
        }
        None => bytes::BufMut::put_u8(&mut b, 0),
    }
    b.freeze()
}

fn quote(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `AdvancementProgress.OBTAINED_TIME_FORMAT` (`yyyy-MM-dd HH:mm:ss Z`), in UTC.
pub(crate) fn format_time(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000", t / 3600, t / 60 % 60, t % 60)
}

/// Parses `yyyy-MM-dd HH:mm:ss Z` (any offset) to epoch milliseconds.
pub(crate) fn parse_time(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once(' ')?;
    let (time, zone) = rest.split_once(' ')?;
    let mut dp = date.splitn(3, '-');
    let (y, m, d): (i64, u32, u32) = (dp.next()?.parse().ok()?, dp.next()?.parse().ok()?, dp.next()?.parse().ok()?);
    let mut tp = time.splitn(3, ':');
    let (hh, mm, ss): (i64, i64, i64) = (tp.next()?.parse().ok()?, tp.next()?.parse().ok()?, tp.next()?.parse().ok()?);
    let (sign, digits) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => return None,
    };
    if digits.len() != 4 {
        return None;
    }
    let off = sign * (digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60);
    Some(((days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss) - off) * 1000)
}

/// Milliseconds since the epoch now (`Instant.now()` for obtained times).
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obtained_time_format() {
        assert_eq!(format_time(0), "1970-01-01 00:00:00 +0000");
        let t = 1_790_000_000_000;
        let s = format_time(t);
        assert_eq!(parse_time(&s), Some(t));
        // Vanilla writes the server's zone.
        assert_eq!(parse_time("2026-09-28 20:00:00 +0800"), parse_time("2026-09-28 12:00:00 +0000"));
        assert_eq!(parse_time("2024-02-29 23:59:59 -0130"), parse_time("2024-03-01 01:29:59 +0000"));
    }
}
