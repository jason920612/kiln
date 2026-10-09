//! Text components as `ComponentArgument` reads them (`/tellraw`): SNBT decoded with the
//! rules of `ComponentSerialization.CODEC` (a string, a list, or a compound with one kind of
//! content, style fields and `extra`), resolved against the source (`ComponentUtils.resolve`:
//! selectors become names, scores their values) and encoded back to network NBT.

use crate::error::CommandError;
use crate::reader::StringReader;
use crate::selector::{EntitySelector, SelectorTarget, SelectorWorld};
use crate::snbt;
use crate::text::{Arg, Text};
use crate::tr;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// A decoded component.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub content: Contents,
    /// Style fields in vanilla's encoding order, already validated.
    pub style: Vec<(&'static str, Tag)>,
    pub extra: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Contents {
    Text(String),
    Translate { key: String, fallback: Option<String>, with: Vec<TranslateArg> },
    Score { name: String, objective: String },
    Selector { pattern: String, selector: Box<EntitySelector>, separator: Option<Box<Component>> },
    Keybind(String),
    /// `nbt` contents; kept as written (resolving them needs block entity, entity or storage
    /// data).
    Nbt(Vec<(String, Tag)>),
    /// `object` contents (sprites, player heads), passed through.
    Object(Vec<(String, Tag)>),
}

/// A `with` argument: a component or a primitive kept as is.
#[derive(Debug, Clone, PartialEq)]
pub enum TranslateArg {
    Component(Component),
    Primitive(Tag),
}

/// `ChatFormatting` colors by name.
const NAMED_COLORS: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

const BOOL_STYLES: [&str; 5] = ["bold", "italic", "underlined", "strikethrough", "obfuscated"];

/// `ComponentArgument.parse`: SNBT, then the component codec.
/// Codec failures point at the start of the argument (`CommandArgumentParser.withCodec`).
pub fn parse(reader: &mut StringReader) -> Result<Component> {
    let start = reader.cursor();
    let tag = snbt::parse_tag(reader)?;
    decode(&tag).map_err(|message| {
        reader.set_cursor(start);
        CommandError::new(tr!("argument.component.invalid", message)).at(reader)
    })
}

fn invalid<T>(message: impl Into<String>) -> std::result::Result<T, String> {
    Err(message.into())
}

/// `ComponentSerialization.CODEC.parse(NbtOps, tag)`.
pub fn decode(tag: &Tag) -> std::result::Result<Component, String> {
    decode_depth(tag, 0)
}

/// The codec is `either(either(string, nonEmptyList(listOf(component))), map)`; failures read
/// like DataFixerUpper's (`Failed to parse either. First: ...; Second: ...`), except that an
/// `either` whose branch failed with a partial result reports only that branch.
fn decode_depth(tag: &Tag, depth: usize) -> std::result::Result<Component, String> {
    if depth > 512 {
        return invalid("Component nested too deeply");
    }
    if let Tag::String(s) = tag {
        return Ok(Component::text(s));
    }
    let list = match list_elements(tag) {
        Some(items) => match decode_list(&items, depth) {
            Ok(c) => return Ok(c),
            Err((message, true)) => return Err(message),
            Err((message, false)) => message,
        },
        None => format!("Not a list: {}", snbt::to_snbt(tag)),
    };
    let map = match tag {
        Tag::Compound(fields) => match decode_compound(fields, depth) {
            Ok(c) => return Ok(c),
            Err((message, true)) => return Err(message),
            Err((message, false)) => message,
        },
        _ => format!("Not a map: {}", snbt::to_snbt(tag)),
    };
    Err(format!("Failed to parse either. First: Failed to parse either. First: Not a string; Second: {list}; Second: {map}"))
}

/// `NbtOps.getList`: lists and typed arrays.
fn list_elements(tag: &Tag) -> Option<Vec<Tag>> {
    Some(match tag {
        Tag::List(items) => items.iter().map(|t| t.unwrap_list_element().clone()).collect(),
        Tag::ByteArray(v) => v.iter().map(|&b| Tag::Byte(b)).collect(),
        Tag::IntArray(v) => v.iter().map(|&i| Tag::Int(i)).collect(),
        Tag::LongArray(v) => v.iter().map(|&l| Tag::Long(l)).collect(),
        _ => return None,
    })
}

/// `ExtraCodecs.nonEmptyList(listOf)`: the first element with the rest as siblings. Errors
/// carry whether some elements decoded (a partial result); messages of later elements come
/// first, as `DataResult.apply2stable` joins them.
fn decode_list(items: &[Tag], depth: usize) -> std::result::Result<Component, (String, bool)> {
    let mut parts = Vec::new();
    let mut error: Option<String> = None;
    for item in items {
        match decode_depth(item, depth + 1) {
            Ok(c) => parts.push(c),
            Err(e) => error = Some(error.map_or(e.clone(), |prev| format!("{e}; {prev}"))),
        }
    }
    const EMPTY: &str = "List must have contents";
    match (error, parts.is_empty()) {
        (Some(e), false) => Err((e, true)),
        (Some(e), true) => Err((format!("{e}; {EMPTY}"), false)),
        (None, true) => Err((EMPTY.to_owned(), false)),
        (None, false) => {
            let mut parts = parts.into_iter();
            let mut first = parts.next().expect("non-empty");
            first.extra.extend(parts);
            Ok(first)
        }
    }
}

fn field<'t>(fields: &'t [(String, Tag)], key: &str) -> Option<&'t Tag> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn string_field(fields: &[(String, Tag)], key: &str) -> std::result::Result<Option<String>, String> {
    match field(fields, key) {
        None => Ok(None),
        Some(Tag::String(s)) => Ok(Some(s.clone())),
        Some(other) => invalid(format!("Not a string: {}", snbt::to_snbt(other))),
    }
}

