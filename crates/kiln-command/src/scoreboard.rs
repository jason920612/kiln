//! The server scoreboard (`ServerScoreboard`): objectives, scores, display slots and teams.
//! Hosts own one and expose it through [`SelectorWorld::scoreboard`](crate::SelectorWorld)
//! and [`Host::scoreboard_mut`](crate::Host). Changes queue the packets vanilla broadcasts
//! for them ([`take_packets`](Scoreboard::take_packets)); [`join_packets`](Scoreboard::join_packets)
//! is the state a joining player gets (`PlayerList.updateEntireScoreboard`), and
//! [`to_nbt`](Scoreboard::to_nbt) / [`load_nbt`](Scoreboard::load_nbt) are the `scoreboard`
//! saved data.

use crate::arguments::TEAM_COLORS;
use crate::text::{Content, Text};
use bytes::Bytes;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::scoreboard as sp;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub use sp::TeamRule;

/// How scores are drawn (`NumberFormat`).
#[derive(Debug, Clone, PartialEq)]
pub enum NumberFormat {
    Blank,
    /// The number in a style (a compound of style fields).
    Styled(Tag),
    /// This component (network NBT) instead of the number.
    Fixed(Tag),
}

impl NumberFormat {
    fn wire(&self) -> sp::NumberFormat<'_> {
        match self {
            NumberFormat::Blank => sp::NumberFormat::Blank,
            NumberFormat::Styled(s) => sp::NumberFormat::Styled(s),
            NumberFormat::Fixed(t) => sp::NumberFormat::Fixed(t),
        }
    }

    /// `NumberFormatTypes.CODEC`: dispatched on `type`.
    pub fn to_nbt(&self) -> Tag {
        let ty = |t: &str| ("type".to_owned(), Tag::String(t.to_owned()));
        Tag::Compound(match self {
            NumberFormat::Blank => vec![ty("blank")],
            NumberFormat::Styled(s) => vec![ty("styled"), ("style".into(), s.clone())],
            NumberFormat::Fixed(t) => vec![ty("fixed"), ("value".into(), t.clone())],
        })
    }

    pub fn from_nbt(tag: &Tag) -> Option<Self> {
        match tag.get("type")?.as_str()? {
            "blank" => Some(NumberFormat::Blank),
            "styled" => Some(NumberFormat::Styled(tag.get("style").cloned().unwrap_or(Tag::Compound(Vec::new())))),
            "fixed" => Some(NumberFormat::Fixed(tag.get("value")?.clone())),
            _ => None,
        }
    }
}

/// An objective: name, criterion (`dummy`, `trigger`, `health`, ...) and display settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    pub name: String,
    pub criterion: String,
    pub display_name: Text,
    /// `integer` or `hearts`.
    pub render_type: &'static str,
    pub display_auto_update: bool,
    pub number_format: Option<NumberFormat>,
}

impl Objective {
    /// A new objective with the criterion's default render type.
    pub fn new(name: &str, criterion: &str, display_name: Text) -> Self {
        Objective {
            name: name.to_owned(),
            criterion: criterion.to_owned(),
            display_name,
            render_type: if criterion == "health" { "hearts" } else { "integer" },
            display_auto_update: false,
            number_format: None,
        }
    }

    /// `ObjectiveCriteria.isReadOnly`: criteria the game maintains itself.
    pub fn is_read_only(&self) -> bool {
        matches!(self.criterion.as_str(), "health" | "food" | "air" | "armor" | "xp" | "level")
    }

    /// `Objective.getFormattedDisplayName`: the display name in brackets, hovering the name.
    pub fn formatted_display_name(&self) -> Text {
        with_style(&self.display_name, Some(Text::literal(&self.name)), None).bracketed()
    }
}

/// `display_name.copy().withStyle(...)`: the hover text and insertion override the
/// component's own. Components given as NBT get the fields written into their compound.
pub(crate) fn with_style(text: &Text, hover: Option<Text>, insertion: Option<&str>) -> Text {
    match &text.content {
        Content::Raw(tag) => {
            let mut fields = match tag {
                Tag::Compound(f) => f.clone(),
                Tag::String(s) => vec![("text".to_owned(), Tag::String(s.clone()))],
                other => return Text::raw(other.clone()),
            };
            let mut put = |k: &str, v: Tag| match fields.iter_mut().find(|(n, _)| n == k) {
                Some((_, old)) => *old = v,
                None => fields.push((k.to_owned(), v)),
            };
            if let Some(h) = hover {
                put(
                    "hover_event",
                    Tag::Compound(vec![
                        ("action".into(), Tag::String("show_text".into())),
                        ("value".into(), h.to_nbt()),
                    ]),
                );
            }
            if let Some(i) = insertion {
                put("insertion", Tag::String(i.to_owned()));
            }
            Text::raw(Tag::Compound(fields))
        }
        _ => {
            let mut t = text.clone();
            if let Some(h) = hover {
                t = t.hover(h);
            }
            if let Some(i) = insertion {
                t = t.insertion(i);
            }
            t
        }
    }
}

/// One score: its value, whether `trigger` may change it, and per-score display settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub value: i32,
    pub locked: bool,
    /// Replaces the holder's name in the sidebar.
    pub display: Option<Text>,
    pub number_format: Option<NumberFormat>,
}

