//! The dimensions of a world (`WorldDimensions`, `LevelStem`) and the dimension types they use.
//!
//! Vanilla's dimensions are the level stems of the saved `world_gen_settings.dat` (a new world's come
//! from its world preset) with the stems the data packs define in `dimension/*.json` laid over them:
//! `WorldDimensions.bake` takes a stem from the packs' registry first and the save's second. The
//! overworld, the nether and the end always come first (`BUILTIN_ORDER`), the others after them. The
//! dimension types are the `dimension_type` entries of the packs (see [`crate::registry_data`]); this
//! reads them into the simulation's [`kiln_data::DimensionType`].

use kiln_data::{BedWhen, DimensionType};
use kiln_proto::nbt::Tag;
use kiln_worldgen::json::Json;
use std::path::PathBuf;
use tracing::{error, warn};

/// A level stem as the world is built from it.
pub(crate) struct LevelStem {
    pub key: String,
    pub json: Json,
}

/// NBT as the JSON the codecs read it from (`NbtOps` and `JsonOps` carry the same data): numbers keep their
/// value, a byte is a number (the stem codecs take a boolean from a 0 or 1).
pub(crate) fn tag_to_json(tag: &Tag) -> Json {
    match tag {
        Tag::Byte(v) => Json::Number(v.to_string()),
        Tag::Short(v) => Json::Number(v.to_string()),
        Tag::Int(v) => Json::Number(v.to_string()),
        Tag::Long(v) => Json::Number(v.to_string()),
        Tag::Float(v) => Json::Number(v.to_string()),
        Tag::Double(v) => Json::Number(v.to_string()),
        Tag::String(s) => Json::String(s.clone()),
        Tag::List(items) => Json::Array(items.iter().map(tag_to_json).collect()),
        Tag::Compound(fields) => Json::Object(fields.iter().map(|(k, v)| (k.clone(), tag_to_json(v))).collect()),
        Tag::ByteArray(a) => Json::Array(a.iter().map(|v| Json::Number(v.to_string())).collect()),
        Tag::IntArray(a) => Json::Array(a.iter().map(|v| Json::Number(v.to_string())).collect()),
        Tag::LongArray(a) => Json::Array(a.iter().map(|v| Json::Number(v.to_string())).collect()),
    }
}

/// The world's level stems: `saved` (the save's `dimensions`), each replaced by the stem of the same key in
/// the data packs (`roots`, vanilla's data first), the vanilla three added where they are missing, and the
/// vanilla three first.
pub(crate) fn stems(saved: &[(String, Tag)], roots: &[PathBuf]) -> Vec<LevelStem> {
    let mut map: std::collections::BTreeMap<String, Json> = std::collections::BTreeMap::new();
    for (key, stem) in saved {
        map.insert(key.clone(), tag_to_json(stem));
    }
    match kiln_worldgen::datapack::pack_dimensions(roots) {
        Ok(list) => map.extend(list),
        Err(e) => error!("cannot read the dimensions of the data packs: {e}"),
    }
    let mut out = Vec::new();
    for key in ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"] {
        let json = map.remove(key).unwrap_or_else(|| kiln_worldgen::stem::vanilla_stem(key).expect("default stem"));
        out.push(LevelStem { key: key.to_owned(), json });
    }
    out.extend(map.into_iter().map(|(key, json)| LevelStem { key, json }));
    out
}

/// The default biome of a stem's chunks nothing generated: the flat biome or the fixed one, else plains
/// (the nether's and the end's for their vanilla stems).
fn default_biome(key: &str, stem: &Json) -> String {
    match key {
        "minecraft:the_nether" => return "minecraft:nether_wastes".to_owned(),
        "minecraft:the_end" => return "minecraft:the_end".to_owned(),
        _ => {}
    }
    let generator = stem.get("generator");
    let biome = generator
        .and_then(|g| g.get("settings")?.get("biome"))
        .or_else(|| generator.and_then(|g| g.get("biome_source")?.get("biome")))
        .and_then(Json::as_str);
    biome.map_or_else(|| "minecraft:plains".to_owned(), |b| if b.contains(':') { b.to_owned() } else { format!("minecraft:{b}") })
}

