//! Drowned: an underwater zombie that swims after targets in the water and throws tridents.
//!
//! It navigates on land and in water alike (`AmphibiousPathNavigation`), swims with its own move
//! control when it wants to (a target in the water, or looking for land), heads for water by day
//! and for the shore (or up to the surface) by night, attacks with a held trident from range
//! (`ThrownTrident`, see [`crate::ext_entity::trident`]) and in melee only at night or at targets
//! in the water (`okTarget`).

use super::zombie::{self, HasZombie, IRON_GOLEM, VILLAGERS, ZombieState, hurt_by, nearest};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE, MeleeKind, Wanted};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, control, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Drowned;

pub static KIND: Drowned = Drowned;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:drowned", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0), (StepHeight, 1.0)])
};

#[derive(Clone, Debug)]
pub struct DrownedState {
    pub zombie: ZombieState,
    pub searching_for_land: bool,
    /// `rangedAttackUncertainty` (-1: the difficulty's).
    pub ranged_uncertainty: f32,
    /// The swimming flag (`updateSwimming`).
    pub swimming: bool,
}

impl HasZombie for DrownedState {
    fn zombie(&self) -> &ZombieState {
        &self.zombie
    }
    fn zombie_mut(&mut self) -> &mut ZombieState {
        &mut self.zombie
    }
}

fn st(m: &MobData) -> &DrownedState {
    crate::mob::ext::state::<DrownedState>(m).expect("drowned state")
}

fn st_mut(m: &mut MobData) -> &mut DrownedState {
    crate::mob::ext::state_mut::<DrownedState>(m).expect("drowned state")
}

/// `isInWater` of a target: a mob's own flag; for a player, water in its box reaching its feet.
pub fn in_water(level: &dyn EntityLevel, t: &Living) -> bool {
    if !t.player {
        return level.entity(t.id).is_some_and(|e| e.is_in_water());
    }
    if let Some(w) = level.player(t.id).and_then(|p| p.in_water) {
        return w;
    }
    let b = t.bb.deflate(0.001, 0.001, 0.001);
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::ceil(b.max_x), crate::math::ceil(b.max_y), crate::math::ceil(b.max_z));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let p = BlockPos::new(x, y, z);
                let f = crate::physics::fluid_state(level.block(p));
                if f.kind.is_water() && y as f64 + crate::fluid::height(level, p, &f) as f64 >= b.min_y {
                    return true;
                }
            }
        }
    }
    false
}

/// `okTarget`: at night anything, by day only targets in the water.
fn ok_target(level: &dyn EntityLevel, t: Option<Living>) -> bool {
    t.is_some_and(|t| !level.is_bright_outside() || in_water(level, &t))
}

fn raw_target(m: &MobData, level: &dyn EntityLevel) -> Option<Living> {
    m.target.and_then(|id| goals::living(level, id))
}

/// `wantsToSwim`.
fn wants_to_swim(m: &MobData, level: &dyn EntityLevel) -> bool {
    st(m).searching_for_land || raw_target(m, level).is_some_and(|t| in_water(level, &t))
}

/// `Entity.isUnderWater`.
fn under_water(e: &Entity) -> bool {
    e.was_eye_in_water && e.is_in_water()
}

fn holds_trident(m: &MobData) -> bool {
    mob::item_name(&m.equipment[mob::MAINHAND]) == "minecraft:trident"
}

