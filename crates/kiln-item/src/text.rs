//! Text components (`net.minecraft.network.chat.Component`) as item components carry them.
//!
//! A [`Text`] holds the component's canonical NBT (what `ComponentSerialization.CODEC` writes
//! with `NbtOps`, and what its network codec sends), so it round-trips byte-exactly. It is not
//! parsed into a typed tree; [`Text::to_value`] applies the codec's schema where NBT is
//! ambiguous (booleans, embedded item stacks) so hashes match vanilla's. Non-canonical input
//! (e.g. `{"text":"a"}` instead of `"a"`) is kept as sent rather than normalized.

use crate::value::{DataError, DataResult, Value};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::nbt::Tag;
use kiln_proto::{DecodeError, Reader};

#[derive(Debug, Clone, PartialEq)]
pub struct Text(Tag);

/// Style and content flags that the codec reads as booleans.
const BOOL_KEYS: &[&str] = &["bold", "italic", "underlined", "strikethrough", "obfuscated", "interpret", "plain", "hat"];

impl Text {
    /// A plain string (how the codec writes unstyled literal text).
    pub fn literal(s: impl Into<String>) -> Text {
        Text(Tag::String(s.into()))
    }

    /// `{"translate": key}`.
    pub fn translatable(key: impl Into<String>) -> Text {
        Text(Tag::Compound(vec![("translate".into(), Tag::String(key.into()))]))
    }

    /// Wraps NBT that decodes as a component: a string, a compound or a non-empty list.
    pub fn from_nbt(tag: Tag) -> Option<Text> {
        match &tag {
            Tag::String(_) | Tag::Compound(_) => Some(Text(tag)),
            Tag::List(items) if !items.is_empty() => Some(Text(tag)),
            _ => None,
        }
    }

    pub fn nbt(&self) -> &Tag {
        &self.0
    }

    pub fn into_nbt(self) -> Tag {
        self.0
    }

    /// The literal string when this is plain text.
    pub fn as_plain(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// `ComponentSerialization.STREAM_CODEC`: network NBT.
    pub fn read(r: &mut Reader<'_>) -> WireResult<Text> {
        Text::from_nbt(wire::read_nbt(r)?).ok_or(DecodeError::Invalid("text component"))
    }

    pub fn write(&self, out: &mut BytesMut) {
        self.0.write_network(out);
    }

    pub fn to_value(&self) -> Value {
        typed(Value::from_nbt(&self.0))
    }

    pub fn from_value(v: &Value) -> DataResult<Text> {
        Text::from_nbt(v.to_nbt()).ok_or_else(|| DataError("not a text component".into()))
    }
}

/// Re-types a text component value read from NBT the way the codec would have produced it.
fn typed(v: Value) -> Value {
    match v {
        Value::List(items) => Value::List(items.into_iter().map(typed).collect()),
        Value::Map(entries) => Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| {
                    let v = match k.as_str().unwrap_or("") {
                        key if BOOL_KEYS.contains(&key) => match v {
                            Value::Byte(b) => Value::Bool(b != 0),
                            other => other,
                        },
                        "extra" | "separator" | "fallback" | "name" => typed(v),
                        "with" => match v {
                            // Arguments are components or primitives; primitives stay as they are.
                            Value::List(args) => {
                                Value::List(args.into_iter().map(|a| if matches!(a, Value::Map(_) | Value::List(_)) { typed(a) } else { a }).collect())
                            }
                            other => other,
                        },
                        "hover_event" => hover(v),
                        _ => v,
                    };
                    (k, v)
                })
                .collect(),
        ),
        other => other,
    }
}

fn hover(v: Value) -> Value {
    let Value::Map(entries) = v else { return v };
    let action = entries.iter().find(|(k, _)| k.as_str() == Ok("action")).and_then(|(_, v)| v.as_str().ok()).map(str::to_owned);
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| {
                let v = match (action.as_deref(), k.as_str().unwrap_or("")) {
                    (Some("show_text"), "value") | (Some("show_entity"), "name") => typed(v),
                    (Some("show_item"), "components") => crate::patch::DataComponentPatch::from_value(&v)
                        .map(|p| p.to_value())
                        .unwrap_or(v),
                    _ => v,
                };
                (k, v)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_are_typed_for_hashing() {
        let tag = Tag::Compound(vec![
            ("text".into(), Tag::String("a".into())),
            ("bold".into(), Tag::Byte(1)),
            ("extra".into(), Tag::List(vec![Tag::Compound(vec![("italic".into(), Tag::Byte(0))])])),
        ]);
        let v = Text::from_nbt(tag.clone()).unwrap().to_value();
        let m = v.as_map().unwrap();
        assert_eq!(m.get("bold"), Some(&Value::Bool(true)));
        assert_eq!(m.get("extra").unwrap().as_list().unwrap()[0].as_map().unwrap().get("italic"), Some(&Value::Bool(false)));
        // NBT reorders the keys, which hashing ignores.
        assert_eq!(crate::hash::hash(&Text::from_value(&v).unwrap().to_value()), crate::hash::hash(&v));
    }
}
