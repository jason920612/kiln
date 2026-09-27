//! Differential test against vanilla 26.3: replays the cases `tools/LootVectors.java` recorded
//! (`$KILN_WORK/wp4-loot/<kind>.jsonl`, skipped when absent) with the same contexts and seeds and
//! requires byte-identical stacks (kiln-item's `OPTIONAL_STREAM_CODEC`, with the components
//! vanilla keeps in hash order — enchantment maps, block state properties — sorted first).
//!
//! Without `KILN_PARITY=1` only the first 400 cases of each kind run.

use kiln_inventory::RecipeManager;
use kiln_inventory::recipe::CookingKind;
use kiln_item::component::EquipmentSlotGroup;
use kiln_item::{Component, Identifier, ItemStack, Text};
use kiln_loot::predicate::{DamageSourcePredicate, EntityPredicate, LocationPredicate};
use kiln_loot::random::{RandomSequences, seeded};
use kiln_loot::{EntityTarget, LootContext, LootData, Source};
use kiln_proto::Reader;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

fn work() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

fn full() -> bool {
    std::env::var("KILN_PARITY").is_ok_and(|v| v == "1")
}

fn data() -> Option<&'static (LootData, RecipeManager)> {
    static DATA: OnceLock<Option<(LootData, RecipeManager)>> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = work().join("generated");
        if !dir.join("data").is_dir() {
            return None;
        }
        let loot = LootData::load(&dir).expect("load loot");
        let recipes = RecipeManager::load(&dir).expect("load recipes");
        Some((loot, recipes))
    })
    .as_ref()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn stack(hex: &str) -> ItemStack {
    let bytes = unhex(hex);
    let mut r = Reader::new(&bytes);
    let s = ItemStack::read_optional(&mut r).unwrap_or_else(|e| panic!("stack {hex}: {e:?}"));
    assert_eq!(r.remaining(), 0);
    s
}

/// The wire bytes of a stack with hash-ordered component contents sorted.
fn canonical_bytes(s: &ItemStack) -> Vec<u8> {
    let mut patch = kiln_item::DataComponentPatch::new();
    for (id, v) in s.patch().iter() {
        match v {
            None => patch.remove(id),
            Some(c) => patch.set(match c {
                Component::Enchantments(e) => {
                    let mut e = e.clone();
                    e.0.sort();
                    Component::Enchantments(e)
                }
                Component::StoredEnchantments(e) => {
                    let mut e = e.clone();
                    e.0.sort();
                    Component::StoredEnchantments(e)
                }
                Component::BlockState(b) => {
                    let mut b = b.clone();
                    b.0.sort();
                    Component::BlockState(b)
                }
                other => other.clone(),
            }),
        }
    }
    let mut out = bytes::BytesMut::new();
    if s.is_empty() {
        ItemStack::empty().write_optional(&mut out);
    } else {
        // Rebuild with the patch as is (no re-normalization against defaults).
        out.extend_from_slice(&{
            let mut b = bytes::BytesMut::new();
            kiln_proto::WriteExt::put_varint(&mut b, s.count());
            kiln_proto::WriteExt::put_varint(&mut b, s.item());
            patch.write(&mut b);
            b
        });
    }
    out.to_vec()
}

fn show(s: &ItemStack) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let mut out = format!("{}x{}", s.count(), s.item_name().trim_start_matches("minecraft:"));
    for (id, v) in s.patch().iter() {
        match v {
            Some(c) => out += &format!(" {}={:?}", kiln_item::component::name(id).trim_start_matches("minecraft:"), c),
            None => out += &format!(" !{}", kiln_item::component::name(id)),
        }
    }
    out
}

struct Entity {
    enchantments: HashMap<i32, i32>,
}

struct BlockEntity {
    components: Vec<Component>,
    /// `None`: not nameable; `Some(None)`: no custom name.
    name: Option<Option<Text>>,
}

/// A context replaying one recorded case.
struct Case<'a> {
    recipes: &'a RecipeManager,
    origin: Option<[f64; 3]>,
    tool: Option<ItemStack>,
    state: Option<u16>,
    explosion: Option<f32>,
    luck: f32,
    damage: bool,
    entities: HashMap<EntityTarget, Entity>,
    block_entity: Option<BlockEntity>,
    entity_answers: HashMap<String, bool>,
    damage_answers: HashMap<String, bool>,
    location_answers: HashMap<String, bool>,
    /// A predicate vanilla could not answer was asked (the case is inconclusive).
    unanswered: Cell<bool>,
}

