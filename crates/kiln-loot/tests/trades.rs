//! Villager trades of the vanilla 26.3 datapack (`$KILN_WORK/generated`, skipped when absent):
//! every trade set and trade decodes, and each profession level rolls its amount of offers.

use kiln_item::Identifier;
use kiln_loot::LootData;
use kiln_loot::random::seeded;
use kiln_loot::trade::{TradeContext, Trades};
use std::path::PathBuf;

fn work() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

#[test]
fn vanilla_trades() {
    let dir = work().join("generated");
    if !dir.join("data").is_dir() {
        return;
    }
    let data = LootData::load_lenient(&dir).expect("load datapack");
    let trades = Trades::load(&[&dir], &data);
    for e in &trades.errors {
        eprintln!("{e}");
    }
    assert!(trades.errors.is_empty(), "{} trade files failed", trades.errors.len());
    assert!(trades.trades.len() > 350, "{} trades", trades.trades.len());
    let profs = ["armorer", "butcher", "cartographer", "cleric", "farmer", "fisherman", "fletcher", "leatherworker", "librarian", "mason", "shepherd", "toolsmith", "weaponsmith"];
    for prof in profs {
        for level in 1..=5 {
            let id = Identifier::parse(&format!("minecraft:{prof}/level_{level}")).unwrap();
            let want = if prof == "librarian" && level == 5 { 3 } else { 2 };
            let ctx = TradeContext { origin: [0.5, 64.0, 0.5], entity_type: "minecraft:villager", villager_type: "minecraft:plains" };
            for seed in 1..20 {
                let offers = trades.offers(&data, &id, &ctx, &mut seeded(seed));
                // Some levels have fewer trades than their amount, type-restricted trades drop
                // out, and cartographer maps need a structure search Kiln does not have.
                let pool = trades.sets[&id].trades.len();
                if prof != "cartographer" {
                    assert!(!offers.is_empty() && offers.len() <= want.min(pool), "{id} seed {seed}: {offers:?}");
                }
                if pool >= 4 && prof != "cartographer" {
                    assert_eq!(offers.len(), want, "{id} seed {seed}");
                }
                for o in &offers {
                    assert!(o.cost_a.count >= 1 && o.max_uses >= 1 && !o.result.is_empty(), "{id}: {o:?}");
                }
            }
        }
    }
    // Enchanted books carry their enchantment's price.
    let lib = Identifier::parse("minecraft:librarian/level_1").unwrap();
    let ctx = TradeContext { origin: [0.5, 64.0, 0.5], entity_type: "minecraft:villager", villager_type: "minecraft:plains" };
    let mut books = 0;
    for seed in 1..60 {
        for o in trades.offers(&data, &lib, &ctx, &mut seeded(seed)) {
            if o.result.item_name() == "minecraft:enchanted_book" {
                books += 1;
                assert!(o.cost_a.count >= 2 && o.cost_b.is_some(), "{o:?}");
                assert!(!o.result.has(kiln_item::component::ids::ADDITIONAL_TRADE_COST));
            }
        }
    }
    assert!(books > 0);
}
