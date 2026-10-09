//! Ghast: floats to random points (`GhastMoveControl`, `RandomFloatAroundGoal`), faces where it
//! goes or its target (`GhastLookGoal`) and shoots large fireballs (`GhastShootFireballGoal`).
//! Only its own fireball sent back by a player hurts it (for 1000).

use crate::entity::{Entity, MoverType};
use crate::ext_entity::fireball;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::control::Operation;
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, SpawnView, state, state_mut};
use crate::mob::goals::{self, Goal, LOOK, MOVE, Wanted};
use crate::mob::{self, DamageSource, MobData, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Ghast;

pub static KIND: Ghast = Ghast;

static INFO: Info = Info {
    fire_immune: true,
    monster_base: false,
    extends_monster: false,
    ..Info::monster("minecraft:ghast", &[(MaxHealth, 10.0), (FollowRange, 100.0), (CameraDistance, 8.0), (FlyingSpeed, 0.06)])
};

#[derive(Clone, Debug)]
pub struct GhastState {
    pub charging: bool,
    pub explosion_power: i32,
    /// `GhastMoveControl.floatDuration`.
    pub float_duration: i32,
}

/// `LivingEntity.travelFlying(input, water, lava, air)`: no gravity, a drag per medium.
pub fn travel_flying(e: &mut Entity, level: &mut dyn EntityLevel, input: Vec3, water: f32, lava: f32, air: f32) {
    let (speed, drag) = if e.is_in_water() {
        (water, 0.800000011920929)
    } else if e.is_in_lava() {
        (lava, 0.5)
    } else {
        (air, 0.9100000262260437)
    };
    mob::move_relative(e, speed, input);
    let d = e.delta;
    e.do_move(level, MoverType::SelfMove, d);
    e.delta = e.delta.scale(drag);
}

/// `GhastMoveControl.canReach` (not careful): no block collision on the way (blocks the ghast
/// already overlaps are ignored).
fn can_reach(e: &Entity, level: &dyn EntityLevel, delta: Vec3, careful: bool) -> bool {
    let bb = e.bounding_box();
    let moved = bb.offset_vec(delta);
    let from = e.position();
    let to = from + delta;
    let (in_water, in_lava) = (e.is_in_water(), e.is_in_lava());
    // `blockTraversalPossible` for a careful ghast (a happy one): never into a block it avoids, nor fluid it is not in.
    let careful_ok = |pos: BlockPos, state: u16| -> Option<bool> {
        if !careful {
            return None;
        }
        if crate::blocks::has_tag(state, crate::blocks::Tag::HappyGhastAvoids) {
            return Some(false);
        }
        let f = crate::physics::fluid_state(state);
        if f.kind.is_water() {
            return Some(in_water);
        }
        if f.kind.is_lava() {
            return Some(in_lava);
        }
        let _ = pos;
        None
    };
    if careful {
        // The box the move ends in, one block bigger all round: all of it must be passable (`betweenClosed(inflate(1))`).
        let area = moved.inflate(1.0, 1.0, 1.0);
        let (x0, y0, z0) = (crate::math::floor(area.min_x), crate::math::floor(area.min_y), crate::math::floor(area.min_z));
        let (x1, y1, z1) = (crate::math::floor(area.max_x), crate::math::floor(area.max_y), crate::math::floor(area.max_z));
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let p = BlockPos::new(x, y, z);
                    let state = level.block(p);
                    if kiln_data::blocks_types::is_air(state) {
                        continue;
                    }
                    // (No positions of travel here: any fluid counts and a shape with a collision is no way through.)
                    let f = crate::physics::fluid_state(state);
                    if crate::blocks::has_tag(state, crate::blocks::Tag::HappyGhastAvoids) || f.kind.is_water() || f.kind.is_lava() {
                        return false;
                    }
                    let (shape, _) = crate::collision::collision_shape(state, p, &crate::collision::CollisionContext::EMPTY);
                    if !shape.is_empty() {
                        return false;
                    }
                }
            }
        }
    }
    crate::inside::for_each_block_intersected_between(from, to, &moved, |pos, _| {
        if bb.intersects_block(pos) {
            return true;
        }
        let state = level.block(pos);
        if kiln_data::blocks_types::is_air(state) {
            return true;
        }
        if let Some(ok) = careful_ok(pos, state) {
            return ok;
        }
        let (shape, _) = crate::collision::collision_shape(state, pos, &crate::collision::CollisionContext::EMPTY);
        let boxes: Vec<_> = shape.boxes().iter().map(|b| b.offset(pos.x as f64, pos.y as f64, pos.z as f64)).collect();
        !e.make_bounding_box(from).collided_along_vector(to - from, &boxes)
    })
}

