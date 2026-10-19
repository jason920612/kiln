//! Timelines (`net.minecraft.world.timeline`): the data-pack JSON that makes environment attributes follow a world clock, and names moments of
//! the clock (time markers). A dimension type lists the timelines of its levels; the value of an attribute in a level is the dimension
//! type's own value (or the attribute's default) changed by the tracks of those timelines in order, each track sampled at the total ticks of
//! its timeline's clock (`AttributeTrackSampler.applyTimeBased`).
//!
//! Only the tracks of the attributes the simulation reads are modelled: booleans (`monsters_burn`, `creaking_active`, `bees_stay_in_hive`,
//! `eyeblossom_open`, `firefly_bush_sounds`, `can_pillager_patrol_spawn`) and floats (`sky_light_level` and the chances); a track of any other
//! attribute (the visual ones: colours, angles) is kept out.
//!
//! The vanilla timelines are the data pack's (`data/minecraft/timeline`); without that data the built-in ones below stand in (the same values).

use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::Arc;

/// One value of a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Value {
    Bool(bool),
    Float(f32),
}

/// `AttributeModifier`: how a track's value changes the value below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Override,
    Or,
    And,
    Nand,
    Nor,
    Xor,
    Xnor,
    Add,
    Subtract,
    Multiply,
    Minimum,
    Maximum,
}

impl Modifier {
    fn by_name(name: &str) -> Modifier {
        match name.strip_prefix("minecraft:").unwrap_or(name) {
            "or" => Modifier::Or,
            "and" => Modifier::And,
            "nand" => Modifier::Nand,
            "nor" => Modifier::Nor,
            "xor" => Modifier::Xor,
            "xnor" => Modifier::Xnor,
            "add" => Modifier::Add,
            "subtract" => Modifier::Subtract,
            "multiply" => Modifier::Multiply,
            "minimum" => Modifier::Minimum,
            "maximum" => Modifier::Maximum,
            _ => Modifier::Override,
        }
    }

    fn apply(self, base: Value, arg: Value) -> Value {
        match (base, arg) {
            (Value::Bool(b), Value::Bool(a)) => Value::Bool(match self {
                Modifier::Or => b || a,
                Modifier::And => b && a,
                Modifier::Nand => !(b && a),
                Modifier::Nor => !(b || a),
                Modifier::Xor => b != a,
                Modifier::Xnor => b == a,
                _ => a,
            }),
            (Value::Float(b), Value::Float(a)) => Value::Float(match self {
                Modifier::Add => b + a,
                Modifier::Subtract => b - a,
                Modifier::Multiply => b * a,
                Modifier::Minimum => b.min(a),
                Modifier::Maximum => b.max(a),
                _ => a,
            }),
            _ => arg,
        }
    }
}

/// `EasingType`: `constant` keeps the segment's first value until its end; anything else is taken as `linear` (the cubic Béziers of the
/// visual tracks are not sampled here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ease {
    Constant,
    Linear,
}

#[derive(Clone, Debug)]
struct Track {
    modifier: Modifier,
    ease: Ease,
    /// `(ticks, value)` in order.
    keys: Vec<(i64, Value)>,
}

impl Track {
    /// `KeyframeTrackSampler.sample` at `ticks` of the timeline's clock (`period`: it repeats).
    fn sample(&self, ticks: i64, period: Option<i64>) -> Value {
        let keys = &self.keys;
        if keys.len() == 1 {
            return keys[0].1;
        }
        let t = match period {
            Some(p) if p > 0 => ticks.rem_euclid(p),
            _ => ticks,
        };
        // The segments: (from ticks, from value, to ticks, to value), with the wrap-around ones of a periodic track.
        let mut segments: Vec<(i64, Value, i64, Value)> = Vec::with_capacity(keys.len() + 1);
        let (first, last) = (keys[0], keys[keys.len() - 1]);
        if let Some(p) = period {
            segments.push((last.0 - p, last.1, first.0, first.1));
        }
        for w in keys.windows(2) {
            segments.push((w[0].0, w[0].1, w[1].0, w[1].1));
        }
        if let Some(p) = period {
            segments.push((last.0, last.1, first.0 + p, first.1));
        }
        let seg = segments.iter().find(|s| t < s.2).unwrap_or_else(|| segments.last().expect("a segment"));
        if t <= seg.0 {
            return seg.1;
        }
        if t >= seg.2 {
            return seg.3;
        }
        let f = (t - seg.0) as f32 / (seg.2 - seg.0) as f32;
        let f = match self.ease {
            Ease::Constant => 0.0,
            Ease::Linear => f,
        };
        match (seg.1, seg.3) {
            (Value::Float(a), Value::Float(b)) => Value::Float(a + f * (b - a)),
            // (Booleans lerp as constants: `LerpFunction.ofConstant`.)
            (a, _) => a,
        }
    }
}