/// The levels `stems` make: `(key, dimension type, default biome)`. A stem whose dimension type or biome is not in
/// the registries is left out (the vanilla three keep their defaults), as vanilla would not load the world.
pub(crate) fn levels(stems: &[LevelStem]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (i, stem) in stems.iter().enumerate() {
        let ty = stem.json.get("type").and_then(Json::as_str).map(|t| if t.contains(':') { t.to_owned() } else { format!("minecraft:{t}") });
        let Some(ty) = ty.filter(|t| kiln_data::synced_id("minecraft:dimension_type", t).is_some()) else {
            error!("the dimension {} has no dimension type that exists: left out", stem.key);
            if i < 3 {
                out.push((stem.key.clone(), stem.key.clone(), default_biome(&stem.key, &stem.json)));
            }
            continue;
        };
        let mut biome = default_biome(&stem.key, &stem.json);
        if kiln_data::synced_id("minecraft:worldgen/biome", &biome).is_none() {
            warn!("the dimension {} fills chunks with {biome}, which does not exist: plains", stem.key);
            biome = "minecraft:plains".to_owned();
        }
        out.push((stem.key.clone(), ty, biome));
    }
    out
}

/// A `dimension_type` entry as the simulation reads it (`DimensionType.DIRECT_CODEC`): `Err` for an entry the
/// codec would reject. `timeline_tag` lists the timelines of a `timeline` tag.
pub(crate) fn dimension_type_from_json(name: &str, v: &serde_json::Value, timeline_tag: &dyn Fn(&str) -> Vec<String>) -> Result<DimensionType, String> {
    let int = |k: &str| v.get(k).and_then(serde_json::Value::as_number).and_then(serde_json::Number::as_f64).map(|f| f as i64).ok_or_else(|| format!("{k} is missing or not a number"));
    let boolean = |k: &str| v.get(k).and_then(serde_json::Value::as_bool).ok_or_else(|| format!("{k} is missing or not a boolean"));
    let (min_y, height, logical_height) = (int("min_y")?, int("height")?, int("logical_height")?);
    if !(-2032..=2031).contains(&min_y) || !(16..=4064).contains(&height) || !(0..=4064).contains(&logical_height) {
        return Err(format!("min_y {min_y}, height {height} or logical_height {logical_height} is out of range"));
    }
    if min_y % 16 != 0 || height % 16 != 0 {
        return Err("min_y and height must be multiples of 16".into());
    }
    if min_y + height > 2032 {
        return Err("min_y + height cannot be higher than 2032".into());
    }
    if logical_height > height {
        return Err("logical_height cannot be higher than height".into());
    }
    let coordinate_scale = v.get("coordinate_scale").and_then(serde_json::Value::as_f64).ok_or("coordinate_scale is missing or not a number")?;
    if !(9.999999747378752e-6..=3.0e7).contains(&coordinate_scale) {
        return Err("coordinate_scale is out of range".into());
    }
    let attrs = v.get("attributes");
    let attr = |k: &str| attrs.and_then(|a| a.get(k));
    let bool_attr = |k: &str, default: bool| attr(k).and_then(serde_json::Value::as_bool).unwrap_or(default);
    let bed = attr("minecraft:gameplay/bed_rule");
    let when = |k: &str| bed.and_then(|b| b.get(k)).and_then(serde_json::Value::as_str).and_then(BedWhen::parse).unwrap_or(BedWhen::Always);
    let name = kiln_data::levels::leak_name(name);
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str);
    let timelines: Vec<String> = match v.get("timelines") {
        Some(serde_json::Value::String(s)) => match s.strip_prefix('#') {
            Some(tag) => timeline_tag(tag),
            None => vec![s.clone()],
        },
        Some(serde_json::Value::Array(list)) => list.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    };
    let timelines: Vec<&'static str> = timelines.iter().map(|t| kiln_data::levels::leak_name(t)).collect();
    Ok(DimensionType {
        name,
        min_y: min_y as i32,
        height: height as i32,
        logical_height: logical_height as i32,
        coordinate_scale,
        has_skylight: boolean("has_skylight")?,
        has_ceiling: boolean("has_ceiling")?,
        has_fixed_time: v.get("has_fixed_time").and_then(serde_json::Value::as_bool).unwrap_or(false),
        has_ender_dragon_fight: boolean("has_ender_dragon_fight")?,
        ambient_light: v.get("ambient_light").and_then(serde_json::Value::as_f64).ok_or("ambient_light is missing or not a number")? as f32,
        default_clock: text("default_clock").map(kiln_data::levels::leak_name),
        infiniburn: kiln_data::levels::leak_name(text("infiniburn").ok_or("infiniburn is missing")?),
        fast_lava: bool_attr("minecraft:gameplay/fast_lava", false),
        water_evaporates: bool_attr("minecraft:gameplay/water_evaporates", false),
        respawn_anchor_works: bool_attr("minecraft:gameplay/respawn_anchor_works", false),
        bed_sets_spawn: bed.and_then(|b| b.get("can_set_spawn")).and_then(serde_json::Value::as_str).is_none_or(|r| r != "never"),
        bed_explodes: bed.and_then(|b| b.get("destroy_on_use")).and_then(serde_json::Value::as_bool).unwrap_or(false),
        monster_spawn_block_light_limit: v.get("monster_spawn_block_light_limit").and_then(serde_json::Value::as_i64).unwrap_or(0),
        bed_can_sleep: when("can_sleep"),
        bed_can_set_spawn: when("can_set_spawn"),
        bed_error: bed.and_then(|b| b.get("error_message")?.get("translate")?.as_str()).map(kiln_data::levels::leak_name),
        piglins_zombify: bool_attr("minecraft:gameplay/piglins_zombify", true),
        can_start_raid: bool_attr("minecraft:gameplay/can_start_raid", true),
        nether_portal_spawns_piglin: bool_attr("minecraft:gameplay/nether_portal_spawns_piglin", false),
        timelines: Box::leak(timelines.into_boxed_slice()),
    })
}