/// `GhastMoveControl.tick` (`float_duration`: its counter; `careful`: a happy ghast avoids what it should; `stopped`:
/// `shouldBeStopped`, the happy ghast standing still for a rider).
pub fn tick_ghast_move(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, float_duration: &mut i32, careful: bool, stopped: bool) {
    if stopped {
        // `Mob.stopInPlace`.
        m.mov.operation = Operation::Wait;
        m.nav.stop();
        m.xxa = 0.0;
        m.yya = 0.0;
        m.zza = 0.0;
        crate::mob::control::set_speed(m, 0.0);
        e.delta = Vec3::ZERO;
    }
    if m.mov.operation != Operation::MoveTo {
        return;
    }
    let d = *float_duration;
    *float_duration -= 1;
    if d > 0 {
        return;
    }
    *float_duration += e.random.next_int_bounded(5) + 2;
    let [wx, wy, wz] = m.mov.wanted;
    let delta = Vec3::new(wx - e.x(), wy - e.y(), wz - e.z());
    if can_reach(e, level, delta, careful) {
        let speed = m.attrs.value(Attr::FlyingSpeed) * 5.0 / 3.0;
        e.delta = e.delta + delta.normalize().scale(speed);
    } else {
        m.mov.operation = Operation::Wait;
    }
}

/// `GhastMoveControl.tick` of a ghast.
fn tick_move(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let Some(s) = state_mut::<GhastState>(m) else { return };
    let mut fd = s.float_duration;
    tick_ghast_move(e, m, level, &mut fd, false, false);
    if let Some(s) = state_mut::<GhastState>(m) {
        s.float_duration = fd;
    }
}

/// `Ghast.faceMovementDirection`.
pub fn face_movement_direction(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    match goals::target(m, level) {
        None => {
            let v = e.delta;
            e.y_rot = -(mth::atan2(v.x, v.z) as f32) * 57.295776;
            m.y_body_rot = e.y_rot;
        }
        Some(t) => {
            if t.pos.distance_to_sqr(e.position()) < 4096.0 {
                let (dx, dz) = (t.pos.x - e.x(), t.pos.z - e.z());
                e.y_rot = -(mth::atan2(dx, dz) as f32) * 57.295776;
                m.y_body_rot = e.y_rot;
            }
        }
    }
}

impl Kind for Ghast {
    fn sound_volume(&self, _m: &MobData) -> f32 {
        5.0
    }
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(GhastState { charging: false, explosion_power: 1, float_duration: 0 }))
    }
    fn register_goals(&self, m: &mut MobData) {
        m.goals.add(5, Goal::Custom(Box::new(RandomFloatAround { distance_to_blocks: 0 })));
        m.goals.add(7, Goal::Custom(Box::new(GhastLook)));
        m.goals.add(7, Goal::Custom(Box::new(GhastShootFireball { charge_time: 0 })));
        m.targets.add(
            1,
            Goal::NearestAttackable { wanted: Wanted::PlayerWithinDy(4), interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false },
        );
    }
    fn travel(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        travel_flying(e, level, input, 0.02, 0.02, 0.02);
        true
    }
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        tick_move(e, m, level);
        true
    }
    fn checks_fall_damage(&self) -> bool {
        false
    }
    /// Fire immune, except that its own fireball sent back by a player is decided in `hurt`.
    fn is_invulnerable_to(&self, _m: &MobData, kind: DamageKind) -> bool {
        kind != DamageKind::Fireball && kind.is_tag("minecraft:is_fire")
    }
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        if source.kind != DamageKind::Fireball {
            return None;
        }
        // `isReflectedFireball`: a fireball whose owner is now a player.
        if source.attacker_is_player || source.attacker.is_some_and(|a| level.player(a).is_some()) {
            mob::hurt_base(e, m, level, *source, 1000.0);
            return Some(true);
        }
        Some(false)
    }
    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let p = r.byte_or("ExplosionPower", 1) as i32;
        if let Some(s) = state_mut::<GhastState>(m) {
            s.explosion_power = p;
        }
    }
    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("ExplosionPower", Tag::Byte(state::<GhastState>(m).map_or(1, |s| s.explosion_power) as i8));
    }
    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::ghast::IS_CHARGING, &DataValue::Boolean(state::<GhastState>(m).is_some_and(|s| s.charging)));
    }
    fn max_spawn_cluster(&self) -> i32 {
        1
    }
    fn spawn_ignores_light(&self) -> bool {
        true
    }
    /// `checkGhastSpawnRules`: one in 20, off peaceful, on a valid spawn block.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        Some(view.difficulty() != 0 && r.next_int_bounded(20) == 0 && super::slime::check_mob_spawn_rules(view, pos))
    }
}

// ---------------------------------------------------------------------- goals

/// `RandomFloatAroundGoal(mob, distanceToBlocks)`: a random point in a 16-block cube (within its home, and with a
/// block or more near it when `distance_to_blocks` is set), brought down below where it is when it would be under
/// the ground's surface.
#[derive(Clone, Debug)]
pub struct RandomFloatAround {
    pub distance_to_blocks: i32,
}

