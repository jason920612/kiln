//! [`Value`]: a data component value in the form vanilla's persistent codecs (`Codec`) produce,
//! independent of the target format. The same tree serves three purposes:
//! - NBT (`NbtOps`), for playerdata and chunks: [`Value::to_nbt`] / [`Value::from_nbt`];
//! - the per-component hashes of `container_click` (`HashOps`): [`crate::hash::hash`];
//! - values whose network form is NBT of the codec (`ByteBufCodecs.fromCodec`).
//!
//! It keeps the distinctions `DynamicOps` makes that NBT loses: booleans versus bytes, and
//! lists versus primitive streams (`createIntList`, ...), which hash differently.

use crate::javamap;
use kiln_proto::nbt::Tag;
use std::borrow::Cow;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `ops.empty()`.
    Empty,
    Bool(bool),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    List(Vec<Value>),
    /// Entries in encoding order (keys are strings for everything NBT can hold).
    Map(Vec<(Value, Value)>),
    /// `createByteList` (NBT byte array).
    ByteList(Vec<i8>),
    /// `createIntList` (NBT int array), e.g. UUIDs.
    IntList(Vec<i32>),
    /// `createLongList` (NBT long array).
    LongList(Vec<i64>),
}

/// A persistent-codec decoding failure (the value does not match the component's schema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError(pub String);

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DataError {}

