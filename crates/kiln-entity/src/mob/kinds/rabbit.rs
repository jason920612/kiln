//! Rabbit: hops (its own jump and move controls: a hop toward each path node, a pause after
//! landing), flees players, wolves and monsters, raids grown carrots, breeds on carrots and
//! dandelions; brown/white/black/splotched/gold/salt coats by biome, and the killer bunny
//! (`EVIL`, type 99) that hunts players and wolves.

use super::common_a::{self, Avoid, AvoidEntityGoal, MoveToBlock, Named};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::control::Operation;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{Goal, Living, Wanted, JUMP, MOVE};
use crate::mob::kinds::wolf::{biome_is, block_in_tag};
use crate::mob::mth;
use crate::mob::{GroupData, MobData, SpawnContext, item_tag};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Rabbit;

pub static KIND: Rabbit = Rabbit;

static INFO: Info = Info::animal("minecraft:rabbit", &[(MaxHealth, 3.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 3.0)]);

/// `Rabbit.Variant` ids.
pub const BROWN: i32 = 0;
pub const WHITE: i32 = 1;
pub const BLACK: i32 = 2;
pub const WHITE_SPLOTCHED: i32 = 3;
pub const GOLD: i32 = 4;
pub const SALT: i32 = 5;
pub const EVIL: i32 = 99;

const EVIL_MODIFIER: &str = "minecraft:evil";