/// A named moment of a clock (`ClockTimeMarker`): `ticks` into the timeline's period.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Marker {
    pub id: String,
    pub ticks: i64,
    /// `show_in_commands`.
    pub show_in_commands: bool,
}

/// A timeline: the clock it follows, its period (none: it does not repeat), its markers and the tracks of the attributes the simulation reads.
#[derive(Clone, Debug)]
pub(crate) struct Timeline {
    pub clock: String,
    pub period: Option<i64>,
    pub markers: Vec<Marker>,
    tracks: HashMap<String, Track>,
}

/// All the timelines of the world, by id.
#[derive(Clone, Debug, Default)]
pub(crate) struct Timelines {
    by_id: HashMap<String, Timeline>,
    order: Vec<String>,
}

fn value_of(attribute: &str, v: &Json) -> Option<Value> {
    // Which attributes are booleans and which are floats: those the simulation reads (the others have colour strings or lists).
    let _ = attribute;
    match v {
        Json::Bool(b) => Some(Value::Bool(*b)),
        Json::Number(n) => n.as_f64().map(|f| Value::Float(f as f32)),
        _ => None,
    }
}

fn parse_timeline(j: &Json) -> Option<Timeline> {
    let clock = j.get("clock")?.as_str()?.to_owned();
    let period = j.get("period_ticks").and_then(Json::as_i64);
    let mut markers = Vec::new();
    if let Some(Json::Object(m)) = j.get("time_markers") {
        for (id, v) in m {
            let (ticks, show) = match v {
                Json::Number(n) => (n.as_i64()?, false),
                Json::Object(o) => (o.get("ticks")?.as_i64()?, o.get("show_in_commands").and_then(Json::as_bool).unwrap_or(false)),
                _ => continue,
            };
            markers.push(Marker { id: id.clone(), ticks, show_in_commands: show });
        }
    }
    let mut tracks = HashMap::new();
    if let Some(Json::Object(t)) = j.get("tracks") {
        for (attribute, track) in t {
            let Some(Json::Array(list)) = track.get("keyframes") else { continue };
            let keys: Vec<(i64, Value)> = list.iter().filter_map(|k| Some((k.get("ticks")?.as_i64()?, value_of(attribute, k.get("value")?)?))).collect();
            if keys.is_empty() || keys.len() != list.len() {
                continue;
            }
            let modifier = track.get("modifier").and_then(Json::as_str).map_or(Modifier::Override, Modifier::by_name);
            let ease = match track.get("ease") {
                Some(Json::String(s)) if s.strip_prefix("minecraft:").unwrap_or(s) == "constant" => Ease::Constant,
                _ => Ease::Linear,
            };
            tracks.insert(attribute.clone(), Track { modifier, ease, keys });
        }
    }
    Some(Timeline { clock, period, markers, tracks })
}

