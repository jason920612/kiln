//! Tadpole: a fish that hatches from frogspawn, swims about (water-bound navigation and the
//! smooth swimming controls), follows slime balls, and grows into a frog after 24000 ticks (or
//! sooner when fed slime balls). A water bucket scoops it up with its age; a golden dandelion
//! keeps it small.
//!
//! Driven by the brain of `TadpoleAi` (core: panic, look and move sinks, temptation cooldown;
//! idle: look at players, follow a slime ball, swim), next to the goals `AbstractFish` registers
//! (panic, avoiding players, `FishSwimGoal`), as in vanilla.

use crate::entity::{Entity, MoverType};
use crate::level::{EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::brain::amphibian;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::sensors;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Gate, Mem, Sensor, Status, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::Goal;
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::path::PathType;
use crate::mob::{self, Category, GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Tadpole;

pub static KIND: Tadpole = Tadpole;

/// `Tadpole.ticksToBeFrog`.
const TICKS_TO_BE_FROG: i32 = 24000;

static INFO: Info = Info {
    category: Category::Creature,
    breathes_under_water: true,
    ambient_interval: 120,
    ..Info::misc("minecraft:tadpole", &[(MaxHealth, 6.0), (MovementSpeed, 1.0), (TemptRange, 10.0)])
};

#[derive(Clone, Debug)]
pub struct State {
    /// `Tadpole.age` (its own, not `AgeableMob`'s).
    pub age: i32,
    pub age_lock_particle_timer: i32,
    /// `AGE_LOCKED`.
    pub age_locked: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("tadpole state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("tadpole state")
}

fn is_food(item: i32) -> bool {
    mob::item_tag(item, "minecraft:frog_food")
}

/// `Tadpole.setAge`: reaching 24000 turns it into a frog.
fn set_age(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, age: i32) {
    st_mut(m).age = age;
    if age >= TICKS_TO_BE_FROG {
        grow_up(e, m, level);
    }
}

/// `Tadpole.ageUp()`: `convertTo(FROG)` with the frog's `finalizeSpawn` (biome variant).
fn grow_up(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let silent = e.silent;
    let old_dims = (e.width, e.height, e.eye_height);
    mob::convert::convert_to(e, m, level, MobKind::Frog, false, false, |ne, nm, level| {
        let pos = ne.block_position();
        let eff = level.effective_difficulty(pos);
        let ctx = SpawnContext {
            biome: level.biome(pos),
            moon_brightness: 1.0,
            special_multiplier: super::zombie::special_multiplier(eff),
            effective_difficulty: eff,
            hard: false,
            halloween: false,
        };
        super::frog::KIND.finalize_spawn(ne, nm, level.random(), &ctx, &mut GroupData::default());
        nm.persistence_required = true;
        // `fudgePositionAfterSizeChange(tadpole dimensions)`: the frog is bigger.
        let (w, h, eye) = (ne.width, ne.height, ne.eye_height);
        (ne.width, ne.height, ne.eye_height) = old_dims;
        let first = std::mem::replace(&mut ne.first_tick, false);
        mob::refresh_dimensions_in(ne, nm, level);
        ne.first_tick = first;
        if ne.width != w || ne.height != h {
            (ne.width, ne.height, ne.eye_height) = (w, h, eye);
            let p = ne.position();
            ne.set_pos(p);
        }
        if !silent {
            level.emit(Event::Sound { pos: ne.position(), sound: "minecraft:entity.tadpole.grow_up", source: "neutral", volume: 0.15, pitch: 1.0 });
        }
    });
}

impl Kind for Tadpole {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// `WaterAnimal` (water costs nothing) and `Tadpole`'s constructor (water-bound navigation).
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        super::tame::set_malus(m, PathType::Water, 0.0);
        m.nav.water_bound = true;
        Some(Box::new(State { age: 0, age_lock_particle_timer: 0, age_locked: false }))
    }

    /// `AbstractFish.registerGoals`: the goals run beside the brain.
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Panic { speed: 1.25, pos: Vec3::ZERO });
        g.add(2, Goal::Custom(Box::new(super::fish::AvoidPlayerGoal::new("AvoidEntityGoal", 8.0, 1.6, 1.4, None))));
        g.add(4, Goal::Custom(Box::new(super::fish::RandomSwimmingGoal { name: "FishSwimGoal", speed: 1.0, interval: 40, wanted: Vec3::ZERO })));
    }

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn is_food(&self, item: i32) -> bool {
        is_food(item)
    }

    /// `Tadpole.customServerAiStep`: the brain, then `TadpoleAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Idle]);
        }
    }

    /// `AbstractFish.aiStep` before `LivingEntity.aiStep`: flopping on land.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !e.is_in_water() && e.on_ground && e.vertical_collision {
            let x = (e.random.next_float() * 2.0 - 1.0) * 0.05;
            let z = (e.random.next_float() * 2.0 - 1.0) * 0.05;
            e.delta = e.delta.add(x as f64, 0.4f32 as f64, z as f64);
            e.set_on_ground(&*level, false);
            e.needs_sync = true;
            mob::make_sound(e, m, level, "minecraft:entity.tadpole.flop");
        }
    }

    /// `Tadpole.aiStep` after `super.aiStep()`: growing older, the age lock's particles.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !st(m).age_locked {
            let age = st(m).age + 1;
            set_age(e, m, level, age);
            if e.is_removed() {
                return;
            }
        }
        // `AgeableMob.makeAgeLockedParticle`.
        let s = st_mut(m);
        if s.age_lock_particle_timer > 0 {
            if s.age_lock_particle_timer % 2 == 0 {
                let _ = mob::random_point(e, 1.0);
            }
            let s = st_mut(m);
            s.age_lock_particle_timer -= 1;
        }
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        amphibian::smooth_swimming_move(e, m, 85, 10, 0.02, 0.1, true);
        true
    }

    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        amphibian::smooth_swimming_look(e, m, 10);
        true
    }

    /// `AbstractFish.travelInWater`: slow, sinking without a target.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        if !e.is_in_water() {
            return false;
        }
        mob::move_relative(e, 0.01, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        if m.target.is_none() {
            e.delta = e.delta.add(0.0, -0.005, 0.0);
        }
        true
    }

    fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
        ext::water_animal_air(e, m, level, air_before);
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    fn swim_sound(&self) -> Option<&'static str> {
        Some("minecraft:entity.fish.swim")
    }

    /// `shouldDropExperience` is false.
    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(0)
    }

    /// `fromBucket` is always true: never despawns.
    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    /// `Mob.checkDespawn` for a mob that `requiresCustomPersistence`: it never idles.
    fn check_despawn(&self, e: &mut Entity, _level: &dyn EntityLevel) -> bool {
        if let Some(m) = mob::data_mut(e) {
            m.no_action_time = 0;
        }
        true
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: crate::math::BlockPos) -> Option<f32> {
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

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: crate::math::BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::squid::surface_water_rules(view, pos))
    }

    fn max_spawn_cluster(&self) -> i32 {
        8
    }

    /// `Tadpole.mobInteract`: slime balls speed the growing up, a golden dandelion locks the age,
    /// a water bucket takes the tadpole.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if stack.is_empty() {
            return None;
        }
        if is_food(stack.item()) && !st(m).age_locked {
            // `ageUp(getSpeedUpSecondsWhenFeeding(ticksLeftUntilAdult))`, then the particle's draws.
            let left = (TICKS_TO_BE_FROG - st(m).age).max(0);
            let seconds = mob::breed::speed_up_seconds_when_feeding(left);
            let age = st(m).age + seconds * 20;
            let _ = mob::random_point(e, 1.0);
            set_age(e, m, level, age);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        let name = mob::item_name(stack);
        if name == "minecraft:golden_dandelion" && st(m).age_lock_particle_timer == 0 && !mob::entity_type_tag(e.type_name, "minecraft:cannot_be_age_locked") {
            // `setAgeLockedData`.
            let locked = !st(m).age_locked;
            {
                let s = st_mut(m);
                s.age_locked = locked;
                s.age = 0;
                s.age_lock_particle_timer = 40;
            }
            if locked {
                m.persistence_required = true;
            }
            let sound = if locked { "minecraft:item.golden_dandelion.use" } else { "minecraft:item.golden_dandelion.unuse" };
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event(sound), source: "player", volume: 1.0, pitch: 1.0 });
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if name == "minecraft:water_bucket" && mob::is_alive(e, m) {
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.bucket.fill_tadpole", source: "neutral", volume: 1.0, pitch: 1.0 });
            }
            let mut filled = ItemStack::of("minecraft:tadpole_bucket", 1)?;
            super::fish::save_to_bucket(e, m, &mut filled);
            add_bucket_data(m, &mut filled);
            e.discard();
            return Some(Outcome::success(HeldChange::Fill(filled)));
        }
        None
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let age = r.int_or("Age", 0);
        let locked = r.bool_or("AgeLocked", false);
        let s = st_mut(m);
        s.age = age;
        s.age_locked = locked;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("FromBucket", Tag::Byte(1));
        o.put("Age", Tag::Int(s.age));
        o.put("AgeLocked", Tag::Byte(s.age_locked as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(data::tadpole::AGE_LOCKED, &DataValue::Boolean(st(m).age_locked));
    }
}