fn bool_value(tag: &Tag) -> Option<bool> {
    match tag {
        Tag::Byte(v) => Some(*v != 0),
        Tag::Short(v) => Some(*v != 0),
        Tag::Int(v) => Some(*v != 0),
        Tag::Long(v) => Some(*v != 0),
        Tag::Float(v) => Some(*v != 0.0),
        Tag::Double(v) => Some(*v != 0.0),
        _ => None,
    }
}

/// The component map codec: contents, then style and siblings. Contents failures carry no
/// partial result (the caller's `either` wraps them); style and sibling failures do.
fn decode_compound(fields: &[(String, Tag)], depth: usize) -> std::result::Result<Component, (String, bool)> {
    // `ComponentSerialization.createLegacyComponentMatcher`: an explicit `type`, else the
    // first contents type that decodes.
    let content = match string_field(fields, "type").map_err(|e| (e, false))? {
        Some(kind) => decode_contents(fields, &kind, depth).map_err(|e| (e, false))?,
        None => ["text", "translatable", "score", "selector", "keybind", "nbt", "object"]
            .into_iter()
            .find_map(|kind| decode_contents(fields, kind, depth).ok())
            .ok_or_else(|| ("No matching codec found".to_owned(), false))?,
    };
    let style = decode_style(fields, depth).map_err(|e| (e, true))?;
    let extra = match field(fields, "extra") {
        None => Vec::new(),
        Some(Tag::List(items)) if !items.is_empty() => items
            .iter()
            .map(|t| decode_depth(t.unwrap_list_element(), depth + 1))
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| (e, true))?,
        Some(Tag::List(_)) => return Err(("List must have contents".to_owned(), true)),
        Some(other) => return Err((format!("Not a list: {}", snbt::to_snbt(other)), true)),
    };
    Ok(Component { content, style, extra })
}

