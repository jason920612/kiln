//! Guardian and elder guardian (`Guardian`, `ElderGuardian`): water-bound monsters that swim
//! (`WaterBoundPathNavigation`, `GuardianMoveControl` with its wobble) and fire a beam
//! (`GuardianAttackGoal`: 80 ticks, 60 for the elder, of locking on, then magic damage and a
//! bite). Their spikes hurt melee attackers for 2 while they are not moving; out of water they
//! flop about. The elder keeps to its home and gives players within 50 blocks mining fatigue
//! III every minute (with the elder guardian's ghost on their screens).

use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::control::{self, Operation};
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, Placement, SpawnView, state, state_mut};
use crate::mob::goals::{self, Goal, LOOK, Living, MOVE, Wanted};
use crate::mob::{self, DamageSource, MobData, MobKind, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct GuardianKind {
    elder: bool,
}

pub static GUARDIAN: GuardianKind = GuardianKind { elder: false };
pub static ELDER: GuardianKind = GuardianKind { elder: true };

static INFO: Info = Info {
    breathes_under_water: true,
    head: (75, 180, 10),
    ambient_interval: 160,
    sounds: Some("guardian"),
    ..Info::monster("minecraft:guardian", &[(AttackDamage, 6.0), (MovementSpeed, 0.5), (MaxHealth, 30.0)])
};

static ELDER_INFO: Info = Info {
    breathes_under_water: true,
    head: (75, 180, 10),
    ambient_interval: 160,
    sounds: Some("elder_guardian"),
    ..Info::monster("minecraft:elder_guardian", &[(AttackDamage, 8.0), (MovementSpeed, 0.30000001192092896), (MaxHealth, 80.0)])
};

#[derive(Clone, Debug, Default)]
pub struct GuardianState {
    /// `DATA_ID_MOVING` (the spikes are in while it swims).
    pub moving: bool,
    /// `DATA_ID_ATTACK_TARGET`: the beam's target (0: none).
    pub attack_target: i32,
    /// `randomStrollGoal.trigger()` waiting for the stroll goal.
    pub stroll_trigger: bool,
    /// `Mob.homePosition` / `homeRadius` (the elder sets its own).
    pub home: Option<(BlockPos, i32)>,
}

fn st(m: &MobData) -> &GuardianState {
    state::<GuardianState>(m).expect("guardian state")
}

fn st_mut(m: &mut MobData) -> &mut GuardianState {
    state_mut::<GuardianState>(m).expect("guardian state")
}

fn is_elder(m: &MobData) -> bool {
    m.kind == MobKind::ElderGuardian
}

/// `getAttackDuration`.
fn attack_duration(m: &MobData) -> i32 {
    if is_elder(m) { 60 } else { 80 }
}

/// The beam's target (`getActiveAttackTarget` on the server: the target while locked on).
pub fn beam_target(m: &MobData) -> Option<i32> {
    state::<GuardianState>(m).filter(|s| s.attack_target != 0).map(|s| s.attack_target)
}

/// `GuardianMoveControl.tick`: swims toward the wanted spot with a sine wobble, easing its
/// speed and looking ahead; the spikes go in while it moves.
fn tick_move(e: &mut Entity, m: &mut MobData) {
    if m.mov.operation == Operation::MoveTo && !m.nav.is_done() {
        let [wx, wy, wz] = m.mov.wanted;
        let delta = Vec3::new(wx - e.x(), wy - e.y(), wz - e.z());
        let length = delta.length();
        let (xd, yd, zd) = (delta.x / length, delta.y / length, delta.z / length);
        let y_rot_d = (mth::atan2(delta.z, delta.x) * 57.2957763671875) as f32 - 90.0;
        e.y_rot = control::rotlerp(e.y_rot, y_rot_d, 90.0);
        m.y_body_rot = e.y_rot;
        let target_speed = (m.mov.speed_modifier * m.attrs.value(Attr::MovementSpeed)) as f32;
        let speed = mth::lerp_f(0.125, m.speed, target_speed);
        control::set_speed(m, speed);
        let t = (e.tick_count + e.id) as f64;
        let push = kiln_javamath::trig::sin(t * 0.5) * 0.05;
        let r = (e.y_rot * (std::f64::consts::PI / 180.0) as f32) as f64;
        let cos = kiln_javamath::trig::cos(r);
        let sin = kiln_javamath::trig::sin(r);
        let y_push = kiln_javamath::trig::sin(t * 0.75) * 0.05;
        e.delta = e.delta.add(push * cos, y_push * (sin + cos) * 0.25 + speed as f64 * yd * 0.1, push * sin);
        let new = [e.x() + xd * 2.0, e.eye_y() + yd / length, e.z() + zd * 2.0];
        let old = if m.look.is_looking_at_target() { m.look.wanted } else { new };
        let lerp = |a: f64, b: f64| a + 0.125 * (b - a);
        m.look.set_look_at(lerp(old[0], new[0]), lerp(old[1], new[1]), lerp(old[2], new[2]), 10.0, 40.0);
        st_mut(m).moving = true;
    } else {
        control::set_speed(m, 0.0);
        st_mut(m).moving = false;
    }
}

/// `Guardian.travelInWater`: a steady 0.1 push and 0.9 drag; an idle guardian sinks slowly.
fn travel_in_water(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) {
    mob::move_relative(e, 0.1, input);
    let d = e.delta;
    e.do_move(level, MoverType::SelfMove, d);
    e.delta = e.delta.scale(0.9);
    if !st(m).moving && goals::target(m, level).is_none() {
        e.delta = e.delta.add(0.0, -0.005, 0.0);
    }
}

impl GuardianKind {
    fn kind_info(&self) -> &'static Info {
        if self.elder { &ELDER_INFO } else { &INFO }
    }
}