impl Default for Score {
    /// `new Score()`: 0 and locked.
    fn default() -> Self {
        Score { value: 0, locked: true, display: None, number_format: None }
    }
}

/// `Scoreboard.getOrCreatePlayerScore`: a handle on one score. The first change through a
/// handle that created its score is always sent, later ones only when something changed.
#[derive(Debug, Clone)]
pub struct ScoreAccess {
    holder: String,
    objective: String,
    requires_sync: bool,
    /// `ScoreHolder.getDisplayName` (entities only), for `displayautoupdate` objectives.
    holder_display: Option<Text>,
}

/// A team (`PlayerTeam`).
#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub name: String,
    pub display_name: Text,
    pub prefix: Text,
    pub suffix: Text,
    pub friendly_fire: bool,
    pub see_friendly_invisibles: bool,
    pub name_tag_visibility: TeamRule,
    pub death_message_visibility: TeamRule,
    pub collision_rule: TeamRule,
    /// Index into [`TEAM_COLORS`].
    pub color: Option<usize>,
    players: JavaHashSet,
}

impl Team {
    fn new(name: &str) -> Self {
        Team {
            name: name.to_owned(),
            display_name: Text::literal(name),
            prefix: Text::literal(""),
            suffix: Text::literal(""),
            friendly_fire: true,
            see_friendly_invisibles: true,
            name_tag_visibility: TeamRule::Always,
            death_message_visibility: TeamRule::Always,
            collision_rule: TeamRule::Always,
            color: None,
            players: JavaHashSet::default(),
        }
    }

    /// `getPlayers()` in `HashSet` order.
    pub fn players(&self) -> Vec<String> {
        self.players.iter().map(str::to_owned).collect()
    }

    fn color_name(&self) -> Option<&'static str> {
        self.color.map(|c| TEAM_COLORS[c])
    }

    /// `getFormattedDisplayName`: `[display name]` (hovering and inserting the team name) in
    /// the team color.
    pub fn formatted_display_name(&self) -> Text {
        let t = with_style(&self.display_name, Some(Text::literal(&self.name)), Some(&self.name)).bracketed();
        match self.color_name() {
            Some(c) => t.color(c),
            None => t,
        }
    }

    /// `getFormattedName`: prefix, name and suffix in the team color.
    pub fn format_name(&self, name: Text) -> Text {
        let t = Text::empty().append(self.prefix.clone()).append(name).append(self.suffix.clone());
        match self.color_name() {
            Some(c) => t.color(c),
            None => t,
        }
    }

    fn params(&self) -> (Tag, Tag, Tag) {
        (self.display_name.to_nbt(), self.prefix.to_nbt(), self.suffix.to_nbt())
    }

    fn packet(&self, method: u8) -> Bytes {
        let (display, prefix, suffix) = self.params();
        let params = sp::TeamParameters {
            display_name: &display,
            prefix: &prefix,
            suffix: &suffix,
            name_tag_visibility: self.name_tag_visibility,
            collision_rule: self.collision_rule,
            color: self.color.map(|c| WIRE_COLORS[c]),
            friendly_fire: self.friendly_fire,
            see_friendly_invisibles: self.see_friendly_invisibles,
        };
        let players = self.players();
        let names: Vec<&str> = players.iter().map(String::as_str).collect();
        let method = if method == 0 { sp::TeamMethod::Add(params, &names) } else { sp::TeamMethod::Change(params) };
        sp::set_player_team(&self.name, &method)
    }
}

const WIRE_COLORS: [sp::TeamColor; 16] = [
    sp::TeamColor::Black,
    sp::TeamColor::DarkBlue,
    sp::TeamColor::DarkGreen,
    sp::TeamColor::DarkAqua,
    sp::TeamColor::DarkRed,
    sp::TeamColor::DarkPurple,
    sp::TeamColor::Gold,
    sp::TeamColor::Gray,
    sp::TeamColor::DarkGray,
    sp::TeamColor::Blue,
    sp::TeamColor::Green,
    sp::TeamColor::Aqua,
    sp::TeamColor::Red,
    sp::TeamColor::LightPurple,
    sp::TeamColor::Yellow,
    sp::TeamColor::White,
];

/// `Team.Visibility` names.
pub const VISIBILITY_NAMES: [&str; 4] = ["always", "never", "hideForOtherTeams", "hideForOwnTeam"];
/// `Team.CollisionRule` names.
pub const COLLISION_NAMES: [&str; 4] = ["always", "never", "pushOtherTeams", "pushOwnTeam"];

fn rule_index(rule: TeamRule) -> usize {
    rule as usize
}

fn rule_by_index(i: usize) -> TeamRule {
    [TeamRule::Always, TeamRule::Never, TeamRule::OtherTeams, TeamRule::OwnTeam][i]
}

fn rule_by_name(names: &[&str; 4], name: &str) -> Option<TeamRule> {
    names.iter().position(|n| *n == name).map(rule_by_index)
}

