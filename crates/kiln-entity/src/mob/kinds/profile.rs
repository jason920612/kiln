//! `ResolvableProfile` as a mannequin keeps it: the saved form (`CODEC`: a player name, or a compound of a
//! profile and a skin patch) and the form sent to viewers (`STREAM_CODEC`). A profile is not looked up: the
//! client resolves a name or id it is sent.

use bytes::{BufMut, BytesMut};
use kiln_proto::WriteExt;
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

/// A profile and the skin parts it overrides.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Profile {
    pub name: Option<String>,
    pub id: Option<u128>,
    pub properties: Vec<Property>,
    /// A complete game profile (an id and a name): `Either.left` in the codecs.
    pub stored: bool,
    pub texture: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    /// `PlayerModelType`: 0 wide, 1 slim.
    pub model: Option<u8>,
}

/// `StringUtil.isValidPlayerName`.
fn valid_name(s: &str) -> bool {
    s.chars().count() <= 16 && s.chars().all(|c| c as u32 > 32 && (c as u32) < 127)
}

/// `UUIDUtil.CODEC`: four ints, or the text form.
fn uuid_of(t: &Tag) -> Option<u128> {
    match t {
        Tag::IntArray(v) if v.len() == 4 => Some(v.iter().fold(0u128, |a, x| a << 32 | *x as u32 as u128)),
        Tag::String(s) => {
            let hex: String = s.chars().filter(|c| *c != '-').collect();
            let dashes_ok = s.len() == 36 && [8, 13, 18, 23].iter().all(|&i| s.as_bytes()[i] == b'-') || s.len() == 32;
            (dashes_ok && hex.len() == 32).then(|| u128::from_str_radix(&hex, 16).ok()).flatten()
        }
        _ => None,
    }
}

fn uuid_tag(id: u128) -> Tag {
    Tag::IntArray(vec![(id >> 96) as i32, (id >> 64) as i32, (id >> 32) as i32, id as i32])
}

/// `Identifier` text with the default namespace; `None` when it is not one.
fn identifier(s: &str) -> Option<String> {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ns_ok = !ns.is_empty() && ns.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.'));
    let path_ok = !path.is_empty() && path.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'));
    (ns_ok && path_ok).then(|| format!("{ns}:{path}"))
}

/// `ExtraCodecs.PROPERTY_MAP`: a list of properties, or a map of names to lists of values.
fn properties_of(t: &Tag) -> Option<Vec<Property>> {
    match t {
        Tag::List(items) => items
            .iter()
            .map(|p| {
                Some(Property {
                    name: p.get("name")?.as_str()?.to_owned(),
                    value: p.get("value")?.as_str()?.to_owned(),
                    signature: match p.get("signature") {
                        Some(s) => Some(s.as_str()?.to_owned()),
                        None => None,
                    },
                })
            })
            .collect(),
        Tag::Compound(fields) => {
            let mut out = Vec::new();
            for (k, v) in fields {
                let Tag::List(values) = v else { return None };
                for value in values {
                    out.push(Property { name: k.clone(), value: value.as_str()?.to_owned(), signature: None });
                }
            }
            Some(out)
        }
        _ => None,
    }
}

impl Profile {
    /// `ResolvableProfile.CODEC` (a bare name, or the full compound); `None` when it does not decode.
    pub fn from_nbt(t: &Tag) -> Option<Profile> {
        if let Tag::String(name) = t {
            return valid_name(name).then(|| Profile { name: Some(name.clone()), ..Default::default() });
        }
        let Tag::Compound(_) = t else { return None };
        let name = match t.get("name") {
            Some(n) => {
                let n = n.as_str()?;
                if !valid_name(n) {
                    return None;
                }
                Some(n.to_owned())
            }
            None => None,
        };
        let id = match t.get("id") {
            Some(i) => Some(uuid_of(i)?),
            None => None,
        };
        let properties = match t.get("properties") {
            Some(p) => properties_of(p)?,
            None => Vec::new(),
        };
        let texture = |key: &str| -> Option<Option<String>> {
            match t.get(key) {
                Some(v) => Some(Some(identifier(v.as_str()?)?)),
                None => Some(None),
            }
        };
        let model = match t.get("model") {
            Some(m) => Some(match m.as_str()? {
                "wide" => 0,
                "slim" => 1,
                _ => return None,
            }),
            None => None,
        };
        // `STORED_GAME_PROFILE` first: an id and a name.
        let stored = name.is_some() && id.is_some();
        Some(Profile { name, id, properties, stored, texture: texture("texture")?, cape: texture("cape")?, elytra: texture("elytra")?, model })
    }

    /// `ResolvableProfile.CODEC` encoded.
    pub fn to_nbt(&self) -> Tag {
        let mut out: Vec<(String, Tag)> = Vec::new();
        // (`stored` writes the id first; the partial form the name first. Compound order does not matter.)
        if let Some(n) = &self.name {
            out.push(("name".into(), Tag::String(n.clone())));
        }
        if let Some(i) = self.id {
            out.push(("id".into(), uuid_tag(i)));
        }
        if !self.properties.is_empty() {
            let list = self
                .properties
                .iter()
                .map(|p| {
                    let mut c = vec![("name".to_owned(), Tag::String(p.name.clone())), ("value".to_owned(), Tag::String(p.value.clone()))];
                    if let Some(s) = &p.signature {
                        c.push(("signature".into(), Tag::String(s.clone())));
                    }
                    Tag::Compound(c)
                })
                .collect();
            out.push(("properties".into(), Tag::List(list)));
        }
        for (key, v) in [("texture", &self.texture), ("cape", &self.cape), ("elytra", &self.elytra)] {
            if let Some(v) = v {
                out.push((key.into(), Tag::String(v.clone())));
            }
        }
        if let Some(m) = self.model {
            out.push(("model".into(), Tag::String(if m == 0 { "wide" } else { "slim" }.into())));
        }
        Tag::Compound(out)
    }

    /// `ResolvableProfile.STREAM_CODEC`.
    pub fn write(&self, b: &mut BytesMut) {
        let props = |b: &mut BytesMut| {
            b.put_varint(self.properties.len() as i32);
            for p in &self.properties {
                b.put_string(&p.name);
                b.put_string(&p.value);
                b.put_bool(p.signature.is_some());
                if let Some(s) = &p.signature {
                    b.put_string(s);
                }
            }
        };
        if self.stored {
            b.put_bool(true);
            b.put_slice(&self.id.unwrap_or(0).to_be_bytes());
            b.put_string(self.name.as_deref().unwrap_or(""));
            props(b);
        } else {
            b.put_bool(false);
            b.put_bool(self.name.is_some());
            if let Some(n) = &self.name {
                b.put_string(n);
            }
            b.put_bool(self.id.is_some());
            if let Some(i) = self.id {
                b.put_slice(&i.to_be_bytes());
            }
            props(b);
        }
        for t in [&self.texture, &self.cape, &self.elytra] {
            b.put_bool(t.is_some());
            if let Some(t) = t {
                b.put_string(t);
            }
        }
        b.put_bool(self.model.is_some());
        if let Some(m) = self.model {
            b.put_varint(m as i32);
        }
    }
}
