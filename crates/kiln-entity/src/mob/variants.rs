//! The variants a new animal can spawn with (`VariantUtils.selectVariantToSpawn`): wolf, cat, pig, cow,
//! chicken, frog and zombie nautilus. Every variant of the registry carries `spawn_conditions`, a list of
//! selectors (`priority`, an optional `condition`: `minecraft:biome`, `minecraft:structure` or
//! `minecraft:moon_brightness`); the variants whose highest passing selector has the highest priority are
//! the candidates, and the spawn draws one of them (`PriorityProvider.select` + `Util.getRandomSafe`).
//!
//! The selectors of a variant come from its entry when a data pack defines or overrides it, else from the
//! vanilla datapack (the table below, `data/minecraft/<animal>_variant`, checked against the extracted
//! data by a test).

use super::kinds::wolf::biome_is;
use kiln_javamath::random::RandomSource;
use kiln_loot::Json;

/// A selector's condition (`SpawnCondition`).
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// `minecraft:biome`: a biome name, a `#tag`, or several names.
    Biome(Vec<String>),
    /// `minecraft:structure`: the same for structures; true when the spawn position is inside a piece of one.
    Structure(Vec<String>),
    /// `minecraft:moon_brightness`: `range.min <= brightness <= range.max`.
    MoonBrightness { min: Option<f64>, max: Option<f64> },
}

/// One selector of a variant (`PriorityProvider.Selector`).
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub priority: i32,
    /// `None`: always true.
    pub condition: Option<Condition>,
}

/// What the conditions look at (`SpawnContext`).
pub struct Where<'a> {
    pub biome: Option<i32>,
    pub moon_brightness: f32,
    /// The structure specs ([`structure_specs`]) the position is inside a piece of.
    pub structures: &'a [String],
}

impl Condition {
    fn test(&self, at: &Where) -> bool {
        match self {
            Condition::Biome(specs) => at.biome.is_some_and(|b| specs.iter().any(|s| biome_is(b, s))),
            Condition::Structure(specs) => specs.iter().any(|s| at.structures.contains(s)),
            Condition::MoonBrightness { min, max } => {
                let v = f64::from(at.moon_brightness);
                min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m)
            }
        }
    }
}

/// The animals that spawn with a variant, and the registry of their variants.
pub fn registry_of(kind: super::MobKind) -> Option<&'static str> {
    use super::MobKind as K;
    Some(match kind {
        K::Wolf => "minecraft:wolf_variant",
        K::Cat => "minecraft:cat_variant",
        K::Pig => "minecraft:pig_variant",
        K::Cow => "minecraft:cow_variant",
        K::Chicken => "minecraft:chicken_variant",
        K::Frog => "minecraft:frog_variant",
        K::ZombieNautilus => "minecraft:zombie_nautilus_variant",
        _ => return None,
    })
}

struct Variant {
    id: i32,
    selectors: Vec<Selector>,
}

