//! Breeze: a trial-chamber monster that slides around its target, leaps far on gusts of wind
//! and shoots wind charges ([`crate::ext_entity::wind_charge`]) whose wind burst knocks things
//! back. Other breezes cannot hurt it; it only fights players and iron golems.
//!
//! Approximation: vanilla drives it with a `Brain` (`BreezeAi`: `Shoot`, `LongJump`, `Slide`,
//! `ShootWhenStuck`); here one goal runs the fight with the same timings (15 ticks of inhaling
//! before a shot, 4 of recovery, 10 of cooldown; 10 of inhaling before a leap along the first
//! workable angle of 40..80 degrees to a spot behind the target, then shooting for 100 ticks),
//! and the idle activity strolls.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE, Wanted};
use crate::mob::{self, DamageSource, MobData, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_data::entities::pose;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Breeze;

pub static KIND: Breeze = Breeze;

static INFO: Info = Info {
    head: (30, 40, 25),
    monster_base: false,
    ..Info::monster("minecraft:breeze", &[(MovementSpeed, 0.6299999952316284), (MaxHealth, 30.0), (FollowRange, 24.0), (AttackDamage, 3.0)])
};

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `getPose` (a `kiln_data::entities::pose` id).
    pub pose: i32,
    sound_tick: i32,
    /// Game times the brain's timed memories run out at.
    shoot_until: i64,
    shoot_charging_until: i64,
    shoot_recovering_until: i64,
    shoot_cooldown_until: i64,
    jump_cooldown_until: i64,
    jump_inhaling_until: i64,
    jump_target: Option<BlockPos>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("breeze state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("breeze state")
}

/// `getFiringYPosition`.
fn firing_y(e: &Entity) -> f64 {
    e.y() + (e.height / 2.0) as f64 + 0.30000001192092896
}

fn play(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, volume: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume, pitch: 1.0 });
    }
}

/// `BreezeUtil.randomPointBehindTarget`.
fn point_behind(e: &mut Entity, t: &Living, head_yaw: f32) -> Vec3 {
    let angle = head_yaw + 180.0 + e.random.next_gaussian() as f32 * 90.0 / 2.0;
    let r = mth::lerp_f(e.random.next_float(), 4.0, 8.0);
    let dir = crate::ext_entity::fireball::view_vector(0.0, angle).scale(r as f64);
    t.pos + dir
}

/// `LongJumpUtil.calculateJumpVectorForAngle` without the collision check.
fn jump_vector(e: &Entity, m: &MobData, target: Vec3, max_v: f32, angle: i32) -> Option<Vec3> {
    let pos = e.position();
    let plane = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z).normalize().scale(0.5);
    let d = (target - plane) - pos;
    let a = angle as f32 * std::f32::consts::PI / 180.0;
    let xz = d.z.atan2(d.x);
    let r2 = Vec3::new(d.x, 0.0, d.z).length_sqr();
    let r = r2.sqrt();
    let g = m.attrs.value(Attr::Gravity);
    let v0sqr = r2 * g / (r * ((2.0 * a) as f64).sin() - 2.0 * d.y * (a as f64).cos().powi(2));
    if v0sqr < 0.0 {
        return None;
    }
    let v0 = v0sqr.sqrt();
    if v0 > max_v as f64 {
        return None;
    }
    let (v0r, v0y) = (v0 * (a as f64).cos(), v0 * (a as f64).sin());
    Some(Vec3::new(v0r * xz.cos(), v0y, v0r * xz.sin()))
}

