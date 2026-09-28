//! Magma cube: a fire-immune cube ([`super::slime`] has the shared `AbstractCubeMob` code) with
//! armor by size, higher jumps (also out of lava), slower hops and touch damage at any size.

use super::slime;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Living;
use crate::mob::{GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::EntityData;

pub struct MagmaCube;

pub static KIND: MagmaCube = MagmaCube;

static INFO: Info = Info {
    fire_immune: true,
    ageable: true,
    head: (75, 0, 10),
    extends_monster: false,
    ..Info::monster("minecraft:magma_cube", &[(MovementSpeed, 0.20000000298023224)])
};

impl Kind for MagmaCube {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        slime::new_state(m)
    }
    fn register_goals(&self, m: &mut MobData) {
        slime::register_goals(m);
    }
    fn pre_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        slime::pre_tick(m);
    }
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        slime::post_tick(e, m, level);
    }
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        slime::ai_step(e, m, level);
    }
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        slime::tick_move(e, m, level);
        true
    }
    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        slime::jump_from_ground(e, m, level);
        true
    }
    /// `jumpInLiquid(LAVA)`: straight up by size.
    fn jump_in_liquid(&self, e: &mut Entity, m: &mut MobData, lava: bool) -> bool {
        if !lava {
            return false;
        }
        e.delta = Vec3::new(e.delta.x, (0.22f32 + slime::size(m) as f32 * 0.05) as f64, e.delta.z);
        e.needs_sync = true;
        true
    }
    fn player_touch(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: &Living) {
        slime::player_touch(e, m, level, player);
    }
    fn on_killed_removal(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        slime::on_killed_removal(e, m, level);
    }
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        slime::finalize_spawn(e, m, r, ctx, group);
    }
    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        slime::load(e, m, r);
    }
    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        slime::save(m, o);
    }
    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        slime::entity_data(m, d);
        // `isOnFire` is always false: no burning flag for viewers.
        d.set(kiln_data::entities::data::entity::SHARED_FLAGS, &kiln_proto::packets::entity::DataValue::Byte(0));
    }
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        slime::dimensions(m, base)
    }
    fn experience(&self, _e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(slime::size(m))
    }
    fn spawn_ignores_light(&self) -> bool {
        true
    }
    /// `checkMagmaCubeSpawnRules`: anywhere but on peaceful.
    fn check_spawn_rules(&self, view: &dyn SpawnView, _pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(view.difficulty() != 0)
    }
}