impl Kind for Drowned {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.amphibious = true;
        m.maluses.push((path::PathType::Water, 0.0));
        Some(Box::new(DrownedState { zombie: ZombieState::default(), searching_for_land: false, ranged_uncertainty: -1.0, swimming: false }))
    }

    /// `Zombie.registerGoals` with `Drowned.addBehaviourGoals`.
    fn register_goals(&self, m: &mut MobData) {
        zombie::register_base_goals(m);
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(GoToWater { wanted: Vec3::ZERO, speed: 1.0 })));
        g.add(2, Goal::Custom(Box::new(TridentAttack { target: None, attack_time: -1, see_time: 0 })));
        let melee =
            Goal::Melee { kind: MeleeKind::Zombie, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 };
        g.add(2, Goal::Custom(Box::new(DrownedAttack { inner: melee })));
        g.add(5, Goal::Custom(Box::new(GoToBeach { next_start: 0, try_ticks: 0, max_stay: 0, block: BlockPos::default() })));
        g.add(6, Goal::Custom(Box::new(SwimUp { stuck: false })));
        g.add(7, zombie::stroll(1.0, false));
        let t = &mut m.targets;
        t.add(1, hurt_by(true));
        t.add(2, nearest(Wanted::Player, true));
        t.add(3, nearest(Wanted::Types(VILLAGERS), false));
        t.add(3, nearest(Wanted::Types(IRON_GOLEM), true));
        t.add(3, nearest(Wanted::Types(&["minecraft:axolotl"]), true));
        // Baby turtles on land.
        t.add(5, nearest(Wanted::BabyTurtlesOnLand, true));
    }

    fn player_target_ok(&self, _e: &Entity, _m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        ok_target(level, Some(*t))
    }

    /// `updateSwimming` (the flag viewers see).
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let swim = !m.no_ai && under_water(e) && wants_to_swim(m, level);
        st_mut(m).swimming = swim;
    }

    /// `DrownedMoveControl.tick`: swimming toward the wanted position.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !(wants_to_swim(m, level) && e.is_in_water()) {
            if !e.on_ground {
                e.delta = e.delta.add(0.0, -0.008, 0.0);
            }
            return false;
        }
        let target = raw_target(m, level);
        if target.is_some_and(|t| t.pos.y > e.y()) || st(m).searching_for_land {
            e.delta = e.delta.add(0.0, 0.002, 0.0);
        }
        if m.mov.operation != control::Operation::MoveTo || m.nav.is_done() {
            control::set_speed(m, 0.0);
            return true;
        }
        let [wx, wy, wz] = m.mov.wanted;
        let (dx, mut dy, dz) = (wx - e.x(), wy - e.y(), wz - e.z());
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        dy /= dist;
        let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
        e.y_rot = control::rotlerp(e.y_rot, yaw, 90.0);
        m.y_body_rot = e.y_rot;
        let target_speed = (m.mov.speed_modifier * m.attrs.value(Attr::MovementSpeed)) as f32;
        let speed = mth::lerp_f(0.125, m.speed, target_speed);
        control::set_speed(m, speed);
        e.delta = e.delta.add(speed as f64 * dx * 0.005, speed as f64 * dy * 0.1, speed as f64 * dz * 0.005);
        true
    }

    /// `Drowned.travelInWater`: swimming under water, slower but free of the water's drag.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        if !(e.is_in_water() && under_water(e) && wants_to_swim(m, level)) {
            return false;
        }
        mob::move_relative(e, 0.01, input);
        let d = e.delta;
        e.do_move(level, crate::entity::MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        true
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt {
            zombie::reinforcements(e, m, level, source);
        }
    }

    /// `Drowned.finalizeSpawn`: the zombie's (with the drowned's trident or fishing rod), a rare
    /// nautilus shell, and for natural spawns with a trident the zombie nautilus roll (the
    /// nautilus is not simulated: only the draw).
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        zombie::finalize(e, m, r, ctx, group, false);
        if m.equipment[mob::OFFHAND].is_empty() && r.next_float() < 0.03 {
            if let Some(s) = kiln_item::ItemStack::of("minecraft:nautilus_shell", 1) {
                m.equipment[mob::OFFHAND] = s;
                m.drop_chances[mob::OFFHAND] = 2.0;
            }
        }
        if group.natural && holds_trident(m) {
            let _ = r.next_float() < 0.5;
        }
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        zombie::load(e, m, r);
    }

    fn save(&self, e: &Entity, m: &MobData, o: &mut Output) {
        zombie::save(e, m, o);
    }

    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        zombie::entity_data(e, m, d);
        if st(m).swimming {
            let flags = (e.is_on_fire() as i8) | 0x10;
            d.set(kiln_data::entities::data::entity::SHARED_FLAGS, &DataValue::Byte(flags));
        }
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { zombie::baby_dimensions(m.kind) } else { base }
    }

    /// `checkDrownedSpawnRules`: water below and here, dark enough, then one in 15 in rivers
    /// (`#more_frequent_drowned_spawns`) or one in 40 deeper than 5 below sea level (the draw
    /// always happens).
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        let water = |p: BlockPos| crate::physics::fluid_state(view.block(p)).kind.is_water();
        if !water(pos.below()) {
            return Some(false);
        }
        let ok = view.difficulty() != 0 && zombie::dark_enough_view(view, pos, r) && water(pos);
        let biome = view.biome(pos);
        if biome_tag(biome, "minecraft:more_frequent_drowned_spawns") {
            return Some(r.next_int_bounded(15) == 0 && ok);
        }
        Some(r.next_int_bounded(40) == 0 && pos.y < view.sea_level() - 5 && ok)
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }
}

