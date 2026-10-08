//! Putting up armor stands (`ArmorStandItem.useOn`): the item is used on a block face (not the
//! underside) and the stand stands in the block in front, dropped onto what lies under it
//! (`EntityType.create` with `alignPosition` and `invertY`), when its box is clear of blocks and
//! entities.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use crate::phantom::PhantomLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level};
use kiln_entity::ext_entity::armor_stand;
use kiln_entity::math::{Aabb, Axis, Vec3};
use kiln_javamath::random::RandomSource;

/// `ArmorStandItem.useOn`; true when the click was taken (the item may be used up).
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, clicked: BlockPos, face: Direction, off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    // (An adventure player's item use on a block needs `can_place_on`: not here.)
    if p.game_mode > 1 {
        return true;
    }
    if face == Direction::Down {
        return true;
    }
    // `BlockPlaceContext.getClickedPos`: the clicked block when it can be replaced, else the one in front.
    let replaceable = kiln_data::block_props::replaceable(level.block(clicked));
    let pos = if replaceable { clicked } else { clicked.relative(face) };
    let env = level.env;
    let (cx, cy, cz) = (pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5);
    let (w, h) = (0.5f64, 1.975f64);
    let at_pos = Aabb::new(cx - w / 2.0, cy, cz - w / 2.0, cx + w / 2.0, cy + h, cz + w / 2.0);
    let phantom = PhantomLevel::new(&*level.cells, env.game_time, env.min_y, false);
    let ctx = kiln_entity::collision::CollisionContext::EMPTY;
    let mut blocked = false;
    kiln_entity::collision::for_each_block_collision(&phantom, &ctx, &at_pos, |_, _, _| {
        blocked = true;
        false
    });
    if blocked || level.bodies.iter().any(|b| b.intersects([at_pos.min_x, at_pos.min_y, at_pos.min_z], [at_pos.max_x, at_pos.max_y, at_pos.max_z])) {
        return true;
    }
    // `EntityType.create`: the stand starts a block up and falls (at most two blocks) onto what is under.
    let start = Aabb::new(cx - w / 2.0, cy + 1.0, cz - w / 2.0, cx + w / 2.0, cy + 1.0 + h, cz + w / 2.0);
    let column = Aabb::new(pos.x as f64, pos.y as f64 - 1.0, pos.z as f64, pos.x as f64 + 1.0, pos.y as f64 + 1.0, pos.z as f64 + 1.0);
    let mut drop = -2.0;
    kiln_entity::collision::for_each_block_collision(&phantom, &ctx, &column, |_, shape, _| {
        if drop.abs() >= 1.0e-7 {
            drop = shape.collide(Axis::Y, &start, drop, [0.0; 3]);
        }
        true
    });
    let y = cy + (1.0 + drop);
    // The stand's own random draws (`create` turns it a random way; the item then sets the yaw).
    let _ = level.random().next_float();
    let yaw = (((kiln_entity::mob::mth::wrap_degrees(p.rot[0] - 180.0) + 22.5) / 45.0).floor()) * 45.0;
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (pos.x as u64) << 32 ^ pos.z as u64 ^ (pos.y as u64) << 16);
    let mut stand = armor_stand::new(0, Vec3::new(cx, y, cz), yaw, seed);
    // `EntityType.createDefaultStackConfig`: the item's name and `entity_data`.
    let stack = p.in_hand(off_hand).clone();
    if let Some(name) = stack.get(kiln_item::keys::CUSTOM_NAME) {
        stand.extra.push(("CustomName".into(), name.nbt().clone()));
    }
    if let Some(data) = stack.get(kiln_item::keys::ENTITY_DATA) {
        armor_stand::apply_saved(&mut stand, &data.tag);
    }
    let sound_at = [cx, y, cz];
    if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", "minecraft:entity.armor_stand.place") {
        let seed = level.random().next_long();
        let pkt = kiln_proto::packets::world_fx::sound(&kiln_proto::packets::world_fx::Sound::Registered(id), kiln_proto::packets::world_fx::SoundSource::Blocks, sound_at, 0.75, 0.8, seed);
        level.out.packets.push((sound_at, 16.0, pkt));
    }
    level.effect(Effect::GameEvent { pos: BlockPos::new(pos.x, pos.y, pos.z), event: "minecraft:entity_place" });
    if let Some(kind) = kiln_data::entities::by_name(armor_stand::ARMOR_STAND) {
        spawns.push(Spawn { kind, pos: sound_at, vel: [0.0; 3], body: Body::Ready(Box::new(stand)) });
    }
    let item = p.in_hand(off_hand).item();
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
    // (`itemStack.shrink(1)`; a creative player's count is put back by `useItemOn`.)
    if p.game_mode != 1 {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    true
}
