//! Putting up item frames, glow item frames and paintings (`ItemFrameItem` / `HangingEntityItem.useOn`):
//! the item is used on a block face and the entity hangs in the block in front of it, when it
//! survives there (nothing solid in its box, a solid wall behind it, no other hanging entity in
//! the way). A painting takes the largest variant that fits the wall.

use crate::Player;
use crate::blocks::{EntityBox, RegionLevel};
use crate::entities::{Body, Spawn};
use crate::phantom::PhantomLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level};
use kiln_entity::ext_entity::hanging::HangingWorld;
use kiln_entity::ext_entity::{item_frame, painting};
use kiln_entity::math::{Aabb, Direction as EDir};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::world_fx;

/// The three items.
pub(crate) fn is_hanging_item(name: &str) -> bool {
    matches!(name, "minecraft:item_frame" | "minecraft:glow_item_frame" | "minecraft:painting")
}

fn edir(d: Direction) -> EDir {
    EDir::ALL[d as usize]
}

/// The world a new hanging entity looks at: the region's blocks and the hanging entities among
/// its bodies.
struct PlaceWorld<'a> {
    level: &'a RegionLevel<'a>,
    phantom: PhantomLevel<'a>,
    e: &'a kiln_entity::Entity,
}

impl HangingWorld for PlaceWorld<'_> {
    fn block(&self, pos: kiln_entity::math::BlockPos) -> u16 {
        self.level.block(BlockPos::new(pos.x, pos.y, pos.z))
    }

    fn block_collision(&self, bx: &Aabb) -> bool {
        let mut blocked = false;
        kiln_entity::collision::for_each_block_collision(&self.phantom, &self.e.collision_context(), bx, |_, _, _| {
            blocked = true;
            false
        });
        blocked
    }

    fn hanging_in(&self, bx: &Aabb, _exclude: i32) -> Vec<(EDir, &'static str)> {
        let (lo, hi) = ([bx.min_x, bx.min_y, bx.min_z], [bx.max_x, bx.max_y, bx.max_z]);
        self.level.bodies.iter().filter(|b| b.intersects(lo, hi)).filter_map(|b| b.hanging).collect()
    }
}

/// `HangingEntityItem.useOn` / `ItemFrameItem.useOn`; true when the click was taken (the item may
/// be used up).
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, clicked: BlockPos, face: Direction, off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    let name = p.in_hand(off_hand).item_name();
    let painting_item = name == "minecraft:painting";
    let pos = clicked.relative(face);
    // `mayPlace`: a painting only on a wall; the build height; `Player.mayUseItemAt`.
    let top = level.env.min_y + level.env.height - 1;
    if (painting_item && matches!(face, Direction::Up | Direction::Down)) || pos.y < level.env.min_y || pos.y > top || p.game_mode > 1 {
        return true;
    }
    let env = level.env;
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, p.entity_id, (pos.x as u64) << 32 ^ pos.z as u64 ^ (pos.y as u64) << 16);
    let (epos, dir) = (kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z), edir(face));
    let entity = {
        let phantom = PhantomLevel::new(&*level.cells, env.game_time, env.min_y, false);
        let lvl: &RegionLevel = &*level;
        if painting_item {
            let survives = |e: &kiln_entity::Entity| -> bool {
                let world = PlaceWorld { level: lvl, phantom: PhantomLevel::new(&*lvl.cells, env.game_time, env.min_y, false), e };
                kiln_entity::ext_entity::get::<painting::Painting>(e).is_some_and(|p| p.survives(e, &world))
            };
            painting::create(&survives, 0, epos, dir, seed)
        } else {
            let e = item_frame::new(0, name == "minecraft:glow_item_frame", epos, dir, seed);
            let world = PlaceWorld { level: lvl, phantom, e: &e };
            let survives = kiln_entity::ext_entity::get::<item_frame::ItemFrame>(&e).is_some_and(|f| f.survives(&e, &world));
            survives.then_some(e)
        }
    };
    // A placement that fails (`CONSUME`) uses nothing up, but counts as a use.
    let Some(entity) = entity else {
        let item = p.in_hand(off_hand).item();
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
        return true;
    };
    let at = entity.position();
    let sound = if let Some(f) = kiln_entity::ext_entity::get::<item_frame::ItemFrame>(&entity) {
        f.placement_sound()
    } else {
        "minecraft:entity.painting.place"
    };
    if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) {
        let seed = level.random().next_long();
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Neutral, [at.x, at.y, at.z], 1.0, 1.0, seed);
        level.out.packets.push(([at.x, at.y, at.z], 16.0, pkt));
    }
    level.effect(Effect::GameEvent { pos: BlockPos::new(pos.x, pos.y, pos.z), event: "minecraft:entity_place" });
    if let Some(kind) = kiln_data::entities::by_name(entity.type_name) {
        spawns.push(Spawn { kind, pos: [at.x, at.y, at.z], vel: [0.0; 3], body: Body::Ready(Box::new(entity)) });
    }
    let item = p.in_hand(off_hand).item();
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    let _ = ItemStack::empty;
    true
}

/// The facing and type of a hanging entity, for [`EntityBox`].
pub(crate) fn hanging_of(e: &kiln_entity::Entity) -> Option<(EDir, &'static str)> {
    kiln_entity::ext_entity::hanging::direction_of(e).map(|d| (d, e.type_name))
}

#[allow(dead_code)]
fn unused(_: EntityBox) {}