/// Whether biome `id` (`minecraft:worldgen/biome`) is in `tag`.
fn biome_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:worldgen/biome")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

/// `Drowned.performRangedAttack`: a trident from the eyes toward the target's lower third.
fn perform_ranged_attack(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let uuid = (seed as u64 as u128) << 64 | (id as u32 as u128);
    let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
    let mut trident = crate::ext_entity::trident::new(id, uuid, pos, Some(e.id), seed);
    let dx = t.pos.x - e.x();
    let dy = t.pos.y + (t.bb.max_y - t.bb.min_y) * 0.3333333333333333 - trident.y();
    let dz = t.pos.z - e.z();
    let horizontal = (dx * dx + dz * dz).sqrt();
    let u = st(m).ranged_uncertainty;
    let uncertainty = if u < 0.0 { (14 - level.difficulty() as i32 * 4) as f32 } else { u };
    mob::species::shoot(&mut trident, dx, dy + horizontal * 0.20000000298023224, dz, 1.6, uncertainty);
    trident.set_old_pos_and_rot();
    level.add_entity(trident);
    let pitch = 1.0 / (e.random.next_float() * 0.4 + 0.8);
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.drowned.shoot", source: "hostile", volume: 1.0, pitch });
    }
}

// ---------------------------------------------------------------------- goals

/// `DrownedGoToWaterGoal`: by day, out of the water, to a random water block nearby.
#[derive(Clone, Debug)]
struct GoToWater {
    wanted: Vec3,
    speed: f64,
}

impl CustomGoal for GoToWater {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DrownedGoToWaterGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !level.is_bright_outside() || e.is_in_water() {
            return false;
        }
        let origin = e.block_position();
        for _ in 0..10 {
            let dx = e.random.next_int_bounded(20) - 10;
            let dy = 2 - e.random.next_int_bounded(8);
            let dz = e.random.next_int_bounded(20) - 10;
            let p = origin.offset(dx, dy, dz);
            if crate::blocks::block_name(level.block(p)) == "minecraft:water" {
                self.wanted = Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
                return true;
            }
        }
        false
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, self.speed);
    }
}

/// `DrownedTridentAttackGoal` (a `RangedAttackGoal`: speed 1, every 40 ticks, radius 10) while
/// holding a trident.
#[derive(Clone, Debug)]
struct TridentAttack {
    target: Option<i32>,
    attack_time: i32,
    see_time: i32,
}

const RADIUS: f32 = 10.0;
const INTERVAL: i32 = 40;

impl CustomGoal for TridentAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DrownedTridentAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = goals::target(m, level) else { return false };
        if !t.alive {
            return false;
        }
        self.target = Some(t.id);
        holds_trident(m)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level) || (self.target.and_then(|id| goals::living(level, id)).is_some_and(|t| t.alive) && !m.nav.is_done())
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.set_aggressive(true);
        if !m.equipment[mob::MAINHAND].is_empty() {
            m.start_using_item();
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.target = None;
        self.see_time = 0;
        self.attack_time = -1;
        m.stop_using_item();
        m.set_aggressive(false);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return };
        let d = e.position().distance_to_sqr(t.pos);
        let sees = mob::has_line_of_sight_cached(e, m, level, &t);
        if sees {
            self.see_time += 1;
        } else {
            self.see_time = 0;
        }
        if !(d > (RADIUS * RADIUS) as f64) && self.see_time >= 5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.0);
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = d.sqrt() as f32 / RADIUS;
            perform_ranged_attack(e, m, level, &t);
            self.attack_time = mth_floor_f(dist * (INTERVAL - INTERVAL) as f32 + INTERVAL as f32);
        } else if self.attack_time < 0 {
            let f = d.sqrt() / RADIUS as f64;
            self.attack_time = crate::math::floor(INTERVAL as f64 + f * (INTERVAL - INTERVAL) as f64);
        }
    }
}

fn mth_floor_f(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}

/// `DrownedAttackGoal`: the zombie's melee attack on a target it is `okTarget` with.
#[derive(Clone, Debug)]
struct DrownedAttack {
    inner: Goal,
}

