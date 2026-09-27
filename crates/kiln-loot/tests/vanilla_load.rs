//! Loads the vanilla 26.3 datapack from `$KILN_WORK/generated` (skipped when absent) and
//! checks every loot file decodes, then smoke-tests a few tables.

use kiln_item::{Identifier, ItemStack};
use kiln_loot::random::{RandomSequences, seeded};
use kiln_loot::{EmptyContext, Kind, LootContext, LootData};
use std::path::PathBuf;
use std::sync::OnceLock;

fn work() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

fn data() -> Option<&'static LootData> {
    static DATA: OnceLock<Option<LootData>> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = work().join("generated");
        dir.join("data").is_dir().then(|| LootData::load_lenient(&dir).expect("load datapack"))
    })
    .as_ref()
}

fn id(s: &str) -> Identifier {
    Identifier::parse(s).unwrap()
}

#[test]
fn every_vanilla_file_decodes() {
    let Some(data) = data() else { return };
    for e in &data.errors {
        eprintln!("{e}");
    }
    assert!(data.errors.is_empty(), "{} files failed", data.errors.len());
    assert!(data.table_ids().len() > 1400, "{} tables", data.table_ids().len());
    assert_eq!(data.ids(Kind::Predicate).len(), 3);
    assert!(data.enchantment(kiln_item::registry::ENCHANTMENT.id("minecraft:fortune").unwrap()).is_some());
}

struct Mining {
    tool: ItemStack,
    state: u16,
}

impl LootContext for Mining {
    fn tool(&self) -> Option<&ItemStack> {
        Some(&self.tool)
    }
    fn block_state(&self) -> Option<u16> {
        Some(self.state)
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some([0.5, 64.5, 0.5])
    }
}

#[test]
fn stone_drops_cobblestone_and_silk_touch_keeps_it() {
    let Some(data) = data() else { return };
    let stone = kiln_data::blocks_types::block_by_name("minecraft:stone").unwrap().default;
    let ctx = Mining { tool: ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap(), state: stone };
    let mut seqs = RandomSequences::new(1);
    let table = id("minecraft:blocks/stone");
    let mut level = seeded(0);
    let mut rng = data.table(&table).unwrap().random(0, &mut seqs, &mut level);
    let items = data.random_items(&table, &ctx, rng.source());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item_name(), "minecraft:cobblestone");

    let mut pick = ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap();
    let silk = kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").unwrap();
    kiln_loot::enchant::enchant(&mut pick, silk, 1);
    let ctx = Mining { tool: pick, state: stone };
    let items = data.random_items(&table, &ctx, rng.source());
    assert_eq!(items[0].item_name(), "minecraft:stone");
}

/// Evaluation throughput (`cargo test --release -p kiln-loot --test vanilla_load -- --ignored
/// --nocapture`).
#[test]
#[ignore]
fn throughput() {
    let Some(data) = data() else { return };
    let ore = kiln_data::blocks_types::block_by_name("minecraft:diamond_ore").unwrap().default;
    let mut pick = ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap();
    kiln_loot::enchant::enchant(&mut pick, kiln_item::registry::ENCHANTMENT.id("minecraft:fortune").unwrap(), 3);
    let ctx = Mining { tool: pick, state: ore };
    let mut seqs = RandomSequences::new(7);
    let mut level = seeded(0);
    for (table, n) in [("minecraft:blocks/diamond_ore", 200_000), ("minecraft:chests/simple_dungeon", 50_000), ("minecraft:entities/zombie", 200_000)] {
        let table = id(table);
        let start = std::time::Instant::now();
        let mut items = 0usize;
        for _ in 0..n {
            let mut rng = data.table(&table).unwrap().random(0, &mut seqs, &mut level);
            items += data.random_items(&table, &ctx, rng.source()).len();
        }
        let per = start.elapsed().as_secs_f64() / n as f64;
        eprintln!("{table}: {:.2} µs per evaluation ({items} stacks)", per * 1e6);
    }
    let start = std::time::Instant::now();
    let dir = work().join("generated");
    let _ = LootData::load(&dir).unwrap();
    eprintln!("load: {:.0} ms", start.elapsed().as_secs_f64() * 1e3);
}

#[test]
fn chests_fill_containers() {
    let Some(data) = data() else { return };
    let table = id("minecraft:chests/simple_dungeon");
    let mut container = vec![ItemStack::empty(); 27];
    let mut rng = seeded(12345);
    data.fill(&table, &EmptyContext, &mut rng, &mut container);
    assert!(container.iter().any(|s| !s.is_empty()));
}