/// `RandomFloatAroundGoal.getSuitableFlyToPosition`.
pub fn suitable_fly_to_position(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, distance_to_blocks: i32) -> Vec3 {
    let here = e.position();
    let mut pos: Option<Vec3> = None;
    let choose = |e: &mut Entity| {
        let r = &mut e.random;
        let x = here.x + ((r.next_float() * 2.0 - 1.0) * 16.0) as f64;
        let y = here.y + ((r.next_float() * 2.0 - 1.0) * 16.0) as f64;
        let z = here.z + ((r.next_float() * 2.0 - 1.0) * 16.0) as f64;
        Vec3::new(x, y, z)
    };
    for _ in 0..64 {
        // `chooseRandomPositionWithRestriction`: outside the home is no position.
        let p = choose(e);
        let p = if m.home.is_some() && !crate::mob::random_pos::within_home(m.home, BlockPos::containing(p.x, p.y, p.z)) { None } else { Some(p) };
        pos = p;
        if let Some(p) = p
            && is_good_target(level, p, distance_to_blocks)
        {
            return p;
        }
    }
    let mut p = pos.unwrap_or_else(|| choose(e));
    let b = BlockPos::containing(p.x, p.y, p.z);
    let height = level.heightmap(b.x, b.z, false);
    if height < b.y && height > level.min_y() {
        p = Vec3::new(p.x, e.y() - (e.y() - p.y).abs(), p.z);
    }
    p
}

/// `RandomFloatAroundGoal.isGoodTarget`: an empty block with a solid one within `distance` blocks of it.
fn is_good_target(level: &dyn EntityLevel, p: Vec3, distance: i32) -> bool {
    if distance <= 0 {
        return true;
    }
    let at = BlockPos::containing(p.x, p.y, p.z);
    if !kiln_data::blocks_types::is_air(level.block(at)) {
        return false;
    }
    for d in crate::math::Direction::ALL {
        let (dx, dy, dz) = d.step();
        for n in 1..distance {
            let b = BlockPos::new(at.x + dx * n, at.y + dy * n, at.z + dz * n);
            if !kiln_data::blocks_types::is_air(level.block(b)) {
                return true;
            }
        }
    }
    false
}

impl CustomGoal for RandomFloatAround {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RandomFloatAroundGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !m.mov.has_wanted() {
            return true;
        }
        let [wx, wy, wz] = m.mov.wanted;
        let (dx, dy, dz) = (wx - e.x(), wy - e.y(), wz - e.z());
        let d = dx * dx + dy * dy + dz * dz;
        d < 1.0 || d > 3600.0
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let p = suitable_fly_to_position(e, m, &*level, self.distance_to_blocks);
        m.mov.set_wanted_position(p.x, p.y, p.z, 1.0);
    }
}

/// `GhastLookGoal`.
#[derive(Clone, Debug)]
struct GhastLook;

impl CustomGoal for GhastLook {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "GhastLookGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        true
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        face_movement_direction(e, m, level);
    }
}

/// `GhastShootFireballGoal`.
#[derive(Clone, Debug)]
struct GhastShootFireball {
    charge_time: i32,
}

fn set_charging(m: &mut MobData, on: bool) {
    if let Some(s) = state_mut::<GhastState>(m) {
        s.charging = on;
    }
}

impl CustomGoal for GhastShootFireball {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "GhastShootFireballGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some()
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.charge_time = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_charging(m, false);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        if t.pos.distance_to_sqr(e.position()) < 4096.0 && mob::has_line_of_sight_cached(e, m, level, &t) {
            self.charge_time += 1;
            if self.charge_time == 10 && !e.silent {
                level.emit(Event::LevelEvent { event: 1015, pos: e.block_position(), data: 0 });
            }
            if self.charge_time == 20 {
                let view = fireball::view_vector(e.x_rot, m.y_head_rot);
                let half = e.y() + e.height as f64 * 0.5;
                let t_half = t.pos.y + (t.bb.max_y - t.bb.min_y) as f32 as f64 * 0.5;
                let dx = t.pos.x - (e.x() + view.x * 4.0);
                let dy = t_half - (0.5 + half);
                let dz = t.pos.z - (e.z() + view.z * 4.0);
                if !e.silent {
                    level.emit(Event::LevelEvent { event: 1016, pos: e.block_position(), data: 0 });
                }
                let power = state::<GhastState>(m).map_or(1, |s| s.explosion_power);
                let id = level.next_entity_id();
                let seed = level.fresh_seed();
                let mut fb = fireball::new(false, id, e, Vec3::new(dx, dy, dz).normalize(), power, seed);
                let fz = fb.z();
                fb.set_pos(Vec3::new(e.x() + view.x * 4.0, half + 0.5, fz + view.z * 4.0));
                level.add_entity(fb);
                self.charge_time = -40;
            }
        } else if self.charge_time > 0 {
            self.charge_time -= 1;
        }
        let c = self.charge_time > 10;
        set_charging(m, c);
    }
}