impl CustomGoal for DrownedAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DrownedAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_use(&mut self.inner, e, m, level) && ok_target(level, raw_target(m, level))
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level) && ok_target(level, raw_target(m, level))
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

/// `DrownedGoToBeachGoal` (a `MoveToBlockGoal`, range 8, 2 up and down): at night, in the
/// water near the surface, to a block with room to stand on it.
#[derive(Clone, Debug)]
struct GoToBeach {
    next_start: i32,
    try_ticks: i32,
    max_stay: i32,
    block: BlockPos,
}

impl GoToBeach {
    fn valid_target(level: &dyn EntityLevel, p: BlockPos) -> bool {
        let air = |p: BlockPos| kiln_data::blocks_types::is_air(level.block(p));
        if !air(p.above()) || !air(p.above().above()) {
            return false;
        }
        // `entityCanStandOn`: the collision shape's top face is full.
        let (shape, _) = crate::collision::collision_shape(level.block(p), p, &crate::collision::CollisionContext::EMPTY);
        shape.boxes().iter().any(|b| b.max_y >= 1.0 && b.min_x <= 0.0 && b.max_x >= 1.0 && b.min_z <= 0.0 && b.max_z >= 1.0)
    }

    /// `findNearestBlock`.
    fn find(&mut self, e: &Entity, level: &dyn EntityLevel) -> bool {
        let (range, vrange) = (8, 2);
        let origin = e.block_position();
        let mut dy = 0;
        while dy <= vrange {
            for r in 0..range {
                let mut dx = 0;
                while dx <= r {
                    let mut dz = if dx < r && dx > -r { r } else { 0 };
                    while dz <= r {
                        let p = origin.offset(dx, dy - 1, dz);
                        if Self::valid_target(level, p) {
                            self.block = p;
                            return true;
                        }
                        dz = if dz > 0 { -dz } else { 1 - dz };
                    }
                    dx = if dx > 0 { -dx } else { 1 - dx };
                }
            }
            dy = if dy > 0 { -dy } else { 1 - dy };
        }
        false
    }
}

impl CustomGoal for GoToBeach {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DrownedGoToBeachGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = mth::reduced_tick_delay(200 + e.random.next_int_bounded(200));
        self.find(e, level) && !level.is_bright_outside() && e.is_in_water() && e.y() >= (level.sea_level() - 3) as f64
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.try_ticks >= -self.max_stay && self.try_ticks <= 1200 && Self::valid_target(level, self.block)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        st_mut(m).searching_for_land = false;
        let b = self.block;
        path::move_to(e, m, level, b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, 1.0);
        self.try_ticks = 0;
        let inner = e.random.next_int_bounded(1200);
        self.max_stay = e.random.next_int_bounded(inner + 1200) + 1200;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let t = self.block.above();
        let c = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= 1.0 {
            self.try_ticks += 1;
            if self.try_ticks % 40 == 0 {
                path::move_to(e, m, level, t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5, 1.0);
            }
        } else {
            self.try_ticks -= 1;
        }
    }
}

/// `DrownedSwimUpGoal`: at night, deep in the water, toward the surface (no flags).
#[derive(Clone, Debug)]
struct SwimUp {
    stuck: bool,
}

impl SwimUp {
    fn usable(e: &Entity, level: &dyn EntityLevel) -> bool {
        !level.is_bright_outside() && e.is_in_water() && e.y() < (level.sea_level() - 2) as f64
    }
}

impl CustomGoal for SwimUp {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DrownedSwimUpGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        Self::usable(e, level)
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        Self::usable(e, level) && !self.stuck
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).searching_for_land = true;
        self.stuck = false;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).searching_for_land = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let sea = level.sea_level();
        // `closeToNextPos`: within 2 blocks of the path's target.
        let close = m.nav.path.as_ref().is_some_and(|p| {
            let t = p.target;
            e.position().distance_to_sqr(Vec3::new(t.x as f64, t.y as f64, t.z as f64)) < 4.0
        });
        if e.y() < (sea - 1) as f64 && (m.nav.is_done() || close) {
            let towards = Vec3::new(e.x(), (sea - 1) as f64, e.z());
            match random_pos::default_pos_towards(e, m, level, 4, 8, towards, 1.5707963705062866) {
                None => self.stuck = true,
                Some(p) => {
                    path::move_to(e, m, level, p.x, p.y, p.z, 1.0);
                }
            }
        }
    }
}
