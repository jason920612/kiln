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
    /// The player's stand-in for the level random.
    pub rng: &'a mut kiln_javamath::random::LegacyRandom,
    /// The player's own random (`Entity.random`).
    pub player_rng: &'a mut kiln_javamath::random::LegacyRandom,
    /// Bookshelves around the open enchanting table.
    pub bookshelves: i32,
    /// `minecraft:limited_crafting` and the player's recipe book.
    pub limited_crafting: bool,
    pub recipes: &'a crate::recipe_book::RecipeBook,
    /// The server's maps (cartography tables, crafted map copies).
    pub maps: &'a crate::maps::SharedMaps,
}

impl SimWorld<'_> {
    fn enchantment(&self, id: i32) -> Option<&kiln_loot::enchant::Enchantment> {
        self.loot?.enchantment(id)
    }
}

impl kiln_inventory::World for SimWorld<'_> {
    fn map_scale(&self, map_id: i32) -> Option<i8> {
        self.maps.lock().ok()?.get(map_id).map(|m| m.scale)
    }

    fn map_locked(&self, map_id: i32) -> Option<bool> {
        self.maps.lock().ok()?.get(map_id).map(|m| m.locked)
    }

    fn post_process_map(&mut self, stack: &mut kiln_item::ItemStack) {
        crate::map_items::post_process(self.maps, stack);
    }

    fn limited_crafting(&self) -> bool {
        self.limited_crafting
    }

    fn knows_recipe(&self, recipe: &str) -> bool {
        self.recipes.contains(recipe)
    }

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

    fn select_enchantments(&self, rng: &mut dyn kiln_javamath::random::RandomSource, stack: &kiln_item::ItemStack, cost: i32) -> Vec<(i32, i32)> {
        let Some(loot) = self.loot else { return Vec::new() };
        let Some(ids) = kiln_inventory::tags::entries("minecraft:enchantment", "minecraft:in_enchanting_table") else { return Vec::new() };
        let candidates: Vec<&kiln_loot::enchant::Enchantment> = ids.iter().filter_map(|&id| loot.enchantment(id)).collect();
        kiln_loot::enchant::select(rng, stack, cost, &candidates)
    }

    fn enchanting_bookshelves(&self) -> i32 {
        self.bookshelves
    }

    fn next_player_int(&mut self) -> i32 {
        use kiln_javamath::random::RandomSource;
        self.player_rng.next_int()
    }
}

/// `EnchantingTableBlock.BOOKSHELF_OFFSETS` with `isValidBookShelf`: bookshelves two blocks out
/// (on the table's level and the one above) with nothing solid-ish between.
pub(crate) fn count_bookshelves(level: &crate::blocks::RegionLevel, pos: kiln_blocks::BlockPos) -> i32 {
    use kiln_blocks::Level;
    let mut n = 0;
    for dx in -2..=2 {
        for dy in 0..=1 {
            for dz in -2..=2 {
                if dx != -2 && dx != 2 && dz != -2 && dz != 2 {
                    continue;
                }
                let shelf = level.block(pos.offset(dx, dy, dz));
                let between = level.block(pos.offset(dx / 2, dy, dz / 2));
                if kiln_blocks::tags::is(shelf, "minecraft:enchantment_power_provider")
                    && kiln_blocks::tags::is(between, "minecraft:enchantment_power_transmitter")
                {
                    n += 1;
                }
            }
        }
    }
    n
}
