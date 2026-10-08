//! Structure spawn overrides (`Structure.spawnOverrides()`): the mobs that spawn inside a
//! structure replace the biome's for the categories the structure lists (fortresses: blazes,
//! wither skeletons, magma cubes and zombified piglins; swamp huts: witches and cats; ocean
//! monuments: guardians; pillager outposts: pillagers; trial chambers and ancient cities: nothing).
//!
//! `ChunkGenerator.getMobsAt` finds the structures at a position from the chunk's saved
//! `structures.References` (the chunks whose structure starts reach it) and the starts those
//! chunks hold; a `piece` override applies inside one of the start's pieces, a `full` one
//! inside the start's bounding box. `NaturalSpawner.mobsAt` first asks whether the position is
//! inside a nether fortress above nether bricks, which has the fortress's own list.
//!
//! A structure start's bounding box is its pieces' box, inflated by 12 blocks when the structure
//! adapts the terrain (`Structure.adjustBoundingBox`: any `terrain_adaptation` but `none`).
//!
//! The structure data is the loaded chunks' own: a start chunk that is not loaded (it lies up to
//! 8 chunks from the chunk it is referenced from; vanilla loads it) counts as no start.

use crate::spawner::{SpawnerData, parse_spawner_list};
use kiln_proto::nbt::Tag;
use kiln_world::ChunkPos;
use std::collections::HashMap;

/// `StructureSpawnOverride.BoundingBoxType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxType {
    /// Inside one of the structure's pieces.
    Piece,
    /// Inside the structure start's bounding box.
    Full,
}

/// The structures' spawn overrides, from the datapack's `worldgen/structure/*.json`.
#[derive(Debug, Default)]
pub(crate) struct StructureSpawns {
    /// By structure id: whether its start's box is inflated, and per category name the box type
    /// and the list (which may be empty).
    by_structure: HashMap<String, Overrides>,
}

#[derive(Debug, Default)]
struct Overrides {
    /// `terrain_adaptation` is not `none`.
    inflated: bool,
    by_category: Vec<(String, BoxType, Vec<SpawnerData>)>,
}

/// `NetherFortressStructure.FORTRESS_ENEMIES`.
pub(crate) fn fortress_enemies() -> &'static [SpawnerData] {
    use kiln_entity::mob::MobKind;
    static LIST: std::sync::OnceLock<Vec<SpawnerData>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let d = |name: &'static str, weight: i32, min: i32, max: i32| SpawnerData { kind: MobKind::by_name(name), type_name: name, weight, min, max, constant: min == max };
        vec![
            d("minecraft:blaze", 10, 2, 3),
            d("minecraft:zombified_piglin", 5, 4, 4),
            d("minecraft:wither_skeleton", 8, 5, 5),
            d("minecraft:skeleton", 2, 5, 5),
            d("minecraft:magma_cube", 3, 4, 4),
        ]
    })
}

