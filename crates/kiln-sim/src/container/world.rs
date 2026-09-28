//! What menus ask of the world ([`kiln_inventory::World`]): enchantment definitions from the
//! datapack's loot data, the server's `en_us` names, and a random for grindstone experience.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The server's `en_us.json` (`assets/minecraft/lang/en_us.json` next to the datapack's `data`,
/// or `KILN_LANG`); empty when not found.
fn lang() -> &'static HashMap<String, String> {
    static LANG: OnceLock<HashMap<String, String>> = OnceLock::new();
    LANG.get_or_init(|| {
        let path = std::env::var_os("KILN_LANG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::datapack_dir(None).join("assets/minecraft/lang/en_us.json"));
        let Ok(text) = std::fs::read_to_string(&path) else { return HashMap::new() };
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
        map.into_iter().filter_map(|(k, v)| Some((k, v.as_str()?.to_owned()))).collect()
    })
}

/// A player's menus in the running simulation.
pub(crate) struct SimWorld<'a> {
    pub loot: Option<&'a kiln_loot::LootData>,
    pub rng: &'a mut kiln_javamath::random::LegacyRandom,
}

impl SimWorld<'_> {
    fn enchantment(&self, id: i32) -> Option<&kiln_loot::enchant::Enchantment> {
        self.loot?.enchantment(id)
    }
}

impl kiln_inventory::World for SimWorld<'_> {
    fn translate(&self, key: &str) -> Option<String> {
        lang().get(key).cloned()
    }

    fn enchantment_can_enchant(&self, enchantment: i32, stack: &kiln_item::ItemStack) -> bool {
        self.enchantment(enchantment).is_some_and(|e| e.can_enchant(stack))
    }

    fn enchantments_compatible(&self, a: i32, b: i32) -> bool {
        match (self.enchantment(a), self.enchantment(b)) {
            (Some(x), Some(y)) => kiln_loot::enchant::compatible(x, y),
            _ => a != b,
        }
    }

    fn enchantment_max_level(&self, enchantment: i32) -> i32 {
        self.enchantment(enchantment).map_or(1, |e| e.max_level)
    }

    fn enchantment_anvil_cost(&self, enchantment: i32) -> i32 {
        self.enchantment(enchantment).map_or(1, |e| e.anvil_cost)
    }

    fn enchantment_min_cost(&self, enchantment: i32, level: i32) -> i32 {
        self.enchantment(enchantment).map_or(0, |e| e.min_cost.calculate(level))
    }

    fn random_int(&mut self, bound: i32) -> i32 {
        use kiln_javamath::random::RandomSource;
        if bound <= 0 { 0 } else { self.rng.next_int_bounded(bound) }
    }
}
