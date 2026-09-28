//! Blaze: falls slowly and rises toward targets above it (`aiStep`, `customServerAiStep` with a
//! random allowed height offset), is hurt by water and rain, and shoots bursts of three small
//! fireballs after a charge (`BlazeAttackGoal`).

use crate::entity::Entity;
use crate::ext_entity::fireball;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, SpawnView, state, state_mut};
use crate::mob::goals::{self, Goal, LOOK, MOVE, Wanted};
use crate::mob::path::PathType;
use crate::mob::{self, MobData, mth};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Blaze;

pub static KIND: Blaze = Blaze;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::monster("minecraft:blaze", &[(AttackDamage, 6.0), (MovementSpeed, 0.23000000417232513), (FollowRange, 48.0)])
};

#[derive(Clone, Debug)]
pub struct BlazeState {
    pub allowed_height_offset: f32,
    pub next_height_offset_change_tick: i32,
    /// `DATA_FLAGS_ID` bit 1: charged (shown burning).
    pub charged: bool,
}

fn set_charged(m: &mut MobData, on: bool) {
    if let Some(s) = state_mut::<BlazeState>(m) {
        s.charged = on;
    }
}

impl Kind for Blaze {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        for (t, v) in [(PathType::Water, -1.0), (PathType::Lava, 8.0), (PathType::FireInNeighbor, 0.0), (PathType::Fire, 0.0)] {
            m.maluses.retain(|(p, _)| *p != t);
            m.maluses.push((t, v));
        }
        Some(Box::new(BlazeState { allowed_height_offset: 0.5, next_height_offset_change_tick: 0, charged: false }))
    }
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(4, Goal::Custom(Box::new(BlazeAttack { attack_step: 0, attack_time: 0, last_seen: 0 })));
        // `MoveTowardsRestrictionGoal`: no home, never starts.
        g.add(5, Goal::Never);
        g.add(7, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.0), wanted: Vec3::ZERO, force: false });
        g.add(8, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::NearestAttackable { wanted: Wanted::Player, interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false });
    }
    /// `Blaze.aiStep` before `super.aiStep()`: a slow fall (the burning sounds and smoke are
    /// the client's).
    fn ai_step_before(&self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        if !e.on_ground && e.delta.y < 0.0 {
            e.delta = e.delta.multiply(1.0, 0.6, 1.0);
        }
    }
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(s) = state_mut::<BlazeState>(m) else { return };
        s.next_height_offset_change_tick -= 1;
        if s.next_height_offset_change_tick <= 0 {
            s.next_height_offset_change_tick = 100;
            s.allowed_height_offset = mth::triangle(&mut e.random, 0.5, 6.891) as f32;
        }
        let offset = s.allowed_height_offset;
        if let Some(t) = goals::target(m, level)
            && t.eye_y > e.eye_y() + offset as f64
        {
            let v = e.delta;
            e.delta = e.delta.add(0.0, (0.30000001192092896 - v.y) * 0.30000001192092896, 0.0);
            e.needs_sync = true;
        }
    }
    fn sensitive_to_water(&self) -> bool {
        true
    }
    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(10)
    }
    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        let charged = state::<BlazeState>(m).is_some_and(|s| s.charged);
        d.set(kiln_data::entities::data::blaze::FLAGS, &DataValue::Byte(charged as i8));
        // `isOnFire` is the charge.
        let _ = e;
        d.set(kiln_data::entities::data::entity::SHARED_FLAGS, &DataValue::Byte(charged as i8));
    }
    /// `Monster.checkAnyLightMonsterSpawnRules`.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(view.difficulty() != 0 && super::slime::check_mob_spawn_rules(view, pos))
    }
}

/// `BlazeAttackGoal`.
#[derive(Clone, Debug)]
struct BlazeAttack {
    attack_step: i32,
    attack_time: i32,
    last_seen: i32,
}

impl CustomGoal for BlazeAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BlazeAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some_and(|t| t.alive)
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.attack_step = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_charged(m, false);
        self.last_seen = 0;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.attack_time -= 1;
        let Some(t) = goals::target(m, level) else { return };
        let can_see = mob::has_line_of_sight_cached(e, m, level, &t);
        if can_see {
            self.last_seen = 0;
        } else {
            self.last_seen += 1;
        }
        let d = e.position().distance_to_sqr(t.pos);
        let follow = m.attrs.value(Attr::FollowRange);
        if d < 4.0 {
            if !can_see {
                return;
            }
            if self.attack_time <= 0 {
                self.attack_time = 20;
                mob::do_hurt_target(e, m, level, &t);
            }
            m.mov.set_wanted_position(t.pos.x, t.pos.y, t.pos.z, 1.0);
        } else if d < follow * follow && can_see {
            let dx = t.pos.x - e.x();
            let dy = (t.pos.y + (t.bb.max_y - t.bb.min_y) * 0.5) - (e.y() + e.height as f64 * 0.5);
            let dz = t.pos.z - e.z();
            if self.attack_time <= 0 {
                self.attack_step += 1;
                if self.attack_step == 1 {
                    self.attack_time = 60;
                    set_charged(m, true);
                } else if self.attack_step <= 4 {
                    self.attack_time = 6;
                } else {
                    self.attack_time = 100;
                    self.attack_step = 0;
                    set_charged(m, false);
                }
                if self.attack_step > 1 {
                    let spread = d.sqrt().sqrt() * 0.5;
                    if !e.silent {
                        level.emit(Event::LevelEvent { event: 1018, pos: e.block_position(), data: 0 });
                    }
                    let x = mth::triangle(&mut e.random, dx, 2.297 * spread);
                    let z = mth::triangle(&mut e.random, dz, 2.297 * spread);
                    let id = level.next_entity_id();
                    let seed = level.fresh_seed();
                    let mut fb = fireball::new(true, id, e, Vec3::new(x, dy, z).normalize(), 1, seed);
                    let (fx, fz) = (fb.x(), fb.z());
                    fb.set_pos(Vec3::new(fx, e.y() + e.height as f64 * 0.5 + 0.5, fz));
                    level.add_entity(fb);
                }
            }
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 10.0, 10.0);
        } else if self.last_seen < 5 {
            m.mov.set_wanted_position(t.pos.x, t.pos.y, t.pos.z, 1.0);
        }
    }
}
