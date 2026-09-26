//! A minimal server scoreboard (`Scoreboard`): objectives and integer scores per holder name.
//! Hosts own one and expose it through [`SelectorWorld::scoreboard`](crate::SelectorWorld)
//! and [`Host::scoreboard_mut`](crate::Host); sending scoreboard packets to clients is the
//! host's concern.

use crate::text::Text;
use std::collections::BTreeMap;

/// An objective: name, criterion (`dummy`, `trigger`, `health`, ...) and display settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    pub name: String,
    pub criterion: String,
    pub display_name: Text,
    /// `integer` or `hearts`.
    pub render_type: &'static str,
    pub display_auto_update: bool,
}

impl Objective {
    /// `ObjectiveCriteria.isReadOnly`: criteria the game maintains itself.
    pub fn is_read_only(&self) -> bool {
        matches!(self.criterion.as_str(), "health" | "food" | "air" | "armor" | "xp" | "level")
    }

    /// `Objective.getFormattedDisplayName`: the display name in brackets, hovering the name.
    pub fn formatted_display_name(&self) -> Text {
        self.display_name.clone().hover(Text::literal(&self.name)).bracketed()
    }
}

/// One score and whether `trigger` may change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    pub value: i32,
    pub locked: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scoreboard {
    objectives: Vec<Objective>,
    objective_order: OpenHashKeys,
    /// Holder -> objective -> score. Holders stay tracked (possibly without scores) when
    /// their objectives are removed, as in vanilla.
    scores: BTreeMap<String, BTreeMap<String, Score>>,
    holder_order: OpenHashKeys,
    /// Display slot name -> objective name.
    display: BTreeMap<String, String>,
}

impl Scoreboard {
    /// `getObjectives()` in the order vanilla's map iterates them.
    pub fn objectives(&self) -> Vec<&Objective> {
        self.objective_order.descending().filter_map(|name| self.objective(name)).collect()
    }

    pub fn objective(&self, name: &str) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.name == name)
    }

    pub fn objective_mut(&mut self, name: &str) -> Option<&mut Objective> {
        self.objectives.iter_mut().find(|o| o.name == name)
    }

    /// Adds an objective; returns false if the name is taken.
    pub fn add_objective(&mut self, objective: Objective) -> bool {
        if self.objective(&objective.name).is_some() {
            return false;
        }
        self.objective_order.insert(&objective.name);
        self.objectives.push(objective);
        true
    }

    /// Removes an objective with its scores and display slots.
    pub fn remove_objective(&mut self, name: &str) {
        self.objectives.retain(|o| o.name != name);
        self.objective_order.remove(name);
        for scores in self.scores.values_mut() {
            scores.remove(name);
        }
        self.display.retain(|_, o| o != name);
    }

    pub fn set_display(&mut self, slot: &str, objective: Option<&str>) {
        match objective {
            Some(o) => self.display.insert(slot.to_owned(), o.to_owned()),
            None => self.display.remove(slot),
        };
    }

    pub fn display(&self, slot: &str) -> Option<&str> {
        self.display.get(slot).map(String::as_str)
    }

    /// `getPlayerScoreInfo`: `None` when the holder has no score for the objective.
    pub fn score(&self, holder: &str, objective: &str) -> Option<i32> {
        self.score_info(holder, objective).map(|s| s.value)
    }

    pub fn score_info(&self, holder: &str, objective: &str) -> Option<Score> {
        self.scores.get(holder)?.get(objective).copied()
    }

    /// `getOrCreatePlayerScore`: new scores are 0 and locked (`new Score()`).
    pub fn score_mut(&mut self, holder: &str, objective: &str) -> &mut Score {
        if !self.scores.contains_key(holder) {
            self.holder_order.insert(holder);
        }
        let entry = self.scores.entry(holder.to_owned()).or_default();
        entry.entry(objective.to_owned()).or_insert(Score { value: 0, locked: true })
    }

    /// `getOrCreatePlayerScore(...).set(value)`.
    pub fn set_score(&mut self, holder: &str, objective: &str, value: i32) {
        self.score_mut(holder, objective).value = value;
    }

    pub fn set_locked(&mut self, holder: &str, objective: &str, locked: bool) {
        self.score_mut(holder, objective).locked = locked;
    }

    /// `resetSinglePlayerScore` (untracking holders left without scores) and
    /// `resetAllPlayerScores`.
    pub fn reset(&mut self, holder: &str, objective: Option<&str>) {
        let Some(scores) = self.scores.get_mut(holder) else { return };
        if let Some(o) = objective {
            scores.remove(o);
            if !scores.is_empty() {
                return;
            }
        }
        self.scores.remove(holder);
        self.holder_order.remove(holder);
    }

    /// `getTrackedPlayers()`, in the order vanilla streams them.
    pub fn holders(&self) -> Vec<String> {
        self.holder_order.ascending().map(str::to_owned).collect()
    }

    /// `listPlayerScores`: a holder's scores by objective name.
    pub fn scores_of(&self, holder: &str) -> Vec<(&str, i32)> {
        self.scores.get(holder).map_or_else(Vec::new, |s| s.iter().map(|(o, v)| (o.as_str(), v.value)).collect())
    }
}

/// The slot layout of fastutil's `Object2ObjectOpenHashMap` over string keys, built as the
/// vanilla scoreboard builds its maps (16 expected entries, load factor 0.5), so listings
/// come out in vanilla's order: streams walk the slots upwards, iterators downwards.
#[derive(Debug, Clone, PartialEq)]
struct OpenHashKeys {
    slots: Vec<Option<String>>,
    size: usize,
}

