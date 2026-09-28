//! Data pack functions (`CommandFunction`), function tags (`ServerFunctionLibrary`), the
//! scheduled function queue (`TimerQueue`) and the data pack list `/datapack` shows.
//!
//! Hosts load functions from their enabled packs ([`CommandFunction::from_lines`], then
//! [`FunctionLibrary::insert`] after checking every plain line parses), expose the library
//! through [`Host::functions`](crate::Host::functions), run `#minecraft:load` after a reload
//! and `#minecraft:tick` every tick ([`run_function`](crate::vanilla::run_function)), and
//! tick the [`TimerQueue`].

use crate::snbt;
use crate::text::Text;
use crate::tr;
use crate::types::Identifier;
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::sync::Arc;

/// `CommandFunction.checkCommandLineLength`.
const MAX_LINE: usize = 2_000_000;

/// One line of a function: a command, or a macro line (`$...`) with `$(name)` variables.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Plain(String),
    /// `StringTemplate`: text segments around variables (indices into the function's
    /// parameters).
    Macro { segments: Vec<String>, variables: Vec<usize> },
}

/// A loaded function (`PlainTextFunction` or `MacroFunction`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandFunction {
    pub id: Identifier,
    pub entries: Vec<Entry>,
    /// Macro parameter names in first-use order; empty for plain functions.
    pub parameters: Vec<String>,
    is_macro: bool,
}

impl CommandFunction {
    /// `CommandFunction.fromLines` without the parsing: comments and blank lines dropped,
    /// `\` continuations joined, `$` lines turned into templates. Errors read like
    /// vanilla's (the host logs them and skips the function).
    pub fn from_lines(id: Identifier, lines: &[&str]) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut parameters: Vec<String> = Vec::new();
        let mut is_macro = false;
        let mut i = 0;
        while i < lines.len() {
            let number = i + 1;
            let mut line = lines[i].trim().to_owned();
            if continues(&line) {
                loop {
                    i += 1;
                    if i == lines.len() {
                        return Err("Line continuation at end of file".into());
                    }
                    line.pop();
                    line.push_str(lines[i].trim());
                    check_length(&line)?;
                    if !continues(&line) {
                        break;
                    }
                }
            }
            check_length(&line)?;
            i += 1;
            let Some(first) = line.chars().next() else { continue };
            match first {
                '#' => continue,
                '/' => {
                    if line[1..].starts_with('/') {
                        return Err(format!(
                            "Unknown or invalid command '{line}' on line {number} (if you intended to make a comment, use '#' not '//')"
                        ));
                    }
                    let word: String = line[1..].chars().take_while(|c| crate::reader::is_allowed_in_unquoted_string(*c)).collect();
                    return Err(format!(
                        "Unknown or invalid command '{line}' on line {number} (did you mean '{word}'? Do not use a preceding forwards slash.)"
                    ));
                }
                '$' => {
                    let (segments, names) =
                        template(&line[1..]).map_err(|e| format!("Can't parse function line {number}: '{line}': {e}"))?;
                    let variables = names
                        .into_iter()
                        .map(|n| match parameters.iter().position(|p| *p == n) {
                            Some(i) => i,
                            None => {
                                parameters.push(n);
                                parameters.len() - 1
                            }
                        })
                        .collect();
                    is_macro = true;
                    entries.push(Entry::Macro { segments, variables });
                }
                _ => entries.push(Entry::Plain(line)),
            }
        }
        Ok(CommandFunction { id, entries, parameters, is_macro })
    }

    pub fn is_macro(&self) -> bool {
        self.is_macro
    }

    /// Plain lines with their 1-based line index among the entries, for checking at load.
    pub fn plain_lines(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Plain(s) => Some(s.as_str()),
            Entry::Macro { .. } => None,
        })
    }

    /// `instantiate`: the command lines with `arguments` substituted into macro lines.
    /// Plain functions ignore the arguments.
    pub fn instantiate(&self, arguments: Option<&Tag>) -> Result<Vec<String>, Text> {
        if !self.is_macro {
            return Ok(self.plain_lines().map(str::to_owned).collect());
        }
        let Some(arguments) = arguments else {
            return Err(tr!("commands.function.error.missing_arguments", self.id.to_string()));
        };
        let mut values = Vec::with_capacity(self.parameters.len());
        for p in &self.parameters {
            match arguments.get(p) {
                Some(v) => values.push(stringify(v)),
                None => return Err(tr!("commands.function.error.missing_argument", self.id.to_string(), p.as_str())),
            }
        }
        let mut out = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            match e {
                Entry::Plain(s) => out.push(s.clone()),
                Entry::Macro { segments, variables } => {
                    let mut line = String::new();
                    for (i, v) in variables.iter().enumerate() {
                        line.push_str(&segments[i]);
                        line.push_str(&values[*v]);
                    }
                    if segments.len() > variables.len() {
                        line.push_str(segments.last().expect("segment"));
                    }
                    out.push(line);
                }
            }
        }
        Ok(out)
    }
}

