//! Turtle: remembers its home beach, swims far out (`TurtleTravelGoal`) and comes back to lay
//! eggs on sand after breeding (seagrass), walks slowly on land and swims with its own move
//! control; babies grow up with a scute (`gameplay/turtle_grow`). Lightning kills it outright.

use super::common_a::{self, MoveToBlock, Named};
use crate::custom_goal_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::control::{Operation, rotlerp, set_speed};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, MOVE, JUMP};
use crate::mob::interact::{Interactor, Outcome};
use crate::mob::kinds::wolf::block_in_tag;
use crate::mob::path::{self, PathType};
use crate::mob::{GroupData, MobData, SpawnContext, item_tag, mth, random_pos};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Turtle;

pub static KIND: Turtle = Turtle;

static INFO: Info = Info {
    ambient_interval: 200,
    ..Info::animal("minecraft:turtle", &[(MaxHealth, 30.0), (MovementSpeed, 0.25), (StepHeight, 1.0)])
};

#[derive(Clone, Debug)]
pub struct State {
    pub home: BlockPos,
    pub travel_pos: Option<BlockPos>,
    pub going_home: bool,
    pub has_egg: bool,
    pub laying_egg: bool,
    lay_egg_counter: i32,
    /// Was a baby at the last `aiStep` (`ageBoundaryReached` drops the scute).
    was_baby: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("turtle state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("turtle state")
}

/// `TurtleEggBlock.isSand`.
pub fn is_sand(level: &dyn EntityLevel, p: BlockPos) -> bool {
    block_in_tag(level.block(p), "minecraft:sand")
}

/// `TurtleEggBlock.onSand`.
pub fn on_sand(level: &dyn EntityLevel, p: BlockPos) -> bool {
    is_sand(level, p.below())
}

fn is_water_block(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::blocks::block_name(level.block(p)) == "minecraft:water"
}

/// `BlockPos.closerToCenterThan(pos, d)`.
fn closer_to_center(b: BlockPos, p: Vec3, d: f64) -> bool {
    Vec3::new(b.x as f64 + 0.5, b.y as f64 + 0.5, b.z as f64 + 0.5).distance_to_sqr(p) < d * d
}

/// `BlockPos.withinBoxByManhattanDistance(center, rx, ry, rz)`: positions by increasing
/// Manhattan distance, x then y ascending, `+z` before `-z`.
pub fn within_manhattan(c: BlockPos, rx: i32, ry: i32, rz: i32) -> impl Iterator<Item = BlockPos> {
    (0..=rx + ry + rz).flat_map(move |depth| {
        let mx = rx.min(depth);
        (-mx..=mx).flat_map(move |x| {
            let my = ry.min(depth - x.abs());
            (-my..=my).flat_map(move |y| {
                let z = depth - x.abs() - y.abs();
                let at = move |z| c.offset(x, y, z);
                let first = (z <= rz).then(|| at(z));
                let mirror = (z <= rz && z != 0).then(|| at(-z));
                first.into_iter().chain(mirror)
            })
        })
    })
}

/// `PanicGoal.lookForWater(level, mob, xz)`: the nearest water within the Manhattan box (one
/// up and down), unless the mob stands in a block with a collision shape.
pub fn look_for_water(e: &Entity, level: &dyn EntityLevel, xz: i32) -> Option<BlockPos> {
    let p = e.block_position();
    let (shape, _) = crate::collision::collision_shape(level.block(p), p, &crate::collision::CollisionContext::EMPTY);
    if !shape.is_empty() {
        return None;
    }
    within_manhattan(p, xz, 1, xz).find(|q| crate::physics::fluid_state(level.block(*q)).kind.is_water())
}

impl Kind for Turtle {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        super::tame::set_malus(m, PathType::Water, 0.0);
        super::tame::set_malus(m, PathType::DoorIronClosed, -1.0);
        super::tame::set_malus(m, PathType::DoorWoodClosed, -1.0);
        super::tame::set_malus(m, PathType::DoorOpen, -1.0);
        m.nav.amphibious = true;
        Some(Box::new(State { home: BlockPos::default(), travel_pos: None, going_home: false, has_egg: false, laying_egg: false, lay_egg_counter: 0, was_baby: false }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(TurtlePanicGoal { speed: 1.2, pos: Vec3::ZERO })));
        g.add(1, Goal::Custom(Box::new(TurtleBreedGoal { inner: common_a::breed(1.0) })));
        g.add(1, Goal::Custom(Box::new(TurtleLayEggGoal { mtb: MoveToBlock::new(1.0, 16, 1) })));
        g.add(2, common_a::tempt(1.1));
        let mut water = MoveToBlock::new(1.0, 24, 1);
        water.vstart = -1;
        g.add(3, Goal::Custom(Box::new(TurtleGoToWaterGoal { mtb: water })));
        g.add(4, Goal::Custom(Box::new(TurtleGoHomeGoal { speed: 1.0, stuck: false, close_ticks: 0 })));
        g.add(7, Goal::Custom(Box::new(TurtleTravelGoal { speed: 1.0, stuck: false })));
        g.add(8, common_a::look(8.0));
        g.add(
            9,
            Named::new("TurtleRandomStrollGoal", Goal::RandomStroll { speed: 1.0, interval: 100, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false })
                .gate(|e, m, _| !e.is_in_water() && !st(m).going_home && !st(m).has_egg)
                .boxed(),
        );
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let baby = m.baby();
        let s = st(m);
        if crate::mob::is_alive(e, m) && s.laying_egg && s.lay_egg_counter >= 1 && s.lay_egg_counter % 5 == 0 {
            let p = e.block_position();
            if on_sand(level, p) {
                let below = level.block(p.below());
                level.emit(Event::LevelEvent { event: 2001, pos: p, data: below as i32 });
                level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
            }
        }
        // `ageBoundaryReached`: grown up, the scute.
        if st(m).was_baby && !baby && level.mob_drops() {
            level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/turtle_grow", pos: e.position() });
        }
        st_mut(m).was_baby = baby;
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        // `updateSpeed`.
        if e.is_in_water() {
            e.delta = e.delta.add(0.0, 0.005, 0.0);
            if !closer_to_center(st(m).home, e.position(), 16.0) {
                set_speed(m, (m.speed / 2.0).max(0.08));
            }
            if m.baby() {
                set_speed(m, (m.speed / 3.0).max(0.06));
            }
        } else if e.on_ground {
            set_speed(m, (m.speed / 2.0).max(0.06));
        }
        if m.mov.operation == Operation::MoveTo && !m.nav.is_done() {
            let [wx, wy, wz] = m.mov.wanted;
            let (xd, mut yd, zd) = (wx - e.x(), wy - e.y(), wz - e.z());
            let dd = (xd * xd + yd * yd + zd * zd).sqrt();
            if dd < 1.0e-5f32 as f64 {
                set_speed(m, 0.0);
            } else {
                yd /= dd;
                let yaw = (mth::atan2(zd, xd) * 57.2957763671875) as f32 - 90.0;
                e.y_rot = rotlerp(e.y_rot, yaw, 90.0);
                m.y_body_rot = e.y_rot;
                let target = (m.mov.speed_modifier * m.attrs.value(MovementSpeed)) as f32;
                let s = mth::lerp_f(0.125, m.speed, target);
                set_speed(m, s);
                e.delta = e.delta.add(0.0, m.speed as f64 * yd * 0.1, 0.0);
            }
        } else {
            set_speed(m, 0.0);
        }
        true
    }

    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        crate::mob::move_relative(e, 0.1, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        let s = st(m);
        if m.target.is_none() && (!s.going_home || !closer_to_center(s.home, e.position(), 20.0)) {
            e.delta = e.delta.add(0.0, -0.005, 0.0);
        }
        true
    }

