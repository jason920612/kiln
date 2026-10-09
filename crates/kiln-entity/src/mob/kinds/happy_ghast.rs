//! Happy ghast (`HappyGhast`): a big, gentle `Animal` that floats about. Grown, it floats to random points like a ghast
//! (`GhastMoveControl`, `RandomFloatAroundGoal`) and drifts toward a player holding snowballs; it stays put while
//! someone stands on it or sits in it, and carries up to four riders once it wears a harness. A ghastling (the baby)
//! is steered by a brain (follows the player holding food, then the adults, wanders). It heals a heart a minute (every
//! second in rain and clouds).

use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::sensors;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Gate, Mem, Sensor, Status::ValueAbsent};
use crate::mob::control::Operation;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, MOVE};
use crate::mob::interact::{self, HeldChange, Interactor, Outcome};
use crate::mob::kinds::ghast::{self, RandomFloatAround};
use crate::mob::{self, DamageSource, MobData, fly};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct HappyGhast;

pub static KIND: HappyGhast = HappyGhast;

/// `Animal.createAnimalAttributes` with the happy ghast's own.
static INFO: Info = Info {
    sound_source: "neutral",
    ..Info::animal(
        "minecraft:happy_ghast",
        &[(MaxHealth, 20.0), (TemptRange, 16.0), (FlyingSpeed, 0.05), (MovementSpeed, 0.05), (FollowRange, 16.0), (CameraDistance, 8.0)],
    )
};

/// `HappyGhast.BABY_DIMENSIONS`: the type's 4 by 4 scaled by 0.2375, the eyes at 0.46875.
const BABY: (f32, f32, f32) = (4.0 * 0.2375f32, 4.0 * 0.2375f32, 0.46875);

#[derive(Clone, Debug)]
pub struct State {
    /// `GhastMoveControl.floatDuration`.
    pub float_duration: i32,
    /// `serverStillTimeout`: ticks it keeps still after someone boarded or stands on it.
    pub still_timeout: i32,
    /// `leashHolderTime`.
    pub leash_holder_time: i32,
    /// The harness (`EquipmentSlot.BODY`) and the chance it drops.
    pub body: ItemStack,
    pub body_drop: f32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("happy ghast state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("happy ghast state")
}

/// `HappyGhast.isOnStillTimeout`: told to stay still, or a rider or someone on it is waiting.
pub(crate) fn on_still_timeout(m: &MobData) -> bool {
    st(m).still_timeout > 0
}

/// `Mob.isWearingBodyArmor`.
fn wearing_harness(m: &MobData) -> bool {
    !st(m).body.is_empty()
}

/// The ghast's flight in clouds, rain and snow heals it faster: `Entity.isInClouds` at the overworld's cloud layer.
fn in_clouds(e: &Entity) -> bool {
    let cloud_height = 192.33f32;
    !(e.y() + (e.height as f64) < cloud_height as f64) && e.y() <= (cloud_height + 4.0) as f64
}

/// `HappyGhast.scanPlayerAboveGhast`: a player (not a spectator, not riding a happy ghast) standing in the box over its
/// back.
fn scan_player_above(e: &Entity, level: &dyn EntityLevel) -> bool {
    let bb = e.bounding_box();
    let area = crate::math::Aabb::new(bb.min_x - 1.0, bb.max_y - 9.999999747378752E-6, bb.min_z - 1.0, bb.max_x + 1.0, bb.max_y + (bb.max_y - bb.min_y) / 2.0, bb.max_z + 1.0);
    level.players().iter().any(|p| {
        if p.spectator {
            return false;
        }
        // `getRootVehicle() instanceof HappyGhast`: a player riding one does not count.
        let rides_ghast = p.vehicle.and_then(|v| level.entity(v)).is_some_and(|v| v.type_name == "minecraft:happy_ghast");
        // `AABB.contains(root.position())`.
        let at = p.pos;
        !rides_ghast && at.x >= area.min_x && at.x < area.max_x && at.y >= area.min_y && at.y < area.max_y && at.z >= area.min_z && at.z < area.max_z
    })
}

// ---------------------------------------------------------------------- the brain

/// `HappyGhastAi.getActivities` with the sensors of `HappyGhast.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Tempting::for_animal()),
        Box::new(sensors::Adult { any_type: true }),
        Box::new(sensors::Players),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            Swim::new(0.8),
            AnimalPanic::with(2.0, "minecraft:panic_causes", Some(0)),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
        ],
    );
    let idle = ActivityData::with_priorities(
        Activity::Idle,
        vec![
            (1, FollowTemptation::with(|_| 1.25, |_| 3.0, true)),
            (2, baby_follow_adult((3, 16), |_| 1.1, Mem::NearestVisiblePlayer, true)),
            (3, baby_follow_adult((3, 16), |_| 1.1, Mem::NearestVisibleAdult, true)),
            (4, Gate::run_one(vec![(stroll(1.0, StrollKind::Fly), 1), (set_walk_target_from_look_target(1.0, 3), 1)])),
        ],
    );
    let panic = ActivityData::full(Activity::Panic, vec![], &[(Mem::IsPanicking, brain::Status::ValuePresent)], &[]);
    let _ = ValueAbsent;
    Brain::new(&[], sensors, vec![core, idle, panic], random)
}

