//! Brigadier suggestions and `SharedSuggestionProvider`'s matching rules.

use crate::text::Text;
use bytes::Bytes;
use kiln_proto::packets::commands::{self as wire, SuggestionEntry};

/// A completion replacing `start..end` (byte offsets) of the input.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub tooltip: Option<Text>,
}

impl Suggestion {
    /// Widens to `start..end`, copying the covered input around the text.
    fn expand(&self, command: &str, start: usize, end: usize) -> Suggestion {
        if (start, end) == (self.start, self.end) {
            return self.clone();
        }
        let mut text = String::new();
        text.push_str(&command[start..self.start]);
        text.push_str(&self.text);
        text.push_str(&command[self.end..end]);
        Suggestion { start, end, text, tooltip: self.tooltip.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Suggestions {
    pub start: usize,
    pub end: usize,
    pub list: Vec<Suggestion>,
}

impl Suggestions {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn texts(&self) -> Vec<&str> {
        self.list.iter().map(|s| s.text.as_str()).collect()
    }

    /// `Suggestions.create`: one common range, deduplicated, sorted ignoring case.
    pub fn create(command: &str, suggestions: Vec<Suggestion>) -> Suggestions {
        if suggestions.is_empty() {
            return Suggestions::default();
        }
        let start = suggestions.iter().map(|s| s.start).min().unwrap();
        let end = suggestions.iter().map(|s| s.end).max().unwrap();
        let mut list: Vec<Suggestion> = Vec::new();
        for s in suggestions {
            let s = s.expand(command, start, end);
            if !list.contains(&s) {
                list.push(s);
            }
        }
        list.sort_by_cached_key(|s| s.text.to_lowercase());
        Suggestions { start, end, list }
    }

    /// `Suggestions.merge`.
    pub fn merge(command: &str, mut input: Vec<Suggestions>) -> Suggestions {
        match input.len() {
            0 => Suggestions::default(),
            1 => input.pop().unwrap(),
            _ => Suggestions::create(command, input.into_iter().flat_map(|s| s.list).collect()),
        }
    }

    /// The `command_suggestions` packet answering request `id` for `command`, keeping at most
    /// 1000 entries like vanilla. Offsets are converted to UTF-16 units.
    pub fn to_packet(&self, id: i32, command: &str) -> Bytes {
        let utf16 = |i: usize| command[..i.min(command.len())].encode_utf16().count() as i32;
        let tooltips: Vec<_> = self.list.iter().take(1000).map(|s| s.tooltip.as_ref().map(Text::to_nbt)).collect();
        let entries: Vec<SuggestionEntry> = self
            .list
            .iter()
            .zip(&tooltips)
            .map(|(s, t)| SuggestionEntry { text: &s.text, tooltip: t.as_ref() })
            .collect();
        let start = utf16(self.start);
        wire::command_suggestions(id, start, utf16(self.end) - start, &entries)
    }
}

/// Collects suggestions for the text from `start` to the end of `input`.
#[derive(Debug, Clone)]
pub struct SuggestionsBuilder<'a> {
    input: &'a str,
    start: usize,
    remaining_lower: String,
    result: Vec<Suggestion>,
}

impl<'a> SuggestionsBuilder<'a> {
    pub fn new(input: &'a str, start: usize) -> Self {
        SuggestionsBuilder { input, start, remaining_lower: input[start..].to_lowercase(), result: Vec::new() }
    }

    pub fn input(&self) -> &'a str {
        self.input
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn remaining(&self) -> &'a str {
        &self.input[self.start..]
    }

    pub fn remaining_lowercase(&self) -> &str {
        &self.remaining_lower
    }

    pub fn suggest(&mut self, text: impl Into<String>) {
        self.push(text.into(), None);
    }

    pub fn suggest_with_tooltip(&mut self, text: impl Into<String>, tooltip: Text) {
        self.push(text.into(), Some(tooltip));
    }

    fn push(&mut self, text: String, tooltip: Option<Text>) {
        if text != self.remaining() {
            self.result.push(Suggestion { start: self.start, end: self.input.len(), text, tooltip });
        }
    }

    /// A builder over the same input starting at `start`.
    pub fn offset(&self, start: usize) -> SuggestionsBuilder<'a> {
        SuggestionsBuilder::new(self.input, start)
    }