    fn stable_destination_for(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<bool> {
        Some(if st(m).travel_pos.is_some() { is_water_block(level, p) } else { !kiln_data::blocks_types::is_air(level.block(p.below())) })
    }

    fn walk_target_value(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        if !st(m).going_home && crate::physics::fluid_state(level.block(p)).kind.is_water() {
            return Some(10.0);
        }
        Some(if on_sand(level, p) { 10.0 } else { crate::mob::light_magic_value_at(level, p) - 0.5 })
    }

    fn ambient_sound(&self, e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some((!e.is_in_water() && e.on_ground && !m.baby()).then(|| crate::mob::sound_event("minecraft:entity.turtle.ambient_land")))
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:turtle_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    fn interact(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `canFallInLove`: not while carrying an egg.
        (st(m).has_egg && !stack.is_empty() && self.is_food(stack.item()) && m.age == 0).then_some(Outcome::PASS)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (base.0 * 0.3, base.1 * 0.3, base.2 * 0.3) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        st_mut(m).home = e.block_position();
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(pos.y < view.sea_level() + 4 && block_in_tag(view.block(pos.below()), "minecraft:sand") && view.raw_brightness(pos, 0) > 8)
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let home = match r.get("home_pos") {
            Some(Tag::IntArray(v)) if v.len() == 3 => BlockPos::new(v[0], v[1], v[2]),
            _ => e.block_position(),
        };
        let s = st_mut(m);
        s.home = home;
        s.has_egg = r.bool_or("has_egg", false);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("home_pos", Tag::IntArray(vec![s.home.x, s.home.y, s.home.z]));
        o.put("has_egg", Tag::Byte(s.has_egg as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::turtle::HAS_EGG, &DataValue::Boolean(s.has_egg));
        d.set(data::turtle::LAYING_EGG, &DataValue::Boolean(s.laying_egg));
    }
}

/// `setHomePos` for a turtle made elsewhere (hatchlings: the egg's position).
pub fn set_home(m: &mut MobData, home: BlockPos) {
    if let Some(s) = ext::state_mut::<State>(m) {
        s.home = home;
    }
}

fn set_laying_egg(m: &mut MobData, on: bool) {
    let s = st_mut(m);
    s.lay_egg_counter = on as i32;
    s.laying_egg = on;
}

/// `TurtlePanicGoal`: runs for water within seven blocks first.
#[derive(Clone, Debug)]
struct TurtlePanicGoal {
    speed: f64,
    pos: Vec3,
}

impl CustomGoal for TurtlePanicGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtlePanicGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !goals::should_panic(m, level) {
            return false;
        }
        if let Some(p) = look_for_water(e, level, 7) {
            self.pos = Vec3::new(p.x as f64, p.y as f64, p.z as f64);
            return true;
        }
        match random_pos::default_pos(e, m, level, 5, 4) {
            Some(p) => {
                self.pos = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.pos.x, self.pos.y, self.pos.z, self.speed);
    }
}

/// `TurtleBreedGoal`: instead of a baby, the turtle gets an egg to lay at home.
#[derive(Clone, Debug)]
struct TurtleBreedGoal {
    inner: Goal,
}

impl CustomGoal for TurtleBreedGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtleBreedGoal"
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_use(&mut self.inner, e, m, level) && !st(m).has_egg
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Goal::Breed { speed, partner, love_time } = &mut self.inner else { return };
        let Some(p) = partner.and_then(|id| goals::living(level, id)) else { return };
        let max_x = m.max_head_x_rot() as f32;
        m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, 10.0, max_x);
        path::move_to_entity(e, m, level, BlockPos::containing(p.pos.x, p.pos.y, p.pos.z), *speed);
        *love_time += 1;
        if *love_time >= mth::reduced_tick_delay(60) && e.position().distance_to_sqr(p.pos) < 9.0 {
            breed(e, m, level, p.id);
        }
    }
}