pub type DataResult<T> = Result<T, DataError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> DataResult<T> {
    Err(DataError(msg.into()))
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::String(s.into())
    }

    pub fn empty_map() -> Value {
        Value::Map(Vec::new())
    }

    /// NBT as `NbtOps` would produce it: compounds in `HashMap` order, booleans as bytes,
    /// mixed lists wrapped as `{"": value}`.
    pub fn to_nbt(&self) -> Tag {
        match self {
            Value::Empty => Tag::Compound(Vec::new()),
            Value::Bool(b) => Tag::Byte(*b as i8),
            Value::Byte(v) => Tag::Byte(*v),
            Value::Short(v) => Tag::Short(*v),
            Value::Int(v) => Tag::Int(*v),
            Value::Long(v) => Tag::Long(*v),
            Value::Float(v) => Tag::Float(*v),
            Value::Double(v) => Tag::Double(*v),
            Value::String(s) => Tag::String(s.clone()),
            Value::List(items) => Tag::heterogeneous_list(items.iter().map(Value::to_nbt).collect()),
            Value::Map(entries) => {
                // Later duplicates replace earlier ones in place, as in a HashMap.
                let mut fields: Vec<(String, Tag)> = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let key = match k {
                        Value::String(s) => s.clone(),
                        other => {
                            debug_assert!(false, "non-string map key {other:?} in NBT");
                            format!("{other:?}")
                        }
                    };
                    let tag = v.to_nbt();
                    match fields.iter_mut().find(|(n, _)| *n == key) {
                        Some(slot) => slot.1 = tag,
                        None => fields.push((key, tag)),
                    }
                }
                let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
                let order = javamap::iteration_order(&keys);
                let mut slots: Vec<Option<(String, Tag)>> = fields.into_iter().map(Some).collect();
                Tag::Compound(order.into_iter().map(|i| slots[i].take().unwrap()).collect())
            }
            Value::ByteList(v) => Tag::ByteArray(v.clone()),
            Value::IntList(v) => Tag::IntArray(v.clone()),
            Value::LongList(v) => Tag::LongArray(v.clone()),
        }
    }

    /// The value `NbtOps` presents for `tag` (list wrappers `{"": value}` removed, as `ListTag`
    /// does when reading). Bytes stay bytes; decoders accept them where booleans are expected.
    pub fn from_nbt(tag: &Tag) -> Value {
        match tag {
            Tag::Byte(v) => Value::Byte(*v),
            Tag::Short(v) => Value::Short(*v),
            Tag::Int(v) => Value::Int(*v),
            Tag::Long(v) => Value::Long(*v),
            Tag::Float(v) => Value::Float(*v),
            Tag::Double(v) => Value::Double(*v),
            Tag::ByteArray(v) => Value::ByteList(v.clone()),
            Tag::String(s) => Value::String(s.clone()),
            Tag::List(items) => {
                let compounds = items.first().is_some_and(|t| matches!(t, Tag::Compound(_)));
                Value::List(
                    items.iter().map(|t| Value::from_nbt(if compounds { t.unwrap_list_element() } else { t })).collect(),
                )
            }
            Tag::Compound(fields) => {
                Value::Map(fields.iter().map(|(k, v)| (Value::String(k.clone()), Value::from_nbt(v))).collect())
            }
            Tag::IntArray(v) => Value::IntList(v.clone()),
            Tag::LongArray(v) => Value::LongList(v.clone()),
        }
    }

    // ---- lenient accessors, as NbtOps and the primitive codecs read values ----

    fn kind(&self) -> &'static str {
        match self {
            Value::Empty => "empty",
            Value::Bool(_) => "boolean",
            Value::Byte(_) | Value::Short(_) | Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Double(_) => {
                "number"
            }
            Value::String(_) => "string",
            Value::List(_) | Value::ByteList(_) | Value::IntList(_) | Value::LongList(_) => "list",
            Value::Map(_) => "map",
        }
    }

    fn not(&self, what: &str) -> DataError {
        DataError(format!("expected {what}, found {}", self.kind()))
    }

    /// `Number.doubleValue()` of a numeric value.
    pub fn as_f64(&self) -> DataResult<f64> {
        Ok(match *self {
            Value::Byte(v) => v as f64,
            Value::Short(v) => v as f64,
            Value::Int(v) => v as f64,
            Value::Long(v) => v as f64,
            Value::Float(v) => v as f64,
            Value::Double(v) => v,
            _ => return Err(self.not("number")),
        })
    }

    /// `Number.floatValue()`.
    pub fn as_f32(&self) -> DataResult<f32> {
        Ok(match *self {
            Value::Float(v) => v,
            Value::Double(v) => v as f32,
            Value::Long(v) => v as f32,
            _ => self.as_f64()? as f32,
        })
    }

    /// `Number.longValue()` (floats saturate, as Java's casts do).
    pub fn as_i64(&self) -> DataResult<i64> {
        Ok(match *self {
            Value::Byte(v) => v as i64,
            Value::Short(v) => v as i64,
            Value::Int(v) => v as i64,
            Value::Long(v) => v,
            Value::Float(v) => v as i64,
            Value::Double(v) => v as i64,
            _ => return Err(self.not("number")),
        })
    }

    /// `Number.intValue()` (longs wrap, floats saturate).
    pub fn as_i32(&self) -> DataResult<i32> {
        Ok(match *self {
            Value::Float(v) => v as i32,
            Value::Double(v) => v as i32,
            _ => self.as_i64()? as i32,
        })
    }

    /// `Number.shortValue()`.
    pub fn as_i16(&self) -> DataResult<i16> {
        Ok(self.as_i32()? as i16)
    }

    /// `Number.byteValue()`.
    pub fn as_i8(&self) -> DataResult<i8> {
        Ok(self.as_i32()? as i8)
    }

    /// A boolean, or a number whose byte value is non-zero (`NbtOps.getBooleanValue`).
    pub fn as_bool(&self) -> DataResult<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            _ => Ok(self.as_i8().map_err(|_| self.not("boolean"))? != 0),
        }
    }

    pub fn as_str(&self) -> DataResult<&str> {
        match self {
            Value::String(s) => Ok(s),
            _ => Err(self.not("string")),
        }
    }

    /// The elements of a list or primitive array.
    pub fn as_list(&self) -> DataResult<Cow<'_, [Value]>> {
        Ok(match self {
            Value::List(v) => Cow::Borrowed(v),
            Value::ByteList(v) => Cow::Owned(v.iter().map(|&x| Value::Byte(x)).collect()),
            Value::IntList(v) => Cow::Owned(v.iter().map(|&x| Value::Int(x)).collect()),
            Value::LongList(v) => Cow::Owned(v.iter().map(|&x| Value::Long(x)).collect()),
            _ => return Err(self.not("list")),
        })
    }

    /// An int stream (`Codec.INT_STREAM`): an int array or a list of numbers.
    pub fn as_int_stream(&self) -> DataResult<Vec<i32>> {
        match self {
            Value::IntList(v) => Ok(v.clone()),
            Value::List(_) | Value::ByteList(_) | Value::LongList(_) => {
                self.as_list()?.iter().map(Value::as_i32).collect()
            }
            _ => Err(self.not("int stream")),
        }
    }

    pub fn as_map(&self) -> DataResult<MapView<'_>> {
        match self {
            Value::Map(entries) => Ok(MapView(entries)),
            _ => Err(self.not("map")),
        }
    }
}

/// Field access on a map value (`MapLike`), with the lookups record codecs perform.
#[derive(Clone, Copy)]
pub struct MapView<'a>(pub &'a [(Value, Value)]);

