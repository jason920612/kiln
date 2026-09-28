//! Strider: walks on lava sources, heads for lava when out of it and shivers (slower, the
//! suffocating flag) away from warm blocks, is hurt by water; saddled and ridden, steered with
//! a warped fungus on a stick. Not modelled: the boost from using the fungus on a stick, the
//! zombified piglin and baby jockeys at spawn (their rolls are kept, the riders are not made).

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::{Attr::*, Op};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{DamageSource, GroupData, MobData, SpawnContext, item_name, item_tag, path, random_pos};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Strider;

pub static KIND: Strider = Strider;

static INFO: Info = Info {
    fire_immune: true,
    ..Info::animal("minecraft:strider", &[(MovementSpeed, 0.17499999701976776)])
};

#[derive(Clone, Debug, Default)]
pub struct State {
    pub suffocating: bool,
    pub saddle: ItemStack,
    /// `DATA_BOOST_TIME`.
    pub boost_time: i32,
    /// `isInLava` as of this tick's base tick (for the walk target values).
    in_lava: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("strider state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("strider state")
}

fn tempted(m: &MobData) -> bool {
    m.goals.is_running(|g| matches!(g, Goal::Tempt { .. }))
}

fn panicking(m: &MobData) -> bool {
    m.goals.is_running(|g| matches!(g, Goal::Panic { .. }))
}

fn is_lava(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::blocks::block_name(level.block(p)) == "minecraft:lava"
}

/// `LivingEntity.makeSound` (the voice pitch draws twice).
fn make_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel, sound: &'static str) {
    let d = (e.random.next_float() - e.random.next_float()) * 0.2;
    let pitch = if m.baby() { d + 1.5 } else { d + 1.0 };
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch });
    }
}