/// `TurtleBreedGoal.breed`.
fn breed(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, partner: i32) {
    let cause = m.love_cause.or_else(|| level.entity(partner).and_then(crate::mob::data).and_then(|p| p.love_cause));
    if let Some(player) = cause.filter(|&c| level.player(c).is_some())
        && let Some(partner_seen) = level.entity(partner).map(crate::level::Seen::of)
    {
        let criterion = crate::level::Criterion::BredAnimals { parent: crate::level::Seen::of_mob(e, m), partner: partner_seen, child: None };
        level.emit(Event::Criterion { player, criterion });
    }
    st_mut(m).has_egg = true;
    crate::mob::set_age(e, m, 6000);
    m.in_love = 0;
    if let Some(o) = level.entity_mut(partner) {
        let mut pm = std::mem::replace(&mut o.kind, crate::entity::EntityKind::MobTicking { gravity: 0.08 });
        if let crate::entity::EntityKind::Mob(pmd) = &mut pm {
            crate::mob::set_age(o, pmd, 6000);
            pmd.in_love = 0;
        }
        o.kind = pm;
    }
    let xp = e.random.next_int_bounded(7) + 1;
    if level.mob_drops() {
        crate::mob::award_experience(level, e.position(), xp);
    }
}

/// `TurtleLayEggGoal`: at home with an egg, digs into sand and lays one to four eggs.
#[derive(Clone, Debug)]
struct TurtleLayEggGoal {
    mtb: MoveToBlock,
}