impl Timelines {
    /// The timelines of the pack `roots` (vanilla's data under them, else the built-in ones).
    pub(crate) fn load(roots: &[std::path::PathBuf]) -> Arc<Timelines> {
        let mut found = crate::world_state::stacked_json(roots, "timeline");
        if found.iter().all(|(id, _)| !id.starts_with("minecraft:")) {
            // (No vanilla data: the timelines the simulation needs, as a pack of the roots may not replace them.)
            let builtin: Vec<(String, Json)> = BUILTIN.iter().filter_map(|(id, text)| Some(((*id).to_owned(), serde_json::from_str(text).ok()?))).collect();
            let packs: Vec<_> = found.drain(..).collect();
            found = builtin;
            for (id, v) in packs {
                match found.iter_mut().find(|(i, _)| *i == id) {
                    Some(slot) => slot.1 = v,
                    None => found.push((id, v)),
                }
            }
        }
        let mut t = Timelines::default();
        for (id, json) in found {
            if let Some(timeline) = parse_timeline(&json) {
                t.order.push(id.clone());
                t.by_id.insert(id, timeline);
            }
        }
        Arc::new(t)
    }

    /// The ids of the timelines that follow `clock`.
    pub(crate) fn of_clock<'a>(&'a self, clock: &'a str) -> impl Iterator<Item = (&'a str, &'a Timeline)> + 'a {
        self.order.iter().filter_map(|id| self.by_id.get(id).map(|t| (id.as_str(), t))).filter(move |(_, t)| t.clock == clock)
    }

    /// The markers of a clock (every timeline of it): `(marker, timeline period, ticks)`.
    pub(crate) fn markers_of(&self, clock: &str) -> Vec<(String, Option<i64>, i64, bool)> {
        self.of_clock(clock).flat_map(|(_, t)| t.markers.iter().map(move |m| (m.id.clone(), t.period, m.ticks, m.show_in_commands))).collect()
    }

    /// The value of `attribute` in a level whose dimension type lists `timelines`: `base` changed by their tracks, in order; `total` gives the
    /// total ticks of a clock.
    pub(crate) fn sample(&self, timelines: &[&str], attribute: &str, base: Value, total: &dyn Fn(&str) -> i64) -> Value {
        let mut value = base;
        for id in timelines {
            let Some(t) = self.by_id.get(*id) else { continue };
            let Some(track) = t.tracks.get(attribute) else { continue };
            value = track.modifier.apply(value, track.sample(total(&t.clock), t.period));
        }
        value
    }

    /// Whether any of the `timelines` has a track for `attribute`.
    pub(crate) fn has_track(&self, timelines: &[&str], attribute: &str) -> bool {
        timelines.iter().any(|id| self.by_id.get(*id).is_some_and(|t| t.tracks.contains_key(attribute)))
    }

    pub(crate) fn bool_attr(&self, timelines: &[&str], attribute: &str, base: bool, total: &dyn Fn(&str) -> i64) -> bool {
        match self.sample(timelines, attribute, Value::Bool(base), total) {
            Value::Bool(b) => b,
            Value::Float(_) => base,
        }
    }

    pub(crate) fn float_attr(&self, timelines: &[&str], attribute: &str, base: f32, total: &dyn Fn(&str) -> i64) -> f32 {
        match self.sample(timelines, attribute, Value::Float(base), total) {
            Value::Float(f) => f,
            Value::Bool(_) => base,
        }
    }
}

