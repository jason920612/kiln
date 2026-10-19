//! What vanilla's codecs accept of the data-driven registries' entries (`RegistryDataLoader` decodes each file with the registry's codec and
//! stops loading when one does not decode), for the registries Kiln reads: the fields, their types and ranges. An entry that does not match
//! is left out with an error, as vanilla would refuse to load the world; sent to a client, its network codec would fail to decode it and the
//! client would disconnect.
//!
//! The JSON rules are `JsonOps`': an integer or float is any number (or a string that reads as one: `Codec.INT` takes `1.5` and `"3"`, cutting
//! off the fraction); a string is a string or a number; a boolean is a boolean or a number; text is anything but null; an identifier is lower
//! case with `[a-z0-9_.-]` in the namespace and `[a-z0-9_./-]` in the path; a color string is `#RRGGBB` or `#AARRGGBB`. Fields a codec does not
//! know are ignored.

use serde_json::Value;

/// A field's type.
#[derive(Clone, Copy)]
enum T {
    Bool,
    /// `intRange(min, max)`.
    Int(i64, i64),
    /// `Codec.FLOAT` with a lower bound (`min`, and whether the bound itself is out).
    Float(f64, bool),
    Str,
    Id,
    Color,
    Text,
    /// `SoundEvent.CODEC`: an identifier, or `{"sound_id": …}`.
    Sound,
    Enum(&'static [&'static str]),
    Obj(&'static [F]),
    /// `Codec.list`.
    List,
    /// Anything but null (a codec Kiln does not look into).
    Any,
}

#[derive(Clone, Copy)]
struct F {
    name: &'static str,
    t: T,
    required: bool,
}

const fn req(name: &'static str, t: T) -> F {
    F { name, t, required: true }
}

const fn opt(name: &'static str, t: T) -> F {
    F { name, t, required: false }
}

const ANY_FLOAT: T = T::Float(f64::NEG_INFINITY, false);

const PAINTING: &[F] = &[req("width", T::Int(1, 16)), req("height", T::Int(1, 16)), req("asset_id", T::Id), opt("title", T::Text), opt("author", T::Text)];
const DAMAGE: &[F] = &[
    req("message_id", T::Str),
    req("scaling", T::Enum(&["never", "always", "when_caused_by_living_non_player"])),
    req("exhaustion", ANY_FLOAT),
    opt("effects", T::Enum(&["hurt", "thorns", "drowning", "burning", "poking", "freezing"])),
    opt("death_message_type", T::Enum(&["default", "fall_variants", "intentional_game_design"])),
];
const SONG: &[F] = &[req("sound_event", T::Sound), req("description", T::Text), req("length_in_seconds", T::Float(0.0, true)), req("comparator_output", T::Int(0, 15))];
const INSTRUMENT: &[F] = &[
    req("sound_event", T::Sound),
    req("use_duration", T::Float(0.0, false)),
    req("range", T::Float(0.0, true)),
    opt("durability_damage", T::Int(0, i32::MAX as i64)),
    req("description", T::Text),
];
const TRIM_PATTERN: &[F] = &[req("asset_id", T::Id), req("description", T::Text), opt("decal", T::Bool)];
const TRIM_MATERIAL: &[F] = &[req("palette_id", T::Id), req("description", T::Text)];
const BANNER: &[F] = &[req("asset_id", T::Id), req("translation_key", T::Str)];
const POT: &[F] = &[req("asset_id", T::Id)];
const WOLF_ASSETS: &[F] = &[req("wild", T::Id), req("tame", T::Id), req("angry", T::Id)];
const WOLF: &[F] = &[req("assets", T::Obj(WOLF_ASSETS)), req("baby_assets", T::Obj(WOLF_ASSETS)), req("spawn_conditions", T::List)];
const MODEL: T = T::Enum(&["normal", "cold", "warm"]);
const BABY_VARIANT: &[F] = &[req("asset_id", T::Id), req("baby_asset_id", T::Id), opt("model", MODEL), req("spawn_conditions", T::List)];
const VARIANT: &[F] = &[req("asset_id", T::Id), opt("model", MODEL), req("spawn_conditions", T::List)];
const DECORATION: &[F] = &[req("translation_key", T::Str), req("parameters", T::List), opt("style", T::Any)];
const CHAT: &[F] = &[req("chat", T::Obj(DECORATION)), req("narration", T::Obj(DECORATION))];
const DIALOG: &[F] = &[req("type", T::Id)];
const EFFECTS: &[F] = &[
    req("water_color", T::Color),
    opt("foliage_color", T::Color),
    opt("dry_foliage_color", T::Color),
    opt("grass_color", T::Color),
    opt("grass_color_modifier", T::Enum(&["none", "dark_forest", "swamp"])),
];
const BIOME: &[F] = &[
    req("has_precipitation", T::Bool),
    req("temperature", ANY_FLOAT),
    opt("temperature_modifier", T::Enum(&["none", "frozen"])),
    req("downfall", ANY_FLOAT),
    req("effects", T::Obj(EFFECTS)),
    opt("attributes", T::Obj(&[])),
];

fn schema(registry: &str) -> Option<&'static [F]> {
    Some(match registry {
        "minecraft:painting_variant" => PAINTING,
        "minecraft:damage_type" => DAMAGE,
        "minecraft:jukebox_song" => SONG,
        "minecraft:instrument" => INSTRUMENT,
        "minecraft:trim_pattern" => TRIM_PATTERN,
        "minecraft:trim_material" => TRIM_MATERIAL,
        "minecraft:banner_pattern" => BANNER,
        "minecraft:decorated_pot_pattern" => POT,
        "minecraft:wolf_variant" => WOLF,
        "minecraft:cat_variant" | "minecraft:pig_variant" | "minecraft:cow_variant" | "minecraft:chicken_variant" => BABY_VARIANT,
        "minecraft:frog_variant" | "minecraft:zombie_nautilus_variant" => VARIANT,
        "minecraft:chat_type" => CHAT,
        "minecraft:dialog" => DIALOG,
        "minecraft:worldgen/biome" => BIOME,
        _ => return None,
    })
}