fn valid_nest(level: &dyn EntityLevel, p: BlockPos) -> bool {
    kiln_data::blocks_types::is_air(level.block(p.above())) && is_sand(level, p)
}

impl CustomGoal for TurtleLayEggGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtleLayEggGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        if !(s.has_egg && closer_to_center(s.home, e.position(), 9.0)) {
            return false;
        }
        let lv: &dyn EntityLevel = level;
        self.mtb.can_use(e, |p| valid_nest(lv, p))
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        self.mtb.in_time() && valid_nest(level, self.mtb.block) && s.has_egg && closer_to_center(s.home, e.position(), 9.0)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.mtb.start(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.mtb.tick(e, m, level);
        let here = e.block_position();
        if e.is_in_water() || !self.mtb.reached {
            return;
        }
        if st(m).lay_egg_counter < 1 {
            set_laying_egg(m, true);
        } else if st(m).lay_egg_counter > 200 {
            let pitch = 0.9 + level.random().next_float() * 0.2;
            let c = Vec3::new(here.x as f64 + 0.5, here.y as f64 + 0.5, here.z as f64 + 0.5);
            level.emit(Event::Sound { pos: c, sound: "minecraft:entity.turtle.lay_egg", source: "blocks", volume: 0.3, pitch });
            let egg_pos = self.mtb.block.above();
            let eggs = e.random.next_int_bounded(4) + 1;
            let egg = kiln_data::blocks_types::block_by_name("minecraft:turtle_egg").map(|b| b.default).unwrap_or(0);
            let state = kiln_data::blocks_types::block_of(egg).with_property(egg, "eggs", &eggs.to_string()).unwrap_or(egg);
            level.set_block(egg_pos, state, 3);
            // `sendBlockUpdated`: the egg's shape changes paths through it (the turtle's own).
            path::on_block_changed(e, m, level, egg_pos);
            level.emit(Event::GameEvent { event: "minecraft:block_place", pos: Vec3::new(egg_pos.x as f64, egg_pos.y as f64, egg_pos.z as f64), entity: Some(e.id) });
            st_mut(m).has_egg = false;
            set_laying_egg(m, false);
            m.in_love = 600;
        }
        if st(m).laying_egg {
            st_mut(m).lay_egg_counter += 1;
        }
    }
}

/// `TurtleGoToWaterGoal`: back to water when on land (babies always).
#[derive(Clone, Debug)]
struct TurtleGoToWaterGoal {
    mtb: MoveToBlock,
}

impl CustomGoal for TurtleGoToWaterGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtleGoToWaterGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        let ok = if m.baby() && !e.is_in_water() { true } else { !s.going_home && !e.is_in_water() && !s.has_egg };
        if !ok {
            return false;
        }
        let lv: &dyn EntityLevel = level;
        self.mtb.can_use(e, |p| is_water_block(lv, p))
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        !e.is_in_water() && self.mtb.try_ticks <= 1200 && is_water_block(level, self.mtb.block)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.mtb.start(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `MoveToBlockGoal.tick` with `shouldRecalculatePath` every 160 ticks.
        let t = self.mtb.block.above();
        let c = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= 1.0 {
            self.mtb.reached = false;
            self.mtb.try_ticks += 1;
            if self.mtb.try_ticks % 160 == 0 {
                path::move_to(e, m, level, t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5, self.mtb.speed);
            }
        } else {
            self.mtb.reached = true;
            self.mtb.try_ticks -= 1;
        }
    }
}