/// The timelines of the overworld the simulation reads, in the form of the data pack (the keyframes are the ones the game has always had).
const BUILTIN: [(&str, &str); 2] = [
    (
        "minecraft:day",
        r#"{"clock":"minecraft:overworld","period_ticks":24000,
"time_markers":{"minecraft:day":{"show_in_commands":true,"ticks":1000},"minecraft:midnight":{"show_in_commands":true,"ticks":18000},
"minecraft:night":{"show_in_commands":true,"ticks":13000},"minecraft:noon":{"show_in_commands":true,"ticks":6000},
"minecraft:roll_village_siege":18000,"minecraft:wake_up_from_sleep":0},
"tracks":{
"minecraft:gameplay/bees_stay_in_hive":{"modifier":"or","keyframes":[{"ticks":12542,"value":true},{"ticks":23460,"value":false}]},
"minecraft:gameplay/creaking_active":{"modifier":"or","keyframes":[{"ticks":12600,"value":true},{"ticks":23401,"value":false}]},
"minecraft:gameplay/eyeblossom_open":{"keyframes":[{"ticks":12600,"value":true},{"ticks":23401,"value":false}]},
"minecraft:gameplay/monsters_burn":{"modifier":"or","keyframes":[{"ticks":12542,"value":false},{"ticks":23460,"value":true}]},
"minecraft:gameplay/sky_light_level":{"modifier":"multiply","keyframes":[{"ticks":133,"value":1.0},{"ticks":11867,"value":1.0},{"ticks":13670,"value":0.26666668},{"ticks":22330,"value":0.26666668}]}}}"#,
    ),
    ("minecraft:early_game", r#"{"clock":"minecraft:overworld"}"#),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin() -> Timelines {
        // (A root without any data: the built-in timelines.)
        let t = Timelines::load(&[std::path::PathBuf::from("/nonexistent-wp87")]);
        (*t).clone()
    }

    fn at(t: i64) -> impl Fn(&str) -> i64 {
        move |_| t
    }

    /// The hard-coded day-time functions the simulation used before it read the timelines.
    fn monsters_burn(day_time: i64) -> bool {
        !(12542..23460).contains(&day_time.rem_euclid(24000))
    }

    fn creaking_active(day_time: i64) -> bool {
        (12600..23401).contains(&day_time.rem_euclid(24000))
    }

    #[test]
    fn the_day_timeline_makes_the_old_hard_coded_ranges() {
        let t = builtin();
        let day = ["minecraft:day"];
        for ticks in 0..72_000 {
            assert_eq!(t.bool_attr(&day, "minecraft:gameplay/monsters_burn", false, &at(ticks)), monsters_burn(ticks), "monsters_burn at {ticks}");
            assert_eq!(t.bool_attr(&day, "minecraft:gameplay/creaking_active", false, &at(ticks)), creaking_active(ticks), "creaking_active at {ticks}");
            let bees = (12542..23460).contains(&ticks.rem_euclid(24000));
            assert_eq!(t.bool_attr(&day, "minecraft:gameplay/bees_stay_in_hive", false, &at(ticks)), bees, "bees at {ticks}");
        }
    }

    #[test]
    fn the_sky_light_level_follows_the_keyframes() {
        let t = builtin();
        let day = ["minecraft:day"];
        let level = |ticks| t.float_attr(&day, "minecraft:gameplay/sky_light_level", 15.0, &at(ticks));
        assert_eq!(level(6000), 15.0);
        assert!((level(22330) - 4.0).abs() < 1e-4);
        assert!((level(12768) - 15.0 * (1.0 + 0.5 * (0.26666668 - 1.0))).abs() < 0.01);
        // The timeline repeats every 24000 ticks.
        assert_eq!(level(6000 + 24000 * 5), 15.0);
    }

    #[test]
    fn a_timeline_of_another_clock_follows_that_clock() {
        let j: Json = serde_json::from_str(
            r#"{"clock":"kiln:short","period_ticks":100,"time_markers":{"kiln:peak":{"ticks":50,"show_in_commands":true}},
"tracks":{"minecraft:gameplay/monsters_burn":{"keyframes":[{"ticks":10,"value":false},{"ticks":60,"value":true}]}}}"#,
        )
        .unwrap();
        let mut t = Timelines::default();
        t.order.push("kiln:short_day".into());
        t.by_id.insert("kiln:short_day".into(), parse_timeline(&j).unwrap());
        let list = ["kiln:short_day"];
        fn clock(c: &'static str, v: i64) -> impl Fn(&str) -> i64 {
            move |name: &str| if name == c { v } else { 0 }
        }
        assert!(!t.bool_attr(&list, "minecraft:gameplay/monsters_burn", true, &clock("kiln:short", 30)));
        assert!(t.bool_attr(&list, "minecraft:gameplay/monsters_burn", false, &clock("kiln:short", 70)));
        // The clock of the overworld does not matter to it.
        assert!(!t.bool_attr(&list, "minecraft:gameplay/monsters_burn", true, &clock("minecraft:overworld", 70)));
        assert_eq!(t.markers_of("kiln:short"), vec![("kiln:peak".to_owned(), Some(100), 50, true)]);
    }
}
