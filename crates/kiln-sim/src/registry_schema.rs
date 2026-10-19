//! What vanilla's codecs accept of the data-driven registries' entries (`RegistryDataLoader` decodes each file with the registry's codec and
//! stops loading when one does not decode), for the registries Kiln reads: the fields, their types and ranges. An entry that does not match
//! is left out with an error, as vanilla would refuse to load the world; sent to a client, its network codec would fail to decode it and the
//! client would disconnect.
//!
//! The rules are the ones vanilla's loader showed on `tools/wp85/packs/kiln85_bad` (see `tools/wp85/schema_accepted.txt`): a number is a JSON
//! number (an integer field takes `1.5` and cuts the fraction off, a string or a boolean is not a number); a string is a string; a boolean is a
//! boolean; text is a string, a list, or an object with component content (`{}` is not text); an identifier is lower case with `[a-z0-9_.-]` in the
//! namespace and `[a-z0-9_./-]` in the path; a color is `#RRGGBB` or any number; a field with `null` counts as missing. Fields a codec does not
//! know are ignored.

use serde_json::Value;

/// A field's type.
#[derive(Clone, Copy)]
enum T {
    Bool,
    /// `intRange(min, max)`; any JSON number, fraction cut off.
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
    req("carvers", T::Any),
    req("features", T::List),
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

fn number(v: &Value) -> Option<f64> {
    v.as_number().and_then(serde_json::Number::as_f64)
}

/// `ComponentSerialization.CODEC`: a string, a list of texts, or an object with the content of one of the component types.
fn text(v: &Value) -> bool {
    match v {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(text),
        Value::Object(o) => ["text", "translate", "score", "selector", "keybind", "nbt", "object"].iter().any(|k| o.contains_key(*k)),
        _ => false,
    }
}

fn check(v: &Value, t: T) -> Result<(), String> {
    match t {
        T::Bool => matches!(v, Value::Bool(_)).then_some(()).ok_or_else(|| "not a boolean".to_owned()),
        T::Int(lo, hi) => {
            let n = number(v).ok_or("not a number")?;
            let n = n.trunc() as i64;
            if (lo..=hi).contains(&n) { Ok(()) } else { Err(format!("{n} is not in {lo}..={hi}")) }
        }
        T::Float(min, exclusive) => {
            let n = number(v).ok_or("not a number")?;
            if n > min || (!exclusive && n == min) { Ok(()) } else { Err(format!("{n} is out of range")) }
        }
        T::Str => matches!(v, Value::String(_)).then_some(()).ok_or_else(|| "not a string".to_owned()),
        T::Id => match v {
            Value::String(s) if identifier(s) => Ok(()),
            Value::String(s) => Err(format!("{s:?} is not an identifier")),
            _ => Err("not an identifier".into()),
        },
        T::Color => match v {
            Value::String(s) if s.strip_prefix('#').is_some_and(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())) => Ok(()),
            Value::Number(_) => Ok(()),
            _ => Err("not a color (#RRGGBB)".into()),
        },
        T::Text => text(v).then_some(()).ok_or_else(|| "not a text".to_owned()),
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
                match o.get(f.name).filter(|x| !x.is_null()) {
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
    fn numbers_read_the_way_vanilla_reads_them() {
        let p = "minecraft:painting_variant";
        assert!(ok(p, r#"{"width":1,"height":2,"asset_id":"a:b"}"#));
        // A float where an integer goes is cut off; a string and a boolean are not numbers.
        assert!(ok(p, r#"{"width":1.5,"height":2,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":1,"height":"2","asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":1,"height":true,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":17,"height":2,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":{"v":1},"height":2,"asset_id":"a:b"}"#));
        assert!(!ok(p, r#"{"width":1,"height":2}"#));
        assert!(!ok(p, r#"{"width":1,"height":2,"asset_id":"A:b"}"#));
        // Text: strings and lists, objects with content; not numbers, booleans or an empty object; null is missing.
        assert!(ok(p, r#"{"width":1,"height":2,"asset_id":"a:b","title":["x",{"text":"y"}],"author":null}"#));
        assert!(!ok(p, r#"{"width":1,"height":2,"asset_id":"a:b","title":5}"#));
        assert!(!ok(p, r#"{"width":1,"height":2,"asset_id":"a:b","title":{}}"#));
    }

    #[test]
    fn enums_and_ranges_are_checked() {
        let d = "minecraft:damage_type";
        assert!(ok(d, r#"{"message_id":"x","scaling":"never","exhaustion":1}"#));
        assert!(!ok(d, r#"{"message_id":"x","scaling":"sometimes","exhaustion":0.1}"#));
        assert!(!ok(d, r#"{"message_id":"x","scaling":"never","exhaustion":"0.5"}"#));
        assert!(!ok(d, r#"{"message_id":7,"scaling":"never","exhaustion":0.5}"#));
        let s = "minecraft:jukebox_song";
        assert!(ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":1.5,"comparator_output":15}"#));
        assert!(ok(s, r#"{"sound_event":{"sound_id":"a:b","range":3.0},"description":"x","length_in_seconds":1.5,"comparator_output":15}"#));
        assert!(!ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":0,"comparator_output":15}"#));
        assert!(!ok(s, r#"{"sound_event":"a:b","description":"x","length_in_seconds":1,"comparator_output":16}"#));
        assert!(!ok(s, r#"{"sound_event":["a"],"description":"x","length_in_seconds":1,"comparator_output":1}"#));
    }

    #[test]
    fn biomes_need_their_climate_colors_and_lists() {
        let b = "minecraft:worldgen/biome";
        let mut biome = serde_json::json!({"has_precipitation":false,"temperature":1,"downfall":0.0,"carvers":[],"features":[],"effects":{"water_color":"#3f76e4"}});
        assert!(validate(b, &biome).is_ok());
        biome["effects"]["water_color"] = serde_json::json!(16711680);
        assert!(validate(b, &biome).is_ok(), "a color can be a number");
        for bad in [serde_json::json!("#fff"), serde_json::json!("#80ff0000"), serde_json::json!("3f76e4")] {
            biome["effects"]["water_color"] = bad;
            assert!(validate(b, &biome).is_err());
        }
        biome["effects"]["water_color"] = serde_json::json!("#3f76e4");
        biome["has_precipitation"] = serde_json::json!(1);
        assert!(validate(b, &biome).is_err(), "a boolean is a boolean");
        biome["has_precipitation"] = serde_json::json!(false);
        biome.as_object_mut().unwrap().remove("features");
        assert!(validate(b, &biome).is_err(), "features are required");
    }
}