/// `TurtleGoHomeGoal`: with an egg (or now and then when more than 64 blocks away), swims
/// and walks back toward the home beach.
#[derive(Clone, Debug)]
struct TurtleGoHomeGoal {
    speed: f64,
    stuck: bool,
    close_ticks: i32,
}

impl CustomGoal for TurtleGoHomeGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtleGoHomeGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if m.baby() {
            return false;
        }
        if st(m).has_egg {
            return true;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(700)) != 0 {
            return false;
        }
        !closer_to_center(st(m).home, e.position(), 64.0)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).going_home = true;
        self.stuck = false;
        self.close_ticks = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).going_home = false;
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !closer_to_center(st(m).home, e.position(), 7.0) && !self.stuck && self.close_ticks <= mth::reduced_tick_delay(600)
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let home = st(m).home;
        let close = closer_to_center(home, e.position(), 16.0);
        if close {
            self.close_ticks += 1;
        }
        if !m.nav.is_done() {
            return;
        }
        let target = Vec3::new(home.x as f64 + 0.5, home.y as f64, home.z as f64 + 0.5);
        let mut next = random_pos::default_pos_towards(e, m, level, 16, 3, target, std::f32::consts::PI as f64 / 10.0);
        if next.is_none() {
            next = random_pos::default_pos_towards(e, m, level, 8, 7, target, std::f32::consts::FRAC_PI_2 as f64);
        }
        if let Some(p) = next
            && !close
            && !is_water_block(level, BlockPos::containing(p.x, p.y, p.z))
        {
            next = random_pos::default_pos_towards(e, m, level, 16, 5, target, std::f32::consts::FRAC_PI_2 as f64);
        }
        let Some(p) = next else {
            self.stuck = true;
            return;
        };
        path::move_to(e, m, level, p.x, p.y, p.z, self.speed);
    }
}

/// `TurtleTravelGoal`: in water, heads for a random point up to 512 blocks away.
#[derive(Clone, Debug)]
struct TurtleTravelGoal {
    speed: f64,
    stuck: bool,
}

impl CustomGoal for TurtleTravelGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TurtleTravelGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        !s.going_home && !s.has_egg && e.is_in_water()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let xt = e.random.next_int_bounded(1025) - 512;
        let mut yt = e.random.next_int_bounded(9) - 4;
        let zt = e.random.next_int_bounded(1025) - 512;
        if yt as f64 + e.y() > (level.sea_level() - 1) as f64 {
            yt = 0;
        }
        st_mut(m).travel_pos = Some(BlockPos::containing(xt as f64 + e.x(), yt as f64 + e.y(), zt as f64 + e.z()));
        self.stuck = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(tp) = st(m).travel_pos else {
            self.stuck = true;
            return;
        };
        if !m.nav.is_done() {
            return;
        }
        let target = Vec3::new(tp.x as f64 + 0.5, tp.y as f64, tp.z as f64 + 0.5);
        let mut next = random_pos::default_pos_towards(e, m, level, 16, 3, target, std::f32::consts::PI as f64 / 10.0);
        if next.is_none() {
            next = random_pos::default_pos_towards(e, m, level, 8, 7, target, std::f32::consts::FRAC_PI_2 as f64);
        }
        if let Some(p) = next {
            let (xc, zc) = (crate::math::floor(p.x), crate::math::floor(p.z));
            let loaded = [(xc - 34, zc - 34), (xc + 34, zc - 34), (xc - 34, zc + 34), (xc + 34, zc + 34)].iter().all(|&(x, z)| level.is_loaded(BlockPos::new(x, 0, z)));
            if !loaded {
                next = None;
            }
        }
        let Some(p) = next else {
            self.stuck = true;
            return;
        };
        path::move_to(e, m, level, p.x, p.y, p.z, self.speed);
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        !m.nav.is_done() && !self.stuck && !s.going_home && m.in_love <= 0 && !s.has_egg
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).travel_pos = None;
    }
}