impl StructureSpawns {
    /// Reads `worldgen/structure/*.json` of the datapack at `dir`.
    pub fn load(dir: &std::path::Path) -> StructureSpawns {
        let mut t = StructureSpawns::default();
        let Ok(entries) = std::fs::read_dir(dir.join("data/minecraft/worldgen/structure")) else { return t };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let Some(overrides) = json["spawn_overrides"].as_object() else { continue };
            let mut list = Vec::new();
            for (category, o) in overrides {
                let boxed = match o["bounding_box"].as_str() {
                    Some("piece") => BoxType::Piece,
                    _ => BoxType::Full,
                };
                list.push((category.clone(), boxed, parse_spawner_list(&o["spawns"])));
            }
            if !list.is_empty() {
                list.sort_by(|a, b| a.0.cmp(&b.0));
                let inflated = json["terrain_adaptation"].as_str().is_some_and(|a| a != "none");
                t.by_structure.insert(format!("minecraft:{name}"), Overrides { inflated, by_category: list });
            }
        }
        t
    }

    pub fn is_empty(&self) -> bool {
        self.by_structure.is_empty()
    }

    /// `ChunkGenerator.getMobsAt` for the structures at block `pos`: the list of the first
    /// structure whose override for `category` applies there, if any. `structures` gives a loaded
    /// chunk's `structures` NBT.
    pub fn mobs_at<'c, F>(&self, structures: &F, pos: [i32; 3], category: &str) -> Option<&[SpawnerData]>
    where
        F: Fn(ChunkPos) -> Option<&'c Tag>,
    {
        if self.by_structure.is_empty() {
            return None;
        }
        let here = structures(ChunkPos::of_block(pos[0], pos[2]))?;
        let Some(Tag::Compound(refs)) = here.get("References") else { return None };
        for (id, chunks) in refs {
            let Some(o) = self.by_structure.get(id.as_str()) else { continue };
            let Some((_, boxed, spawns)) = o.by_category.iter().find(|(c, _, _)| c == category) else { continue };
            let Tag::LongArray(starts) = chunks else { continue };
            let found = starts.iter().any(|&packed| {
                let c = ChunkPos::new(packed as i32, (packed >> 32) as i32);
                start_of(structures, c, id).is_some_and(|pieces| match boxed {
                    BoxType::Piece => pieces.iter().any(|b| inside(b, pos)),
                    BoxType::Full => inside(&start_box(&pieces, o.inflated), pos),
                })
            });
            if found {
                return Some(spawns);
            }
        }
        None
    }

    /// `StructureManager.getStructureAt(pos, fortress).isValid()`: a fortress start in the
    /// chunk's references whose bounding box holds `pos`.
    pub fn in_fortress<'c, F>(&self, structures: &F, pos: [i32; 3]) -> bool
    where
        F: Fn(ChunkPos) -> Option<&'c Tag>,
    {
        const FORTRESS: &str = "minecraft:fortress";
        let inflated = self.by_structure.get(FORTRESS).is_some_and(|o| o.inflated);
        let Some(here) = structures(ChunkPos::of_block(pos[0], pos[2])) else { return false };
        let Some(Tag::LongArray(starts)) = here.get("References").and_then(|r| r.get(FORTRESS)) else { return false };
        starts.iter().any(|&packed| {
            let c = ChunkPos::new(packed as i32, (packed >> 32) as i32);
            start_of(structures, c, FORTRESS).is_some_and(|pieces| inside(&start_box(&pieces, inflated), pos))
        })
    }
}

/// The bounding boxes of the pieces of structure `id`'s start in chunk `c` (none: no valid start).
fn start_of<'c, F>(structures: &F, c: ChunkPos, id: &str) -> Option<Vec<[i32; 6]>>
where
    F: Fn(ChunkPos) -> Option<&'c Tag>,
{
    let start = structures(c)?.get("starts")?.get(id)?;
    let Some(Tag::List(children)) = start.get("Children") else { return None };
    let boxes: Vec<[i32; 6]> = children
        .iter()
        .filter_map(|piece| match piece.get("BB") {
            Some(Tag::IntArray(bb)) if bb.len() == 6 => Some([bb[0], bb[1], bb[2], bb[3], bb[4], bb[5]]),
            _ => None,
        })
        .collect();
    (!boxes.is_empty()).then_some(boxes)
}

/// `StructureStart.getBoundingBox`: the pieces' union, inflated by 12 for terrain adapting structures.
fn start_box(pieces: &[[i32; 6]], inflated: bool) -> [i32; 6] {
    let mut b = union(pieces);
    if inflated {
        for a in 0..3 {
            b[a] -= 12;
            b[a + 3] += 12;
        }
    }
    b
}

fn union(boxes: &[[i32; 6]]) -> [i32; 6] {
    let mut u = boxes[0];
    for b in &boxes[1..] {
        for a in 0..3 {
            u[a] = u[a].min(b[a]);
            u[a + 3] = u[a + 3].max(b[a + 3]);
        }
    }
    u
}

