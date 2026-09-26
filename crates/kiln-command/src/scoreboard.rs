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
    /// Holder -> objective -> score.
    scores: BTreeMap<String, BTreeMap<String, Score>>,
    /// Display slot name -> objective name.
    display: BTreeMap<String, String>,
}

impl Scoreboard {
    pub fn objectives(&self) -> &[Objective] {
        &self.objectives
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
        self.objectives.push(objective);
        true
    }

    /// Removes an objective with its scores and display slots.
    pub fn remove_objective(&mut self, name: &str) {
        self.objectives.retain(|o| o.name != name);
        for scores in self.scores.values_mut() {
            scores.remove(name);
        }
        self.scores.retain(|_, s| !s.is_empty());
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

    /// `getOrCreatePlayerScore(...).set(value)`; `trigger` scores start locked.
    pub fn set_score(&mut self, holder: &str, objective: &str, value: i32) {
        let locked = self.objective(objective).is_some_and(|o| o.criterion == "trigger");
        let entry = self.scores.entry(holder.to_owned()).or_default();
        entry.entry(objective.to_owned()).or_insert(Score { value: 0, locked }).value = value;
    }

    pub fn set_locked(&mut self, holder: &str, objective: &str, locked: bool) {
        let entry = self.scores.entry(holder.to_owned()).or_default();
        entry.entry(objective.to_owned()).or_insert(Score { value: 0, locked }).locked = locked;
    }

    /// `resetSinglePlayerScore` / `resetAllPlayerScores`.
    pub fn reset(&mut self, holder: &str, objective: Option<&str>) {
        match objective {
            Some(o) => {
                if let Some(s) = self.scores.get_mut(holder) {
                    s.remove(o);
                    if s.is_empty() {
                        self.scores.remove(holder);
                    }
                }
            }
            None => {
                self.scores.remove(holder);
            }
        }
    }

    /// `getTrackedPlayers`: holders with at least one score.
    pub fn holders(&self) -> Vec<String> {
        self.scores.keys().cloned().collect()
    }

    /// `listPlayerScores`: a holder's scores by objective name.
    pub fn scores_of(&self, holder: &str) -> Vec<(&str, i32)> {
        self.scores.get(holder).map_or_else(Vec::new, |s| s.iter().map(|(o, v)| (o.as_str(), v.value)).collect())
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
        assert_eq!(sb.holders(), ["Alice", "Bob"]);
        sb.set_display("sidebar", Some("kills"));
        sb.remove_objective("kills");
        assert_eq!(sb.score("Alice", "kills"), None);
        assert_eq!(sb.display("sidebar"), None);
        assert_eq!(sb.holders(), ["Bob"]);
        assert!(objective("h", "health").is_read_only());
    }
}
