//! `CompassItem.inventoryTick` / `LodestoneTracker.tick` (wp80): a tracked lodestone compass in a player's inventory forgets its target once the lodestone is gone
//! (the target is only judged in the level it points at; a level the player is not in leaves it alone).

use kiln_item::component::LodestoneTracker;
use kiln_world::Blocks;

impl crate::Sim {
    pub(crate) fn tick_compasses(&mut self) {
        let conns: Vec<_> = self.players.keys().copied().collect();
        for conn in conns {
            let Some(p) = self.players.get(&conn) else { continue };
            let dim = p.dim;
            let here = crate::DIMENSIONS[dim].0;
            // (slot in the main inventory or equipment, target) of the tracked compasses pointing into this level.
            let mut check: Vec<(bool, usize, [i32; 3])> = Vec::new();
            let all = p.inv.items.iter().enumerate().map(|(i, s)| (false, i, s)).chain(p.inv.equipment.iter().enumerate().map(|(i, s)| (true, i, s)));
            for (equipment, i, stack) in all {
                if stack.is_empty() || stack.item_name() != "minecraft:compass" {
                    continue;
                }
                let Some(t) = stack.get(kiln_item::keys::LODESTONE_TRACKER) else { continue };
                if let (true, Some(target)) = (t.tracked, &t.target)
                    && target.dimension.to_string() == here
                {
                    check.push((equipment, i, target.pos));
                }
            }
            if check.is_empty() {
                continue;
            }
            let mut gone = Vec::new();
            for (equipment, i, pos) in check {
                let inside = pos[1] >= self.dims[dim].provider.dimension.min_y && pos[1] < self.dims[dim].provider.dimension.min_y + self.dims[dim].provider.dimension.height;
                // (A chunk that is not loaded cannot say: the compass keeps its target.)
                let lodestone = self.dims[dim].regions.get_block(pos[0], pos[1], pos[2]).map(|s| kiln_data::blocks_types::block_of(s).name == "minecraft:lodestone");
                if !inside || lodestone == Some(false) {
                    gone.push((equipment, i));
                }
            }
            let Some(p) = self.players.get_mut(&conn) else { continue };
            for (equipment, i) in gone {
                let stack = if equipment { &mut p.inv.equipment[i] } else { &mut p.inv.items[i] };
                stack.insert(kiln_item::keys::LODESTONE_TRACKER, LodestoneTracker { target: None, tracked: true });
                p.inv.times_changed += 1;
            }
        }
    }
}