    pub fn add(&mut self, other: SuggestionsBuilder) {
        self.result.extend(other.result);
    }

    pub fn build(self) -> Suggestions {
        Suggestions::create(self.input, self.result)
    }

    /// `SharedSuggestionProvider.suggest`: candidates matching at word starts.
    pub fn suggest_matching<'s>(&mut self, candidates: impl IntoIterator<Item = &'s str>) {
        let input = self.remaining_lower.clone();
        for c in candidates {
            if matches_sub_str(&input, &c.to_lowercase()) {
                self.suggest(c);
            }
        }
    }

    /// `SharedSuggestionProvider.suggestResource` with a prefix such as `#` or `!`.
    pub fn suggest_resources<'s>(&mut self, ids: impl IntoIterator<Item = &'s str>, prefix: &str) {
        let Some(input) = self.remaining_lower.strip_prefix(prefix).map(str::to_owned) else { return };
        let with_colon = input.contains(':');
        for id in ids {
            let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
            let ok = if with_colon {
                matches_sub_str(&input, id)
            } else {
                matches_sub_str(&input, ns) || (ns == "minecraft" && matches_sub_str(&input, path))
            };
            if ok {
                self.suggest(format!("{prefix}{id}"));
            }
        }
    }
}

/// `SharedSuggestionProvider.matchesSubStr`: `input` is a prefix of `candidate` or of one of
/// its `_`-separated parts.
pub fn matches_sub_str(input: &str, candidate: &str) -> bool {
    let mut i = 0;
    while !candidate[i..].starts_with(input) {
        match candidate[i..].find('_') {
            Some(j) => i += j + 1,
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        assert!(matches_sub_str("gr", "grass_block"));
        assert!(matches_sub_str("bl", "grass_block"));
        assert!(!matches_sub_str("ra", "grass_block"));
        let mut b = SuggestionsBuilder::new("give @s dia", 8);
        b.suggest_resources(
            [
                "minecraft:diamond",
                "minecraft:diamond_ore",
                "minecraft:deepslate_diamond_ore",
                "kiln:dial",
                "minecraft:stone",
            ],
            "",
        );
        assert_eq!(
            b.build().texts(),
            ["minecraft:deepslate_diamond_ore", "minecraft:diamond", "minecraft:diamond_ore"]
        );
        let mut b = SuggestionsBuilder::new("x kiln:d", 2);
        b.suggest_resources(["minecraft:diamond", "kiln:dial"], "");
        assert_eq!(b.build().texts(), ["kiln:dial"]);
    }

    #[test]
    fn merge_expands_to_common_range() {
        let a = Suggestion { start: 4, end: 6, text: "xy".into(), tooltip: None };
        let b = Suggestion { start: 5, end: 6, text: "Q".into(), tooltip: None };
        let s = Suggestions::create("abcdef", vec![a, b]);
        assert_eq!((s.start, s.end), (4, 6));
        assert_eq!(s.texts(), ["eQ", "xy"]);
    }

    #[test]
    fn packet_uses_utf16_offsets() {
        let s = Suggestions {
            start: 4,
            end: 4,
            list: vec![Suggestion { start: 4, end: 4, text: "x".into(), tooltip: None }],
        };
        let p = s.to_packet(3, "/é 𝄞 ");
        let mut r = kiln_proto::Reader::new(&p);
        r.varint().unwrap();
        assert_eq!((r.varint().unwrap(), r.varint().unwrap(), r.varint().unwrap()), (3, 3, 0));
        let s = Suggestions { start: 9, end: 9, list: vec![] };
        let p = s.to_packet(3, "/é 𝄞 ");
        let mut r = kiln_proto::Reader::new(&p);
        r.varint().unwrap();
        assert_eq!((r.varint().unwrap(), r.varint().unwrap()), (3, 6));
    }
}