/// Reads the dimension types of the data packs from the installed registry entries and installs them.
pub(crate) fn install_types() {
    let roots = crate::world_state::worldgen_roots();
    // (Without the vanilla data the timeline tags of the vanilla dimension types are the built-in ones.)
    let timeline_tag = |tag: &str| {
        crate::world_state::tag_entries(&roots, "timeline", tag, 0).unwrap_or_else(|| {
            let of = |name: &str| kiln_data::dimension_type(name).map(|t| t.timelines.iter().map(|s| (*s).to_owned()).collect()).unwrap_or_default();
            match tag {
                "minecraft:in_overworld" => of("minecraft:overworld"),
                "minecraft:in_nether" => of("minecraft:the_nether"),
                "minecraft:in_end" => of("minecraft:the_end"),
                "minecraft:universal" => vec!["minecraft:villager_schedule".to_owned()],
                _ => Vec::new(),
            }
        })
    };
    let mut types = Vec::new();
    for name in kiln_data::live::custom_entries("minecraft:dimension_type") {
        let Some(text) = kiln_data::live::entry_data("minecraft:dimension_type", name) else { continue };
        let parsed = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()).and_then(|v| dimension_type_from_json(name, &v, &timeline_tag));
        match parsed {
            Ok(t) => types.push(t),
            Err(e) => error!("Failed to load dimension_type/{name}: {e}"),
        }
    }
    kiln_data::levels::install_dimension_types(types);
}