fn variants(registry: &'static str) -> &'static [Variant] {
    static ALL: kiln_data::live::Cached<std::collections::HashMap<&'static str, Vec<Variant>>> = kiln_data::live::Cached::new();
    ALL.get(|| {
        [
            "minecraft:wolf_variant",
            "minecraft:cat_variant",
            "minecraft:pig_variant",
            "minecraft:cow_variant",
            "minecraft:chicken_variant",
            "minecraft:frog_variant",
            "minecraft:zombie_nautilus_variant",
        ]
        .into_iter()
        .map(|registry| {
            let names = kiln_data::live::synced_entries(registry).unwrap_or(&[]);
            let list = names
                .iter()
                .enumerate()
                .map(|(id, name)| Variant { id: id as i32, selectors: selectors_of(registry, name) })
                .collect();
            (registry, list)
        })
        .collect()
    })
    .get(registry)
    .map_or(&[][..], Vec::as_slice)
}

/// The selectors of an entry: its own `spawn_conditions` if a pack defines it, else vanilla's.
fn selectors_of(registry: &str, name: &str) -> Vec<Selector> {
    if let Some(json) = kiln_data::live::entry_data(registry, name).and_then(|t| Json::parse(t).ok()) {
        return parse(json.get("spawn_conditions"));
    }
    VANILLA
        .iter()
        .find(|(r, _)| *r == registry)
        .and_then(|(_, variants)| variants.iter().find(|(n, _)| *n == name))
        .map(|(_, sel)| sel.iter().map(|&(priority, c)| Selector { priority, condition: c.map(Condition::from) }).collect())
        .unwrap_or_default()
}

/// `SpawnPrioritySelectors.CODEC`: a list of `{priority, condition?}`; entries that do not decode are skipped.
fn parse(json: Option<&Json>) -> Vec<Selector> {
    let Some(list) = json.and_then(Json::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|s| {
            let priority = s.get("priority").and_then(Json::as_i32)?;
            let condition = match s.get("condition") {
                None => None,
                Some(c) => Some(parse_condition(c)?),
            };
            Some(Selector { priority, condition })
        })
        .collect()
}

fn specs(j: Option<&Json>) -> Option<Vec<String>> {
    let norm = |s: &str| if s.starts_with('#') || s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") };
    match j? {
        Json::Str(s) => Some(vec![norm(s)]),
        Json::Arr(items) => items.iter().map(|i| i.as_str().map(norm)).collect(),
        _ => None,
    }
}

fn parse_condition(c: &Json) -> Option<Condition> {
    let bound = |range: &Json, key: &str| range.get(key).and_then(|v| v.as_f64());
    match c.get("type")?.as_str()?.trim_start_matches("minecraft:") {
        "biome" => Some(Condition::Biome(specs(c.get("biomes"))?)),
        "structure" => Some(Condition::Structure(specs(c.get("structures"))?)),
        "moon_brightness" => {
            let range = c.get("range")?;
            Some(Condition::MoonBrightness { min: bound(range, "min"), max: bound(range, "max") })
        }
        _ => None,
    }
}

/// The structure specs the variants of `registry` ask about: what the caller must check the spawn
/// position against (it has the world) and pass to [`select`] as the `structures` that hold.
pub fn structure_specs(registry: &'static str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in variants(registry) {
        for s in &v.selectors {
            if let Some(Condition::Structure(list)) = &s.condition {
                for spec in list {
                    if !out.contains(spec) {
                        out.push(spec.clone());
                    }
                }
            }
        }
    }
    out
}

/// `VariantUtils.selectVariantToSpawn`: the id of the variant a mob spawning at `at` takes, one draw
/// among the best candidates (none when no variant has a passing selector; no draw then).
pub fn select(registry: &'static str, at: &Where, r: &mut dyn RandomSource) -> Option<i32> {
    // `PriorityProvider.select`: every selector of every variant (registry order), highest priority
    // first (stable), keeping those that pass at the highest priority that has a passing selector.
    let mut all: Vec<(i32, i32, &Option<Condition>)> = Vec::new();
    for v in variants(registry) {
        for s in &v.selectors {
            all.push((v.id, s.priority, &s.condition));
        }
    }
    all.sort_by_key(|&(_, priority, _)| std::cmp::Reverse(priority));
    let mut best = i32::MIN;
    let mut picked: Vec<i32> = Vec::new();
    for (id, priority, condition) in all {
        if priority < best {
            continue;
        }
        if condition.as_ref().is_none_or(|c| c.test(at)) {
            best = priority;
            picked.push(id);
        }
    }
    (!picked.is_empty()).then(|| picked[r.next_int_bounded(picked.len() as i32) as usize])
}

/// [`select`] for the animal `kind` spawning in `ctx` (`None` for a kind without variants).
pub fn select_for(kind: super::MobKind, ctx: &super::SpawnContext, group: &super::GroupData, r: &mut dyn RandomSource) -> Option<i32> {
    let registry = registry_of(kind)?;
    select(registry, &Where { biome: ctx.biome, moon_brightness: ctx.moon_brightness, structures: &group.structures }, r)
}