impl<'a> MapView<'a> {
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        // The last duplicate wins, as when NbtOps builds a MapLike from a compound.
        self.0.iter().rev().find(|(k, _)| matches!(k, Value::String(s) if s == key)).map(|(_, v)| v)
    }

    /// A required field (`fieldOf`).
    pub fn req(&self, key: &str) -> DataResult<&'a Value> {
        self.get(key).ok_or_else(|| DataError(format!("missing field {key}")))
    }

    /// A required field decoded with `f`, with the field name added to errors.
    pub fn req_with<T>(&self, key: &str, f: impl FnOnce(&'a Value) -> DataResult<T>) -> DataResult<T> {
        f(self.req(key)?).map_err(|e| DataError(format!("{key}: {e}")))
    }

    /// An optional field (`optionalFieldOf`): absent is `None`, present but invalid is an error.
    pub fn opt<T>(&self, key: &str, f: impl FnOnce(&'a Value) -> DataResult<T>) -> DataResult<Option<T>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => f(v).map(Some).map_err(|e| DataError(format!("{key}: {e}"))),
        }
    }

    /// `optionalFieldOf(key, default)`.
    pub fn opt_or<T>(&self, key: &str, default: T, f: impl FnOnce(&'a Value) -> DataResult<T>) -> DataResult<T> {
        Ok(self.opt(key, f)?.unwrap_or(default))
    }

    /// `lenientOptionalFieldOf(key, default)`: an invalid value also yields the default.
    pub fn lenient_or<T>(&self, key: &str, default: T, f: impl FnOnce(&'a Value) -> DataResult<T>) -> T {
        self.get(key).and_then(|v| f(v).ok()).unwrap_or(default)
    }

    pub fn entries(&self) -> &'a [(Value, Value)] {
        self.0
    }
}

/// Builds a map value in field order (`RecordCodecBuilder` encoding).
#[derive(Default)]
pub struct MapBuilder(Vec<(Value, Value)>);

impl MapBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// A field that is always written.
    pub fn put(&mut self, key: &str, value: Value) -> &mut Self {
        self.0.push((Value::str(key), value));
        self
    }

    /// `optionalFieldOf(key)`: written when present.
    pub fn opt<T>(&mut self, key: &str, value: Option<T>, f: impl FnOnce(T) -> Value) -> &mut Self {
        if let Some(v) = value {
            self.put(key, f(v));
        }
        self
    }

    /// `optionalFieldOf(key, default)`: omitted when equal to the default.
    pub fn opt_default<T: PartialEq>(&mut self, key: &str, value: T, default: T, f: impl FnOnce(T) -> Value) -> &mut Self {
        if value != default {
            self.put(key, f(value));
        }
        self
    }

    /// Appends the fields of another map value (`MapCodec` composition such as `forGetter`
    /// on an inlined map codec).
    pub fn merge(&mut self, value: Value) -> &mut Self {
        if let Value::Map(entries) = value {
            self.0.extend(entries);
        }
        self
    }

    pub fn build(&mut self) -> Value {
        Value::Map(std::mem::take(&mut self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_compound_order_and_booleans() {
        let v = Value::Map(vec![
            (Value::str("text"), Value::str("Named")),
            (Value::str("color"), Value::str("red")),
            (Value::str("bold"), Value::Bool(true)),
            (Value::str("italic"), Value::Bool(false)),
        ]);
        let Tag::Compound(fields) = v.to_nbt() else { panic!() };
        let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["color", "text", "bold", "italic"]);
        assert_eq!(fields[2].1, Tag::Byte(1));
    }

    #[test]
    fn mixed_lists_wrap_and_unwrap() {
        let v = Value::List(vec![Value::str("a"), Value::Map(vec![(Value::str("text"), Value::str("b"))])]);
        let tag = v.to_nbt();
        assert_eq!(
            tag,
            Tag::List(vec![
                Tag::Compound(vec![(String::new(), Tag::String("a".into()))]),
                Tag::Compound(vec![("text".into(), Tag::String("b".into()))]),
            ])
        );
        assert_eq!(Value::from_nbt(&tag), v);
    }

    #[test]
    fn lenient_numbers() {
        assert_eq!(Value::Double(3.9).as_i32().unwrap(), 3);
        assert_eq!(Value::Long(1 << 32 | 5).as_i32().unwrap(), 5);
        assert!(Value::Byte(2).as_bool().unwrap());
        assert!(Value::str("x").as_i32().is_err());
    }
}