fn continues(line: &str) -> bool {
    line.ends_with('\\')
}

fn check_length(line: &str) -> Result<(), String> {
    if line.len() > MAX_LINE {
        let head: String = line.chars().take(512).collect();
        return Err(format!("Command too long: {} characters, contents: {head}...", line.len()));
    }
    Ok(())
}

/// `StringTemplate.fromString`: segments and variable names.
fn template(s: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut segments = Vec::new();
    let mut names = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while let Some(off) = s[i..].find('$') {
        let pos = i + off;
        if pos + 1 < bytes.len() && bytes[pos + 1] == b'(' {
            segments.push(s[start..pos].to_owned());
            let Some(close) = s[pos + 1..].find(')') else { return Err("Unterminated macro variable".into()) };
            let end = pos + 1 + close;
            let name = &s[pos + 2..end];
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(format!("Invalid macro variable name '{name}'"));
            }
            names.push(name.to_owned());
            start = end + 1;
            i = start;
        } else {
            i = pos + 1;
        }
    }
    if names.is_empty() {
        return Err("No variables in macro".into());
    }
    if start != s.len() {
        segments.push(s[start..].to_owned());
    }
    Ok((segments, names))
}

/// `MacroFunction.stringify`: numbers without type suffixes, strings unquoted, anything else
/// as SNBT.
fn stringify(tag: &Tag) -> String {
    match tag {
        Tag::Float(f) => decimal(f64::from(*f)),
        Tag::Double(d) => decimal(*d),
        Tag::Byte(b) => b.to_string(),
        Tag::Short(s) => s.to_string(),
        Tag::Long(l) => l.to_string(),
        Tag::String(s) => s.clone(),
        other => snbt::to_snbt(other),
    }
}

/// `new DecimalFormat("#")` with 15 fraction digits (`Locale.ROOT`).
fn decimal(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "\u{221e}".into() } else { "-\u{221e}".into() };
    }
    let mut s = format!("{v:.15}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    // No minimum integer digits: `0.5` is `.5`.
    if let Some(rest) = s.strip_prefix("0.") {
        return format!(".{rest}");
    }
    if let Some(rest) = s.strip_prefix("-0.") {
        return format!("-.{rest}");
    }
    s
}

/// Loaded functions and function tags (`ServerFunctionLibrary`).
#[derive(Debug, Clone, Default)]
pub struct FunctionLibrary {
    functions: BTreeMap<Identifier, Arc<CommandFunction>>,
    tags: BTreeMap<Identifier, Vec<Identifier>>,
}

impl FunctionLibrary {
    pub fn insert(&mut self, function: CommandFunction) {
        self.functions.insert(function.id.clone(), Arc::new(function));
    }

    /// Sets a tag's (resolved) functions.
    pub fn set_tag(&mut self, id: Identifier, functions: Vec<Identifier>) {
        self.tags.insert(id, functions);
    }

    pub fn get(&self, id: &Identifier) -> Option<Arc<CommandFunction>> {
        self.functions.get(id).cloned()
    }

    /// `getTag`: the tag's functions (empty for unknown tags).
    pub fn tag(&self, id: &Identifier) -> Vec<Arc<CommandFunction>> {
        self.tags.get(id).map_or_else(Vec::new, |ids| ids.iter().filter_map(|i| self.get(i)).collect())
    }