/// `DisplaySlot` names by id.
pub const DISPLAY_SLOTS: [&str; 19] = [
    "list",
    "sidebar",
    "below_name",
    "sidebar.team.black",
    "sidebar.team.dark_blue",
    "sidebar.team.dark_green",
    "sidebar.team.dark_aqua",
    "sidebar.team.dark_red",
    "sidebar.team.dark_purple",
    "sidebar.team.gold",
    "sidebar.team.gray",
    "sidebar.team.dark_gray",
    "sidebar.team.blue",
    "sidebar.team.green",
    "sidebar.team.aqua",
    "sidebar.team.red",
    "sidebar.team.light_purple",
    "sidebar.team.yellow",
    "sidebar.team.white",
];

fn slot_id(name: &str) -> Option<usize> {
    DISPLAY_SLOTS.iter().position(|s| *s == name)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scoreboard {
    objectives: Vec<Objective>,
    objective_order: OpenHashKeys,
    /// Holder -> objective -> score. Holders stay tracked (possibly without scores) when
    /// their objectives are removed, as in vanilla.
    scores: BTreeMap<String, BTreeMap<String, Score>>,
    holder_order: OpenHashKeys,
    /// Objective name per display slot id.
    display: [Option<String>; 19],
    teams: Vec<Team>,
    team_order: OpenHashKeys,
    team_of: HashMap<String, String>,
    /// Objectives clients know about (`trackedObjectives`): the displayed ones.
    tracked: BTreeSet<String>,
    packets: Vec<Bytes>,
    dirty: bool,
}

impl Default for Scoreboard {
    fn default() -> Self {
        Scoreboard {
            objectives: Vec::new(),
            objective_order: OpenHashKeys::default(),
            scores: BTreeMap::new(),
            holder_order: OpenHashKeys::default(),
            display: Default::default(),
            teams: Vec::new(),
            // `new Object2ObjectOpenHashMap<>()`: load factor 0.75.
            team_order: OpenHashKeys::with_load(3),
            team_of: HashMap::new(),
            tracked: BTreeSet::new(),
            packets: Vec::new(),
            dirty: false,
        }
    }
}

impl Scoreboard {
    /// Packets for every player since the last call, in order.
    pub fn take_packets(&mut self) -> Vec<Bytes> {
        std::mem::take(&mut self.packets)
    }

    /// Whether the saved data changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn broadcast(&mut self, pkt: Bytes) {
        self.packets.push(pkt);
    }

    /// `getObjectives()` in the order vanilla's map iterates them.
    pub fn objectives(&self) -> Vec<&Objective> {
        self.objective_order.descending().filter_map(|name| self.objective(name)).collect()
    }

    pub fn objective(&self, name: &str) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.name == name)
    }

    /// Changes go to clients once [`changed_objective`](Self::changed_objective) is called.
    pub fn objective_mut(&mut self, name: &str) -> Option<&mut Objective> {
        self.objectives.iter_mut().find(|o| o.name == name)
    }

    /// `onObjectiveChanged`: updates clients that track the objective.
    pub fn changed_objective(&mut self, name: &str) {
        if self.tracked.contains(name)
            && let Some(o) = self.objective(name)
        {
            let pkt = objective_packet(o, 2);
            self.broadcast(pkt);
        }
        self.dirty = true;
    }

    /// Adds an objective; returns false if the name is taken.
    pub fn add_objective(&mut self, objective: Objective) -> bool {
        if self.objective(&objective.name).is_some() {
            return false;
        }
        self.objective_order.insert(&objective.name);
        self.objectives.push(objective);
        self.dirty = true;
        true
    }

    /// Removes an objective with its scores and display slots.
    pub fn remove_objective(&mut self, name: &str) {
        self.objective_order.remove(name);
        for slot in 0..DISPLAY_SLOTS.len() {
            if self.display[slot].as_deref() == Some(name) {
                self.set_display_id(slot, None);
            }
        }
        for scores in self.scores.values_mut() {
            scores.remove(name);
        }
        if self.tracked.contains(name) {
            self.stop_tracking(name);
        }
        self.objectives.retain(|o| o.name != name);
        self.dirty = true;
    }

    pub fn set_display(&mut self, slot: &str, objective: Option<&str>) {
        if let Some(id) = slot_id(slot) {
            self.set_display_id(id, objective);
        }
    }

    /// `ServerScoreboard.setDisplayObjective`.
    fn set_display_id(&mut self, slot: usize, objective: Option<&str>) {
        let old = std::mem::replace(&mut self.display[slot], objective.map(str::to_owned));
        if let Some(old) = old.filter(|o| Some(o.as_str()) != objective) {
            if self.slot_count(&old) > 0 {
                self.broadcast(sp::set_display_objective(slot as i32, objective.unwrap_or("")));
            } else {
                self.stop_tracking(&old);
            }
        }
        if let Some(o) = objective {
            if self.tracked.contains(o) {
                self.broadcast(sp::set_display_objective(slot as i32, o));
            } else {
                self.start_tracking(o);
            }
        }
        self.dirty = true;
    }

    fn slot_count(&self, objective: &str) -> usize {
        self.display.iter().filter(|d| d.as_deref() == Some(objective)).count()
    }

    pub fn display(&self, slot: &str) -> Option<&str> {
        self.display[slot_id(slot)?].as_deref()
    }

    /// `getStartTrackingPackets`: the objective, its display slots and its scores.
    fn start_packets(&self, name: &str) -> Vec<Bytes> {
        let Some(o) = self.objective(name) else { return Vec::new() };
        let mut out = vec![objective_packet(o, 0)];
        for (slot, d) in self.display.iter().enumerate() {
            if d.as_deref() == Some(name) {
                out.push(sp::set_display_objective(slot as i32, name));
            }
        }
        for holder in self.holder_order.descending() {
            if let Some(score) = self.scores.get(holder).and_then(|s| s.get(name)) {
                out.push(score_packet(holder, name, score));
            }
        }
        out
    }

    fn start_tracking(&mut self, name: &str) {
        let packets = self.start_packets(name);
        self.packets.extend(packets);
        self.tracked.insert(name.to_owned());
    }

    /// `getStopTrackingPackets`: removes the objective (and so its slots) from clients.
    fn stop_tracking(&mut self, name: &str) {
        self.broadcast(sp::set_objective(name, &sp::ObjectiveMethod::Remove));
        for slot in 0..DISPLAY_SLOTS.len() {
            if self.display[slot].as_deref() == Some(name) {
                self.broadcast(sp::set_display_objective(slot as i32, name));
            }
        }
        self.tracked.remove(name);
    }

    /// `PlayerList.updateEntireScoreboard`: the teams with their members, then every
    /// displayed objective.
    pub fn join_packets(&self) -> Vec<Bytes> {
        let mut out: Vec<Bytes> = self.teams().into_iter().map(|t| t.packet(0)).collect();
        let mut sent: Vec<&str> = Vec::new();
        for d in self.display.iter().flatten() {
            if !sent.contains(&d.as_str()) {
                out.extend(self.start_packets(d));
                sent.push(d);
            }
        }
        out
    }

    /// `getPlayerScoreInfo`: `None` when the holder has no score for the objective.
    pub fn score(&self, holder: &str, objective: &str) -> Option<i32> {
        self.score_info(holder, objective).map(|s| s.value)
    }

    pub fn score_info(&self, holder: &str, objective: &str) -> Option<&Score> {
        self.scores.get(holder)?.get(objective)
    }

    /// `getOrCreatePlayerScore(holder, objective)`: new scores are 0 and locked.
    pub fn access(&mut self, holder: &str, objective: &str) -> ScoreAccess {
        self.access_as(holder, None, objective)
    }

    /// [`access`](Self::access) for an entity holder with its display name.
    pub fn access_as(&mut self, holder: &str, display: Option<Text>, objective: &str) -> ScoreAccess {
        if !self.scores.contains_key(holder) {
            self.holder_order.insert(holder);
        }
        let entry = self.scores.entry(holder.to_owned()).or_default();
        let created = !entry.contains_key(objective);
        entry.entry(objective.to_owned()).or_default();
        ScoreAccess {
            holder: holder.to_owned(),
            objective: objective.to_owned(),
            requires_sync: created,
            holder_display: display,
        }
    }

    fn entry(&mut self, a: &ScoreAccess) -> &mut Score {
        let entry = self.scores.entry(a.holder.clone()).or_default();
        entry.entry(a.objective.clone()).or_default()
    }

    pub fn get(&self, a: &ScoreAccess) -> i32 {
        self.score(&a.holder, &a.objective).unwrap_or(0)
    }

    /// `ScoreAccess.set`.
    pub fn set(&mut self, a: &mut ScoreAccess, value: i32) {
        let auto = self.objective(&a.objective).is_some_and(|o| o.display_auto_update);
        let display = a.holder_display.clone();
        let score = self.entry(a);
        let mut sync = a.requires_sync;
        if auto
            && let Some(d) = display
            && score.display.as_ref() != Some(&d)
        {
            score.display = Some(d);
            sync = true;
        }
        if score.value != value {
            score.value = value;
            sync = true;
        }
        if sync {
            self.send_score(a);
        }
    }

    /// `ScoreAccess.add`: the new value.
    pub fn add(&mut self, a: &mut ScoreAccess, delta: i32) -> i32 {
        let v = self.get(a).wrapping_add(delta);
        self.set(a, v);
        v
    }

    /// `ScoreAccess.lock` / `unlock`.
    pub fn set_locked(&mut self, a: &mut ScoreAccess, locked: bool) {
        self.entry(a).locked = locked;
        if a.requires_sync {
            self.send_score(a);
        }
        self.dirty = true;
    }

    /// `ScoreAccess.display`.
    pub fn set_score_display(&mut self, a: &mut ScoreAccess, display: Option<Text>) {
        let score = self.entry(a);
        if a.requires_sync || score.display != display {
            score.display = display;
            self.send_score(a);
        }
    }

    /// `ScoreAccess.numberFormatOverride`.
    pub fn set_score_number_format(&mut self, a: &mut ScoreAccess, format: Option<NumberFormat>) {
        self.entry(a).number_format = format;
        self.send_score(a);
    }

    /// `sendScoreToPlayers` / `ServerScoreboard.onScoreChanged`.
    fn send_score(&mut self, a: &mut ScoreAccess) {
        if self.tracked.contains(&a.objective)
            && let Some(score) = self.score_info(&a.holder, &a.objective)
        {
            let pkt = score_packet(&a.holder, &a.objective, score);
            self.broadcast(pkt);
        }
        a.requires_sync = false;
        self.dirty = true;
    }

    /// `getOrCreatePlayerScore(...).set(value)`.
    pub fn set_score(&mut self, holder: &str, objective: &str, value: i32) {
        let mut a = self.access(holder, objective);
        self.set(&mut a, value);
    }

    /// `resetSinglePlayerScore` (untracking holders left without scores) and
    /// `resetAllPlayerScores`.
    pub fn reset(&mut self, holder: &str, objective: Option<&str>) {
        let Some(scores) = self.scores.get_mut(holder) else { return };
        if let Some(o) = objective {
            let removed = scores.remove(o).is_some();
            if !scores.is_empty() {
                if removed {
                    if self.tracked.contains(o) {
                        self.broadcast(sp::reset_score(holder, Some(o)));
                    }
                    self.dirty = true;
                }
                return;
            }
        }
        self.scores.remove(holder);
        self.holder_order.remove(holder);
        // `onPlayerRemoved`.
        if !self.tracked.is_empty() {
            self.broadcast(sp::reset_score(holder, None));
        }
        self.dirty = true;
    }

    /// `getTrackedPlayers()`, in the order vanilla streams them.
    pub fn holders(&self) -> Vec<String> {
        self.holder_order.ascending().map(str::to_owned).collect()
    }

    /// `listPlayerScores`: a holder's scores by objective name.
    pub fn scores_of(&self, holder: &str) -> Vec<(&str, i32)> {
        self.scores.get(holder).map_or_else(Vec::new, |s| s.iter().map(|(o, v)| (o.as_str(), v.value)).collect())
    }

    // ---- teams ----------------------------------------------------------------------------

    /// `getPlayerTeams()` in iteration order.
    pub fn teams(&self) -> Vec<&Team> {
        self.team_order.descending().filter_map(|n| self.team(n)).collect()
    }

    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.iter().find(|t| t.name == name)
    }

    /// `getPlayersTeam`.
    pub fn team_of(&self, holder: &str) -> Option<&Team> {
        self.team(self.team_of.get(holder)?)
    }

    /// `addPlayerTeam`; returns false if the name is taken.
    pub fn add_team(&mut self, name: &str) -> bool {
        if self.team(name).is_some() {
            return false;
        }
        let team = Team::new(name);
        self.broadcast(team.packet(0));
        self.team_order.insert(name);
        self.teams.push(team);
        self.dirty = true;
        true
    }

    /// Changes a team's settings and sends them (`onTeamChanged`).
    pub fn modify_team(&mut self, name: &str, f: impl FnOnce(&mut Team)) {
        let Some(team) = self.teams.iter_mut().find(|t| t.name == name) else { return };
        f(team);
        let pkt = team.packet(2);
        self.broadcast(pkt);
        self.dirty = true;
    }

    /// `removePlayerTeam`.
    pub fn remove_team(&mut self, name: &str) {
        let Some(i) = self.teams.iter().position(|t| t.name == name) else { return };
        let team = self.teams.remove(i);
        self.team_order.remove(name);
        for p in team.players.iter() {
            self.team_of.remove(p);
        }
        self.broadcast(sp::set_player_team(name, &sp::TeamMethod::Remove));
        self.dirty = true;
    }

    /// `addPlayerToTeam`: leaves the old team first; true unless the name is already listed
    /// (never, since leaving removes it).
    pub fn join_team(&mut self, holder: &str, team: &str) -> bool {
        if self.team(team).is_none() {
            return false;
        }
        if self.team_of.contains_key(holder) {
            self.leave_team(holder);
        }
        self.team_of.insert(holder.to_owned(), team.to_owned());
        let t = self.teams.iter_mut().find(|t| t.name == team).expect("team exists");
        let added = t.players.insert(holder);
        if added {
            self.broadcast(sp::set_player_team(team, &sp::TeamMethod::Join(&[holder])));
            self.dirty = true;
        }
        added
    }

    /// `removePlayerFromTeam(name)`: whether the holder was on a team.
    pub fn leave_team(&mut self, holder: &str) -> bool {
        let Some(team) = self.team_of.remove(holder) else { return false };
        if let Some(t) = self.teams.iter_mut().find(|t| t.name == team) {
            t.players.remove(holder);
        }
        self.broadcast(sp::set_player_team(&team, &sp::TeamMethod::Leave(&[holder])));
        self.dirty = true;
        true
    }

    /// A player's name as others see it: formatted by their team, if any.
    pub fn player_display_name(&self, name: &str) -> Text {
        match self.team_of(name) {
            Some(t) => t.format_name(Text::literal(name)),
            None => Text::literal(name),
        }
    }

    // ---- saved data -----------------------------------------------------------------------

    /// `ScoreboardSaveData.Packed` (the `data` compound of `data/minecraft/scoreboard.dat`).
    pub fn to_nbt(&self) -> Tag {
        let mut objectives = Vec::new();
        for o in self.objectives() {
            let mut f = vec![("Name".to_owned(), Tag::String(o.name.clone()))];
            if o.criterion != "dummy" {
                f.push(("CriteriaName".into(), Tag::String(o.criterion.clone())));
            }
            f.push(("DisplayName".into(), o.display_name.to_nbt()));
            if o.render_type != "integer" {
                f.push(("RenderType".into(), Tag::String(o.render_type.into())));
            }
            if o.display_auto_update {
                f.push(("display_auto_update".into(), Tag::Byte(1)));
            }
            if let Some(nf) = &o.number_format {
                f.push(("format".into(), nf.to_nbt()));
            }
            objectives.push(Tag::Compound(f));
        }
        let mut scores = Vec::new();
        for holder in self.holder_order.descending() {
            let Some(map) = self.scores.get(holder) else { continue };
            for (objective, s) in map {
                let mut f = vec![
                    ("Name".to_owned(), Tag::String(holder.to_owned())),
                    ("Objective".to_owned(), Tag::String(objective.clone())),
                ];
                if s.value != 0 {
                    f.push(("Score".into(), Tag::Int(s.value)));
                }
                if s.locked {
                    f.push(("Locked".into(), Tag::Byte(1)));
                }
                if let Some(d) = &s.display {
                    f.push(("display".into(), d.to_nbt()));
                }
                if let Some(nf) = &s.number_format {
                    f.push(("format".into(), nf.to_nbt()));
                }
                scores.push(Tag::Compound(f));
            }
        }
        let slots = self
            .display
            .iter()
            .enumerate()
            .filter_map(|(i, d)| Some((DISPLAY_SLOTS[i].to_owned(), Tag::String(d.clone()?))))
            .collect();
        let mut teams = Vec::new();
        for t in self.teams() {
            let mut f =
                vec![("Name".to_owned(), Tag::String(t.name.clone())), ("DisplayName".into(), t.display_name.to_nbt())];
            if let Some(c) = t.color_name() {
                f.push(("TeamColor".into(), Tag::String(c.into())));
            }
            if !t.friendly_fire {
                f.push(("AllowFriendlyFire".into(), Tag::Byte(0)));
            }
            if !t.see_friendly_invisibles {
                f.push(("SeeFriendlyInvisibles".into(), Tag::Byte(0)));
            }
            let empty = Text::literal("").to_nbt();
            if t.prefix.to_nbt() != empty {
                f.push(("MemberNamePrefix".into(), t.prefix.to_nbt()));
            }
            if t.suffix.to_nbt() != empty {
                f.push(("MemberNameSuffix".into(), t.suffix.to_nbt()));
            }
            for (key, rule, names) in [
                ("NameTagVisibility", t.name_tag_visibility, &VISIBILITY_NAMES),
                ("DeathMessageVisibility", t.death_message_visibility, &VISIBILITY_NAMES),
                ("CollisionRule", t.collision_rule, &COLLISION_NAMES),
            ] {
                if rule != TeamRule::Always {
                    f.push((key.into(), Tag::String(names[rule_index(rule)].into())));
                }
            }
            let players = t.players();
            if !players.is_empty() {
                f.push(("Players".into(), Tag::List(players.into_iter().map(Tag::String).collect())));
            }
            teams.push(Tag::Compound(f));
        }
        let mut out = Vec::new();
        for (key, list) in [("Objectives", objectives), ("PlayerScores", scores)] {
            if !list.is_empty() {
                out.push((key.to_owned(), Tag::List(list)));
            }
        }
        if !self.display.iter().all(Option::is_none) {
            out.push(("DisplaySlots".into(), Tag::Compound(slots)));
        }
        if !teams.is_empty() {
            out.push(("Teams".into(), Tag::List(teams)));
        }
        Tag::Compound(out)
    }

    /// `ServerScoreboard.load`: objectives, scores, display slots, then teams. Queues no
    /// packets (nobody is online yet).
    pub fn load_nbt(&mut self, data: &Tag) {
        let list = |k: &str| data.get(k).and_then(Tag::as_list).unwrap_or(&[]);
        for o in list("Objectives") {
            let Some(name) = o.get("Name").and_then(Tag::as_str) else { continue };
            let criterion = o.get("CriteriaName").and_then(Tag::as_str).unwrap_or("dummy");
            let display = o.get("DisplayName").cloned().map_or_else(|| Text::literal(name), Text::raw);
            let mut objective = Objective::new(name, criterion, display);
            objective.render_type =
                if o.get("RenderType").and_then(Tag::as_str) == Some("hearts") { "hearts" } else { "integer" };
            objective.display_auto_update = o.get("display_auto_update").and_then(Tag::as_i64).is_some_and(|v| v != 0);
            objective.number_format = o.get("format").and_then(NumberFormat::from_nbt);
            self.add_objective(objective);
        }
        for s in list("PlayerScores") {
            let (Some(holder), Some(objective)) =
                (s.get("Name").and_then(Tag::as_str), s.get("Objective").and_then(Tag::as_str))
            else {
                continue;
            };
            if self.objective(objective).is_none() {
                continue;
            }
            let a = self.access(holder, objective);
            let score = self.entry(&a);
            score.value = s.get("Score").and_then(Tag::as_i64).unwrap_or(0) as i32;
            score.locked = s.get("Locked").and_then(Tag::as_i64).is_some_and(|v| v != 0);
            score.display = s.get("display").cloned().map(Text::raw);
            score.number_format = s.get("format").and_then(NumberFormat::from_nbt);
        }
        if let Some(Tag::Compound(slots)) = data.get("DisplaySlots") {
            for (slot, o) in slots {
                if let Some(o) = o.as_str().filter(|o| self.objective(o).is_some()) {
                    self.set_display(slot, Some(o));
                }
            }
        }
        for t in list("Teams") {
            let Some(name) = t.get("Name").and_then(Tag::as_str) else { continue };
            if !self.add_team(name) {
                continue;
            }
            let team = self.teams.iter_mut().find(|x| x.name == name).expect("just added");
            if let Some(d) = t.get("DisplayName") {
                team.display_name = Text::raw(d.clone());
            }
            team.color = t.get("TeamColor").and_then(Tag::as_str).and_then(|c| TEAM_COLORS.iter().position(|n| *n == c));
            team.friendly_fire = t.get("AllowFriendlyFire").and_then(Tag::as_i64).is_none_or(|v| v != 0);
            team.see_friendly_invisibles = t.get("SeeFriendlyInvisibles").and_then(Tag::as_i64).is_none_or(|v| v != 0);
            if let Some(p) = t.get("MemberNamePrefix") {
                team.prefix = Text::raw(p.clone());
            }
            if let Some(s) = t.get("MemberNameSuffix") {
                team.suffix = Text::raw(s.clone());
            }
            let rule = |k: &str, names| t.get(k).and_then(Tag::as_str).and_then(|n| rule_by_name(names, n));
            team.name_tag_visibility = rule("NameTagVisibility", &VISIBILITY_NAMES).unwrap_or_default();
            team.death_message_visibility = rule("DeathMessageVisibility", &VISIBILITY_NAMES).unwrap_or_default();
            team.collision_rule = rule("CollisionRule", &COLLISION_NAMES).unwrap_or_default();
            for p in t.get("Players").and_then(Tag::as_list).unwrap_or(&[]) {
                if let Some(p) = p.as_str() {
                    self.join_team(p, name);
                }
            }
        }
        self.packets.clear();
        self.dirty = false;
    }
}