impl LootContext for Case<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        self.entities.contains_key(&target)
    }
    fn origin(&self) -> Option<[f64; 3]> {
        self.origin
    }
    fn block_state(&self) -> Option<u16> {
        self.state
    }
    fn has_block_entity(&self) -> bool {
        self.block_entity.is_some()
    }
    fn tool(&self) -> Option<&ItemStack> {
        self.tool.as_ref()
    }
    fn explosion_radius(&self) -> Option<f32> {
        self.explosion
    }
    fn has_damage_source(&self) -> bool {
        self.damage
    }
    fn luck(&self) -> f32 {
        self.luck
    }
    fn entity_enchantment_level(&self, target: EntityTarget, enchantment: i32, _slots: &[EquipmentSlotGroup]) -> i32 {
        self.entities.get(&target).and_then(|e| e.enchantments.get(&enchantment).copied()).unwrap_or(0)
    }
    fn entity_matches(&self, target: EntityTarget, predicate: &EntityPredicate) -> bool {
        let key = format!("{}|{}", target.name(), predicate.json.canonical());
        match self.entity_answers.get(&key) {
            Some(v) => *v,
            None => {
                self.unanswered.set(true);
                false
            }
        }
    }
    fn damage_source_matches(&self, predicate: &DamageSourcePredicate) -> bool {
        match self.damage_answers.get(&predicate.json.canonical()) {
            Some(v) => *v,
            None => {
                self.unanswered.set(true);
                false
            }
        }
    }
    fn location_matches(&self, predicate: &LocationPredicate, pos: [f64; 3]) -> bool {
        let o = self.origin.unwrap();
        let key = format!(
            "{}|{},{},{}",
            predicate.json.canonical(),
            (pos[0] - o[0]).round() as i32,
            (pos[1] - o[1]).round() as i32,
            (pos[2] - o[2]).round() as i32
        );
        match self.location_answers.get(&key) {
            Some(v) => *v,
            None => {
                self.unanswered.set(true);
                false
            }
        }
    }
    fn custom_name(&self, source: Source) -> Option<Option<Text>> {
        match source {
            Source::BlockEntity => self.block_entity.as_ref().and_then(|b| b.name.clone()),
            _ => None,
        }
    }
    fn components(&self, source: Source) -> Option<Vec<Component>> {
        match source {
            Source::BlockEntity => self.block_entity.as_ref().map(|b| b.components.clone()),
            Source::Tool => self.tool.as_ref().map(|t| {
                (0..kiln_item::component::count() as u16).filter_map(|id| t.component(id).cloned()).collect()
            }),
            Source::Entity(_) => None,
        }
    }
    fn smelt(&self, input: &ItemStack) -> Option<ItemStack> {
        self.recipes.find_cooking(CookingKind::Smelting, input, None).map(|i| self.recipes.assemble_single(i))
    }
}

fn answers(v: &serde_json::Value) -> HashMap<String, bool> {
    v.as_object().map(|m| m.iter().map(|(k, b)| (k.clone(), b.as_bool().unwrap())).collect()).unwrap_or_default()
}

fn case<'a>(recipes: &'a RecipeManager, c: &serde_json::Value) -> Case<'a> {
    let ctx = &c["context"];
    let enchantments = |e: &serde_json::Value| -> HashMap<i32, i32> {
        e.as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, l)| (kiln_item::registry::ENCHANTMENT.id(k).unwrap(), l.as_i64().unwrap() as i32))
                    .collect()
            })
            .unwrap_or_default()
    };
    let entities = ctx["entities"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, e)| (EntityTarget::by_name(k).unwrap(), Entity { enchantments: enchantments(&e["enchantments"]) }))
                .collect()
        })
        .unwrap_or_default();
    let block_entity = ctx.get("block_entity").map(|be| BlockEntity {
        components: be["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| {
                let bytes = unhex(h.as_str().unwrap());
                let mut r = Reader::new(&bytes);
                let id = r.varint().unwrap() as u16;
                Component::read(id, &mut r).unwrap()
            })
            .collect(),
        name: match &be["name"] {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) if s.is_empty() => Some(None),
            serde_json::Value::String(s) => {
                let bytes = unhex(s);
                Some(Some(Text::read(&mut Reader::new(&bytes)).unwrap()))
            }
            other => panic!("name {other}"),
        },
    });
    Case {
        recipes,
        origin: ctx.get("origin").map(|o| {
            let a = o.as_array().unwrap();
            [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
        }),
        tool: ctx.get("tool").map(|t| stack(t.as_str().unwrap())),
        state: ctx.get("block_state").map(|s| s.as_u64().unwrap() as u16),
        explosion: ctx.get("explosion_radius").map(|e| e.as_f64().unwrap() as f32),
        luck: ctx["luck"].as_f64().unwrap_or(0.0) as f32,
        damage: ctx.get("damage_source").is_some(),
        entities,
        block_entity,
        entity_answers: answers(&ctx["entity_predicates"]),
        damage_answers: answers(&ctx["damage_predicates"]),
        location_answers: answers(&ctx["location_predicates"]),
        unanswered: Cell::new(false),
    }
}