    pub fn has_tag(&self, id: &Identifier) -> bool {
        self.tags.contains_key(id)
    }

    pub fn function_names(&self) -> impl Iterator<Item = &Identifier> {
        self.functions.keys()
    }

    pub fn tag_names(&self) -> impl Iterator<Item = &Identifier> {
        self.tags.keys()
    }

    pub fn len(&self) -> usize {
        self.functions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

/// What a scheduled event runs (`FunctionCallback` / `FunctionTagCallback`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerCallback {
    Function(Identifier),
    Tag(Identifier),
}

/// One scheduled event.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerEvent {
    /// `kiln:f` for functions, `#kiln:t` for tags; `schedule clear` removes by this.
    pub id: String,
    pub trigger_time: i64,
    pub sequence: u64,
    pub callback: TimerCallback,
}

/// `TimerQueue`: events by trigger time, then scheduling order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimerQueue {
    events: Vec<TimerEvent>,
    sequence: u64,
    dirty: bool,
}

impl TimerQueue {
    /// `schedule`: one event per id and time (a second one at the same time is ignored).
    pub fn schedule(&mut self, id: &str, trigger_time: i64, callback: TimerCallback) {
        if self.events.iter().any(|e| e.id == id && e.trigger_time == trigger_time) {
            return;
        }
        self.events.push(TimerEvent { id: id.to_owned(), trigger_time, sequence: self.sequence, callback });
        self.sequence += 1;
        self.dirty = true;
    }

    /// `remove`: every event with this id; how many there were.
    pub fn remove(&mut self, id: &str) -> usize {
        let before = self.events.len();
        self.events.retain(|e| e.id != id);
        let n = before - self.events.len();
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    /// The earliest trigger time of a queued event.
    pub fn next_trigger(&self) -> Option<i64> {
        self.events.iter().map(|e| e.trigger_time).min()
    }

    /// Takes the events due at `time`, in order.
    pub fn due(&mut self, time: i64) -> Vec<TimerCallback> {
        let mut due: Vec<TimerEvent> = Vec::new();
        self.events.retain(|e| {
            if e.trigger_time <= time {
                due.push(e.clone());
                false
            } else {
                true
            }
        });
        if !due.is_empty() {
            self.dirty = true;
        }
        due.sort_by_key(|e| (e.trigger_time, e.sequence));
        due.into_iter().map(|e| e.callback).collect()
    }

    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.events.iter().map(|e| e.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// `TimerQueue.Packed` (`data/minecraft/scheduled_events.dat`): events in order.
    pub fn to_nbt(&self) -> Tag {
        let mut events = self.events.clone();
        events.sort_by_key(|e| (e.trigger_time, e.sequence));
        let list = events
            .into_iter()
            .map(|e| {
                let (ty, target) = match &e.callback {
                    TimerCallback::Function(id) => ("minecraft:function", id),
                    TimerCallback::Tag(id) => ("minecraft:function_tag", id),
                };
                Tag::Compound(vec![
                    ("trigger_time".into(), Tag::Long(e.trigger_time)),
                    ("id".into(), Tag::String(e.id)),
                    (
                        "callback".into(),
                        Tag::Compound(vec![
                            ("type".into(), Tag::String(ty.into())),
                            ("id".into(), Tag::String(target.to_string())),
                        ]),
                    ),
                ])
            })
            .collect();
        Tag::Compound(vec![("events".into(), Tag::List(list))])
    }

    pub fn load_nbt(&mut self, data: &Tag) {
        for e in data.get("events").and_then(Tag::as_list).unwrap_or(&[]) {
            let (Some(name), Some(time), Some(cb)) =
                (e.get("id").and_then(Tag::as_str), e.get("trigger_time").and_then(Tag::as_i64), e.get("callback"))
            else {
                continue;
            };
            let Some(target) = cb.get("id").and_then(Tag::as_str).and_then(Identifier::parse) else { continue };
            let callback = match cb.get("type").and_then(Tag::as_str) {
                Some("minecraft:function") => TimerCallback::Function(target),
                Some("minecraft:function_tag") => TimerCallback::Tag(target),
                _ => continue,
            };
            self.schedule(name, time, callback);
        }
        self.dirty = false;
    }
}

/// Where a pack comes from (`PackSource`), shown after its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackSource {
    BuiltIn,
    Feature,
    World,
}

/// A data pack the server knows (`Pack`).
#[derive(Debug, Clone, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub source: PackSource,
    pub description: Text,
    /// Feature flags the pack needs (`minecraft:trade_rebalance`, ...).
    pub required_features: Vec<String>,
}