/// `ClientboundSetObjectivePacket` with method 0 (add) or 2 (change).
fn objective_packet(o: &Objective, method: u8) -> Bytes {
    let display = o.display_name.to_nbt();
    let wire = sp::Objective {
        display_name: &display,
        render_type: if o.render_type == "hearts" { sp::RenderType::Hearts } else { sp::RenderType::Integer },
        number_format: o.number_format.as_ref().map(NumberFormat::wire),
    };
    let method = if method == 0 { sp::ObjectiveMethod::Add(wire) } else { sp::ObjectiveMethod::Change(wire) };
    sp::set_objective(&o.name, &method)
}

fn score_packet(holder: &str, objective: &str, s: &Score) -> Bytes {
    let display = s.display.as_ref().map(Text::to_nbt);
    let format = s.number_format.as_ref().map(NumberFormat::wire);
    sp::set_score(holder, objective, s.value, display.as_ref(), format.as_ref())
}

/// `String.hashCode`.
pub(crate) fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// Iteration order of a `java.util.HashSet` (default capacity 16, load factor 0.75) of keys
/// with the given hash codes: by bucket, then insertion order within a bucket (resizes keep
/// the relative order). Buckets never shrink.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JavaHashSet {
    /// (key, hashCode, insertion sequence).
    entries: Vec<(String, i32, u64)>,
    capacity: usize,
    seq: u64,
}

