//! Books and signs.

use super::ComponentValue;
use super::basic::DyeColor;
use crate::text::Text;
use crate::value::{DataResult, MapBuilder, Value, err};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::{Reader, WriteExt};

/// `Filterable<T>`: raw content and the chat-filtered version, if it differs.
#[derive(Debug, Clone, PartialEq)]
pub struct Filterable<T> {
    pub raw: T,
    pub filtered: Option<T>,
}

impl<T> Filterable<T> {
    pub fn pass_through(raw: T) -> Self {
        Filterable { raw, filtered: None }
    }

    /// `Filterable.streamCodec`: the raw value, then the optional filtered one.
    pub fn read(r: &mut Reader<'_>, mut f: impl FnMut(&mut Reader<'_>) -> WireResult<T>) -> WireResult<Self> {
        let raw = f(r)?;
        Ok(Filterable { raw, filtered: wire::read_opt(r, f)? })
    }

    pub fn write(&self, out: &mut BytesMut, mut f: impl FnMut(&T, &mut BytesMut)) {
        f(&self.raw, out);
        wire::write_opt(out, &self.filtered, f);
    }

    /// `Filterable.codec`: `{raw, filtered?}`.
    pub fn to_value(&self, f: impl Fn(&T) -> Value) -> Value {
        MapBuilder::new().put("raw", f(&self.raw)).opt("filtered", self.filtered.as_ref(), &f).build()
    }

    /// The map form, or (the codec's alternative) a bare value with nothing filtered.
    pub fn from_value(v: &Value, f: impl Fn(&Value) -> DataResult<T>) -> DataResult<Self> {
        if let Value::Map(_) = v {
            let m = v.as_map()?;
            if let Some(Ok(raw)) = m.get("raw").map(&f)
                && let Ok(filtered) = m.opt("filtered", &f)
            {
                return Ok(Filterable { raw, filtered });
            }
        }
        f(v).map(Filterable::pass_through)
    }
}

fn read_string_max(max: usize) -> impl FnMut(&mut Reader<'_>) -> WireResult<String> {
    move |r| wire::read_string(r, max)
}

fn string_of_len(v: &Value, max: usize) -> DataResult<String> {
    let s = v.as_str()?;
    if s.encode_utf16().count() > max {
        return err(format!("string longer than {max} characters"));
    }
    Ok(s.to_owned())
}

/// `writable_book_content`: up to 100 pages of at most 1024 characters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WritableBookContent {
    pub pages: Vec<Filterable<String>>,
}

impl ComponentValue for WritableBookContent {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let pages = wire::read_list(r, 100, |r| Filterable::read(r, read_string_max(1024)))?;
        Ok(WritableBookContent { pages })
    }
    fn write(&self, out: &mut BytesMut) {
        wire::write_list(out, &self.pages, |p, o| p.write(o, |s, o| o.put_string(s)));
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .opt_default("pages", &self.pages, &Vec::new(), |p| {
                Value::List(p.iter().map(|page| page.to_value(|s| Value::str(s.clone()))).collect())
            })
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let pages = v.as_map()?.opt_or("pages", Vec::new(), |l| {
            let l = l.as_list()?;
            if l.len() > 100 {
                return err("more than 100 pages");
            }
            l.iter().map(|p| Filterable::from_value(p, |s| string_of_len(s, 1024))).collect()
        })?;
        Ok(WritableBookContent { pages })
    }
}

/// `written_book_content`.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenBookContent {
    /// At most 32 characters.
    pub title: Filterable<String>,
    pub author: String,
    /// 0 (original) to 3 (tattered).
    pub generation: i32,
    pub pages: Vec<Filterable<Text>>,
    /// Whether selectors and scores in the pages have been resolved.
    pub resolved: bool,
}

