//! Squid and glow squid (`Squid`, `GlowSquid`, `AgeableWaterCreature`): they swim by pulsing
//! their tentacles (each pulse sets the velocity to the movement vector the random movement goal
//! picked), flee along the line from whoever hurt them, squirt ink when a mob hurts them, and
//! drown slowly on land. A glow squid goes dark for 100 ticks when hurt.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::{self, Goal};
use crate::mob::{self, Category, DamageSource, MobData, mth, path};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Squid;
pub struct GlowSquid;

pub static KIND: Squid = Squid;
pub static GLOW: GlowSquid = GlowSquid;

/// `AgeableWaterCreature` on `Mob.createMobAttributes`.
const fn info(name: &'static str, category: Category) -> Info {
    Info { category, ageable: true, breathes_under_water: true, ambient_interval: 120, ..Info::misc(name, &[(MaxHealth, 10.0)]) }
}

static INFO: Info = info("minecraft:squid", Category::WaterCreature);
static GLOW_INFO: Info = info("minecraft:glow_squid", Category::UndergroundWaterCreature);

#[derive(Clone, Debug)]
pub struct SquidState {
    pub x_body_rot: f32,
    pub x_body_rot_o: f32,
    pub z_body_rot: f32,
    pub tentacle_movement: f32,
    tentacle_speed: f32,
    rotate_speed: f32,
    /// `movementVector`.
    pub movement: Vec3,
    /// `GlowSquid.DATA_DARK_TICKS_REMAINING`.
    pub dark_ticks: i32,
}

fn st(m: &MobData) -> &SquidState {
    ext::state::<SquidState>(m).expect("squid state")
}

fn st_mut(m: &mut MobData) -> &mut SquidState {
    ext::state_mut::<SquidState>(m).expect("squid state")
}

fn is_glow(m: &MobData) -> bool {
    m.kind == mob::MobKind::GlowSquid
}

/// The squid's constructor: the tentacle speed from its random, water costs nothing.
fn new_state(m: &mut MobData, random: &mut dyn RandomSource) -> Box<dyn MobExt> {
    m.maluses.push((path::PathType::Water, 0.0));
    let tentacle_speed = 1.0 / (random.next_float() + 1.0) * 0.2;
    Box::new(SquidState {
        x_body_rot: 0.0,
        x_body_rot_o: 0.0,
        z_body_rot: 0.0,
        tentacle_movement: 0.0,
        tentacle_speed,
        rotate_speed: 0.0,
        movement: Vec3::ZERO,
        dark_ticks: 0,
    })
}

fn register_goals(m: &mut MobData) {
    m.goals.add(0, Goal::Custom(Box::new(SquidRandomMovementGoal)));
    m.goals.add(1, Goal::Custom(Box::new(SquidFleeGoal { flee_ticks: 0 })));
}

/// `Squid.aiStep` after `AgeableMob.aiStep`.
fn ai_step(e: &mut Entity, m: &mut MobData) {
    let in_water = e.is_in_water();
    let gravity = if e.no_gravity { 0.0 } else { m.attrs.value(Gravity) };
    let s = st_mut(m);
    s.x_body_rot_o = s.x_body_rot;
    s.tentacle_movement += s.tentacle_speed;
    // A new pulse (the caller broadcasts entity event 19 when the movement wrapped).
    if s.tentacle_movement as f64 > std::f64::consts::PI * 2.0 {
        s.tentacle_movement -= (std::f64::consts::PI * 2.0) as f32;
        if e.random.next_int_bounded(10) == 0 {
            s.tentacle_speed = 1.0 / (e.random.next_float() + 1.0) * 0.2;
        }
    }
    if in_water {
        if s.tentacle_movement < std::f32::consts::PI {
            let scale = s.tentacle_movement / std::f32::consts::PI;
            if scale as f64 > 0.75 {
                e.delta = s.movement;
                s.rotate_speed = 1.0;
            } else {
                s.rotate_speed *= 0.8;
            }
        } else {
            e.delta = e.delta.scale(0.9);
            s.rotate_speed *= 0.99;
        }
        let v = e.delta;
        let h = v.horizontal_distance();
        let body = m.y_body_rot + (-(mth::atan2(v.x, v.z) as f32) * (180.0 / std::f32::consts::PI) - m.y_body_rot) * 0.1;
        m.y_body_rot = body;
        e.y_rot = body;
        let s = st_mut(m);
        s.z_body_rot += std::f32::consts::PI * s.rotate_speed * 1.5;
        s.x_body_rot += (-(mth::atan2(h, v.y) as f32) * (180.0 / std::f32::consts::PI) - s.x_body_rot) * 0.1;
    } else {
        // Levitation is not simulated on mobs: the squid falls.
        let yd = e.delta.y - gravity;
        e.delta = Vec3::new(0.0, yd * 0.98f32 as f64, 0.0);
        s.x_body_rot += (-90.0 - s.x_body_rot) * 0.02;
    }
}

