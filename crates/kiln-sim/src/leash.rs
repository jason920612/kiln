//! Leads in the simulation: a click on a fence block with leads tied to the player moves them
//! to the knot on the fence (`FenceBlock.useWithoutItem`, `LeadItem.useOn`). Clicks on mobs and
//! boats are `kiln_entity::leash::interact`'s; the rest of a lead's life is the entities' tick.

use crate::blocks::RegionLevel;
use crate::entities::{Entities, Spawn};
use crate::{Player, health};
use kiln_entity::blocks::{Tag, has_tag};
use kiln_item::component::EquipmentSlot;

/// Whether a click on `pos` with `hand` (0 main, 1 off) would reach `bindPlayerMobs`: the fence
/// reacts to a main-hand click (unless sneaking with something in hand), a lead in the clicked
/// hand binds when the fence passes the click on.
pub(crate) fn intercepts(
    p: &Player,
    cells: &kiln_region::CellSet<kiln_world::Cell>,
    env: &crate::blocks::BlockEnv,
    hand: i32,
    pos: [i32; 3],
    face: i32,
    cursor: [f32; 3],
) -> bool {
    use kiln_world::Blocks;
    if p.game_mode == 3 || p.awaiting_teleport.is_some() || crate::blocks::direction(face).is_none() {
        return false;
    }
    if !p.can_reach_block(pos, 1.0) || cursor.iter().any(|&c| (c as f64 - 0.5).abs() >= 1.0000001) || pos[1] > env.min_y + env.height - 1 {
        return false;
    }
    if !cells.get_block(pos[0], pos[1], pos[2]).is_some_and(|s| has_tag(s, Tag::Fences)) {
        return false;
    }
    let main = hand == 0;
    let have_something = !p.inv.selected_item().is_empty() || !p.inv.equipped(EquipmentSlot::OffHand).is_empty();
    let held = if main { p.inv.selected_item() } else { p.inv.equipped(EquipmentSlot::OffHand) };
    let lead = !held.is_empty() && held.item_name() == "minecraft:lead";
    (main && !(p.sneaking && have_something)) || lead
}

/// `LeadItem.bindPlayerMobs` for player `i` and the fence at `pos`; whether any lead moved.
pub(crate) fn bind(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    i: usize,
    pos: [i32; 3],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
) -> bool {
    let who = players[i].entity_id;
    let bp = kiln_entity::math::BlockPos::new(pos[0], pos[1], pos[2]);
    let salt = (pos[0] as u64) << 40 ^ (pos[2] as u64) << 16 ^ pos[1] as u64 ^ 0x6c65_6173;
    crate::entities::with_level(entities, level, players, spawns, deaths, salt, |lvl| kiln_entity::leash::bind_player_mobs(lvl, who, bp))
}