/// One contents type's map codec.
fn decode_contents(fields: &[(String, Tag)], kind: &str, depth: usize) -> std::result::Result<Contents, String> {
    let missing = |key: &str| format!("No key {key} in MapLike[{}]", snbt::to_snbt(&Tag::Compound(fields.to_vec())));
    Ok(match kind {
        "text" => Contents::Text(string_field(fields, "text")?.ok_or_else(|| missing("text"))?),
        "translatable" => {
            let key = string_field(fields, "translate")?.ok_or("No key translate in MapLike")?;
            let fallback = string_field(fields, "fallback")?;
            let with = match field(fields, "with") {
                None => Vec::new(),
                Some(Tag::List(items)) => items
                    .iter()
                    .map(|t| match t.unwrap_list_element() {
                        p @ (Tag::Byte(_)
                        | Tag::Short(_)
                        | Tag::Int(_)
                        | Tag::Long(_)
                        | Tag::Float(_)
                        | Tag::Double(_)) => Ok(TranslateArg::Primitive(p.clone())),
                        other => decode_depth(other, depth + 1).map(TranslateArg::Component),
                    })
                    .collect::<std::result::Result<_, _>>()?,
                Some(other) => return invalid(format!("Not a list: {}", snbt::to_snbt(other))),
            };
            Contents::Translate { key, fallback, with }
        }
        "score" => {
            let Some(Tag::Compound(score)) = field(fields, "score") else { return invalid("No key score in MapLike") };
            let name = string_field(score, "name")?.ok_or("No key name in MapLike")?;
            let objective = string_field(score, "objective")?.ok_or("No key objective in MapLike")?;
            Contents::Score { name, objective }
        }
        "selector" => {
            let pattern = string_field(fields, "selector")?.ok_or("No key selector in MapLike")?;
            let selector = EntitySelector::parse(&mut StringReader::new(&pattern), true)
                .map_err(|e| format!("Invalid selector component: {pattern}: {}", e.message().to_plain()))?;
            let separator = field(fields, "separator").map(|t| decode_depth(t, depth + 1)).transpose()?.map(Box::new);
            Contents::Selector { pattern, selector: Box::new(selector), separator }
        }
        "keybind" => Contents::Keybind(string_field(fields, "keybind")?.ok_or("No key keybind in MapLike")?),
        "nbt" => {
            string_field(fields, "nbt")?.ok_or_else(|| missing("nbt"))?;
            if !["block", "entity", "storage"].iter().any(|k| field(fields, k).is_some()) {
                return invalid("No matching codec found");
            }
            let keep: Vec<(String, Tag)> = fields
                .iter()
                .filter(|(k, _)| matches!(k.as_str(), "nbt" | "interpret" | "separator" | "block" | "entity" | "storage" | "source" | "plain"))
                .cloned()
                .collect();
            Contents::Nbt(keep)
        }
        "object" => {
            string_field(fields, "object")?.ok_or_else(|| missing("object"))?;
            let keep: Vec<(String, Tag)> =
                fields.iter().filter(|(k, _)| !is_style_key(k) && k != "extra" && k != "type").cloned().collect();
            Contents::Object(keep)
        }
        other => return invalid(format!("Unknown component type: {other}")),
    })
}

fn is_style_key(k: &str) -> bool {
    matches!(
        k,
        "color"
            | "shadow_color"
            | "bold"
            | "italic"
            | "underlined"
            | "strikethrough"
            | "obfuscated"
            | "click_event"
            | "hover_event"
            | "insertion"
            | "font"
    )
}