impl Kind for GuardianKind {
    fn info(&self) -> &'static Info {
        self.kind_info()
    }

    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.swim = true;
        m.nav.can_pass_doors = false;
        m.maluses.push((path::PathType::Water, 0.0));
        // `clientSideTailAnimation`.
        random.next_float();
        if self.elder {
            m.persistence_required = true;
        }
        Some(Box::new(GuardianState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let interval = if self.elder { 400 } else { 80 };
        let g = &mut m.goals;
        g.add(4, Goal::Custom(Box::new(GuardianAttack { attack_time: 0 })));
        g.add(5, Goal::Custom(Box::new(MoveTowardsRestriction { wanted: Vec3::ZERO })));
        g.add(7, Goal::Custom(Box::new(Stroll { wanted: Vec3::ZERO, interval, force: false })));
        g.add(8, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::Custom(Box::new(LookAtGuardian { look_at: None, look_time: 0 })));
        g.add(9, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        m.targets.add(
            1,
            Goal::NearestAttackable { wanted: Wanted::Player, interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false },
        );
    }

    /// `GuardianAttackSelector`: players more than 3 blocks away (squids and axolotls are not
    /// simulated).
    fn player_target_ok(&self, e: &Entity, _m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        t.pos.distance_to_sqr(e.position()) > 9.0
    }

    /// `Guardian.aiStep` before `super.aiStep()`: air in the water, flopping on land, facing
    /// the beam's target.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if mob::is_alive(e, m) {
            if e.is_in_water() {
                e.air_supply = 300;
            } else if e.on_ground {
                let x = (e.random.next_float() * 2.0 - 1.0) * 0.4;
                let z = (e.random.next_float() * 2.0 - 1.0) * 0.4;
                e.delta = e.delta.add(x as f64, 0.5, z as f64);
                e.y_rot = e.random.next_float() * 360.0;
                e.on_ground = false;
                e.needs_sync = true;
            }
            if st(m).attack_target != 0 {
                e.y_rot = m.y_head_rot;
            }
        }
        // `Monster.aiStep`: `updateNoActionTime`.
        if mob::light_magic_value(e, level) > 0.5 {
            m.no_action_time += 2;
        }
    }

    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        if e.is_in_water() {
            travel_in_water(e, m, level, input);
        } else {
            mob::travel(e, m, level, input);
        }
        true
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        tick_move(e, m);
        true
    }

    /// `ElderGuardian.customServerAiStep`: mining fatigue for players around every minute, and
    /// a home where it first finds itself.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !self.elder {
            return;
        }
        if (e.tick_count + e.id) % 1200 == 0 {
            // `MobEffectUtil.addEffectToPlayersAround(.., 50, mining fatigue 6000 III, 1200)`.
            let pos = e.position();
            let players: Vec<i32> = level
                .players()
                .iter()
                .filter(|p| !p.creative && !p.spectator && p.alive && p.pos.distance_to_sqr(pos) < 50.0 * 50.0)
                .map(|p| p.id)
                .collect();
            for id in players {
                let refresh = match level.player_effect(id, "minecraft:mining_fatigue") {
                    None => true,
                    Some((amplifier, duration)) => amplifier < 2 || (duration != -1 && duration <= 1199),
                };
                if refresh {
                    level.add_effect(id, "minecraft:mining_fatigue", 6000, 2, Some(e.id));
                    level.emit(Event::PlayerGameEvent { player: id, event: 10, param: if e.silent { 0.0 } else { 1.0 } });
                }
            }
        }
        if st(m).home.is_none() {
            st_mut(m).home = Some((e.block_position(), 16));
        }
    }

    /// `Guardian.hurtServer`: the spikes hurt a living attacker for 2 unless the guardian is
    /// swimming (or the hit was magic, thorns or an explosion); a hit sends it strolling.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        if !st(m).moving
            && !source.kind.is_tag("minecraft:avoids_guardian_thorns")
            && source.kind != DamageKind::Thorns
            && let Some(cause) = source.direct.or(source.attacker).and_then(|id| goals::living(level, id))
        {
            let thorns = DamageSource { kind: DamageKind::Thorns, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
            mob::hurt_living(level, &cause, thorns, 2.0);
        }
        st_mut(m).stroll_trigger = true;
        None
    }

    fn ambient_sound(&self, e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let name = if is_elder(m) { "elder_guardian" } else { "guardian" };
        let what = if e.is_in_water() { "ambient" } else { "ambient_land" };
        Some(Some(mob::sound_event(&format!("minecraft:entity.{name}.{what}"))))
    }

    /// `Guardian.getWalkTargetValue`: water is worth 10 more.
    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        let water = crate::physics::fluid_state(level.block(p)).kind.is_water();
        let light = mob::light_magic_value_at(level, p) - 0.5;
        Some(if water { 10.0 + light } else { -light })
    }

    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        // `xpReward = 10`, plus the usual equipment bonus.
        let mut xp = 10;
        for i in 0..6 {
            if !m.equipment[i].is_empty() && m.drop_chances[i] <= 1.0 {
                xp += 1 + e.random.next_int_bounded(3);
            }
        }
        Some(xp)
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }

    /// `checkGuardianSpawnRules`: water here and below, off peaceful, one in 20 where the sky
    /// shows through the water (the rest always).
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        let water = |p: BlockPos| crate::physics::fluid_state(view.block(p)).kind.is_water();
        let sky = r.next_int_bounded(20) == 0 || !sky_from_below_water(view, pos);
        Some(sky && view.difficulty() != 0 && water(pos) && water(pos.below()))
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let radius = r.int_or("home_radius", -1);
        if radius >= 0 {
            let home = match r.get("home_pos") {
                Some(Tag::IntArray(v)) if v.len() == 3 => BlockPos::new(v[0], v[1], v[2]),
                _ => BlockPos::default(),
            };
            st_mut(m).home = Some((home, radius));
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let (home, radius) = st(m).home.unwrap_or((BlockPos::default(), -1));
        o.put("home_radius", Tag::Int(radius));
        o.put("home_pos", Tag::IntArray(vec![home.x, home.y, home.z]));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::guardian as g;
        let s = st(m);
        d.set(g::ID_MOVING, &DataValue::Boolean(s.moving));
        d.set(g::ID_ATTACK_TARGET, &DataValue::Int(s.attack_target));
    }
}