#[derive(Clone, Copy)]
enum C {
    Biome(&'static str),
    Structure(&'static str),
    Moon(f64),
}

impl From<C> for Condition {
    fn from(c: C) -> Condition {
        match c {
            C::Biome(s) => Condition::Biome(vec![s.to_owned()]),
            C::Structure(s) => Condition::Structure(vec![s.to_owned()]),
            C::Moon(min) => Condition::MoonBrightness { min: Some(min), max: None },
        }
    }
}

type Selectors = &'static [(i32, Option<C>)];

/// `spawn_conditions` of the vanilla variants: `(registry, [(variant, [(priority, condition)])])`.
const VANILLA: &[(&str, &[(&str, Selectors)])] = &[
    (
        "minecraft:wolf_variant",
        &[
            ("minecraft:ashen", &[(1, Some(C::Biome("minecraft:snowy_taiga")))]),
            ("minecraft:black", &[(1, Some(C::Biome("minecraft:old_growth_pine_taiga")))]),
            ("minecraft:chestnut", &[(1, Some(C::Biome("minecraft:old_growth_spruce_taiga")))]),
            ("minecraft:pale", &[(0, None)]),
            ("minecraft:rusty", &[(1, Some(C::Biome("#minecraft:is_jungle")))]),
            ("minecraft:snowy", &[(1, Some(C::Biome("minecraft:grove")))]),
            ("minecraft:spotted", &[(1, Some(C::Biome("#minecraft:is_savanna")))]),
            ("minecraft:striped", &[(1, Some(C::Biome("#minecraft:is_badlands")))]),
            ("minecraft:woods", &[(1, Some(C::Biome("minecraft:forest")))]),
        ],
    ),
    (
        "minecraft:cat_variant",
        &[
            ("minecraft:all_black", &[(1, Some(C::Structure("#minecraft:cats_spawn_as_black"))), (0, Some(C::Moon(0.9)))]),
            ("minecraft:black", &[(0, None)]),
            ("minecraft:british_shorthair", &[(0, None)]),
            ("minecraft:calico", &[(0, None)]),
            ("minecraft:jellie", &[(0, None)]),
            ("minecraft:persian", &[(0, None)]),
            ("minecraft:ragdoll", &[(0, None)]),
            ("minecraft:red", &[(0, None)]),
            ("minecraft:siamese", &[(0, None)]),
            ("minecraft:tabby", &[(0, None)]),
            ("minecraft:white", &[(0, None)]),
        ],
    ),
    (
        "minecraft:pig_variant",
        &[
            ("minecraft:cold", &[(1, Some(C::Biome("#minecraft:spawns_cold_variant_farm_animals")))]),
            ("minecraft:temperate", &[(0, None)]),
            ("minecraft:warm", &[(1, Some(C::Biome("#minecraft:spawns_warm_variant_farm_animals")))]),
        ],
    ),
    (
        "minecraft:cow_variant",
        &[
            ("minecraft:cold", &[(1, Some(C::Biome("#minecraft:spawns_cold_variant_farm_animals")))]),
            ("minecraft:temperate", &[(0, None)]),
            ("minecraft:warm", &[(1, Some(C::Biome("#minecraft:spawns_warm_variant_farm_animals")))]),
        ],
    ),
    (
        "minecraft:chicken_variant",
        &[
            ("minecraft:cold", &[(1, Some(C::Biome("#minecraft:spawns_cold_variant_farm_animals")))]),
            ("minecraft:temperate", &[(0, None)]),
            ("minecraft:warm", &[(1, Some(C::Biome("#minecraft:spawns_warm_variant_farm_animals")))]),
        ],
    ),
    (
        "minecraft:frog_variant",
        &[
            ("minecraft:cold", &[(1, Some(C::Biome("#minecraft:spawns_cold_variant_frogs")))]),
            ("minecraft:temperate", &[(0, None)]),
            ("minecraft:warm", &[(1, Some(C::Biome("#minecraft:spawns_warm_variant_frogs")))]),
        ],
    ),
    (
        "minecraft:zombie_nautilus_variant",
        &[
            ("minecraft:temperate", &[(0, None)]),
            ("minecraft:warm", &[(1, Some(C::Biome("#minecraft:spawns_coral_variant_zombie_nautilus")))]),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_javamath::random::LegacyRandom;

    fn at(biome: &str, moon: f32, structures: &[String]) -> (Option<i32>, f32, Vec<String>) {
        (kiln_data::synced_id("minecraft:worldgen/biome", biome), moon, structures.to_vec())
    }

    fn pick(registry: &'static str, w: &(Option<i32>, f32, Vec<String>), seed: i64) -> Option<&'static str> {
        let at = Where { biome: w.0, moon_brightness: w.1, structures: &w.2 };
        select(registry, &at, &mut LegacyRandom::new(seed)).and_then(|id| kiln_data::live::synced_name(registry, id))
    }

    #[test]
    fn vanilla_selectors_pick_the_variants_of_the_biomes() {
        let _x = kiln_data::live::exclusive();
        kiln_data::live::reset();
        let w = "minecraft:wolf_variant";
        assert_eq!(pick(w, &at("minecraft:snowy_taiga", 0.0, &[]), 1), Some("minecraft:ashen"));
        assert_eq!(pick(w, &at("minecraft:jungle", 0.0, &[]), 1), Some("minecraft:rusty"));
        assert_eq!(pick(w, &at("minecraft:plains", 0.0, &[]), 1), Some("minecraft:pale"));
        let p = "minecraft:pig_variant";
        assert_eq!(pick(p, &at("minecraft:snowy_plains", 0.0, &[]), 1), Some("minecraft:cold"));
        assert_eq!(pick(p, &at("minecraft:desert", 0.0, &[]), 1), Some("minecraft:warm"));
        assert_eq!(pick(p, &at("minecraft:plains", 0.0, &[]), 1), Some("minecraft:temperate"));
        // Cats: the structure's all-black one wins everything; at a full moon the all-black one joins the others.
        let c = "minecraft:cat_variant";
        let hut = ["#minecraft:cats_spawn_as_black".to_owned()];
        assert_eq!(pick(c, &at("minecraft:swamp", 0.0, &hut), 5), Some("minecraft:all_black"));
        let dark: Vec<_> = (0..200).filter_map(|s| pick(c, &at("minecraft:plains", 0.5, &[]), s)).collect();
        assert!(!dark.contains(&"minecraft:all_black") && dark.contains(&"minecraft:tabby"));
        let moon: Vec<_> = (0..300).filter_map(|s| pick(c, &at("minecraft:plains", 1.0, &[]), s)).collect();
        assert!(moon.contains(&"minecraft:all_black") && moon.contains(&"minecraft:tabby"));
        assert_eq!(structure_specs(c), ["#minecraft:cats_spawn_as_black"]);
    }

    /// The vanilla table is what the extracted datapack says (when this machine has it).
    #[test]
    fn the_vanilla_table_matches_the_extracted_datapack() {
        let root = std::env::var_os("KILN_DATAPACK").map(std::path::PathBuf::from).unwrap_or_else(|| "../../work/generated".into());
        let mut checked = 0;
        for (registry, variants) in VANILLA {
            let dir = root.join("data/minecraft").join(registry.trim_start_matches("minecraft:"));
            for (name, selectors) in *variants {
                let file = dir.join(format!("{}.json", name.trim_start_matches("minecraft:")));
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                let json = Json::parse(&text).unwrap();
                let want = parse(json.get("spawn_conditions"));
                let have: Vec<Selector> = selectors.iter().map(|&(priority, c)| Selector { priority, condition: c.map(Condition::from) }).collect();
                assert_eq!(have, want, "{registry} {name}");
                checked += 1;
            }
        }
        if checked == 0 {
            eprintln!("skipped: no extracted datapack at {}", root.display());
        }
    }
}
