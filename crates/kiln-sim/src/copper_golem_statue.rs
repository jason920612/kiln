//! Copper golem statues (`CopperGolemStatueBlock`, `WeatheringCopperGolemStatueBlock`): a click
//! turns the pose to the next (standing, sitting, running, star), and an axe on an unweathered
//! statue brings the golem back (named as the statue is) and takes the statue away; honeycomb and
//! the axe on the others go on to the waxing and scraping every copper block has.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use crate::tools::hand_slot;
use kiln_blocks::{BlockPos, Effect, Level, flags, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_data::blocks::default_state as d;
use kiln_inventory::stack::StackExt as _;
use kiln_world::Blocks as _;
use kiln_entity::mob::MobKind;

/// The poses in order (`CopperGolemStatueBlock.Pose`).
const POSES: [&str; 4] = ["standing", "sitting", "running", "star"];

/// `useItemOn` of a statue: `Some(true)` it did something, `None` not a statue (or `PASS`: the item's
/// own use on the block goes on).
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool, spawns: &mut Vec<Spawn>) -> Option<bool> {
    let class = logic::block_class(s);
    if !matches!(class, C::CopperGolemStatueBlock | C::WeatheringCopperGolemStatueBlock) {
        return None;
    }
    let held = p.in_hand(off_hand).clone();
    let axe = !held.is_empty() && kiln_inventory::tags::contains("minecraft:item", "minecraft:axes", held.effective_item());
    if class == C::CopperGolemStatueBlock {
        // The waxed ones: an axe passes (it scrapes the wax off), everything else turns the pose.
        if axe {
            return None;
        }
        update_pose(level, pos, s);
        return Some(true);
    }
    // `WeatheringCopperGolemStatueBlock.useItemOn`: needs its block entity.
    if axe {
        if !is_unaffected(s) {
            return None;
        }
        // `removeStatue`: the golem, named as the statue, stands in the middle of the block facing as it does.
        let name = statue_name(level, pos);
        p.hurt_and_break(hand_slot(off_hand), 1, None);
        let facing = state::get(s, "facing").unwrap_or("north");
        let yaw = match facing {
            "south" => 0.0,
            "west" => 90.0,
            "north" => 180.0,
            _ => 270.0,
        };
        let at = [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5];
        let mut golem = kiln_entity::mob::new(MobKind::CopperGolem, 0, 0, p.entity_id as i64 ^ level.env.game_time);
        golem.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
        golem.y_rot = yaw;
        golem.set_old_pos_and_rot();
        if let Some(m) = kiln_entity::mob::data_mut(&mut golem) {
            m.y_head_rot = yaw;
            m.y_body_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot_o = yaw;
        }
        if let Some(n) = name {
            golem.extra.push(("CustomName".into(), n));
        }
        let entity_type = kiln_data::entities::by_name("minecraft:copper_golem").expect("copper golem type");
        spawns.push(Spawn { kind: entity_type, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(golem)) });
        level.effect(Effect::Sound { pos, sound: "minecraft:entity.copper_golem.spawn", volume: 1.0, pitch: 1.0 });
        kiln_blocks::set_block(level, pos, d::AIR, flags::ALL);
        return Some(true);
    }
    if !held.is_empty() && held.item_name() == "minecraft:honeycomb" {
        return None;
    }
    update_pose(level, pos, s);
    Some(true)
}

/// Whether the statue is the unweathered one.
fn is_unaffected(s: u16) -> bool {
    kiln_data::blocks_types::block_of(s).name == "minecraft:copper_golem_statue"
}

/// The custom name its block entity carries (`components`).
fn statue_name(level: &RegionLevel, pos: BlockPos) -> Option<kiln_proto::nbt::Tag> {
    let chunk = level.cells.chunk(kiln_world::ChunkPos::new(pos.x >> 4, pos.z >> 4))?;
    let be = chunk.block_entity((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize)?;
    be.nbt.get("components")?.get("minecraft:custom_name").cloned()
}

/// `CopperGolemStatueBlock.updatePose`: the sound, the next pose, the game event.
fn update_pose(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    level.effect(Effect::Sound { pos, sound: "minecraft:entity.copper_golem_become_statue", volume: 1.0, pitch: 1.0 });
    let now = state::get(s, "copper_golem_pose").and_then(|v| POSES.iter().position(|p| *p == v)).unwrap_or(0);
    let next = POSES[(now + 1) % POSES.len()];
    let info = kiln_data::blocks_types::block_of(s);
    let ns = info.with_property(s, "copper_golem_pose", next).unwrap_or(s);
    kiln_blocks::set_block(level, pos, ns, flags::ALL);
    let after = level.block(pos);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: after });
}