/// `Identifier.tryParse`.
fn identifier(s: &str) -> bool {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    !path.is_empty()
        && ns.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'))
        && path.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-' | '/'))
}

/// A number the way `JsonOps.getNumberValue` reads one: a number, or a string that reads as one.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn check(v: &Value, t: T) -> Result<(), String> {
    match t {
        T::Bool => matches!(v, Value::Bool(_) | Value::Number(_)).then_some(()).ok_or_else(|| "not a boolean".to_owned()),
        T::Int(lo, hi) => {
            let n = number(v).ok_or("not a number")?;
            let n = n.trunc() as i64;
            if (lo..=hi).contains(&n) { Ok(()) } else { Err(format!("{n} is not in {lo}..={hi}")) }
        }
        T::Float(min, exclusive) => {
            let n = number(v).ok_or("not a number")?;
            if n > min || (!exclusive && n == min) { Ok(()) } else { Err(format!("{n} is out of range")) }
        }
        T::Str => matches!(v, Value::String(_) | Value::Number(_)).then_some(()).ok_or_else(|| "not a string".to_owned()),
        T::Id => match v {
            Value::String(s) if identifier(s) => Ok(()),
            Value::String(s) => Err(format!("{s:?} is not an identifier")),
            _ => Err("not an identifier".into()),
        },
        T::Color => match v {
            Value::String(s) if s.strip_prefix('#').is_some_and(|h| matches!(h.len(), 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())) => Ok(()),
            _ => Err("not a color (#RRGGBB)".into()),
        },
        T::Text => (!v.is_null()).then_some(()).ok_or_else(|| "not a text".to_owned()),
        T::Sound => match v {
            Value::String(s) if identifier(s) => Ok(()),
            Value::Object(o) if o.get("sound_id").and_then(Value::as_str).is_some_and(identifier) => Ok(()),
            _ => Err("not a sound event".into()),
        },
        T::Enum(names) => match v.as_str() {
            Some(s) if names.contains(&s) => Ok(()),
            Some(s) => Err(format!("{s:?} is none of {names:?}")),
            None => Err("not a string".into()),
        },
        T::Obj(fields) => {
            let o = v.as_object().ok_or("not an object")?;
            for f in fields {
                match o.get(f.name) {
                    Some(x) => check(x, f.t).map_err(|e| format!("{}: {e}", f.name))?,
                    None if f.required => return Err(format!("missing {}", f.name)),
                    None => {}
                }
            }
            Ok(())
        }
        T::List => v.is_array().then_some(()).ok_or_else(|| "not a list".to_owned()),
        T::Any => (!v.is_null()).then_some(()).ok_or_else(|| "null".to_owned()),
    }
}

/// Whether `v` is an entry of `registry` that vanilla's codec decodes; `Err` says why not. Registries without a schema here are not checked.
pub(crate) fn validate(registry: &str, v: &Value) -> Result<(), String> {
    match schema(registry) {
        Some(fields) => check(v, T::Obj(fields)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(registry: &str, json: &str) -> bool {
        validate(registry, &serde_json::from_str(json).unwrap()).is_ok()
    }

    #[test]
    fn numbers_read_the_way_jsonops_reads_them() {
        let p = "minecraft:painting_variant";
        assert!(ok(p, r#"{"width":1,"height":2,"asset_id":"a:b"}"#));
        // A float where an integer goes is cut off, a numeric string is a number (`JsonOps`).
        assert!(ok(p, r#"{"width":1.5,"height":"2","asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":17,"height":2,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":{"v":1},"height":2,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":1,"height":2}"#));
        assert!(!ok(p, r#"{"width":1,"height":2,"asset_id":"A:b"}"#));
    }

    #[test]
    fn enums_and_ranges_are_checked() {
        let d = "minecraft:damage_type";
        assert!(ok(d, r#"{"message_id":"x","scaling":"never","exhaustion":0.1}"#));
        assert!(!ok(d, r#"{"message_id":"x","scaling":"sometimes","exhaustion":0.1}"#));
        assert!(!ok(d, r#"{"message_id":"x","scaling":"never","exhaustion":"lots"}"#));
        let s = "minecraft:jukebox_song";
        assert!(ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":1.5,"comparator_output":15}"#));
        assert!(!ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":0,"comparator_output":15}"#));
        assert!(!ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":1,"comparator_output":16}"#));
        assert!(!ok(s, r#"{"sound_event":{"x":1},"description":"x","length_in_seconds":1,"comparator_output":1}"#));
    }

    #[test]
    fn biomes_need_their_climate_and_colors() {
        let b = "minecraft:worldgen/biome";
        assert!(ok(b, r##"{"has_precipitation":false,"temperature":0.5,"downfall":0.0,"effects":{"water_color":"#3f76e4"}}"##));
        assert!(!ok(b, r##"{"has_precipitation":"maybe","temperature":0.5,"downfall":0.0,"effects":{"water_color":"#3f76e4"}}"##));
        assert!(!ok(b, r##"{"has_precipitation":false,"temperature":0.5,"downfall":0.0,"effects":{"water_color":16711680}}"##));
        assert!(!ok(b, r##"{"has_precipitation":false,"temperature":0.5,"effects":{"water_color":"#3f76e4"}}"##));
    }
}