/// `BoundingBox.isInside`: the edges count.
fn inside(b: &[i32; 6], p: [i32; 3]) -> bool {
    (0..3).all(|a| p[a] >= b[a] && p[a] <= b[a + 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawner::SpawnTable;
    use kiln_entity::mob::Category;
    use serde_json::Value;
    use std::path::PathBuf;

    fn work() -> PathBuf {
        std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
    }

    /// Typed JSON NBT (see `SpawnVectors.tagJson`).
    fn tag_of(v: &Value) -> Tag {
        let (k, x) = v.as_object().unwrap().iter().next().unwrap();
        match k.as_str() {
            "c" => Tag::Compound(x.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tag_of(v))).collect()),
            "l" => Tag::List(x.as_array().unwrap().iter().map(tag_of).collect()),
            "ia" => Tag::IntArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect()),
            "str" => Tag::String(x.as_str().unwrap().to_owned()),
            "i" => Tag::Int(x.as_i64().unwrap() as i32),
            "la" => Tag::LongArray(x.as_array().unwrap().iter().map(|v| v.as_str().unwrap().parse().unwrap()).collect()),
            _ => panic!("tag {k}"),
        }
    }

    fn category(name: &str) -> Category {
        Category::SPAWNING.into_iter().chain([Category::Misc]).find(|c| c.name() == name).unwrap()
    }

    fn key(d: &SpawnerData) -> (String, i64, i64, i64) {
        (d.type_name.to_owned(), d.weight as i64, d.min as i64, d.max as i64)
    }

    /// The lists `NaturalSpawner.mobsAt` returned in vanilla (`tools/spawn_vectors.py`), per
    /// position and mob category, against the spawn table's: the biome's list, a structure's
    /// override (`piece` or `full` boxes), or the fortress's.
    #[test]
    fn spawn_overrides_match_vanilla() {
        let path = std::env::var_os("KILN_STRUCTURE_SPAWN_VECTORS").map(PathBuf::from).unwrap_or_else(|| work().join("wp44/spawner/structure_spawns.jsonl"));
        let pack = std::env::var_os("KILN_DATAPACK").map(PathBuf::from).unwrap_or_else(|| work().join("generated"));
        if !path.exists() || !pack.join("data/minecraft/worldgen/structure").is_dir() {
            eprintln!("structure spawns: no vectors or datapack (tools/spawn_vectors.py); skipped");
            return;
        }
        let table = SpawnTable::load(&pack).expect("the spawn table");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines();
        let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let lists = head["lists"].as_object().unwrap();
        let categories: Vec<Category> = head["categories"].as_array().unwrap().iter().map(|c| category(c.as_str().unwrap())).collect();
        let mut chunks: HashMap<(String, ChunkPos), Tag> = HashMap::new();
        let mut samples = Vec::new();
        for l in lines {
            let v: Value = serde_json::from_str(l).unwrap();
            if let Some(c) = v.get("chunk") {
                chunks.insert((c[0].as_str().unwrap().to_owned(), ChunkPos::new(c[1].as_i64().unwrap() as i32, c[2].as_i64().unwrap() as i32)), tag_of(&v["structures"]));
            } else {
                samples.push(v["sample"].clone());
            }
        }
        let (mut checked, mut overridden, mut bad) = (0, 0, Vec::new());
        let mut by_kind: std::collections::BTreeMap<String, u32> = Default::default();
        for s in &samples {
            let dim = s[0].as_str().unwrap();
            let at = [s[1].as_i64().unwrap() as i32, s[2].as_i64().unwrap() as i32, s[3].as_i64().unwrap() as i32];
            let biome = kiln_data::synced_id("minecraft:worldgen/biome", s[4].as_str().unwrap()).unwrap() as u16;
            let bricks = s[5].as_bool().unwrap();
            let structures = |c: ChunkPos| chunks.get(&(dim.to_owned(), c));
            for (i, cat) in categories.iter().enumerate() {
                let want: Vec<(String, i64, i64, i64)> = lists[&s[6][i].as_i64().unwrap().to_string()]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| (e[0].as_str().unwrap().to_owned(), e[1].as_i64().unwrap(), e[2].as_i64().unwrap(), e[3].as_i64().unwrap()))
                    .collect();
                let got: Vec<_> = table.mobs_in(&structures, || bricks, biome, *cat, at).iter().map(key).collect();
                checked += 1;
                if got != table.list(biome, *cat).iter().map(key).collect::<Vec<_>>() {
                    overridden += 1;
                    *by_kind.entry(format!("{dim} {}", cat.name())).or_default() += 1;
                }
                if got != want && bad.len() < 12 {
                    bad.push(format!("{dim} {at:?} {} biome {}: kiln {got:?} vs vanilla {want:?}", cat.name(), s[4]));
                }
            }
        }
        eprintln!("structure spawns: {} samples, {checked} lists compared, {overridden} not the biome's; by dimension and category: {by_kind:?}", samples.len());
        assert!(bad.is_empty(), "lists differ from vanilla's, for example:\n{}", bad.join("\n"));
        assert!(overridden > 50, "only {overridden} lists were overrides: the vectors hold no structure spawns");
    }
}
