//! Steering sticks (`FoodOnAStickItem`): a carrot on a stick boosts the saddled pig a player rides,
//! a warped fungus on a stick the strider. The boost starts only when the stick steers the mount
//! (`ItemSteerable`, the player holding it); it costs the stick durability, and a broken stick
//! turns into a fishing rod.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Entities, Spawn};
use crate::health;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

/// Whether the player's `hand` holds a steering stick.
pub(crate) fn holds_stick(p: &Player, off_hand: bool) -> bool {
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let s = p.inv.equipped(slot);
    !s.is_empty() && matches!(s.item_name(), "minecraft:carrot_on_a_stick" | "minecraft:warped_fungus_on_a_stick")
}

/// `FoodOnAStickItem.use` by player `i`.
pub(crate) fn use_stick(entities: &mut Entities, level: &mut RegionLevel, players: &mut [&mut Player], i: usize, off_hand: bool, spawns: &mut Vec<Spawn>, deaths: &mut Vec<health::Death>) {
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let held = players[i].inv.equipped(slot).clone();
    let name = held.item_name();
    let boosted = match players[i].vehicle {
        Some(vehicle) => {
            let view = crate::entities::view(&*players[i], level.env.game_time);
            crate::entities::with_entity(entities, level, players, vehicle, spawns, deaths, 0x7374_6b, |phys, _| kiln_entity::mob::boost_with_stick(phys, name, &view)).flatten()
        }
        None => None,
    };
    let p = &mut *players[i];
    match boosted {
        Some(damage) => {
            // `hurtAndConvertOnBreak(damage, Items.FISHING_ROD, player, slot)`.
            p.hurt_and_break(slot, damage, None);
            if p.inv.equipped(slot).is_empty()
                && p.game_mode != 1
                && let Some(rod) = kiln_item::registry::ITEM.id("minecraft:fishing_rod")
            {
                let mut converted = ItemStack::from_parts(rod, 1, held.patch().clone());
                converted.insert(kiln_item::keys::DAMAGE, 0);
                p.set_in_hand(off_hand, converted);
            }
        }
        // `player.awardStat(Stats.ITEM_USED.get(this))`.
        None => p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, held.item()), 1),
    }
}
