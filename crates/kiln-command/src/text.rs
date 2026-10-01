//! Chat components for command feedback and errors, encoded as network NBT.

use kiln_proto::nbt::Tag;
use std::fmt::Write as _;

/// A chat component: literal or translatable content, a style and siblings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub content: Content,
    pub style: Style,
    pub extra: Vec<Text>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Literal(String),
    /// A translation key rendered by the client, with its `%s` arguments.
    Translate {
        key: String,
        args: Vec<Arg>,
    },
    /// A component already in network NBT form (e.g. from `/tellraw`); style and siblings
    /// of the surrounding [`Text`] are ignored.
    Raw(Tag),
    /// An object component (`Component.object`): its NBT fields and the text plain renderings
    /// show (`ObjectInfo.defaultFallback`).
    Object { fields: Vec<(String, Tag)>, fallback: String },
}

impl Default for Content {
    fn default() -> Self {
        Content::Literal(String::new())
    }
}

/// A translation argument. Numbers stay numbers on the wire so the client formats them the
/// way vanilla does (`Component.translatable` keeps `Integer`, `Float`, ... as primitives).
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Text(Text),
    Str(String),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    /// A named color (`red`, `gray`, ...) or `#rrggbb`.
    pub color: Option<&'static str>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub click: Option<ClickEvent>,
    pub hover: Option<Box<Text>>,
    pub insertion: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClickEvent {
    SuggestCommand(String),
    RunCommand(String),
    CopyToClipboard(String),
}

impl Text {
    pub fn literal(s: impl Into<String>) -> Self {
        Text { content: Content::Literal(s.into()), ..Default::default() }
    }

    pub fn empty() -> Self {
        Text::default()
    }

    pub fn translate(key: impl Into<String>, args: Vec<Arg>) -> Self {
        Text { content: Content::Translate { key: key.into(), args }, ..Default::default() }
    }

    /// A component given as network NBT.
    pub fn raw(tag: Tag) -> Self {
        Text { content: Content::Raw(tag), ..Default::default() }
    }

    pub fn color(mut self, color: &'static str) -> Self {
        self.style.color = Some(color);
        self
    }

    pub fn italic(mut self) -> Self {
        self.style.italic = Some(true);
        self
    }

    pub fn underlined(mut self) -> Self {
        self.style.underlined = Some(true);
        self
    }

    pub fn click(mut self, event: ClickEvent) -> Self {
        self.style.click = Some(event);
        self
    }

    pub fn hover(mut self, text: Text) -> Self {
        self.style.hover = Some(Box::new(text));
        self
    }

    pub fn insertion(mut self, s: impl Into<String>) -> Self {
        self.style.insertion = Some(s.into());
        self
    }

    pub fn append(mut self, child: Text) -> Self {
        self.extra.push(child);
        self
    }

    /// The translation key, if this is a translatable component.
    pub fn key(&self) -> Option<&str> {
        match &self.content {
            Content::Translate { key, .. } => Some(key),
            Content::Literal(_) | Content::Raw(_) | Content::Object { .. } => None,
        }
    }

    pub fn args(&self) -> &[Arg] {
        match &self.content {
            Content::Translate { args, .. } => args,
            Content::Literal(_) | Content::Raw(_) | Content::Object { .. } => &[],
        }
    }

    /// An object component (a player head sprite): `fields` are its NBT fields (`object`, ...);
    /// consoles and clients without the sprite show `fallback`.
    pub fn object(fields: Vec<(String, Tag)>, fallback: impl Into<String>) -> Self {
        Text { content: Content::Object { fields, fallback: fallback.into() }, ..Default::default() }
    }

    /// `[text]` as `ComponentUtils.wrapInSquareBrackets` builds it.
    pub fn bracketed(self) -> Self {
        Text::translate("chat.square_brackets", vec![Arg::Text(self)])
    }