/// Replays one case; `Err` describes the first difference.
fn replay(data: &LootData, recipes: &RecipeManager, c: &serde_json::Value) -> Result<bool, String> {
    let table = Identifier::parse(c["table"].as_str().unwrap()).unwrap();
    let ctx = case(recipes, c);
    let lt = data.table(&table).ok_or("table not loaded")?;
    let expected: Vec<Vec<ItemStack>> = c["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run.as_array().unwrap().iter().map(|h| stack(h.as_str().unwrap())).collect())
        .collect();
    let mut level = seeded(0);
    let mut got: Vec<Vec<ItemStack>> = Vec::new();
    match c["mode"].as_str().unwrap() {
        "sequence" => {
            let mut seqs = RandomSequences::new(c["world_seed"].as_i64().unwrap());
            for _ in 0..expected.len() {
                let mut rng = lt.random(0, &mut seqs, &mut level);
                got.push(data.random_items(&table, &ctx, rng.source()));
            }
        }
        "seed" => {
            let mut seqs = RandomSequences::new(0);
            let mut rng = lt.random(c["seed"].as_i64().unwrap(), &mut seqs, &mut level);
            got.push(data.random_items(&table, &ctx, rng.source()));
        }
        "fill" => {
            let mut seqs = RandomSequences::new(0);
            let mut rng = lt.random(c["seed"].as_i64().unwrap(), &mut seqs, &mut level);
            let mut container = vec![ItemStack::empty(); 27];
            container[4] = ItemStack::of("minecraft:stone", 1).unwrap();
            data.fill(&table, &ctx, rng.source(), &mut container);
            got.push(container);
        }
        other => return Err(format!("unknown mode {other}")),
    }
    if ctx.unanswered.get() {
        return Ok(false);
    }
    for (run, (want, have)) in expected.iter().zip(&got).enumerate() {
        let w: Vec<Vec<u8>> = want.iter().map(canonical_bytes).collect();
        let h: Vec<Vec<u8>> = have.iter().map(canonical_bytes).collect();
        if w != h {
            return Err(format!(
                "run {run}:\n  vanilla: [{}]\n  kiln:    [{}]",
                want.iter().map(show).collect::<Vec<_>>().join(", "),
                have.iter().map(show).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(true)
}

#[test]
fn loot_tables_match_vanilla() {
    let Some((data, recipes)) = data() else { return };
    let dir = work().join("wp4-loot");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
    files.sort();
    let limit = if full() { usize::MAX } else { 400 };
    let mut report: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    let mut failures = Vec::new();
    for f in &files {
        let kind = f.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(f).unwrap();
        let entry = report.entry(kind.clone()).or_default();
        let mut tables = std::collections::HashSet::new();
        for line in text.lines().take(limit) {
            let c: serde_json::Value = serde_json::from_str(line).unwrap();
            tables.insert(c["table"].as_str().unwrap().to_owned());
            match replay(data, recipes, &c) {
                Ok(true) => entry.0 += 1,
                Ok(false) => entry.2 += 1,
                Err(e) => {
                    entry.1 += 1;
                    if failures.len() < 40 {
                        failures.push(format!("{} [{}] {}", c["table"], c["mode"], e));
                    }
                }
            }
        }
        entry.3 = tables.len();
    }
    for (kind, (pass, fail, skip, tables)) in &report {
        eprintln!("{kind:16} tables {tables:5}  cases {:6}  pass {pass:6}  fail {fail:5}  inconclusive {skip}", pass + fail + skip);
    }
    for f in &failures {
        eprintln!("{f}");
    }
    let total_fail: usize = report.values().map(|r| r.1).sum();
    assert_eq!(total_fail, 0, "{total_fail} cases differ from vanilla");
}