/// `Style.Serializer.MAP_CODEC`.
fn decode_style(fields: &[(String, Tag)], depth: usize) -> std::result::Result<Vec<(&'static str, Tag)>, String> {
    let mut style: Vec<(&'static str, Tag)> = Vec::new();
    if let Some(color) = string_field(fields, "color")? {
        let valid = NAMED_COLORS.contains(&color.as_str())
            || color.strip_prefix('#').is_some_and(|h| !h.is_empty() && h.len() <= 8 && u32::from_str_radix(h, 16).is_ok());
        if !valid {
            return invalid(format!("Invalid color name: {color}"));
        }
        // `TextColor.serialize`: a color by name, any other as `#RRGGBB` in capitals.
        let color = match color.strip_prefix('#').and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(rgb) if rgb <= 0xFF_FFFF => format!("#{rgb:06X}"),
            _ => color,
        };
        style.push(("color", Tag::String(color)));
    }
    if let Some(shadow) = field(fields, "shadow_color") {
        match shadow {
            Tag::Int(_) => style.push(("shadow_color", shadow.clone())),
            Tag::List(l) if l.len() == 4 => style.push(("shadow_color", shadow.clone())),
            other => return invalid(format!("Not a number: {}", snbt::to_snbt(other))),
        }
    }
    for key in BOOL_STYLES {
        if let Some(v) = field(fields, key) {
            let b = bool_value(v).ok_or_else(|| format!("Not a boolean: {}", snbt::to_snbt(v)))?;
            style.push((key, Tag::Byte(b as i8)));
        }
    }
    if let Some(click) = field(fields, "click_event") {
        style.push(("click_event", decode_click(click)?));
    }
    if let Some(hover) = field(fields, "hover_event") {
        style.push(("hover_event", decode_hover(hover, depth)?));
    }
    if let Some(insertion) = string_field(fields, "insertion")? {
        style.push(("insertion", Tag::String(insertion)));
    }
    if let Some(font) = field(fields, "font") {
        style.push(("font", font.clone()));
    }
    Ok(style)
}

fn decode_click(tag: &Tag) -> std::result::Result<Tag, String> {
    let Tag::Compound(f) = tag else { return invalid(format!("Not a map: {}", snbt::to_snbt(tag))) };
    let action = string_field(f, "action")?.ok_or("No key action in MapLike")?;
    let required: &[(&str, bool)] = match action.as_str() {
        "open_url" => &[("url", true)],
        "open_file" => &[("path", true)],
        "run_command" | "suggest_command" => &[("command", true)],
        "change_page" => &[("page", false)],
        "copy_to_clipboard" => &[("value", true)],
        "show_dialog" => &[("dialog", false)],
        "custom" => &[("id", true)],
        other => return invalid(format!("Unknown element name:{other}")),
    };
    let mut out = vec![("action".to_owned(), Tag::String(action.clone()))];
    for &(key, is_string) in required {
        let v = field(f, key).ok_or_else(|| format!("No key {key} in MapLike"))?;
        if is_string && !matches!(v, Tag::String(_)) {
            return invalid(format!("Not a string: {}", snbt::to_snbt(v)));
        }
        if action == "change_page" && !matches!(v, Tag::Int(p) if *p > 0) {
            return invalid(format!("Value must be positive: {}", snbt::to_snbt(v)));
        }
        out.push((key.to_owned(), v.clone()));
    }
    if action == "custom"
        && let Some(p) = field(f, "payload")
    {
        out.push(("payload".to_owned(), p.clone()));
    }
    Ok(Tag::Compound(out))
}

fn decode_hover(tag: &Tag, depth: usize) -> std::result::Result<Tag, String> {
    let Tag::Compound(f) = tag else { return invalid(format!("Not a map: {}", snbt::to_snbt(tag))) };
    let action = string_field(f, "action")?.ok_or("No key action in MapLike")?;
    match action.as_str() {
        "show_text" => {
            let value = field(f, "value").ok_or("No key value in MapLike")?;
            let c = decode_depth(value, depth + 1)?;
            Ok(Tag::Compound(vec![("action".into(), Tag::String(action)), ("value".into(), c.to_nbt())]))
        }
        "show_item" | "show_entity" => {
            string_field(f, "id")?.ok_or("No key id in MapLike")?;
            Ok(tag.clone())
        }
        other => invalid(format!("Unknown element name:{other}")),
    }
}

impl Component {
    pub fn text(s: impl Into<String>) -> Self {
        Component { content: Contents::Text(s.into()), style: Vec::new(), extra: Vec::new() }
    }