/// `LevelAccessor.canSeeSkyFromBelowWater`: up through water to where the sky shows.
fn sky_from_below_water(view: &dyn SpawnView, pos: BlockPos) -> bool {
    if pos.y >= view.sea_level() {
        return view.sky_light(pos) >= 15;
    }
    let mut p = BlockPos::new(pos.x, view.sea_level(), pos.z);
    if view.sky_light(p) < 15 {
        return false;
    }
    p = p.below();
    while p.y > pos.y {
        let s = view.block(p);
        let liquid = matches!(crate::blocks::block_name(s), "minecraft:water" | "minecraft:lava");
        if kiln_data::block_props::light_dampening(s) > 0 && !liquid {
            return false;
        }
        p = p.below();
    }
    true
}

// ---------------------------------------------------------------------- goals

/// `GuardianAttackGoal`: locks the beam on (entity event 21), then after the attack duration
/// deals magic damage (1, +2 on hard, +2 for the elder) and a bite, and lets go.
#[derive(Clone, Debug)]
struct GuardianAttack {
    attack_time: i32,
}

impl CustomGoal for GuardianAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "GuardianAttackGoal"
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
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level) && (is_elder(m) || goals::target(m, level).is_some_and(|t| e.position().distance_to_sqr(t.pos) > 9.0))
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.attack_time = -10;
        m.nav.stop();
        if let Some(t) = goals::target(m, level) {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 90.0, 90.0);
        }
        e.needs_sync = true;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).attack_target = 0;
        mob::set_target(e, m, None);
        st_mut(m).stroll_trigger = true;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        m.nav.stop();
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 90.0, 90.0);
        if !mob::has_line_of_sight_cached(e, m, level, &t) {
            mob::set_target(e, m, None);
            return;
        }
        self.attack_time += 1;
        if self.attack_time == 0 {
            st_mut(m).attack_target = t.id;
            if !e.silent {
                level.emit(Event::EntityEvent { entity: e.id, event: 21 });
            }
        } else if self.attack_time >= attack_duration(m) {
            let mut damage = 1.0;
            if level.difficulty() == 3 {
                damage += 2.0;
            }
            if is_elder(m) {
                damage += 2.0;
            }
            let source = DamageSource { kind: DamageKind::IndirectMagic, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
            mob::hurt_living(level, &t, source, damage);
            mob::do_hurt_target(e, m, level, &t);
            mob::set_target(e, m, None);
        }
    }
}