impl PackInfo {
    /// `Pack.getChatLink`: `[id (source)]`, green when enabled (hover shows the description).
    pub fn chat_link(&self, enabled: bool) -> Text {
        let source = match self.source {
            PackSource::BuiltIn => "pack.source.builtin",
            PackSource::Feature => "pack.source.feature",
            PackSource::World => "pack.source.world",
        };
        let label = tr!("pack.nameAndSource", self.id.as_str(), Text::translate(source, vec![]));
        let t = label.hover(self.description.clone()).insertion(format!("\"{}\"", self.id)).bracketed();
        if enabled { t.color("green") } else { t.color("red") }
    }
}

/// Available packs and the enabled ones in load order (`PackRepository`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataPacks {
    /// Sorted by id.
    pub available: Vec<PackInfo>,
    pub selected: Vec<String>,
    /// Feature flags enabled in the world.
    pub features: Vec<String>,
}

impl DataPacks {
    pub fn get(&self, id: &str) -> Option<&PackInfo> {
        self.available.iter().find(|p| p.id == id)
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.iter().any(|s| s == id)
    }

    pub fn selected_packs(&self) -> Vec<&PackInfo> {
        self.selected.iter().filter_map(|id| self.get(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }

    #[test]
    fn lines_and_macros() {
        let f = CommandFunction::from_lines(id("k:f"), &["# c", "", "say a", "scoreboard players \\", "  add x o 1"]).unwrap();
        assert_eq!(f.instantiate(None).unwrap(), ["say a", "scoreboard players add x o 1"]);
        let m = CommandFunction::from_lines(id("k:m"), &["$say $(a) and $(b)!", "$tp $(b)", "say x"]).unwrap();
        assert_eq!(m.parameters, ["a", "b"]);
        let args = Tag::Compound(vec![("a".into(), Tag::Float(1.5)), ("b".into(), Tag::String("s".into()))]);
        assert_eq!(m.instantiate(Some(&args)).unwrap(), ["say 1.5 and s!", "tp s", "say x"]);
        assert_eq!(m.instantiate(None).unwrap_err().key(), Some("commands.function.error.missing_arguments"));
        let partial = Tag::Compound(vec![("a".into(), Tag::Int(1))]);
        assert_eq!(m.instantiate(Some(&partial)).unwrap_err().key(), Some("commands.function.error.missing_argument"));
        assert!(CommandFunction::from_lines(id("k:e"), &["say \\"]).is_err());
        assert!(CommandFunction::from_lines(id("k:e"), &["$say no vars"]).unwrap_err().contains("No variables"));
        assert!(CommandFunction::from_lines(id("k:e"), &["/say"]).unwrap_err().contains("did you mean 'say'"));
        assert_eq!(decimal(0.1f32 as f64), ".100000001490116");
        assert_eq!(decimal(-0.5), "-.5");
        assert_eq!(decimal(0.0), "0");
        assert_eq!(decimal(2.0), "2");
    }

    #[test]
    fn timer_queue_order() {
        let mut q = TimerQueue::default();
        q.schedule("k:b", 10, TimerCallback::Function(id("k:b")));
        q.schedule("k:a", 5, TimerCallback::Function(id("k:a")));
        q.schedule("#k:t", 5, TimerCallback::Tag(id("k:t")));
        q.schedule("k:a", 5, TimerCallback::Function(id("k:a")));
        assert_eq!(q.due(4), []);
        assert_eq!(q.due(5), [TimerCallback::Function(id("k:a")), TimerCallback::Tag(id("k:t"))]);
        let nbt = q.to_nbt();
        let mut loaded = TimerQueue::default();
        loaded.load_nbt(&nbt);
        assert_eq!(loaded.remove("k:b"), 1);
        assert_eq!(q.due(100), [TimerCallback::Function(id("k:b"))]);
    }
}