    /// Joins with `, ` like `ComponentUtils.formatList`.
    pub fn join(items: impl IntoIterator<Item = Text>) -> Self {
        let mut out = Text::empty();
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                out.extra.push(Text::literal(", ").color("gray"));
            }
            out.extra.push(item);
        }
        out
    }

    fn is_plain(&self) -> bool {
        matches!(self.content, Content::Literal(_)) && self.style == Style::default() && self.extra.is_empty()
    }

    /// Network NBT form: a bare string for plain literals, otherwise a compound.
    pub fn to_nbt(&self) -> Tag {
        if let (true, Content::Literal(s)) = (self.is_plain(), &self.content) {
            return Tag::String(s.clone());
        }
        let mut fields: Vec<(String, Tag)> = Vec::new();
        match &self.content {
            Content::Raw(tag) => return tag.clone(),
            Content::Object { fields: object, .. } => fields.extend(object.iter().cloned()),
            Content::Literal(s) => fields.push(("text".into(), Tag::String(s.clone()))),
            Content::Translate { key, args } => {
                fields.push(("translate".into(), Tag::String(key.clone())));
                if !args.is_empty() {
                    fields.push(("with".into(), list(args.iter().map(Arg::to_nbt).collect())));
                }
            }
        }
        let s = &self.style;
        if let Some(c) = s.color {
            fields.push(("color".into(), Tag::String(c.into())));
        }
        for (name, v) in [("bold", s.bold), ("italic", s.italic), ("underlined", s.underlined)] {
            if let Some(v) = v {
                fields.push((name.into(), Tag::Byte(v as i8)));
            }
        }
        if let Some(click) = &s.click {
            let (action, key, value) = match click {
                ClickEvent::SuggestCommand(c) => ("suggest_command", "command", c),
                ClickEvent::RunCommand(c) => ("run_command", "command", c),
                ClickEvent::CopyToClipboard(v) => ("copy_to_clipboard", "value", v),
            };
            fields.push((
                "click_event".into(),
                Tag::Compound(vec![
                    ("action".into(), Tag::String(action.into())),
                    (key.into(), Tag::String(value.clone())),
                ]),
            ));
        }
        if let Some(hover) = &s.hover {
            fields.push((
                "hover_event".into(),
                Tag::Compound(vec![
                    ("action".into(), Tag::String("show_text".into())),
                    ("value".into(), hover.to_nbt()),
                ]),
            ));
        }
        if let Some(ins) = &s.insertion {
            fields.push(("insertion".into(), Tag::String(ins.clone())));
        }
        if !self.extra.is_empty() {
            fields.push(("extra".into(), list(self.extra.iter().map(Text::to_nbt).collect())));
        }
        Tag::Compound(fields)
    }

    /// Plain rendering for logs and the console: literals as-is, translations as `key[args]`.
    pub fn to_plain(&self) -> String {
        let mut out = String::new();
        self.write_plain(&mut out);
        out
    }

    /// `Component.getString()` with `lang` as the active language, the way vanilla's dedicated
    /// server prints to its console (with its bundled `en_us`).
    pub fn to_string_in(&self, lang: &Language) -> String {
        let mut out = String::new();
        self.write_in(lang, &mut out);
        out
    }

    fn write_in(&self, lang: &Language, out: &mut String) {
        match &self.content {
            Content::Literal(s) => out.push_str(s),
            Content::Raw(tag) => write_nbt_in(tag, lang, out),
            Content::Object { fallback, .. } => out.push_str(fallback),
            Content::Translate { key, args } => {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| match a {
                        Arg::Text(t) => t.to_string_in(lang),
                        other => {
                            let mut s = String::new();
                            other.write_plain(&mut s);
                            s
                        }
                    })
                    .collect();
                lang.format(key, &args, out);
            }
        }
        for e in &self.extra {
            e.write_in(lang, out);
        }
    }

    fn write_plain(&self, out: &mut String) {
        match &self.content {
            Content::Literal(s) => out.push_str(s),
            Content::Raw(tag) => write_plain_nbt(tag, out),
            Content::Object { fallback, .. } => out.push_str(fallback),
            Content::Translate { key, args } if key == "chat.square_brackets" && args.len() == 1 => {
                out.push('[');
                args[0].write_plain(out);
                out.push(']');
            }
            Content::Translate { key, args } => {
                out.push_str(key);
                if !args.is_empty() {
                    out.push('[');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        a.write_plain(out);
                    }
                    out.push(']');
                }
            }
        }
        for e in &self.extra {
            e.write_plain(out);
        }
    }
}

/// A translation table in the format of `assets/minecraft/lang/*.json`.
#[derive(Debug, Clone, Default)]
pub struct Language(std::collections::HashMap<String, String>);