#[derive(Clone, Debug)]
pub struct State {
    pub variant: i32,
    jump_ticks: i32,
    jump_duration: i32,
    was_on_ground: bool,
    jump_delay_ticks: i32,
    pub more_carrot_ticks: i32,
    /// `RabbitJumpControl.canJump`.
    can_jump: bool,
    /// `RabbitMoveControl.nextJumpSpeed`.
    next_jump_speed: f64,
    /// `setWantedPosition` calls seen (see [`crate::mob::control::MoveControl::sets`]).
    seen_sets: u32,
    seen_positive_sets: u32,
    /// The evil goals were added.
    evil_goals: bool,
    /// Hopped this tick (`jumpFromGround` broadcasts entity event 1).
    hopped: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("rabbit state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("rabbit state")
}

pub fn variant(m: &MobData) -> i32 {
    ext::state::<State>(m).map_or(BROWN, |s| s.variant)
}

fn by_id(id: i32) -> i32 {
    if (0..=5).contains(&id) || id == EVIL { id } else { BROWN }
}

/// `setVariant`: the killer bunny gets armor, its attack goals and 5 more attack damage.
fn set_variant(e: &mut Entity, m: &mut MobData, v: i32) {
    let v = by_id(v);
    if v == EVIL {
        if let Some(a) = m.attrs.get_mut(Armor) {
            a.base = 8.0;
        }
        if !st(m).evil_goals {
            st_mut(m).evil_goals = true;
            m.goals.add(4, Goal::Custom(Box::new(common_a::MeleeGoal::new("MeleeAttackGoal", 1.4, true, common_a::plain_attack))));
            m.targets.add(1, common_a::hurt_by(true));
            m.targets.add(2, common_a::nearest(Wanted::Player, 10, true));
            m.targets.add(2, common_a::nearest(Wanted::Types(&["minecraft:wolf"]), 10, true));
        }
        m.attrs.set_modifier(AttackDamage, EVIL_MODIFIER, 5.0, crate::mob::attributes::Op::AddValue);
        if !e.extra.iter().any(|(k, _)| k == "CustomName") {
            e.extra.push(("CustomName".into(), Tag::Compound(vec![("translate".into(), Tag::String("entity.minecraft.killer_bunny".into()))])));
        }
    } else {
        m.attrs.remove_modifier(AttackDamage, EVIL_MODIFIER);
    }
    st_mut(m).variant = v;
}

/// `getRandomRabbitVariant`: one draw from the level's random, the coat by biome.
fn random_variant(biome: Option<i32>, r: &mut dyn RandomSource) -> i32 {
    let v = r.next_int_bounded(100);
    if biome.is_some_and(|b| biome_is(b, "#minecraft:spawns_white_rabbits")) {
        if v < 80 { WHITE } else { WHITE_SPLOTCHED }
    } else if biome.is_some_and(|b| biome_is(b, "#minecraft:spawns_gold_rabbits")) {
        GOLD
    } else if v < 50 {
        BROWN
    } else if v < 90 {
        SALT
    } else {
        BLACK
    }
}

/// `Rabbit.setSpeedModifier`: the navigation's speed and the move control's.
fn set_speed_modifier(e: &Entity, m: &mut MobData, speed: f64) {
    m.nav.speed_modifier = speed;
    let [x, y, z] = m.mov.wanted;
    set_wanted(e, m, x, y, z, speed);
}

/// `RabbitMoveControl.setWantedPosition`: swimming rabbits hurry; a positive speed is the next
/// hop's.
fn set_wanted(e: &Entity, m: &mut MobData, x: f64, y: f64, z: f64, speed: f64) {
    let speed = if e.is_in_water() { 1.5 } else { speed };
    m.mov.set_wanted_position(x, y, z, speed);
    let (sets, positive) = (m.mov.sets, m.mov.positive_sets);
    let s = st_mut(m);
    if speed > 0.0 {
        s.next_jump_speed = speed;
    }
    s.seen_sets = sets;
    s.seen_positive_sets = positive;
}

/// Applies the rabbit's `setWantedPosition` override to calls the shared code made (the
/// navigation following its path).
fn catch_up_wanted(e: &Entity, m: &mut MobData) {
    let (sets, positive, last) = (m.mov.sets, m.mov.positive_sets, m.mov.last_positive_speed);
    let s = st_mut(m);
    if sets == s.seen_sets {
        return;
    }
    if positive != s.seen_positive_sets {
        s.next_jump_speed = last;
    }
    if e.is_in_water() {
        s.next_jump_speed = 1.5;
        m.mov.speed_modifier = 1.5;
    }
    let s = st_mut(m);
    s.seen_sets = sets;
    s.seen_positive_sets = positive;
}

/// `setJumping(true)`: the hop sound.
fn set_jumping(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, jump: bool) {
    m.jumping = jump;
    if jump {
        let pitch = common_a::voice(e) * 0.8;
        common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.rabbit.jump"), 1.0, pitch);
    }
}

fn start_jumping(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    set_jumping(e, m, level, true);
    let s = st_mut(m);
    s.jump_duration = 15;
    s.jump_ticks = 0;
}

fn face_point(e: &mut Entity, x: f64, z: f64) {
    e.y_rot = (mth::atan2(z - e.z(), x - e.x()) * 57.2957763671875) as f32 - 90.0;
}

impl Kind for Rabbit {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        // `idleAnimationTimeout` (client animation) draws in the constructor.
        let _ = random.next_int_bounded(40);
        // The constructor's `setSpeedModifier(0.0)`: a move order to where the control was made.
        m.nav.speed_modifier = 0.0;
        m.mov.set_wanted_position(0.0, 0.0, 0.0, 0.0);
        Some(Box::new(State {
            variant: BROWN,
            jump_ticks: 0,
            jump_duration: 0,
            was_on_ground: false,
            jump_delay_ticks: 0,
            more_carrot_ticks: 0,
            can_jump: false,
            next_jump_speed: 0.0,
            seen_sets: m.mov.sets,
            seen_positive_sets: m.mov.positive_sets,
            evil_goals: false,
            hopped: false,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(1, Goal::Custom(Box::new(ClimbOnTopOfPowderSnowGoal)));
        g.add(1, Named::new("RabbitPanicGoal", common_a::panic(2.2)).after_tick(|g, e, m, _| {
            if let Goal::Panic { speed, .. } = g {
                set_speed_modifier(e, m, *speed);
            }
        }).boxed());
        g.add(2, common_a::breed(0.8));
        g.add(3, common_a::tempt(1.0));
        let not_evil = |m: &MobData, _: &dyn EntityLevel| variant(m) != EVIL;
        g.add(4, Goal::Custom(Box::new(AvoidEntityGoal::new("RabbitAvoidEntityGoal", Avoid::Players, 8.0, 2.2, 2.2).gate(not_evil))));
        g.add(4, Goal::Custom(Box::new(AvoidEntityGoal::new("RabbitAvoidEntityGoal", Avoid::Types(&["minecraft:wolf"]), 10.0, 2.2, 2.2).gate(not_evil))));
        g.add(4, Goal::Custom(Box::new(AvoidEntityGoal::new("RabbitAvoidEntityGoal", Avoid::Monsters, 4.0, 2.2, 2.2).gate(not_evil))));
        g.add(5, Goal::Custom(Box::new(RaidGardenGoal { mtb: MoveToBlock::new(0.7, 16, 1), wants_to_raid: false, can_raid: false })));
        g.add(6, common_a::stroll(0.6));
        g.add(11, common_a::look(10.0));
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        catch_up_wanted(e, m);
        {
            let s = st_mut(m);
            if s.jump_delay_ticks > 0 {
                s.jump_delay_ticks -= 1;
            }
        }
        if st(m).more_carrot_ticks > 0 {
            let d = e.random.next_int_bounded(3);
            let s = st_mut(m);
            s.more_carrot_ticks = (s.more_carrot_ticks - d).max(0);
        }
        if e.on_ground {
            if !st(m).was_on_ground {
                set_jumping(e, m, level, false);
                // `checkLandingDelay`.
                let delay = if m.mov.speed_modifier < 2.2 { 10 } else { 3 };
                let s = st_mut(m);
                s.jump_delay_ticks = delay;
                s.can_jump = false;
            }
            if variant(m) == EVIL
                && st(m).jump_delay_ticks == 0
                && let Some(t) = crate::mob::goals::target(m, level)
                && e.position().distance_to_sqr(t.pos) < 16.0
            {
                face_point(e, t.pos.x, t.pos.z);
                let speed = m.mov.speed_modifier;
                set_wanted(e, m, t.pos.x, t.pos.y, t.pos.z, speed);
                start_jumping(e, m, level);
                st_mut(m).was_on_ground = true;
            }
            if !m.jump.jump {
                if m.mov.has_wanted() && st(m).jump_delay_ticks == 0 {
                    let [x, y, z] = m.mov.wanted;
                    let mut pos = Vec3::new(x, y, z);
                    if let Some(p) = m.nav.path.as_ref().filter(|p| !p.is_done()) {
                        pos = p.next_entity_pos(e.width);
                    }
                    face_point(e, pos.x, pos.z);
                    start_jumping(e, m, level);
                }
            } else if !st(m).can_jump {
                st_mut(m).can_jump = true;
            }
        }
        st_mut(m).was_on_ground = e.on_ground;
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.on_ground && !m.jumping && !m.jump.jump {
            set_speed_modifier(e, m, 0.0);
        } else if m.mov.has_wanted() || m.mov.operation == Operation::Jumping {
            let s = st(m).next_jump_speed;
            set_speed_modifier(e, m, s);
        }
        crate::mob::control::tick_move(e, m, level);
        let (sets, positive) = (m.mov.sets, m.mov.positive_sets);
        let s = st_mut(m);
        s.seen_sets = sets;
        s.seen_positive_sets = positive;
        true
    }

    fn tick_jump(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.jump.jump {
            start_jumping(e, m, level);
            m.jump.jump = false;
        }
        true
    }

    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        // `getJumpPower`.
        let mut base = 0.3f32;
        if m.mov.speed_modifier <= 0.6 {
            base = 0.2;
        }
        if let Some(p) = m.nav.path.as_ref().filter(|p| !p.is_done())
            && p.next_entity_pos(e.width).y > e.y() + 0.5
        {
            base = 0.5;
        }
        if e.horizontal_collision || (m.jumping && m.mov.wanted[1] > e.y() + 0.5) {
            base = 0.5;
        }
        let power = (m.attrs.value(JumpStrength) as f32) * (base / 0.42) * e.block_jump_factor(level);
        if power > 1.0e-5 {
            e.delta = Vec3::new(e.delta.x, (power as f64).max(e.delta.y), e.delta.z);
            e.needs_sync = true;
        }
        if m.mov.speed_modifier > 0.0 {
            let v = e.delta;
            if v.x * v.x + v.z * v.z < 0.01 {
                crate::mob::move_relative(e, 0.1, Vec3::new(0.0, if m.baby() { 0.5 } else { 1.5 }, 1.0));
            }
        }
        // The hop event (1) goes out from `ai_step` (the level is read-only here).
        st_mut(m).hopped = true;
        true
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if std::mem::take(&mut st_mut(m).hopped) {
            level.emit(Event::EntityEvent { entity: e.id, event: 1 });
        }
        let s = st_mut(m);
        if s.jump_ticks != s.jump_duration {
            s.jump_ticks += 1;
        } else if s.jump_duration != 0 {
            s.jump_ticks = 0;
            s.jump_duration = 0;
            m.jumping = false;
        }
    }

    fn after_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _t: &Living) {
        if variant(m) == EVIL {
            let pitch = common_a::voice(e);
            common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.rabbit.attack"), 1.0, pitch);
        }
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:rabbit_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.24, 0.4, 0.39) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        let drawn = random_variant(ctx.biome, r);
        let v = *group.variant.get_or_insert(drawn);
        set_variant(e, m, v);
        // `RabbitGroupData` is an `AgeableMobGroupData(1.0)`: every rabbit after the first is a baby.
        ext::ageable_finalize(e, m, r, group, 1.0);
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(block_in_tag(view.block(pos.below()), "minecraft:rabbits_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        let biome = level.biome(e.block_position());
        let mut v = random_variant(biome, level.random());
        if e.random.next_int_bounded(20) != 0 {
            v = if e.random.next_bool() { variant(partner) } else { variant(m) };
        }
        // `setVariant` on the offspring (an entity of its own: the name goes with its load).
        let mut stub = Entity::new("minecraft:rabbit", 0, 0, crate::entity::EntityKind::MobTicking { gravity: 0.08 }, 0);
        set_variant(&mut stub, child, v);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let v = r.int_or("RabbitType", BROWN);
        set_variant(e, m, v);
        st_mut(m).more_carrot_ticks = r.int_or("MoreCarrotTicks", 0);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("RabbitType", Tag::Int(s.variant));
        o.put("MoreCarrotTicks", Tag::Int(s.more_carrot_ticks));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(data::rabbit::TYPE, &DataValue::Int(st(m).variant));
    }

    fn ambient_sound(&self, _e: &mut Entity, _m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        None
    }
}

/// Whether mob `m` is a killer bunny (for other types' checks).
pub fn is_evil(m: &MobData) -> bool {
    m.kind == crate::mob::MobKind::Rabbit && variant(m) == EVIL
}

/// `ClimbOnTopOfPowderSnowGoal`: jumps out of powder snow.
#[derive(Clone, Debug)]
pub struct ClimbOnTopOfPowderSnowGoal;

impl CustomGoal for ClimbOnTopOfPowderSnowGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ClimbOnTopOfPowderSnowGoal"
    }
    fn flags(&self) -> u8 {
        JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !(e.was_in_powder_snow || e.is_in_powder_snow) {
            return false;
        }
        let above = e.block_position().above();
        let s = level.block(above);
        if crate::blocks::block_name(s) == "minecraft:powder_snow" {
            return true;
        }
        let (shape, _) = crate::collision::collision_shape(s, above, &crate::collision::CollisionContext::EMPTY);
        shape.is_empty()
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.jump.jump = true;
    }
}