/// `HashCommon.arraySize(16, 0.5f)`.
const MIN_SLOTS: usize = 32;

impl Default for OpenHashKeys {
    fn default() -> Self {
        Self { slots: vec![None; MIN_SLOTS], size: 0 }
    }
}

impl OpenHashKeys {
    fn mask(&self) -> usize {
        self.slots.len() - 1
    }

    /// `HashCommon.maxFill(n, 0.5f)`.
    fn max_fill(&self) -> usize {
        self.slots.len() / 2
    }

    /// `HashCommon.mix(key.hashCode()) & mask`.
    fn home(key: &str, mask: usize) -> usize {
        let h = key.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)));
        let h = (h as u32).wrapping_mul(0x9E37_79B9);
        (h ^ (h >> 16)) as usize & mask
    }

    fn insert(&mut self, key: &str) {
        let mask = self.mask();
        let mut pos = Self::home(key, mask);
        while let Some(k) = &self.slots[pos] {
            if k == key {
                return;
            }
            pos = (pos + 1) & mask;
        }
        self.slots[pos] = Some(key.to_owned());
        self.size += 1;
        // `if (size++ >= maxFill)`: the size before this insertion.
        if self.size > self.max_fill() {
            // `arraySize(size + 1, 0.5f)`
            self.rehash(((self.size + 1) * 2).next_power_of_two());
        }
    }

    fn remove(&mut self, key: &str) {
        let mask = self.mask();
        let mut pos = Self::home(key, mask);
        loop {
            match &self.slots[pos] {
                None => return,
                Some(k) if k == key => break,
                Some(_) => pos = (pos + 1) & mask,
            }
        }
        self.size -= 1;
        self.shift_keys(pos);
        let n = self.slots.len();
        if n > MIN_SLOTS && self.size < self.max_fill() / 4 {
            self.rehash(n / 2);
        }
    }

    /// `shiftKeys`: closes the gap at `pos` (backward-shift deletion).
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.mask();
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(curr) = &self.slots[pos] else {
                    self.slots[last] = None;
                    return;
                };
                let slot = Self::home(curr, mask);
                let stays = if last <= pos { last >= slot || slot > pos } else { last >= slot && slot > pos };
                if stays {
                    break;
                }
                pos = (pos + 1) & mask;
            }
            self.slots[last] = self.slots[pos].take();
        }
    }

    /// `rehash`: reinserts from the highest slot down.
    fn rehash(&mut self, n: usize) {
        let mask = n - 1;
        let mut slots = vec![None; n];
        for key in self.slots.iter().rev().flatten() {
            let mut pos = Self::home(key, mask);
            while slots[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            slots[pos] = Some(key.clone());
        }
        self.slots = slots;
    }

    fn ascending(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().flatten().map(String::as_str)
    }

    fn descending(&self) -> impl Iterator<Item = &str> {
        self.slots.iter().rev().flatten().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objective(name: &str, criterion: &str) -> Objective {
        Objective {
            name: name.into(),
            criterion: criterion.into(),
            display_name: Text::literal(name),
            render_type: "integer",
            display_auto_update: false,
        }
    }

    #[test]
    fn scores() {
        let mut sb = Scoreboard::default();
        assert!(sb.add_objective(objective("kills", "dummy")));
        assert!(!sb.add_objective(objective("kills", "dummy")));
        sb.set_score("Alice", "kills", 3);
        assert_eq!(sb.score("Alice", "kills"), Some(3));
        assert_eq!(sb.score("Bob", "kills"), None);
        sb.add_objective(objective("t", "trigger"));
        sb.set_score("Bob", "t", 0);
        assert_eq!(sb.score_info("Bob", "t"), Some(Score { value: 0, locked: true }));
        sb.set_display("sidebar", Some("kills"));
        sb.remove_objective("kills");
        assert_eq!(sb.score("Alice", "kills"), None);
        assert_eq!(sb.display("sidebar"), None);
        // Holders stay tracked without scores until a reset finds them empty.
        assert_eq!(sb.holders().len(), 2);
        sb.reset("Alice", Some("kills"));
        assert_eq!(sb.holders(), ["Bob"]);
        assert!(objective("h", "health").is_read_only());
    }

    #[test]
    fn holders_come_out_in_vanilla_order() {
        // Seen on a vanilla 26.3 server after the same sequence of score changes.
        let mut sb = Scoreboard::default();
        sb.add_objective(objective("k", "dummy"));
        for h in ["Diff0", "#fake", "#zero"] {
            sb.set_score(h, "k", 0);
        }
        sb.reset("#zero", Some("k"));
        sb.reset("#fake", None);
        for h in ["#count", "#s", "#r", "#t", "#u"] {
            sb.set_score(h, "k", 0);
        }
        assert_eq!(sb.holders(), ["#r", "#t", "#count", "#s", "Diff0", "#u"]);
    }

    #[test]
    fn open_hash_keys_grow_and_shrink() {
        let mut keys = OpenHashKeys::default();
        let names: Vec<String> = (0..40).map(|i| format!("h{i}")).collect();
        for n in &names {
            keys.insert(n);
        }
        assert_eq!(keys.slots.len(), 128);
        assert_eq!(keys.ascending().count(), 40);
        for n in &names[..38] {
            keys.remove(n);
        }
        assert_eq!(keys.slots.len(), 32);
        let mut left: Vec<&str> = keys.ascending().collect();
        left.sort_unstable();
        assert_eq!(left, ["h38", "h39"]);
    }
}