impl Kind for Strider {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        super::tame::set_malus(m, path::PathType::Water, -1.0);
        super::tame::set_malus(m, path::PathType::Lava, 0.0);
        super::tame::set_malus(m, path::PathType::FireInNeighbor, 0.0);
        super::tame::set_malus(m, path::PathType::Fire, 0.0);
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Panic { speed: 1.65, pos: Vec3::ZERO });
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(3, Goal::Tempt { speed: 1.4, calm_down: 0, player: None });
        g.add(4, Goal::Custom(Box::new(StriderGoToLavaGoal { next_start: 0, try_ticks: 0, max_stay: 0, block: BlockPos::default() })));
        g.add(5, Goal::FollowParent { speed: 1.0, parent: None, recalc: 0 });
        g.add(7, Goal::Custom(Box::new(RandomStrollGoal { speed: 1.0, interval: 60, wanted: Vec3::ZERO })));
        g.add(8, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        g.add(9, Goal::Custom(Box::new(LookAtStriderGoal { look_at: None, look_time: 0 })));
    }

    fn tempted_by(&self, item: i32) -> bool {
        item_tag(item, "minecraft:strider_tempt_items")
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:strider_food")
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        e.stands_on_lava = true;
        if tempted(m) && e.random.next_int_bounded(140) == 0 {
            make_sound(e, m, level, "minecraft:entity.strider.happy");
        } else if panicking(m) && e.random.next_int_bounded(60) == 0 {
            make_sound(e, m, level, "minecraft:entity.strider.retreat");
        }
        if !m.no_ai {
            let warm_tag = |s: u16| super::wolf::block_in_tag(s, "minecraft:strider_warm_blocks");
            let here = level.block(e.block_position());
            let on = level.block(e.on_pos_legacy(level));
            let warm = warm_tag(here) || warm_tag(on) || e.fluid_height_lava() > 0.0;
            let riding_warm = e.vehicle.and_then(|v| level.entity(v)).and_then(crate::mob::data).is_some_and(|vm| ext::state::<State>(vm).is_some_and(|s| !s.suffocating));
            let suffocating = !warm && !riding_warm;
            st_mut(m).suffocating = suffocating;
            if suffocating {
                m.attrs.set_modifier(MovementSpeed, "minecraft:suffocating", -0.3400000035762787, Op::AddMultipliedBase);
            } else {
                m.attrs.remove_modifier(MovementSpeed, "minecraft:suffocating");
            }
        }
    }

    fn post_tick(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        // `floatStrider`.
        if e.is_in_lava() {
            let p = e.block_position();
            let above_shape = e.y() > p.y as f64 + 0.5 - 9.999999747378752e-6;
            if above_shape && !crate::physics::fluid_state(level.block(p.above())).kind.is_lava() {
                e.on_ground = true;
            } else {
                e.delta = e.delta.scale(0.5).add(0.0, 0.05, 0.0);
            }
        }
    }

    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        // `shouldTravelInFluid` is false on the lava it stands on: it moves as on land.
        if (e.is_in_water() || e.is_in_lava()) && crate::physics::fluid_state(level.block(e.block_position())).kind.is_lava() {
            crate::mob::travel_in_air(e, m, level, input);
            return true;
        }
        false
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).in_lava = e.is_in_lava();
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `isSensitiveToWater`: water and rain hurt.
        if crate::mob::is_alive(e, m) && (e.is_in_water() || level.is_raining_at(e.block_position())) {
            crate::mob::hurt(e, m, level, DamageSource::of(DamageKind::Drown), 1.0);
        }
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if panicking(m) || tempted(m) {
            return Some(None);
        }
        None
    }

    fn walk_target_value(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        if crate::physics::fluid_state(level.block(p)).kind.is_lava() {
            return Some(10.0);
        }
        Some(if st(m).in_lava { f32::NEG_INFINITY } else { 0.0 })
    }

    fn stable_destination(&self, level: &dyn EntityLevel, p: BlockPos) -> Option<bool> {
        Some(is_lava(level, p) || path::is_stable_destination(level, p))
    }

    fn steerable_by(&self, m: &MobData, rider: &PlayerView) -> bool {
        let fungus = kiln_data::builtin_id("minecraft:item", "minecraft:warped_fungus_on_a_stick");
        !st(m).saddle.is_empty() && (Some(rider.main_hand) == fungus || Some(rider.off_hand) == fungus)
    }

    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &PlayerView) {
        e.y_rot = rider.yaw % 360.0;
        e.x_rot = (rider.pitch * 0.5) % 360.0;
        e.y_rot_o = e.y_rot;
        m.y_body_rot = e.y_rot;
        m.y_head_rot = e.y_rot;
    }

    fn passenger_offset(&self, e: &Entity, m: &MobData) -> Option<Vec3> {
        let h = if m.baby() { 0.65625 } else { e.height as f64 };
        Some(Vec3::new(0.0, h, 0.0))
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.45, 0.85, 0.4375) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        if !m.baby() {
            // Jockeys: the rolls happen, the riders are not simulated. Every strider gets group
            // data of its own, so none turns into a baby.
            if r.next_int_bounded(30) == 0 {
                let _ = r.next_float();
            } else {
                let _ = r.next_int_bounded(10);
            }
        }
        let _ = (e, group);
        ext::mob_finalize(m, r);
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let food = !stack.is_empty() && self.is_food(stack.item());
        if !food && !st(m).saddle.is_empty() && e.passengers.is_empty() && !who.sneaking {
            let mut out = Outcome::success(HeldChange::None);
            out.ride = true;
            return Some(out);
        }
        let out = crate::mob::interact::animal_interact(e, m, level, who, stack);
        if !out.success {
            if !stack.is_empty() && item_name(stack) == "minecraft:saddle" && st(m).saddle.is_empty() && !m.baby() && crate::mob::is_alive(e, m) {
                let mut one = stack.clone();
                one.set_count(1);
                st_mut(m).saddle = one;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.strider.saddle", source: "neutral", volume: 0.5, pitch: 1.0 });
                }
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            return Some(Outcome::PASS);
        }
        if food && !e.silent {
            let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.strider.eat", source: "neutral", volume: 1.0, pitch });
        }
        Some(out)
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        if s.saddle.is_empty() { Vec::new() } else { vec![(7, s.saddle.clone())] }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        if let Some(Tag::Compound(eq)) = r.get("equipment")
            && let Some(sd) = eq.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| ItemStack::from_nbt(v).ok())
        {
            st_mut(m).saddle = sd;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        if !s.saddle.is_empty() {
            let entry = ("saddle".to_owned(), s.saddle.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::strider::BOOST_TIME, &DataValue::Int(s.boost_time));
        d.set(data::strider::SUFFOCATING, &DataValue::Boolean(s.suffocating));
    }
}