impl HappyGhast {
    /// `babyGhastSetup`: a flying move control and navigation, no goals.
    fn baby_setup(&self, m: &mut MobData) {
        m.nav.fly = true;
        m.nav.can_float = true;
        m.nav.can_open_doors = false;
        m.nav.required_path_length = 48.0;
        m.goals.remove_where(|_| true);
        st_mut(m).still_timeout = 0;
    }

    /// `adultGhastSetup`: the ghast's move control and goals again, the brain forgetting everything.
    fn adult_setup(&self, m: &mut MobData) {
        m.nav.fly = false;
        m.goals.remove_where(|_| true);
        self.register_goals(m);
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.clear_all();
        }
    }
}

impl Kind for HappyGhast {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State { float_duration: 0, still_timeout: 0, leash_holder_time: 0, body: ItemStack::empty(), body_drop: 0.085 }))
    }

    /// `HappyGhast.registerGoals`: float (3), tempt by snowballs (4), float around (5).
    fn register_goals(&self, m: &mut MobData) {
        m.goals.add(3, super::common_a::Named::new("HappyGhastFloatGoal", Goal::Float).gate(super::happy_ghast_goals::float_gate).boxed());
        m.goals.add(4, Goal::Custom(Box::new(super::happy_ghast_goals::TemptForNonPathfinders::new(1.0, 7.0))));
        m.goals.add(5, Goal::Custom(Box::new(RandomFloatAround { distance_to_blocks: 16 })));
    }

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn age_boundary_reached(&self, _e: &mut Entity, m: &mut MobData) {
        if m.baby() {
            self.baby_setup(m);
        } else {
            self.adult_setup(m);
        }
    }

    /// The ghastling's brain, then `checkRestriction`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if m.baby() {
            brain::tick_brain(e, m, level);
            if let Some(b) = m.brain.as_mut() {
                b.st.set_active_activity_to_first_valid(&[Activity::Panic, Activity::Idle]);
            }
        }
        // `checkRestriction`: a free ghast keeps a home around where it is (64 blocks grown and unharnessed, else 32).
        if e.leash.is_none() && e.passengers.is_empty() {
            let radius = if !m.baby() && !wearing_harness(m) { 64 } else { 32 };
            let home_here = m.home.is_some_and(|(c, r)| {
                let (dx, dy, dz) = ((c.x - e.block_position().x) as f64, (c.y - e.block_position().y) as f64, (c.z - e.block_position().z) as f64);
                dx * dx + dy * dy + dz * dz < ((radius + 16) * (radius + 16)) as f64 && r == radius
            });
            if !home_here {
                m.home = Some((e.block_position(), radius));
            }
        }
    }

    /// `HappyGhast.tick` after `Mob.tick`.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        {
            let s = st_mut(m);
            if s.leash_holder_time > 0 {
                s.leash_holder_time -= 1;
            }
        }
        if st(m).still_timeout > 0 {
            if e.tick_count > 60 {
                st_mut(m).still_timeout -= 1;
            }
        }
        if scan_player_above(e, &*level) {
            st_mut(m).still_timeout = 10;
        }
    }

    /// `HappyGhast.aiStep` after `Mob.aiStep`: `continuousHeal`.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !mob::is_alive(e, m) || m.death_time != 0 || m.max_health() == m.health {
            return;
        }
        let in_weather = in_clouds(e) || level.is_raining_at(e.block_position());
        let every = if in_weather { 20 } else { 600 };
        if e.tick_count % every == 0 {
            let h = m.health + 1.0;
            m.set_health(h);
        }
    }

    /// `GhastMoveControl(this, true, this::isOnStillTimeout)` grown, `FlyingMoveControl(180, true)` as a ghastling.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.baby() {
            fly::tick_move(e, m, 180.0, true);
            return true;
        }
        let stopped = on_still_timeout(m);
        let mut fd = st(m).float_duration;
        ghast::tick_ghast_move(e, m, &*level, &mut fd, true, stopped);
        st_mut(m).float_duration = fd;
        true
    }

    /// `HappyGhastLookControl.tick`: facing its way of flight (a ghast's `faceMovementDirection`), still when it stays still.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.baby() {
            return false;
        }
        if on_still_timeout(m) {
            // `Mth.wrapDegrees90(yRot)`: the nearest quarter turn.
            let y = e.y_rot;
            let rest = wrap_degrees_90(y);
            e.y_rot = y - rest;
            m.y_head_rot = e.y_rot;
            return true;
        }
        if m.look.cooldown > 0 {
            m.look.cooldown -= 1;
            let [wx, _, wz] = m.look.wanted;
            let (dx, dz) = (wx - e.x(), wz - e.z());
            e.y_rot = -(mob::mth::atan2(dx, dz) as f32) * 57.295776;
            m.y_body_rot = e.y_rot;
            m.y_head_rot = m.y_body_rot;
            return true;
        }
        ghast::face_movement_direction(e, m, &*level);
        true
    }

    /// `travelFlying(input, flyingSpeed * 5 / 3)`.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let speed = m.attrs.value(FlyingSpeed) as f32 * 5.0 / 3.0;
        fly::travel(e, level, input, speed);
        true
    }

    /// `HappyGhast.getWalkTargetValue`: nowhere solid is nothing, over air that is held up better.
    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        let air = |q: BlockPos| kiln_data::blocks_types::is_air(level.block(q));
        Some(if !air(p) {
            0.0
        } else if air(p.below()) && !air(p.below().below()) {
            10.0
        } else {
            5.0
        })
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    /// `HappyGhast.canBreatheUnderwater`: a ghastling does.
    fn breathes_under_water_now(&self, m: &MobData) -> Option<bool> {
        Some(m.baby())
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:happy_ghast_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    /// `canFallInLove`: never.
    fn can_mate(&self, _m: &MobData, _partner: &MobData) -> bool {
        false
    }

    fn max_spawn_cluster(&self) -> i32 {
        1
    }

    fn sound_volume(&self, m: &MobData) -> f32 {
        if m.baby() { 1.0 } else { 4.0 }
    }

    fn voice_pitch(&self, _m: &MobData, _pitch: f32) -> f32 {
        1.0
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(mob::sound_event(if m.baby() { "minecraft:entity.ghastling.ambient" } else { "minecraft:entity.happy_ghast.ambient" })))
    }

    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(mob::sound_event(if m.baby() { "minecraft:entity.ghastling.hurt" } else { "minecraft:entity.happy_ghast.hurt" }))
    }

    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(mob::sound_event(if m.baby() { "minecraft:entity.ghastling.death" } else { "minecraft:entity.happy_ghast.death" }))
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { BABY } else { base }
    }

    /// `HappyGhast.mobInteract`: a ghastling like any animal; a grown one takes a harness, and a player who does not sneak
    /// sits on it once it wears one.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if m.baby() {
            return Some(interact::animal_interact(e, m, level, who, stack));
        }
        if !stack.is_empty()
            && mob::is_alive(e, m)
            && st(m).body.is_empty()
            && super::horse::equippable_in_slot(stack, kiln_item::component::EquipmentSlot::Body, e.type_name)
        {
            let mut one = stack.clone();
            one.set_count(1);
            let s = st_mut(m);
            s.body = one;
            s.body_drop = 2.0;
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if wearing_harness(m) && !who.sneaking {
            // `doPlayerRide`.
            let mut out = Outcome::success(HeldChange::None);
            out.ride = e.passengers.len() < 4;
            return Some(out);
        }
        Some(interact::animal_interact(e, m, level, who, stack))
    }

    /// Whether a player riding first steers it: harnessed, grown, and not standing still (`getControllingPassenger`).
    fn steerable_by(&self, m: &MobData, _rider: &crate::level::PlayerView) -> bool {
        wearing_harness(m) && !m.baby() && !on_still_timeout(m)
    }

    /// `getRiddenRotation` and `tickRidden`: the rider's yaw (8% of the way each tick) and half its pitch.
    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &crate::level::PlayerView) {
        let delta = mob::mth::wrap_degrees(rider.yaw - e.y_rot);
        let yaw = e.y_rot + delta * 0.08;
        e.y_rot = yaw;
        e.x_rot = rider.pitch * 0.5;
        m.y_head_rot = yaw;
        m.y_body_rot = yaw;
        e.y_rot_o = yaw;
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        if s.body.is_empty() { Vec::new() } else { vec![(6, s.body.clone())] }
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = st_mut(m);
        vec![(std::mem::take(&mut s.body), s.body_drop)]
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let still = r.int_or("still_timeout", 0);
        let body = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "body").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let body_drop = match r.get("drop_chances") {
            Some(Tag::Compound(dc)) => dc.iter().find(|(k, _)| k == "body").and_then(|(_, v)| v.as_f64()).map(|f| f as f32),
            _ => None,
        };
        let s = st_mut(m);
        s.still_timeout = still;
        if let Some(b) = body {
            s.body = b;
        }
        if let Some(d) = body_drop {
            s.body_drop = d;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("still_timeout", Tag::Int(s.still_timeout));
        if !s.body.is_empty() {
            let entry = ("body".to_owned(), s.body.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        if s.body_drop != 0.085 {
            let entry = ("body".to_owned(), Tag::Float(s.body_drop));
            match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                Some((_, Tag::Compound(dc))) => dc.push(entry),
                _ => o.put("drop_chances", Tag::Compound(vec![entry])),
            }
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(kiln_data::entities::data::happy_ghast::IS_LEASH_HOLDER, &DataValue::Boolean(s.leash_holder_time > 0));
        d.set(kiln_data::entities::data::happy_ghast::STAYS_STILL, &DataValue::Boolean(s.still_timeout > 0));
    }

    fn is_invulnerable_to(&self, _m: &MobData, _kind: DamageKind) -> bool {
        false
    }

    fn hurt(&self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        None
    }
}

/// `Mth.wrapDegrees90`: the angle's distance to the nearest multiple of 90 degrees, with sign.
fn wrap_degrees_90(v: f32) -> f32 {
    let mut d = mob::mth::wrap_degrees(v) % 90.0;
    if d >= 45.0 {
        d -= 90.0;
    } else if d < -45.0 {
        d += 90.0;
    }
    d
}

