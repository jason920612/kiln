//! Tools and break times: which tool a block wants and how many ticks the vanilla client needs
//! to break it (`Player.getDestroySpeed`, `BlockBehaviour.getDestroyProgress`).

use kiln_data::block_props;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hand,
    Pickaxe,
    Shovel,
    Axe,
    Sword,
    Hoe,
}

/// What the bots carry. Tier speeds: wood 2, stone 4, iron 6, diamond 8.
pub const TIER_SPEED: f32 = 6.0;

/// Hotbar slot of each tool kind in a bot's kit.
pub fn slot_of(kind: Kind) -> u8 {
    match kind {
        Kind::Pickaxe => 0,
        Kind::Shovel => 1,
        Kind::Axe => 2,
        Kind::Sword => 3,
        Kind::Hoe => 6,
        Kind::Hand => 7,
    }
}

pub fn item_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Pickaxe => "iron_pickaxe",
        Kind::Shovel => "iron_shovel",
        Kind::Axe => "iron_axe",
        Kind::Sword => "iron_sword",
        Kind::Hoe => "iron_hoe",
        Kind::Hand => "bread",
    }
}

fn has(name: &str, parts: &[&str]) -> bool {
    parts.iter().any(|p| name.contains(p))
}

/// The tool whose block tag (`mineable/...`) holds `state`, judged by name.
pub fn wanted(state: u16) -> Kind {
    let name = crate::world::name(state).trim_start_matches("minecraft:");
    if has(name, &["dirt", "grass_block", "podzol", "mycelium", "mud", "clay", "gravel", "sand", "snow", "soul_soil", "farmland", "dirt_path", "concrete_powder", "moss_block"])
        && !has(name, &["sandstone", "mud_brick", "bricks"])
    {
        return Kind::Shovel;
    }
    if has(name, &["leaves", "hay_block", "sculk", "shroomlight", "sponge", "nether_wart_block", "target"]) {
        return Kind::Hoe;
    }
    if block_props::requires_correct_tool(state)
        || has(
            name,
            &[
                "stone", "terracotta", "concrete", "prismarine", "brick", "ore", "deepslate", "tuff", "basalt", "andesite", "diorite", "granite",
                "netherrack", "purpur", "quartz", "obsidian", "furnace", "hopper", "piston", "dispenser", "dropper", "observer", "iron_", "copper",
                "lantern", "rail", "cauldron", "anvil", "ice", "dripstone", "calcite", "end_", "blackstone",
            ],
        )
    {
        return Kind::Pickaxe;
    }
    if has(name, &["log", "wood", "planks", "stem", "chest", "crafting_table", "barrel", "bookshelf", "fence", "door", "sign", "pumpkin", "melon", "bamboo", "ladder", "lectern", "note_block", "jukebox", "mushroom_block", "composter", "loom", "bee"]) {
        return Kind::Axe;
    }
    Kind::Hand
}

/// Ticks the vanilla client needs to break `state` with `kind` in hand: 0 for blocks that break
/// at once. `on_ground` and `in_water` slow breaking to a fifth each.
pub fn break_ticks(state: u16, kind: Kind, on_ground: bool, in_water: bool) -> Option<u32> {
    let hardness = block_props::hardness(state);
    if hardness < 0.0 {
        return None;
    }
    let right = wanted(state) == kind && kind != Kind::Hand;
    let mut speed = if right { TIER_SPEED } else { 1.0 };
    if kind == Kind::Sword && crate::world::name(state).contains("cobweb") {
        speed = 15.0;
    }
    if in_water {
        speed /= 5.0;
    }
    if !on_ground {
        speed /= 5.0;
    }
    let correct = !block_props::requires_correct_tool(state) || (right && kind == Kind::Pickaxe);
    let progress = speed / hardness / if correct { 30.0 } else { 100.0 };
    if hardness == 0.0 || progress >= 1.0 {
        return Some(0);
    }
    Some((1.0 / progress).ceil() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn matches_vanilla_break_times() {
        // Vanilla: stone 1.5, iron pickaxe 6 -> 6/1.5/30 = 0.133 per tick = 8 ticks.
        assert_eq!(break_ticks(d::STONE, Kind::Pickaxe, true, false), Some(8));
        // Dirt 0.5 by hand: 1/0.5/30 = 0.0667 -> 15 ticks; iron shovel: 6/0.5/30 = 0.4 -> 3.
        assert_eq!(break_ticks(d::DIRT, Kind::Hand, true, false), Some(15));
        assert_eq!(break_ticks(d::DIRT, Kind::Shovel, true, false), Some(3));
        // Stone by hand: needs the right tool for drops: 1/1.5/100 -> 150 ticks.
        assert_eq!(break_ticks(d::STONE, Kind::Hand, true, false), Some(150));
        assert_eq!(break_ticks(d::BEDROCK, Kind::Pickaxe, true, false), None);
        // Airborne is five times slower.
        assert_eq!(break_ticks(d::STONE, Kind::Pickaxe, false, false), Some(38));
    }

    #[test]
    fn classifies_blocks() {
        assert_eq!(wanted(d::STONE), Kind::Pickaxe);
        assert_eq!(wanted(d::DEEPSLATE_DIAMOND_ORE), Kind::Pickaxe);
        assert_eq!(wanted(d::DIRT), Kind::Shovel);
        assert_eq!(wanted(d::GRAVEL), Kind::Shovel);
        assert_eq!(wanted(d::OAK_LOG), Kind::Axe);
        assert_eq!(wanted(d::OAK_PLANKS), Kind::Axe);
        assert_eq!(wanted(d::COBBLESTONE), Kind::Pickaxe);
    }
}