/// `MoveTowardsRestrictionGoal`: back toward the home when outside it.
#[derive(Clone, Debug)]
struct MoveTowardsRestriction {
    wanted: Vec3,
}

impl CustomGoal for MoveTowardsRestriction {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveTowardsRestrictionGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let home = st(m).home;
        if random_pos::within_home(home, e.block_position()) {
            return false;
        }
        let (c, _) = home.unwrap();
        let to = Vec3::new(c.x as f64 + 0.5, c.y as f64, c.z as f64 + 0.5);
        match random_pos::default_pos_towards_home(e, m, level, 16, 7, to, std::f32::consts::FRAC_PI_2 as f64, home) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, 1.0);
    }
}

/// `RandomStrollGoal(1.0, 80)` (400 for the elder), also moving the look; `trigger()` (a hit,
/// the end of an attack) makes it go at once.
#[derive(Clone, Debug)]
struct Stroll {
    wanted: Vec3,
    interval: i32,
    force: bool,
}

impl CustomGoal for Stroll {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RandomStrollGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if std::mem::take(&mut st_mut(m).stroll_trigger) {
            self.force = true;
        }
        if !self.force {
            if m.no_action_time >= 100 {
                return false;
            }
            if e.random.next_int_bounded(mth::reduced_tick_delay(self.interval)) != 0 {
                return false;
            }
        }
        let home = st(m).home;
        match random_pos::default_pos_home(e, m, level, 10, 7, home) {
            Some(p) => {
                self.wanted = p;
                self.force = false;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, 1.0);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// `LookAtPlayerGoal(Guardian.class, 12, 0.01)`: now and then at the nearest guardian in view.
#[derive(Clone, Debug)]
struct LookAtGuardian {
    look_at: Option<i32>,
    look_time: i32,
}

impl CustomGoal for LookAtGuardian {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_float() >= 0.01 {
            return false;
        }
        let area = e.bounding_box().inflate(12.0, 3.0, 12.0);
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Living, i32::MIN) {
            let Some(t) = goals::living(level, id) else { continue };
            if !matches!(t.type_name, "minecraft:guardian" | "minecraft:elder_guardian") {
                continue;
            }
            if !goals::targeting_ok(e, m, level, &t, false, 12.0, true) {
                continue;
            }
            let (dx, dy, dz) = (t.pos.x - e.x(), t.pos.y - e.eye_y(), t.pos.z - e.z());
            let d = dx * dx + dy * dy + dz * dz;
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.look_at = best.map(|(_, id)| id);
        self.look_at.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return false };
        t.alive && e.position().distance_to_sqr(t.pos) <= 144.0 && self.look_time > 0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time = mth::reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_at = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return };
        if !t.alive {
            return;
        }
        control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
        self.look_time -= 1;
    }
}