    /// Network NBT (`ComponentSerialization.CODEC.encode`): a bare string for plain text.
    pub fn to_nbt(&self) -> Tag {
        if let (Contents::Text(s), true, true) = (&self.content, self.style.is_empty(), self.extra.is_empty()) {
            return Tag::String(s.clone());
        }
        let mut fields: Vec<(String, Tag)> = Vec::new();
        match &self.content {
            Contents::Text(s) => fields.push(("text".into(), Tag::String(s.clone()))),
            Contents::Translate { key, fallback, with } => {
                fields.push(("translate".into(), Tag::String(key.clone())));
                if let Some(f) = fallback {
                    fields.push(("fallback".into(), Tag::String(f.clone())));
                }
                if !with.is_empty() {
                    let args = with
                        .iter()
                        .map(|a| match a {
                            TranslateArg::Component(c) => c.to_nbt(),
                            TranslateArg::Primitive(p) => p.clone(),
                        })
                        .collect();
                    fields.push(("with".into(), Tag::heterogeneous_list(args)));
                }
            }
            Contents::Score { name, objective } => fields.push((
                "score".into(),
                Tag::Compound(vec![
                    ("name".into(), Tag::String(name.clone())),
                    ("objective".into(), Tag::String(objective.clone())),
                ]),
            )),
            Contents::Selector { pattern, separator, .. } => {
                fields.push(("selector".into(), Tag::String(pattern.clone())));
                if let Some(s) = separator {
                    fields.push(("separator".into(), s.to_nbt()));
                }
            }
            Contents::Keybind(k) => fields.push(("keybind".into(), Tag::String(k.clone()))),
            Contents::Nbt(f) | Contents::Object(f) => fields.extend(f.iter().cloned()),
        }
        for (k, v) in &self.style {
            fields.push(((*k).to_owned(), v.clone()));
        }
        if !self.extra.is_empty() {
            fields.push(("extra".into(), Tag::heterogeneous_list(self.extra.iter().map(Component::to_nbt).collect())));
        }
        Tag::Compound(fields)
    }

    /// `ComponentUtils.resolve`: selector and score contents become text for `world`'s source
    /// (with `self_entity` as `@s`, e.g. the `/tellraw` recipient); `nbt` contents resolve to
    /// empty text, as for missing data.
    pub fn resolve<W: SelectorWorld>(&self, world: &mut W, self_entity: Option<&W::Entity>) -> Result<Component> {
        self.resolve_depth(world, self_entity, 0)
    }

    fn resolve_depth<W: SelectorWorld>(&self, world: &mut W, me: Option<&W::Entity>, depth: usize) -> Result<Component> {
        if depth > 100 {
            return Ok(self.clone());
        }
        let resolved: Option<Component> = match &self.content {
            Contents::Selector { selector, separator, .. } => {
                let entities = with_self(world, me, |w| selector.find_entities(w))?;
                let separator = match separator {
                    Some(s) => s.resolve_depth(world, me, depth + 1)?,
                    None => Component {
                        content: Contents::Text(", ".into()),
                        style: vec![("color", Tag::String("gray".into()))],
                        extra: Vec::new(),
                    },
                };
                let mut out = Component::text("");
                for (i, e) in entities.iter().enumerate() {
                    if i > 0 {
                        out.extra.push(separator.clone());
                    }
                    out.extra.push(from_text(&e.display_name()));
                }
                Some(out)
            }
            Contents::Score { name, objective } => {
                let holder = if name == "*" {
                    me.map(SelectorTarget::scoreboard_name).or_else(|| world.source_entity().map(|e| e.scoreboard_name()))
                } else if name.starts_with('@') {
                    let sel = EntitySelector::parse(&mut StringReader::new(name), true)?;
                    let found = with_self(world, me, |w| sel.find_entities(w))?;
                    match found.as_slice() {
                        [one] => Some(one.scoreboard_name()),
                        [] => None,
                        _ => return Err(CommandError::not_single_entity()),
                    }
                } else {
                    Some(name.clone())
                };
                let value = holder.and_then(|h| world.scoreboard().and_then(|sb| sb.score(&h, objective)));
                Some(Component::text(value.map(|v| v.to_string()).unwrap_or_default()))
            }
            Contents::Nbt(_) => Some(Component::text("")),
            _ => None,
        };
        let mut out = match resolved {
            Some(mut c) => {
                let mut style = self.style.clone();
                for (k, v) in c.style.drain(..) {
                    if !style.iter().any(|(sk, _)| *sk == k) {
                        style.push((k, v));
                    }
                }
                c.style = style;
                c
            }
            None => {
                let content = match &self.content {
                    Contents::Translate { key, fallback, with } => Contents::Translate {
                        key: key.clone(),
                        fallback: fallback.clone(),
                        with: with
                            .iter()
                            .map(|a| match a {
                                TranslateArg::Component(c) => {
                                    c.resolve_depth(world, me, depth + 1).map(TranslateArg::Component)
                                }
                                p => Ok(p.clone()),
                            })
                            .collect::<Result<_>>()?,
                    },
                    other => other.clone(),
                };
                Component { content, style: self.style.clone(), extra: Vec::new() }
            }
        };
        for e in &self.extra {
            out.extra.push(e.resolve_depth(world, me, depth + 1)?);
        }
        Ok(out)
    }