/// `RaidGardenGoal`: walks to fully grown carrots and nibbles them down an age.
#[derive(Clone, Debug)]
struct RaidGardenGoal {
    mtb: MoveToBlock,
    wants_to_raid: bool,
    can_raid: bool,
}

fn carrot_age(state: u16) -> Option<i32> {
    let info = kiln_data::blocks_types::block_of(state);
    if info.name != "minecraft:carrots" {
        return None;
    }
    info.property(state, "age").and_then(|a| a.parse().ok())
}

fn with_carrot_age(state: u16, age: i32) -> u16 {
    kiln_data::blocks_types::block_of(state).with_property(state, "age", &age.to_string()).unwrap_or(state)
}

/// `RaidGardenGoal.isValidTarget`: farmland under grown carrots, while the rabbit wants them.
fn valid_garden(level: &dyn EntityLevel, p: BlockPos, wants: bool, can_raid: &mut bool) -> bool {
    if block_in_tag(level.block(p), "minecraft:supports_crops") && wants && !*can_raid && carrot_age(level.block(p.above())) == Some(7) {
        *can_raid = true;
        return true;
    }
    false
}

impl CustomGoal for RaidGardenGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RaidGardenGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.mtb.next_start <= 0 {
            if !level.mob_griefing() {
                return false;
            }
            self.can_raid = false;
            self.wants_to_raid = st(m).more_carrot_ticks <= 0;
        }
        let (wants, mut can_raid) = (self.wants_to_raid, self.can_raid);
        let lv: &dyn EntityLevel = level;
        let r = self.mtb.can_use(e, |p| valid_garden(lv, p, wants, &mut can_raid));
        self.can_raid = can_raid;
        r
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !self.can_raid || !self.mtb.in_time() {
            return false;
        }
        let b = self.mtb.block;
        let mut can_raid = self.can_raid;
        valid_garden(level, b, self.wants_to_raid, &mut can_raid)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.mtb.start(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.mtb.tick(e, m, level);
        let b = self.mtb.block;
        let max_x = m.max_head_x_rot() as f32;
        m.look.set_look_at(b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, 10.0, max_x);
        if !self.mtb.reached {
            return;
        }
        let crop = b.above();
        let state = level.block(crop);
        if self.can_raid && let Some(age) = carrot_age(state) {
            if age == 0 {
                level.set_block(crop, 0, 2);
                level.destroy_block(crop, true);
            } else {
                level.set_block(crop, with_carrot_age(state, age - 1), 2);
                level.emit(Event::GameEvent { event: "minecraft:block_change", pos: Vec3::new(crop.x as f64, crop.y as f64, crop.z as f64), entity: Some(e.id) });
                level.emit(Event::LevelEvent { event: 2001, pos: crop, data: state as i32 });
            }
            st_mut(m).more_carrot_ticks = 40;
        }
        self.can_raid = false;
        self.mtb.next_start = 10;
    }
}
