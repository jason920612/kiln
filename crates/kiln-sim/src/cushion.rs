//! Putting down cushions (`CushionItem.useOn`): the item is used on the top of a block and the cushion
//! sits at the height of the click, in the middle of the block (the one in front of the click, or the
//! clicked one when it can be replaced), when something is under it, it is not inside solid blocks and
//! no other cushion is there. Cauldrons, hoppers and composters are clicked on their collision shape.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use crate::phantom::PhantomLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level};
use kiln_entity::ext_entity::cushion;
use kiln_entity::math::{Aabb, Vec3};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::world_fx;

/// The sixteen cushion items.
pub(crate) fn is_cushion_item(name: &str) -> bool {
    name.starts_with("minecraft:") && name.ends_with("_cushion")
}

fn edir(d: kiln_entity::math::Direction) -> Direction {
    match d {
        kiln_entity::math::Direction::Down => Direction::Down,
        kiln_entity::math::Direction::Up => Direction::Up,
        kiln_entity::math::Direction::North => Direction::North,
        kiln_entity::math::Direction::South => Direction::South,
        kiln_entity::math::Direction::West => Direction::West,
        kiln_entity::math::Direction::East => Direction::East,
    }
}

/// `CushionItem.recalculateContextForSpecialCollisionShapes`: a click on a block of `#cushion_uses_collision_shape` is
/// aimed again at the collision shape (from the eyes, a thousandth past the click); no hit keeps the click.
fn click_on_collision_shape(p: &Player, level: &RegionLevel, clicked: BlockPos, face: Direction, loc: Vec3) -> (Direction, Vec3) {
    let state = level.block(clicked);
    if !kiln_entity::ext_entity::wither_skull::block_tag(state, "minecraft:cushion_uses_collision_shape") {
        return (face, loc);
    }
    let eye = p.eye_position();
    let eye = Vec3::new(eye[0], eye[1], eye[2]);
    let ray = loc - eye;
    let to = loc + ray.normalize().scale(0.001);
    let shape = kiln_entity::physics::collision_shape(state);
    match kiln_entity::clip::shape_clip(shape, eye, to, kiln_entity::math::BlockPos::new(clicked.x, clicked.y, clicked.z)) {
        Some((at, dir)) => (edir(dir), at),
        None => (face, loc),
    }
}

/// `CushionItem.useOn`; true when the click was dealt with (a failed placement is `FAIL`: nothing is used).
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, clicked: BlockPos, face: Direction, cursor: [f32; 3], off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    let loc = Vec3::new(clicked.x as f64 + cursor[0] as f64, clicked.y as f64 + cursor[1] as f64, clicked.z as f64 + cursor[2] as f64);
    let (face, loc) = click_on_collision_shape(p, level, clicked, face, loc);
    if face != Direction::Up {
        return true;
    }
    // `BlockPlaceContext.getClickedPos`: the clicked block when it can be replaced, else the one in front.
    let state = level.block(clicked);
    let replaceable = if kiln_blocks::state::is(state, kiln_data::blocks::default_state::SNOW) { kiln_blocks::state::get(state, "layers") == Some("1") } else { kiln_data::block_props::replaceable(state) };
    let pos = if replaceable { clicked } else { clicked.relative(face) };
    let at = Vec3::new(pos.x as f64 + 0.5, loc.y, pos.z as f64 + 0.5);
    let bx = cushion::spawn_box(at);
    let env = level.env;
    let phantom = PhantomLevel::new(&*level.cells, env.game_time, env.min_y, false);
    if !cushion::can_be_placed_at(&phantom, &bx) {
        return true;
    }
    if level.bodies.iter().any(|b| b.cushion && b.intersects([bx.min_x, bx.min_y, bx.min_z], [bx.max_x, bx.max_y, bx.max_z])) {
        return true;
    }
    let stack = p.in_hand(off_hand).clone();
    let color = stack.get(kiln_item::keys::CUSHION_COLOR).map_or(0, |c| c.id() as u8);
    // `Direction.fromYRot(rotation).toYRot()`: the cardinal direction the player faces, as a yaw.
    let yaw = (((p.rot[0] / 90.0 + 0.5) as f64).floor() as i32 & 3) as f32 * 90.0;
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (pos.x as u64) << 32 ^ pos.z as u64 ^ (pos.y as u64) << 16);
    let mut entity = cushion::new(0, at, yaw, color, seed);
    // `EntityType.createDefaultStackConfig`: the item's name.
    if let Some(name) = stack.get(kiln_item::keys::CUSTOM_NAME) {
        entity.extra.push(("CustomName".into(), name.nbt().clone()));
    }
    let sound = |level: &mut RegionLevel, name: &str, source: world_fx::SoundSource, volume: f32, pitch: f32| {
        if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", name) {
            let seed = level.random().next_long();
            let pkt = world_fx::sound(&world_fx::Sound::Registered(id), source, [at.x, at.y, at.z], volume, pitch, seed);
            level.out.packets.push(([at.x, at.y, at.z], 16.0, pkt));
        }
    };
    // `addFreshEntity`, then `destroyIfInFire`: a cushion in a fire breaks at once.
    let in_fire = {
        let b = entity.bounding_box().next_deflated();
        let (lo, hi) = (BlockPos::new(b.min_x.floor() as i32, b.min_y.floor() as i32, b.min_z.floor() as i32), BlockPos::new(b.max_x.floor() as i32, b.max_y.floor() as i32, b.max_z.floor() as i32));
        (lo.x..=hi.x).any(|x| (lo.y..=hi.y).any(|y| (lo.z..=hi.z).any(|z| kiln_entity::blocks::has_tag(level.block(BlockPos::new(x, y, z)), kiln_entity::blocks::Tag::Fire))))
    };
    if in_fire {
        sound(level, "minecraft:entity.cushion.break", world_fx::SoundSource::Neutral, 1.0, 1.0);
        let mut item = kiln_item::ItemStack::of(&cushion::item_name(color), 1).unwrap_or_default();
        if let Some(name) = stack.get(kiln_item::keys::CUSTOM_NAME) {
            item.set(kiln_item::component::Component::CustomName(name.clone()));
        }
        spawns.push(crate::mobs::drop_item(item, [at.x, at.y, at.z], seed as u64));
    } else if let Some(kind) = kiln_data::entities::by_name(cushion::TYPE) {
        spawns.push(Spawn { kind, pos: [at.x, at.y, at.z], vel: [0.0; 3], body: Body::Ready(Box::new(entity)) });
    }
    sound(level, "minecraft:entity.cushion.place", world_fx::SoundSource::Blocks, 0.75, 0.8);
    level.effect(Effect::GameEvent { pos: BlockPos::new(pos.x, pos.y, pos.z), event: "minecraft:entity_place" });
    let item = p.in_hand(off_hand).item();
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
    if p.game_mode != 1 {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    let _ = Aabb::new;
    true
}