impl Kind for Breeze {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::OnTopOfTrapdoor, -1.0));
        m.maluses.push((path::PathType::Fire, -1.0));
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        m.goals.add(0, Goal::Float);
        m.goals.add(1, Goal::Custom(Box::new(BreezeFight)));
        m.goals.add(3, Goal::RandomStroll { speed: 0.6, interval: 60, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        m.targets.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 });
        m.targets.add(2, Goal::NearestAttackable { wanted: Wanted::Player, interval: 0, must_see: true, target: None, unseen: 0, spider: false });
        m.targets.add(3, Goal::NearestAttackable { wanted: Wanted::Types(&["minecraft:iron_golem"]), interval: 0, must_see: true, target: None, unseen: 0, spider: false });
    }

    /// `Breeze.tick` before `Mob.tick`: the ground particles' draw, the whirl sound's clock.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if matches!(st(m).pose, pose::SHOOTING | pose::INHALING | pose::STANDING) {
            e.random.next_int_bounded(1);
        }
        let s = st_mut(m);
        s.sound_tick = if s.sound_tick == 0 { mob::mth::next_int_between(&mut e.random, 1, 80) } else { s.sound_tick - 1 };
        if st(m).sound_tick == 0 {
            // `playWhirlSound` (heard only where it is played: the draws).
            e.random.next_float();
            e.random.next_float();
        }
    }

    /// Its ambient sounds are the client's own (`playLocalSound`).
    fn ambient_sound(&self, _e: &mut Entity, _m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(None)
    }

    fn can_attack(&self, _m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        t.player || t.type_name == "minecraft:iron_golem"
    }

    /// `isInvulnerableTo`: another breeze's doing.
    fn hurt(&self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        let by_breeze = source.attacker.and_then(|a| level.entity(a)).is_some_and(|a| a.type_name == "minecraft:breeze");
        by_breeze.then_some(false)
    }

    fn swim_sound(&self) -> Option<&'static str> {
        None
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(10)
    }

    fn tick_body(&self, _e: &mut Entity, m: &mut MobData) -> bool {
        st(m).pose == pose::LONG_JUMPING
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let p = st(m).pose;
        if p != pose::STANDING {
            d.set(kiln_data::entities::data::entity::POSE, &DataValue::Pose(p));
        }
    }

    fn load(&self, _e: &mut Entity, _m: &mut MobData, _r: &mut Input) {}

    fn save(&self, _e: &Entity, _m: &MobData, _o: &mut Output) {}
}

/// The fight activity (`Shoot`, `LongJump`, `Slide`) while it has a target.
#[derive(Clone, Debug)]
struct BreezeFight;

impl BreezeFight {
    fn shoot_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living, now: i64) {
        mob::mob_look_at(e, t, 30.0, 30.0);
        let s = st(m);
        if s.shoot_charging_until <= now && s.shoot_recovering_until <= now {
            st_mut(m).shoot_recovering_until = now + 4;
            let id = level.next_entity_id();
            let seed = level.fresh_seed();
            let mut charge = crate::ext_entity::wind_charge::new(id, e.id, Vec3::new(e.x(), firing_y(e), e.z()), seed);
            let dx = t.pos.x - e.x();
            let dy = t.pos.y + (t.bb.max_y - t.bb.min_y) * 0.3 - firing_y(e);
            let dz = t.pos.z - e.z();
            mob::species::shoot(&mut charge, dx, dy, dz, 0.7, (5 - level.difficulty() as i32 * 4) as f32);
            charge.set_old_pos_and_rot();
            level.add_entity(charge);
            play(e, level, "minecraft:entity.breeze.shoot", 1.5);
        }
    }
}