impl ComponentValue for WrittenBookContent {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Ok(WrittenBookContent {
            title: Filterable::read(r, read_string_max(32))?,
            author: wire::read_string(r, 32767)?,
            generation: r.varint()?,
            pages: wire::read_list(r, i32::MAX as usize, |r| Filterable::read(r, Text::read))?,
            resolved: r.bool()?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        self.title.write(out, |s, o| o.put_string(s));
        out.put_string(&self.author);
        out.put_varint(self.generation);
        wire::write_list(out, &self.pages, |p, o| p.write(o, Text::write));
        out.put_bool(self.resolved);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("title", self.title.to_value(|s| Value::str(s.clone())))
            .put("author", Value::str(self.author.clone()))
            .opt_default("generation", self.generation, 0, Value::Int)
            .opt_default("pages", &self.pages, &Vec::new(), |p| Value::List(p.iter().map(|page| page.to_value(Text::to_value)).collect()))
            .opt_default("resolved", self.resolved, false, Value::Bool)
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let generation = m.opt_or("generation", 0, Value::as_i32)?;
        if !(0..=3).contains(&generation) {
            return err(format!("generation {generation} out of range [0;3]"));
        }
        Ok(WrittenBookContent {
            title: m.req_with("title", |t| Filterable::from_value(t, |s| string_of_len(s, 32)))?,
            author: m.req_with("author", |a| a.as_str().map(str::to_owned))?,
            generation,
            pages: m.opt_or("pages", Vec::new(), |l| l.as_list()?.iter().map(|p| Filterable::from_value(p, Text::from_value)).collect())?,
            resolved: m.opt_or("resolved", false, Value::as_bool)?,
        })
    }
}

/// `sign_text_front` / `sign_text_back`: four lines, their filtered versions, dye color and glow.
#[derive(Debug, Clone, PartialEq)]
pub struct SignText {
    pub messages: [Text; 4],
    /// The same as `messages` when nothing was filtered.
    pub filtered_messages: [Text; 4],
    pub color: DyeColor,
    pub has_glowing_text: bool,
}

impl Default for SignText {
    fn default() -> Self {
        let empty = || std::array::from_fn(|_| Text::literal(""));
        SignText { messages: empty(), filtered_messages: empty(), color: DyeColor::Black, has_glowing_text: false }
    }
}

impl SignText {
    fn filtered_for_serialization(&self) -> Option<&[Text; 4]> {
        (self.filtered_messages != self.messages).then_some(&self.filtered_messages)
    }
}

fn read_lines(r: &mut Reader<'_>) -> WireResult<[Text; 4]> {
    Ok([Text::read(r)?, Text::read(r)?, Text::read(r)?, Text::read(r)?])
}

fn lines_to_value(lines: &[Text; 4]) -> Value {
    Value::List(lines.iter().map(Text::to_value).collect())
}

fn lines_from_value(v: &Value) -> DataResult<[Text; 4]> {
    let l = v.as_list()?;
    if l.len() != 4 {
        return err(format!("sign needs 4 lines, got {}", l.len()));
    }
    Ok([Text::from_value(&l[0])?, Text::from_value(&l[1])?, Text::from_value(&l[2])?, Text::from_value(&l[3])?])
}

impl ComponentValue for SignText {
    /// Four lines (no count), optional filtered lines, color, glowing.
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        let messages = read_lines(r)?;
        let filtered = wire::read_opt(r, read_lines)?;
        Ok(SignText {
            filtered_messages: filtered.unwrap_or_else(|| messages.clone()),
            messages,
            color: DyeColor::read(r)?,
            has_glowing_text: r.bool()?,
        })
    }
    fn write(&self, out: &mut BytesMut) {
        self.messages.iter().for_each(|t| t.write(out));
        out.put_bool(self.filtered_for_serialization().is_some());
        if let Some(lines) = self.filtered_for_serialization() {
            lines.iter().for_each(|t| t.write(out));
        }
        self.color.write(out);
        out.put_bool(self.has_glowing_text);
    }
    fn to_value(&self) -> Value {
        MapBuilder::new()
            .put("messages", lines_to_value(&self.messages))
            .opt("filtered_messages", self.filtered_for_serialization(), lines_to_value)
            .put("color", self.color.to_value())
            .put("has_glowing_text", Value::Bool(self.has_glowing_text))
            .build()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        let m = v.as_map()?;
        let messages = m.req_with("messages", lines_from_value)?;
        let filtered = m.lenient_or("filtered_messages", None, |l| lines_from_value(l).map(Some));
        Ok(SignText {
            filtered_messages: filtered.unwrap_or_else(|| messages.clone()),
            messages,
            color: m.opt_or("color", DyeColor::Black, DyeColor::from_value)?,
            has_glowing_text: m.opt_or("has_glowing_text", false, Value::as_bool)?,
        })
    }
}
