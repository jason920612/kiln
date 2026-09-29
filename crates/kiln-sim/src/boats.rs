//! Boats in players' hands (`BoatItem.use`): the view ray (fluids included) finds where the
//! boat goes; it appears there facing the way the player does, unless something solid is in
//! the way. The item is used up and counted.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use crate::use_item::{FluidMode, pov_hit};
use kiln_blocks::Level;
use kiln_entity::math::Vec3;
use kiln_inventory::Container;

/// Whether `name` is a boat item.
pub(crate) fn is_boat_item(name: &str) -> bool {
    kiln_entity::ext_entity::boat::is_boat(name)
}

/// Whether no block collision shape overlaps the box.
fn free_of_blocks(level: &RegionLevel, bb: &kiln_entity::math::Aabb) -> bool {
    for x in (bb.min_x.floor() as i32 - 1)..=(bb.max_x.floor() as i32 + 1) {
        for y in (bb.min_y.floor() as i32 - 1)..=(bb.max_y.floor() as i32 + 1) {
            for z in (bb.min_z.floor() as i32 - 1)..=(bb.max_z.floor() as i32 + 1) {
                let state = level.block(kiln_blocks::BlockPos::new(x, y, z));
                for b in kiln_data::block_props::collision(state) {
                    let (lo, hi) = ([x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64], [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64]);
                    if lo[0] < bb.max_x && hi[0] > bb.min_x && lo[1] < bb.max_y && hi[1] > bb.min_y && lo[2] < bb.max_z && hi[2] > bb.min_z {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// `BoatItem.use`.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let stack = p.in_hand(off_hand).clone();
    let hit = {
        let lv = &*level;
        pov_hit(p, &|pos| lv.block(pos), FluidMode::Any)
    };
    let Some(hit) = hit else { return };
    let Some(kind) = kiln_data::entities::by_name(stack.item_name()) else { return };
    let at = Vec3::new(hit.location[0], hit.location[1], hit.location[2]);
    let seed = crate::ranged::projectile_seed(level, p, spawns.len() as u64);
    let boat = kiln_entity::ext_entity::boat::new(kind.name, at, p.rot[0], seed);
    if !free_of_blocks(level, &boat.bounding_box()) {
        return;
    }
    crate::ranged::push_spawn(spawns, boat);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
}

/// Whether `name` is a minecart item.
pub(crate) fn is_minecart_item(name: &str) -> bool {
    kiln_entity::ext_entity::minecart::is_minecart(name)
}

/// `MinecartItem.useOn`: a minecart on the clicked rail (raised half a block on a slope).
pub(crate) fn use_minecart_on(p: &mut Player, level: &mut RegionLevel, pos: kiln_blocks::BlockPos, off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    let stack = p.in_hand(off_hand).clone();
    let state = level.block(pos);
    let Some(shape) = kiln_entity::ext_entity::minecart::rail_shape(state) else { return false };
    let Some(kind) = kiln_data::entities::by_name(stack.item_name()) else { return false };
    let lift = if shape.is_slope() { 0.5 } else { 0.0 };
    let at = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.0625 + lift, pos.z as f64 + 0.5);
    let seed = crate::ranged::projectile_seed(level, p, spawns.len() as u64);
    let cart = kiln_entity::ext_entity::minecart::new(kind.name, at, seed);
    crate::ranged::push_spawn(spawns, cart);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    p.used_on_block("minecraft:item_used_on_block", [pos.x, pos.y, pos.z], level.block(pos), &stack, &probe);
    true
}