    /// As a [`Text`] for hosts, keeping the exact NBT.
    pub fn to_text(&self) -> Text {
        Text::raw(self.to_nbt())
    }
}

/// Runs `f` with `me` as the source entity (`ResolutionContext.withEntityOverride`).
fn with_self<W: SelectorWorld, T>(
    world: &mut W,
    me: Option<&W::Entity>,
    f: impl FnOnce(&mut W) -> Result<T>,
) -> Result<T> {
    let Some(me) = me else { return f(world) };
    let previous = world.stack_mut().entity.replace(me.clone());
    let r = f(world);
    world.stack_mut().entity = previous;
    r
}

/// A component equivalent to an entity's display name.
fn from_text(t: &Text) -> Component {
    match t.to_nbt() {
        Tag::String(s) => Component::text(s),
        other => decode(&other).unwrap_or_else(|_| Component::text(t.to_plain())),
    }
}

/// `argument.component.invalid` with a message, for callers validating components.
pub fn invalid_component(message: &str) -> CommandError {
    CommandError::new(tr!("argument.component.invalid", message))
}

impl From<Component> for Arg {
    fn from(c: Component) -> Self {
        Arg::Text(c.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Result<Component> {
        parse(&mut StringReader::new(s))
    }

    #[test]
    fn forms() {
        assert_eq!(c("\"hello\"").unwrap().to_nbt(), Tag::String("hello".into()));
        assert_eq!(c("hi").unwrap().to_nbt(), Tag::String("hi".into()));
        let t = c("{text:'a',color:red,bold:1b,extra:[\"b\",{text:c,italic:true}]}").unwrap();
        let Tag::Compound(f) = t.to_nbt() else { panic!() };
        assert_eq!(f[0], ("text".into(), Tag::String("a".into())));
        assert_eq!(f[1], ("color".into(), Tag::String("red".into())));
        assert_eq!(f[2], ("bold".into(), Tag::Byte(1)));
        assert_eq!(t.extra.len(), 2);
        let l = c("['a', {text:'b'}]").unwrap();
        assert_eq!(l.extra.len(), 1);
        let j = c(r#"{"translate":"chat.type.text","with":["A",{"text":"B"},3]}"#).unwrap();
        assert!(matches!(j.content, Contents::Translate { ref with, .. } if with.len() == 3));
        let s = c("{selector:'@a',separator:{text:'|'}}").unwrap();
        assert!(matches!(s.content, Contents::Selector { .. }));
        let k = c("{type:'keybind',keybind:'key.jump'}").unwrap();
        assert_eq!(k.content, Contents::Keybind("key.jump".into()));
    }

    #[test]
    fn invalid_components() {
        let key = |s: &str| c(s).unwrap_err().key().unwrap().to_owned();
        assert_eq!(key("{color:red}"), "argument.component.invalid");
        assert_eq!(key("{text:a,color:reddish}"), "argument.component.invalid");
        assert_eq!(key("[]"), "argument.component.invalid");
        assert_eq!(key("1"), "argument.component.invalid");
        assert_eq!(key("{text:a,click_event:{action:nope}}"), "argument.component.invalid");
        assert_eq!(key("{selector:'@q'}"), "argument.component.invalid");
        assert_eq!(key("{text:"), "snbt.parser.expected_unquoted_string");
    }
}