/// `Strider.StriderGoToLavaGoal`: a `MoveToBlockGoal` (range 8, 2 up and down) for lava.
#[derive(Clone, Debug)]
struct StriderGoToLavaGoal {
    next_start: i32,
    try_ticks: i32,
    max_stay: i32,
    block: BlockPos,
}

impl StriderGoToLavaGoal {
    fn valid(level: &dyn EntityLevel, p: BlockPos) -> bool {
        is_lava(level, p) && path::pathfindable_land(level.block(p.above()))
    }

    fn find(&mut self, e: &Entity, level: &dyn EntityLevel) -> bool {
        let (range, vrange) = (8, 2);
        let o = e.block_position();
        let mut dy = 0;
        while dy <= vrange {
            for r in 0..range {
                let mut dx = 0;
                while dx <= r {
                    let mut dz = if dx < r && dx > -r { r } else { 0 };
                    while dz <= r {
                        let p = o.offset(dx, dy - 1, dz);
                        if Self::valid(level, p) {
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

impl CustomGoal for StriderGoToLavaGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "StriderGoToLavaGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.is_in_lava() {
            return false;
        }
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = reduced_tick_delay(200 + e.random.next_int_bounded(200));
        self.find(e, level)
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        !e.is_in_lava() && Self::valid(level, self.block)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let b = self.block;
        path::move_to(e, m, level, b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, 1.0);
        self.try_ticks = 0;
        let inner = e.random.next_int_bounded(1200);
        self.max_stay = e.random.next_int_bounded(inner + 1200) + 1200;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let t = self.block;
        let c = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= 1.0 {
            self.try_ticks += 1;
            if self.try_ticks % 20 == 0 {
                path::move_to(e, m, level, t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5, 1.0);
            }
        } else {
            self.try_ticks -= 1;
        }
    }
}

/// `RandomStrollGoal` (not avoiding water): `DefaultRandomPos` within 10 blocks.
#[derive(Clone, Debug)]
struct RandomStrollGoal {
    speed: f64,
    interval: i32,
    wanted: Vec3,
}

impl CustomGoal for RandomStrollGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RandomStrollGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.no_action_time >= 100 {
            return false;
        }
        if e.random.next_int_bounded(reduced_tick_delay(self.interval)) != 0 {
            return false;
        }
        match random_pos::default_pos(e, m, level, 10, 7) {
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
        path::move_to(e, m, level, self.wanted.x, self.wanted.y, self.wanted.z, self.speed);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// `LookAtPlayerGoal(Strider.class, 8)`: now and then looks at another strider.
#[derive(Clone, Debug)]
struct LookAtStriderGoal {
    look_at: Option<i32>,
    look_time: i32,
}

impl CustomGoal for LookAtStriderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_float() >= 0.02 {
            return false;
        }
        if let Some(t) = m.target {
            self.look_at = Some(t);
        }
        let area = e.bounding_box().inflate(8.0, 3.0, 8.0);
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
            let Some(t) = goals::living(level, id) else { continue };
            if t.type_name != "minecraft:strider" || !goals::targeting_ok(e, m, level, &t, false, 8.0, true) {
                continue;
            }
            let d = t.pos.distance_to_sqr(Vec3::new(e.x(), e.eye_y(), e.z()));
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.look_at = best.map(|(_, id)| id);
        self.look_at.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return false };
        t.alive && e.position().distance_to_sqr(t.pos) <= 64.0 && self.look_time > 0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time = reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_at = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return };
        if !t.alive {
            return;
        }
        crate::mob::control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
        self.look_time -= 1;
    }
}