/// `Tadpole.saveToBucketTag`: the age and its lock in the bucket's entity data.
fn add_bucket_data(m: &MobData, bucket: &mut ItemStack) {
    let s = st(m);
    let mut fields = match bucket.get(kiln_item::keys::BUCKET_ENTITY_DATA).map(|c| c.0.clone()) {
        Some(Tag::Compound(f)) => f,
        _ => Vec::new(),
    };
    fields.push(("Age".into(), Tag::Int(s.age)));
    fields.push(("AgeLocked".into(), Tag::Byte(s.age_locked as i8)));
    bucket.insert(kiln_item::keys::BUCKET_ENTITY_DATA, kiln_item::component::CustomData(Tag::Compound(fields)));
}

/// `MobBucketItem.spawn` for a tadpole bucket on a new tadpole: `loadFromBucketTag` (the common
/// fields, `Age`, `AgeLocked`).
pub fn apply_bucket(e: &mut Entity, bucket: &ItemStack) {
    super::fish::apply_bucket(e, bucket);
    let Some(m) = mob::data_mut(e) else { return };
    if let Some(Tag::Compound(fields)) = bucket.get(kiln_item::keys::BUCKET_ENTITY_DATA).map(|c| c.0.clone())
        && ext::state::<State>(m).is_some()
    {
        for (k, v) in &fields {
            match k.as_str() {
                "Age" => {
                    if let Some(a) = v.as_i64() {
                        st_mut(m).age = a as i32;
                    }
                }
                "AgeLocked" => st_mut(m).age_locked = v.as_f64().is_some_and(|b| b != 0.0),
                _ => {}
            }
        }
    }
}

/// `TadpoleAi.getActivities` and `Tadpole.BRAIN_PROVIDER`'s sensors.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Tempting { items: Some(&["minecraft:slime_ball"]) }),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![AnimalPanic::new(2.0), LookAtTargetSink::new(45, 90), MoveToTargetSink::new(), CountDownCooldownTicks::new(Mem::TemptationCooldownTicks)],
    );
    let idle = ActivityData::with_priorities(
        Activity::Idle,
        vec![
            (0, SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (30, 60))),
            (1, FollowTemptation::new(|_| 1.25)),
            (
                2,
                Gate::new(
                    "GateBehavior",
                    &[(Mem::WalkTarget, Status::ValueAbsent)],
                    &[],
                    brain::OrderPolicy::Ordered,
                    brain::RunningPolicy::TryAll,
                    vec![
                        (stroll(0.5, StrollKind::Swim), 2),
                        (set_walk_target_from_look_target(0.5, 3), 3),
                        (shot("", &[], |cx| cx.e.is_in_water()), 5),
                    ],
                ),
            ),
        ],
    );
    Brain::new(&[], sensors, vec![core, idle], random)
}
