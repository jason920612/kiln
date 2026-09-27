//! Text components written in datapack JSON, normalized the way decoding and re-encoding with
//! `ComponentSerialization.CODEC` does: a list is its first component with the rest appended as
//! siblings, and a plain literal without style or siblings becomes a bare string.

use crate::json::Json;
use crate::parse::{PResult, ParseError};
use kiln_item::{Text, Value};

/// Style fields (`Style.Serializer.MAP_CODEC`), and the booleans among them.
const STYLE: &[&str] = &[
    "color",
    "shadow_color",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "click_event",
    "hover_event",
    "insertion",
    "font",
];
const FLAGS: &[&str] = &["bold", "italic", "underlined", "strikethrough", "obfuscated", "interpret"];

/// Decodes a component from JSON into kiln-item's [`Text`], in vanilla's canonical form.
pub fn parse(j: &Json) -> PResult<Text> {
    let v = canonical(j)?;
    Text::from_value(&v).map_err(|e| ParseError::new(e.0))
}

fn canonical(j: &Json) -> PResult<Value> {
    match j {
        Json::Str(s) => Ok(Value::String(s.clone())),
        Json::Arr(items) => {
            let Some((first, rest)) = items.split_first() else {
                return Err(ParseError::new("empty text component list"));
            };
            let mut head = expand(first)?;
            for r in rest {
                head.siblings.push(canonical(r)?);
            }
            Ok(head.encode())
        }
        Json::Obj(_) => Ok(expand(j)?.encode()),
        _ => Ok(j.to_value()),
    }
}

/// A decoded component: its content and style fields and its siblings.
struct Component {
    fields: Vec<(String, Value)>,
    siblings: Vec<Value>,
}

impl Component {
    fn encode(self) -> Value {
        let plain = self.siblings.is_empty()
            && self.fields.len() == 1
            && self.fields[0].0 == "text"
            && matches!(self.fields[0].1, Value::String(_));
        if plain {
            return self.fields.into_iter().next().unwrap().1;
        }
        let mut entries: Vec<(Value, Value)> = self.fields.into_iter().map(|(k, v)| (Value::String(k), v)).collect();
        if !self.siblings.is_empty() {
            entries.push((Value::str("extra"), Value::List(self.siblings)));
        }
        Value::Map(entries)
    }
}

fn expand(j: &Json) -> PResult<Component> {
    match j {
        Json::Str(s) => Ok(Component { fields: vec![("text".into(), Value::String(s.clone()))], siblings: Vec::new() }),
        Json::Arr(_) => {
            // A nested list decodes to one component whose siblings follow it.
            let v = canonical(j)?;
            match v {
                Value::Map(entries) => {
                    let mut fields = Vec::new();
                    let mut siblings = Vec::new();
                    for (k, v) in entries {
                        match (k.as_str().unwrap_or(""), v) {
                            ("extra", Value::List(l)) => siblings = l,
                            (k, v) => fields.push((k.to_owned(), v)),
                        }
                    }
                    Ok(Component { fields, siblings })
                }
                Value::String(s) => Ok(Component { fields: vec![("text".into(), Value::String(s))], siblings: Vec::new() }),
                other => Ok(Component { fields: vec![("text".into(), other)], siblings: Vec::new() }),
            }
        }
        Json::Obj(entries) => {
            let mut fields = Vec::new();
            let mut siblings = Vec::new();
            for (k, v) in entries {
                match k.as_str() {
                    "type" => {}
                    "extra" => {
                        let list = v.as_array().ok_or_else(|| ParseError::new("extra must be a list"))?;
                        if list.is_empty() {
                            return Err(ParseError::new("extra must not be empty"));
                        }
                        for e in list {
                            siblings.push(canonical(e)?);
                        }
                    }
                    "with" => {
                        let args = v.as_array().ok_or_else(|| ParseError::new("with must be a list"))?;
                        let args = args
                            .iter()
                            .map(|a| match a {
                                Json::Num(_) | Json::Bool(_) => Ok(a.to_value()),
                                _ => canonical(a),
                            })
                            .collect::<PResult<Vec<_>>>()?;
                        fields.push((k.clone(), Value::List(args)));
                    }
                    k if FLAGS.contains(&k) => {
                        let b = crate::parse::boolean(v).map_err(|e| e.at(k))?;
                        fields.push((k.to_owned(), Value::Bool(b)));
                    }
                    "hover_event" => fields.push((k.clone(), hover(v)?)),
                    _ => fields.push((k.clone(), v.to_value())),
                }
            }
            // Content fields come before style fields in the codec, then siblings.
            fields.sort_by_key(|(k, _)| STYLE.contains(&k.as_str()));
            Ok(Component { fields, siblings })
        }
        _ => Err(ParseError::new("invalid text component")),
    }
}

/// `HoverEvent`: `show_text` carries a component.
fn hover(j: &Json) -> PResult<Value> {
    let Json::Obj(entries) = j else { return Ok(j.to_value()) };
    let mut out = Vec::new();
    let show_text = j.get("action").and_then(Json::as_str) == Some("show_text");
    for (k, v) in entries {
        let v = if show_text && k == "value" { canonical(v)? } else { v.to_value() };
        out.push((Value::String(k.clone()), v));
    }
    Ok(Value::Map(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Text {
        parse(&Json::parse(s).unwrap()).unwrap()
    }

    #[test]
    fn plain_literals_and_lists_normalize() {
        assert_eq!(text(r#"{"text": "plain"}"#), Text::literal("plain"));
        assert_eq!(text(r#""plain""#), Text::literal("plain"));
        assert_eq!(text(r#"["a", "b"]"#), text(r#"{"text": "a", "extra": ["b"]}"#));
        assert_eq!(text(r#"{"text": "a", "extra": [{"text": "b"}]}"#), text(r#"{"text": "a", "extra": ["b"]}"#));
        assert_ne!(text(r#"{"text": "", "color": "red"}"#), Text::literal(""));
    }
}
