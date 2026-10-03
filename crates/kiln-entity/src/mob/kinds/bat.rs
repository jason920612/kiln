//! Bat (`Bat`, an `AmbientCreature`): hangs from the underside of solid blocks until a player
//! comes within 4 blocks, then flutters about toward random nearby spots, hanging up again now
//! and then under a ceiling. It is not pushed, takes no fall damage and makes no step sounds.

use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::{Category, DamageSource, MobData, goals, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Bat;

pub static KIND: Bat = Bat;

static INFO: Info = Info { category: Category::Ambient, ..Info::misc("minecraft:bat", &[(MaxHealth, 6.0)]) };

#[derive(Clone, Debug)]
pub struct State {
    /// `DATA_ID_FLAGS` (bit 0: resting).
    pub flags: i8,
    target: Option<BlockPos>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("bat state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("bat state")
}

pub fn resting(m: &MobData) -> bool {
    st(m).flags & 1 != 0
}

fn set_resting(m: &mut MobData, on: bool) {
    let s = st_mut(m);
    s.flags = if on { s.flags | 1 } else { s.flags & !1 };
}

fn conductor(level: &dyn EntityLevel, p: BlockPos) -> bool {
    kiln_data::block_logic::is_redstone_conductor(level.block(p))
}

impl Kind for Bat {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructor hangs it up.
    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State { flags: 1, target: None }))
    }

    /// No goals: the flight is `customServerAiStep`'s.
    fn register_goals(&self, _m: &mut MobData) {}

    /// `Bat.tick` after `Mob.tick`: hanging still, or losing some vertical speed.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if resting(m) {
            e.delta = Vec3::ZERO;
            let y = crate::math::floor(e.y()) as f64 + 1.0 - e.height as f64;
            e.set_pos_raw(Vec3::new(e.x(), y, e.z()));
        } else {
            e.delta = e.delta.multiply(1.0, 0.6, 1.0);
        }
    }

    /// `customServerAiStep`: waking when a player comes near (or the ceiling goes), else flying
    /// toward a spot that changes now and then.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let pos = e.block_position();
        let above = pos.above();
        if resting(m) {
            let silent = e.silent;
            if conductor(level, above) {
                if e.random.next_int_bounded(200) == 0 {
                    m.y_head_rot = e.random.next_int_bounded(360) as f32;
                }
                if goals::nearest_player(e, m, level, false, 4.0, true, |_| true).is_some() {
                    set_resting(m, false);
                    if !silent {
                        level.emit(Event::LevelEvent { event: 1025, pos, data: 0 });
                    }
                }
            } else {
                set_resting(m, false);
                if !silent {
                    level.emit(Event::LevelEvent { event: 1025, pos, data: 0 });
                }
            }
            return;
        }
        let mut target = st(m).target;
        if let Some(t) = target
            && (!kiln_data::blocks_types::is_air(level.block(t)) || t.y <= level.min_y())
        {
            target = None;
        }
        let near = |t: BlockPos, e: &Entity| Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5).distance_to_sqr(e.position()) < 4.0;
        if target.is_none() || e.random.next_int_bounded(30) == 0 || target.is_some_and(|t| near(t, e)) {
            let x = e.x() + e.random.next_int_bounded(7) as f64 - e.random.next_int_bounded(7) as f64;
            let y = e.y() + e.random.next_int_bounded(6) as f64 - 2.0;
            let z = e.z() + e.random.next_int_bounded(7) as f64 - e.random.next_int_bounded(7) as f64;
            target = Some(BlockPos::containing(x, y, z));
        }
        let t = target.expect("a target");
        st_mut(m).target = target;
        let dx = t.x as f64 + 0.5 - e.x();
        let dy = t.y as f64 + 0.1 - e.y();
        let dz = t.z as f64 + 0.5 - e.z();
        let v = e.delta;
        let signum = |d: f64| if d > 0.0 { 1.0 } else if d < 0.0 { -1.0 } else { d };
        let f = 0.1f32 as f64;
        let nv = v.add((signum(dx) * 0.5 - v.x) * f, (signum(dy) * 0.7f32 as f64 - v.y) * f, (signum(dz) * 0.5 - v.z) * f);
        e.delta = nv;
        let yaw = (mth::atan2(nv.z, nv.x) * 57.2957763671875) as f32 - 90.0;
        let diff = mth::wrap_degrees(yaw - e.y_rot);
        m.zza = 0.5;
        e.y_rot += diff;
        if e.random.next_int_bounded(100) == 0 && conductor(level, above) {
            set_resting(m, true);
        }
    }

    /// `hurtServer`: a hurt bat stops resting.
    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        if resting(m) {
            set_resting(m, false);
        }
        None
    }

    /// `getAmbientSound`: a resting bat squeaks one time in four.
    fn ambient_sound(&self, e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if resting(m) && e.random.next_int_bounded(4) != 0 {
            return Some(None);
        }
        Some(Some("minecraft:entity.bat.ambient"))
    }

    fn sound_volume(&self, _m: &MobData) -> f32 {
        0.1
    }

    fn voice_pitch(&self, _m: &MobData, pitch: f32) -> f32 {
        pitch * 0.95
    }

    fn swim_sound(&self) -> Option<&'static str> {
        None
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    fn pushable(&self) -> bool {
        false
    }

    /// `Mob.getBaseExperienceReward` with no `xpReward`.
    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(0)
    }

    /// A plain `Mob`: natural spawning does not ask for a walk target value.
    fn spawn_ignores_light(&self) -> bool {
        true
    }

    /// `checkBatSpawnRules`: under the surface, in the dark, on a stone-like floor.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        // `WORLD_SURFACE`: something non-air above.
        let covered = match view.world_surface(pos.x, pos.z) {
            // Some non-air block above `pos`: the highest one (surface - 1) is above it.
            Some(surface) => surface - 1 > pos.y,
            None => (1..=384).any(|dy| !kiln_data::blocks_types::is_air(view.block(pos.offset(0, dy, 0)))),
        };
        if !covered || r.next_bool() {
            return Some(false);
        }
        if view.raw_brightness(pos, view.sky_darken()) > r.next_int_bounded(4) {
            return Some(false);
        }
        let below = view.block(pos.below());
        Some(super::wolf::block_in_tag(below, "minecraft:bats_spawnable_on") && crate::mob::path::valid_spawn(below, false))
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let flags = r.byte_or("BatFlags", 0);
        st_mut(m).flags = flags;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("BatFlags", Tag::Byte(st(m).flags));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::bat::ID_FLAGS, &DataValue::Byte(st(m).flags));
    }
}