impl CustomGoal for BreezeFight {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BreezeFight"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some()
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        s.pose = pose::STANDING;
        s.jump_target = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        let now = level.game_time();
        let head_yaw = level.player(t.id).map_or(0.0, |p| p.yaw);
        match st(m).pose {
            pose::SHOOTING => {
                Self::shoot_tick(e, m, level, &t, now);
                if st(m).shoot_recovering_until <= now && st(m).shoot_charging_until <= now && st(m).shoot_recovering_until > 0 {
                    let s = st_mut(m);
                    s.pose = pose::STANDING;
                    s.shoot_cooldown_until = now + 10;
                    s.shoot_until = 0;
                    s.shoot_recovering_until = 0;
                }
                return;
            }
            pose::INHALING => {
                if st(m).jump_inhaling_until > now {
                    return;
                }
                let target = st(m).jump_target.map(|p| Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5));
                let max_v = 0.058333334f32 * m.attrs.value(FollowRange) as f32;
                let mut angles = [40, 55, 60, 75, 80];
                // `Util.shuffledCopy`.
                for i in (1..angles.len()).rev() {
                    let j = e.random.next_int_bounded(i as i32 + 1) as usize;
                    angles.swap(i, j);
                }
                let v = target.and_then(|to| angles.iter().find_map(|&a| jump_vector(e, m, to, max_v, a)));
                match v {
                    None => st_mut(m).pose = pose::STANDING,
                    Some(v) => {
                        play(e, level, "minecraft:entity.breeze.jump", 1.0);
                        st_mut(m).pose = pose::LONG_JUMPING;
                        e.y_rot = m.y_body_rot;
                        e.delta = v;
                        e.needs_sync = true;
                    }
                }
                return;
            }
            pose::LONG_JUMPING => {
                if e.on_ground || e.is_in_water() {
                    play(e, level, "minecraft:entity.breeze.land", 1.0);
                    let hurt = m.last_hurt_by_mob.is_some();
                    let s = st_mut(m);
                    s.pose = pose::STANDING;
                    s.jump_cooldown_until = now + if hurt { 2 } else { 10 };
                    s.shoot_until = now + 100;
                    s.jump_target = None;
                }
                return;
            }
            pose::SLIDING => {
                if m.nav.is_done() {
                    let s = st_mut(m);
                    s.pose = pose::STANDING;
                    s.shoot_until = now + 60;
                }
                return;
            }
            _ => {}
        }
        let s = st(m);
        let dist2 = e.position().distance_to_sqr(t.pos);
        // `Shoot`.
        if s.shoot_until > now && s.shoot_cooldown_until <= now && dist2 < 256.0 {
            let s = st_mut(m);
            s.pose = pose::SHOOTING;
            s.shoot_charging_until = now + 15;
            s.shoot_recovering_until = 0;
            play(e, level, "minecraft:entity.breeze.inhale", 1.0);
            m.nav.stop();
            return;
        }
        // `LongJump`.
        let clear_above = (1..=4).all(|i| {
            let s = level.block(e.block_position().offset(0, i, 0));
            kiln_data::blocks_types::is_air(s) || crate::physics::fluid_state(s).kind.is_water()
        });
        if s.shoot_until <= now && s.jump_cooldown_until <= now && (e.on_ground || e.is_in_water()) && dist2.sqrt() - 4.0 > 0.0 && clear_above {
            let behind = point_behind(e, &t, head_yaw);
            let down = Vec3::new(behind.x, behind.y - 10.0, behind.z);
            let surface = crate::clip::traverse_blocks(behind, down, |p| {
                let (shape, _) = crate::collision::collision_shape(level.block(p), p, &crate::collision::CollisionContext::EMPTY);
                crate::clip::shape_clip(&shape, behind, down, p).map(|(loc, _)| loc)
            });
            if let Some(loc) = surface {
                let p = BlockPos::containing(loc.x, loc.y, loc.z).above();
                let s = st_mut(m);
                s.jump_target = Some(p);
                s.jump_inhaling_until = now + 10;
                s.pose = pose::INHALING;
                play(e, level, "minecraft:entity.breeze.charge", 1.0);
                m.nav.stop();
                return;
            }
        }
        // `Slide`.
        if st(m).jump_cooldown_until <= now && st(m).shoot_until <= now && e.on_ground && !e.is_in_water() {
            let inner = t.pos.distance_to_sqr(e.position()) < 16.0;
            let away = if inner { random_pos::default_pos_away(e, m, level, 5, 5, t.pos) } else { None };
            let to = match away {
                Some(p) => p,
                None if e.random.next_bool() => point_behind(e, &t, head_yaw),
                None => {
                    let d = t.pos - e.position();
                    let dist = d.length() - crate::math::lerp(e.random.next_double(), 8.0, 4.0);
                    e.position() + d.normalize().scale(dist)
                }
            };
            if path::move_to(e, m, level, to.x, to.y, to.z, 0.6) {
                st_mut(m).pose = pose::SLIDING;
                play(e, level, "minecraft:entity.breeze.slide", 1.0);
            }
        }
    }
}
