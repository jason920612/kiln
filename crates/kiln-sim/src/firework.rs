//! Firework rockets in players' hands (`FireworkRocketItem`): used while gliding, one rides
//! along with the player (the elytra boost); used on a block, one launches from the clicked
//! face. Either way the item is used up (not in creative) and counts as used.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use kiln_blocks::{BlockPos, Direction};
use kiln_entity::math::Vec3;
use kiln_inventory::Container;

pub(crate) const ITEM: &str = "minecraft:firework_rocket";

fn consume(p: &mut Player, off_hand: bool) {
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
}

/// `FireworkRocketItem.use`: only while gliding.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    if !p.fall_flying {
        return;
    }
    let stack = p.in_hand(off_hand).clone();
    let seed = crate::ranged::projectile_seed(level, p, spawns.len() as u64);
    let e = kiln_entity::ext_entity::firework::new(Vec3::new(p.pos[0], p.pos[1], p.pos[2]), stack.with_count(1), Some(p.entity_id), Some(p.entity_id), false, seed);
    crate::ranged::push_spawn(spawns, e);
    consume(p, off_hand);
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
}

/// `FireworkRocketItem.useOn`: `false` (pass) while gliding.
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, cursor: [f32; 3], off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    if p.fall_flying {
        return false;
    }
    let stack = p.in_hand(off_hand).clone();
    let (sx, sy, sz) = match face {
        Direction::Down => (0, -1, 0),
        Direction::Up => (0, 1, 0),
        Direction::North => (0, 0, -1),
        Direction::South => (0, 0, 1),
        Direction::West => (-1, 0, 0),
        Direction::East => (1, 0, 0),
    };
    let at = Vec3::new(
        pos.x as f64 + cursor[0] as f64 + sx as f64 * 0.15,
        pos.y as f64 + cursor[1] as f64 + sy as f64 * 0.15,
        pos.z as f64 + cursor[2] as f64 + sz as f64 * 0.15,
    );
    let seed = crate::ranged::projectile_seed(level, p, spawns.len() as u64);
    let e = kiln_entity::ext_entity::firework::new(at, stack.with_count(1), Some(p.entity_id), None, false, seed);
    crate::ranged::push_spawn(spawns, e);
    consume(p, off_hand);
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    use kiln_blocks::Level;
    p.used_on_block("minecraft:item_used_on_block", [pos.x, pos.y, pos.z], level.block(pos), &stack, &probe);
    true
}