impl Default for JavaHashSet {
    fn default() -> Self {
        Self { entries: Vec::new(), capacity: 16, seq: 0 }
    }
}

impl JavaHashSet {
    pub fn insert(&mut self, key: &str) -> bool {
        self.insert_hashed(key, java_string_hash(key))
    }

    /// Inserts with an explicit `hashCode` (for keys that are not strings).
    pub fn insert_hashed(&mut self, key: &str, hash: i32) -> bool {
        if self.contains(key) {
            return false;
        }
        self.entries.push((key.to_owned(), hash, self.seq));
        self.seq += 1;
        if self.entries.len() > self.capacity * 3 / 4 {
            self.capacity *= 2;
        }
        true
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(k, _, _)| k != key);
        self.entries.len() != before
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _, _)| k == key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let mask = self.capacity - 1;
        let mut v: Vec<&(String, i32, u64)> = self.entries.iter().collect();
        v.sort_by_key(|(_, h, seq)| {
            let h = *h as u32;
            ((h ^ (h >> 16)) as usize & mask, *seq)
        });
        v.into_iter().map(|(k, _, _)| k.as_str())
    }
}

/// The slot layout of fastutil's `Object2ObjectOpenHashMap` over string keys, built as the
/// vanilla scoreboard builds its maps (16 expected entries; load factor 0.5 for objectives
/// and scores, 0.75 for teams), so listings come out in vanilla's order: streams walk the
/// slots upwards, iterators downwards.
#[derive(Debug, Clone, PartialEq)]
struct OpenHashKeys {
    slots: Vec<Option<String>>,
    size: usize,
    /// Load factor in quarters (2 = 0.5, 3 = 0.75).
    quarters: usize,
    min_slots: usize,
}