impl Language {
    /// Reads a language file. A JSON object of strings is also valid SNBT.
    pub fn from_json(text: &str) -> Option<Self> {
        let Ok(Tag::Compound(fields)) = crate::snbt::parse_tag(&mut crate::StringReader::new(text)) else {
            return None;
        };
        let map = fields
            .into_iter()
            .filter_map(|(k, v)| match v {
                Tag::String(s) => Some((k, s)),
                _ => None,
            })
            .collect();
        Some(Self(map))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// `TranslatableContents.decomposeTemplate`: `%s`, `%n$s` and `%%`; a malformed template
    /// or a missing argument shows the template itself. Unknown keys show the key.
    fn format(&self, key: &str, args: &[String], out: &mut String) {
        let template = self.get(key).unwrap_or(key);
        match decompose(template, args) {
            Some(s) => out.push_str(&s),
            None => out.push_str(template),
        }
    }
}

fn decompose(template: &str, args: &[String]) -> Option<String> {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let spec = &rest[at + 1..];
        let digits = spec.bytes().take_while(u8::is_ascii_digit).count();
        let (index, spec) = match spec[digits..].strip_prefix('$') {
            Some(after) if digits > 0 => (Some(spec[..digits].parse::<usize>().ok()?.checked_sub(1)?), after),
            _ => (None, spec),
        };
        match spec.chars().next() {
            Some('%') if index.is_none() => out.push('%'),
            Some('s') => {
                let i = index.unwrap_or_else(|| {
                    next += 1;
                    next - 1
                });
                out.push_str(args.get(i)?);
            }
            _ => return None,
        }
        rest = &spec[1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Plain rendering of a component in NBT form: text as-is, translations as `key[args]`.
/// A component in NBT form as `getString()` renders it with `lang`.
fn write_nbt_in(tag: &Tag, lang: &Language, out: &mut String) {
    match tag {
        Tag::Compound(_) => {
            if let Some(Tag::String(s)) = tag.get("text") {
                out.push_str(s);
            } else if let Some(Tag::String(k)) = tag.get("translate") {
                let args: Vec<String> = match tag.get("with") {
                    Some(Tag::List(args)) => args
                        .iter()
                        .map(|a| {
                            let mut s = String::new();
                            write_nbt_in(a.unwrap_list_element(), lang, &mut s);
                            s
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                match (lang.get(k), tag.get("fallback")) {
                    (None, Some(Tag::String(f))) => lang.format(f, &args, out),
                    _ => lang.format(k, &args, out),
                }
            } else {
                let mut s = String::new();
                write_plain_nbt(tag, &mut s);
                out.push_str(&s);
                return;
            }
            if let Some(Tag::List(extra)) = tag.get("extra") {
                extra.iter().for_each(|t| write_nbt_in(t.unwrap_list_element(), lang, out));
            }
        }
        Tag::List(items) => items.iter().for_each(|t| write_nbt_in(t.unwrap_list_element(), lang, out)),
        other => write_plain_nbt(other, out),
    }
}

fn write_plain_nbt(tag: &Tag, out: &mut String) {
    match tag {
        Tag::String(s) => out.push_str(s),
        Tag::List(items) => items.iter().for_each(|t| write_plain_nbt(t.unwrap_list_element(), out)),
        Tag::Compound(_) => {
            if let Some(Tag::String(s)) = tag.get("text") {
                out.push_str(s);
            } else if let Some(Tag::String(k)) = tag.get("translate") {
                out.push_str(k);
                if let Some(Tag::List(args)) = tag.get("with") {
                    out.push('[');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        write_plain_nbt(a.unwrap_list_element(), out);
                    }
                    out.push(']');
                }
            } else if let Some(Tag::String(s)) = tag.get("selector") {
                out.push_str(s);
            } else if let Some(Tag::String(k)) = tag.get("keybind") {
                out.push_str(k);
            }
            if let Some(Tag::List(extra)) = tag.get("extra") {
                extra.iter().for_each(|t| write_plain_nbt(t.unwrap_list_element(), out));
            }
        }
        Tag::Byte(v) => out.push_str(&v.to_string()),
        Tag::Short(v) => out.push_str(&v.to_string()),
        Tag::Int(v) => out.push_str(&v.to_string()),
        Tag::Long(v) => out.push_str(&v.to_string()),
        Tag::Float(v) => out.push_str(&crate::vanilla::java_float(*v)),
        Tag::Double(v) => out.push_str(&crate::vanilla::java_double(*v)),
        _ => {}
    }
}

impl Arg {
    fn to_nbt(&self) -> Tag {
        match self {
            Arg::Text(t) => t.to_nbt(),
            Arg::Str(s) => Tag::String(s.clone()),
            Arg::Int(v) => Tag::Int(*v),
            Arg::Long(v) => Tag::Long(*v),
            Arg::Float(v) => Tag::Float(*v),
            Arg::Double(v) => Tag::Double(*v),
        }
    }

    fn write_plain(&self, out: &mut String) {
        match self {
            Arg::Text(t) => t.write_plain(out),
            Arg::Str(s) => out.push_str(s),
            Arg::Int(v) => write!(out, "{v}").unwrap(),
            Arg::Long(v) => write!(out, "{v}").unwrap(),
            Arg::Float(v) => out.push_str(&crate::nbt_text::java_float(*v)),
            Arg::Double(v) => out.push_str(&crate::nbt_text::java_double(*v)),
        }
    }
}

impl From<Text> for Arg {
    fn from(t: Text) -> Self {
        Arg::Text(t)
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Str(s.to_owned())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Str(s)
    }
}

impl From<i32> for Arg {
    fn from(v: i32) -> Self {
        Arg::Int(v)
    }
}

impl From<i64> for Arg {
    fn from(v: i64) -> Self {
        Arg::Long(v)
    }
}

impl From<f32> for Arg {
    fn from(v: f32) -> Self {
        Arg::Float(v)
    }
}

impl From<f64> for Arg {
    fn from(v: f64) -> Self {
        Arg::Double(v)
    }
}

/// An NBT list as vanilla's `ListTag` writes it: elements of mixed types become compounds,
/// wrapping each non-compound as `{"": value}`.
fn list(items: Vec<Tag>) -> Tag {
    let id = |t: &Tag| std::mem::discriminant(t);
    let mixed = items.windows(2).any(|w| id(&w[0]) != id(&w[1]));
    if !mixed {
        return Tag::List(items);
    }
    Tag::List(
        items
            .into_iter()
            .map(|t| match t {
                Tag::Compound(f) if !(f.len() == 1 && f[0].0.is_empty()) => Tag::Compound(f),
                other => Tag::Compound(vec![(String::new(), other)]),
            })
            .collect(),
    )
}

/// Builds a translatable component: `tr!("key", a, b)`.
#[macro_export]
macro_rules! tr {
    ($key:expr $(, $arg:expr)* $(,)?) => {
        $crate::Text::translate($key, vec![$($crate::text::Arg::from($arg)),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_literal_is_a_string_tag() {
        assert_eq!(Text::literal("hi").to_nbt(), Tag::String("hi".into()));
    }

    #[test]
    fn translatable_with_mixed_args_wraps_like_list_tag() {
        let t = tr!("commands.give.success.single", 3, Text::literal("[Stone]").color("white"), "Steve").color("red");
        let Tag::Compound(fields) = t.to_nbt() else { panic!() };
        assert_eq!(fields[0], ("translate".into(), Tag::String("commands.give.success.single".into())));
        let Tag::List(with) = &fields[1].1 else { panic!() };
        assert_eq!(with[0], Tag::Compound(vec![(String::new(), Tag::Int(3))]));
        assert!(matches!(&with[1], Tag::Compound(f) if f[0].0 == "text"));
        assert_eq!(with[2], Tag::Compound(vec![(String::new(), Tag::String("Steve".into()))]));
        assert_eq!(fields[2], ("color".into(), Tag::String("red".into())));
    }

    #[test]
    fn homogeneous_args_stay_unwrapped() {
        let Tag::Compound(fields) = tr!("argument.integer.low", 1, 0).to_nbt() else { panic!() };
        assert_eq!(fields[1].1, Tag::List(vec![Tag::Int(1), Tag::Int(0)]));
    }

    #[test]
    fn plain_rendering() {
        let t = Text::literal("a").append(tr!("k", 1, "x")).append(Text::literal("b").bracketed());
        assert_eq!(t.to_plain(), "ak[1, x][b]");
    }

    #[test]
    fn language_rendering() {
        let lang = Language::from_json(r#"{"k": "%s and %2$s, 100%%", "bad": "%d", "chat.square_brackets": "[%s]"}"#)
            .unwrap();
        let t = tr!("k", 1, tr!("bad", 2)).append(Text::literal("b").bracketed()).append(tr!("k", 1));
        assert_eq!(t.to_string_in(&lang), "1 and %d, 100%[b]%s and %2$s, 100%%");
        assert_eq!(tr!("missing.key").to_string_in(&lang), "missing.key");
    }
}