/// `Squid.spawnInk`: the squirt sound and 30 ink particles (only their draws matter here).
fn spawn_ink(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) {
    let sound = if is_glow(m) { "minecraft:entity.glow_squid.squirt" } else { "minecraft:entity.squid.squirt" };
    mob::make_sound(e, m, level, sound);
    for _ in 0..30 {
        e.random.next_float();
        e.random.next_float();
        e.random.next_float();
    }
}

/// `checkSurfaceAgeableWaterCreatureSpawnRules`: from 13 below sea level up to it, in water
/// with water above.
pub fn surface_water_rules(view: &dyn SpawnView, pos: BlockPos) -> bool {
    let sea = view.sea_level();
    pos.y >= sea - 13
        && pos.y <= sea
        && crate::physics::fluid_state(view.block(pos.below())).kind.is_water()
        && crate::blocks::block_name(view.block(pos.above())) == "minecraft:water"
}

macro_rules! squid_kind {
    ($t:ty, $info:expr) => {
        impl Kind for $t {
            fn info(&self) -> &'static Info {
                &$info
            }
            fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
                Some(new_state(m, random))
            }
            fn register_goals(&self, m: &mut MobData) {
                register_goals(m);
            }
            fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
                let before = st(m).tentacle_movement;
                ai_step(e, m);
                if st(m).tentacle_movement < before {
                    level.emit(Event::EntityEvent { entity: e.id, event: 19 });
                }
                if is_glow(m) {
                    let s = st_mut(m);
                    if s.dark_ticks > 0 {
                        s.dark_ticks -= 1;
                    }
                    // The glow particle's position draws.
                    mob::random_point(e, 0.6);
                }
            }
            /// `Squid.travel`: the velocity as it is (gravity and drag are the `aiStep`'s).
            fn travel(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, _input: Vec3) -> bool {
                let d = e.delta;
                e.do_move(level, MoverType::SelfMove, d);
                true
            }
            /// `Squid.hurtServer`: ink when a mob hurt it; the result is false otherwise.
            fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
                let hurt = mob::hurt_base(e, m, level, *source, amount) && m.last_hurt_by_mob.is_some();
                if hurt {
                    spawn_ink(e, m, level);
                    if is_glow(m) {
                        st_mut(m).dark_ticks = 100;
                    }
                }
                Some(hurt)
            }
            fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
                ext::water_animal_air(e, m, level, air_before);
            }
            fn pushed_by_fluid(&self) -> bool {
                false
            }
            fn swim_sound(&self) -> Option<&'static str> {
                None
            }
            fn sound_volume(&self, _m: &MobData) -> f32 {
                0.4
            }
            fn experience(&self, e: &mut Entity, _m: &MobData) -> Option<i32> {
                Some(1 + e.random.next_int_bounded(3))
            }
            fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
                Some(0.0)
            }
            fn spawn_ignores_light(&self) -> bool {
                true
            }
            fn placement(&self) -> Placement {
                Placement::InWater
            }
            fn spawn_in_liquids(&self) -> bool {
                true
            }
            fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
                if $info.category == Category::UndergroundWaterCreature {
                    // `checkGlowSquidSpawnRules`.
                    return Some(
                        pos.y <= view.sea_level() - 33
                            && view.raw_brightness(pos, 0) == 0
                            && crate::blocks::block_name(view.block(pos)) == "minecraft:water",
                    );
                }
                Some(surface_water_rules(view, pos))
            }
            /// `Squid.BABY_DIMENSIONS`.
            fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
                if m.baby() { (0.5, 0.5, 0.37) } else { base }
            }
            fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
                if is_glow(m) {
                    st_mut(m).dark_ticks = r.int_or("DarkTicksRemaining", 0);
                }
            }
            fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
                if is_glow(m) {
                    o.put("DarkTicksRemaining", Tag::Int(st(m).dark_ticks));
                }
            }
            fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
                if is_glow(m) {
                    d.set(kiln_data::entities::data::glow_squid::DARK_TICKS_REMAINING, &DataValue::Int(st(m).dark_ticks));
                }
            }
        }
    };
}