impl Default for OpenHashKeys {
    fn default() -> Self {
        Self::with_load(2)
    }
}

impl OpenHashKeys {
    /// `new Object2ObjectOpenHashMap(16, f)`.
    fn with_load(quarters: usize) -> Self {
        let min_slots = array_size(16, quarters);
        Self { slots: vec![None; min_slots], size: 0, quarters, min_slots }
    }

    fn mask(&self) -> usize {
        self.slots.len() - 1
    }

    /// `HashCommon.maxFill(n, f)`.
    fn max_fill(&self) -> usize {
        let n = self.slots.len();
        (n * self.quarters).div_ceil(4).min(n - 1)
    }

    /// `HashCommon.mix(key.hashCode()) & mask`.
    fn home(key: &str, mask: usize) -> usize {
        let h = (java_string_hash(key) as u32).wrapping_mul(0x9E37_79B9);
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
            self.rehash(array_size(self.size + 1, self.quarters));
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
        if n > self.min_slots && self.size < self.max_fill() / 4 {
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

/// `HashCommon.arraySize(expected, f)`.
fn array_size(expected: usize, quarters: usize) -> usize {
    (expected * 4).div_ceil(quarters).next_power_of_two().max(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objective(name: &str, criterion: &str) -> Objective {
        Objective::new(name, criterion, Text::literal(name))
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
        assert!(sb.score_info("Bob", "t").unwrap().locked);
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
    fn packets_follow_display_slots() {
        let mut sb = Scoreboard::default();
        sb.add_objective(objective("k", "dummy"));
        sb.set_score("A", "k", 1);
        assert!(sb.take_packets().is_empty(), "undisplayed objectives are not sent");
        sb.set_display("sidebar", Some("k"));
        // Objective, its slot and its one score.
        assert_eq!(sb.take_packets().len(), 3);
        sb.set_score("A", "k", 1);
        assert!(sb.take_packets().is_empty(), "unchanged");
        sb.set_score("A", "k", 2);
        assert_eq!(sb.take_packets().len(), 1);
        let mut a = sb.access("B", "k");
        sb.set(&mut a, 0);
        assert_eq!(sb.take_packets().len(), 1, "a new score is sent even at 0");
        sb.set_display("list", Some("k"));
        assert_eq!(sb.take_packets().len(), 1, "already tracked: only the slot");
        sb.set_display("sidebar", None);
        assert_eq!(sb.take_packets().len(), 1, "still shown in the list: slot cleared");
        sb.set_display("list", None);
        assert_eq!(sb.take_packets().len(), 1, "no slot left: the objective is removed");
        assert!(sb.join_packets().is_empty());
        assert!(sb.take_dirty());
    }

    #[test]
    fn teams_and_saved_data() {
        let mut sb = Scoreboard::default();
        assert!(sb.add_team("red"));
        assert!(!sb.add_team("red"));
        sb.modify_team("red", |t| {
            t.color = Some(12);
            t.prefix = Text::literal("[R] ");
        });
        assert!(sb.join_team("Alice", "red"));
        sb.add_team("blue");
        assert!(sb.join_team("Alice", "blue"), "moves between teams");
        assert_eq!(sb.team("red").unwrap().players(), Vec::<String>::new());
        assert!(sb.join_team("Alice", "red"));
        assert_eq!(sb.player_display_name("Alice").to_plain(), "[R] Alice");
        assert!(sb.leave_team("Alice") && !sb.leave_team("Alice"));
        sb.join_team("Bob", "red");
        sb.add_objective(objective("k", "trigger"));
        sb.set_score("Bob", "k", 5);
        sb.set_display("sidebar", Some("k"));
        sb.take_packets();
        let nbt = sb.to_nbt();
        let mut loaded = Scoreboard::default();
        loaded.load_nbt(&nbt);
        assert_eq!(loaded.to_nbt(), nbt);
        assert_eq!(loaded.team_of("Bob").map(|t| t.name.as_str()), Some("red"));
        assert_eq!(loaded.score_info("Bob", "k").map(|s| (s.value, s.locked)), Some((5, true)));
        // Two teams, then the displayed objective with its slot and score.
        assert_eq!(loaded.join_packets().len(), 5);
        assert!(loaded.take_packets().is_empty());
    }

    #[test]
    fn java_hash_set_order() {
        // As `java.util.HashSet` iterates them (seen in jshell); the 13th key resizes to 32.
        let mut s = JavaHashSet::default();
        for k in ["Diff0", "Other0", "#fake", "Alice", "zz", "a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            s.insert(k);
        }
        let order: Vec<&str> = s.iter().collect();
        assert_eq!(order, ["zz", "a", "Other0", "b", "c", "Diff0", "d", "#fake", "e", "Alice", "f", "g", "h", "i"]);
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
        let teams = OpenHashKeys::with_load(3);
        assert_eq!((teams.slots.len(), teams.max_fill()), (32, 24));
    }
}