squid_kind!(Squid, INFO);
squid_kind!(GlowSquid, GLOW_INFO);

// ---------------------------------------------------------------------- goals

/// `Squid.SquidRandomMovementGoal`: a new swimming direction now and then (always when out of
/// the water or without one), none after 100 idle ticks.
#[derive(Clone, Debug)]
struct SquidRandomMovementGoal;

impl CustomGoal for SquidRandomMovementGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SquidRandomMovementGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        true
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if m.no_action_time > 100 {
            st_mut(m).movement = Vec3::ZERO;
            return;
        }
        let has_movement = st(m).movement.length_sqr() > 1.0e-5f32 as f64;
        if e.random.next_int_bounded(mth::reduced_tick_delay(50)) == 0 || !e.was_touching_water || !has_movement {
            let angle = e.random.next_float() * std::f32::consts::TAU;
            let y = -0.1f32 + e.random.next_float() * 0.2;
            st_mut(m).movement = Vec3::new((mth::cos(angle as f64) * 0.2) as f64, y as f64, (mth::sin(angle as f64) * 0.2) as f64);
        }
    }
}

/// `Squid.SquidFleeGoal`: in the water, within 10 blocks of whoever hurt it, away from them (up
/// to 3 blocks a second, less the farther it is).
#[derive(Clone, Debug)]
struct SquidFleeGoal {
    flee_ticks: i32,
}

impl CustomGoal for SquidFleeGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SquidFleeGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(by) = m.last_hurt_by_mob.and_then(|id| goals::living(level, id)) else { return false };
        e.is_in_water() && e.position().distance_to_sqr(by.pos) < 100.0
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.flee_ticks = 0;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.flee_ticks += 1;
        let Some(by) = m.last_hurt_by_mob.and_then(|id| goals::living(level, id)) else { return };
        let mut flee = Vec3::new(e.x() - by.pos.x, e.y() - by.pos.y, e.z() - by.pos.z);
        let at = BlockPos::containing(e.x() + flee.x, e.y() + flee.y, e.z() + flee.z);
        let state = level.block(at);
        let air = kiln_data::blocks_types::is_air(state);
        if crate::physics::fluid_state(state).kind.is_water() || air {
            let length = flee.length();
            if length > 0.0 {
                // (vanilla normalizes and drops the result)
                let mut speed = 3.0;
                if length > 5.0 {
                    speed -= (length - 5.0) / 5.0;
                }
                if speed > 0.0 {
                    flee = flee.scale(speed);
                }
            }
            if air {
                flee = flee.subtract(0.0, flee.y, 0.0);
            }
            st_mut(m).movement = Vec3::new(flee.x / 20.0, flee.y / 20.0, flee.z / 20.0);
        }
    }
}
