//! Nautilus and zombie nautilus (`AbstractNautilus`, a `TamableAnimal` that lives in the water):
//! both swim about on a brain (`NautilusAi`, `ZombieNautilusAi`), are tempted by fish, charge
//! at their target and knock it back (`ChargeAttack`), are tamed with pufferfish (a one in three
//! chance per feeding), wear a saddle and armor once tame and carry one player that steers them
//! and dashes with the jump key. A tame one stays within 16 blocks of where it was tamed (32
//! unsaddled) and gives its rider the Breath of the Nautilus.
//!
//! The nautilus is a water creature that hunts pufferfish now and then and breeds on fish; the
//! zombie nautilus (a drowned's mount when it carries a trident) is a monster of the undead
//! that burns in daylight unless it wears armor and is never a baby.

use super::tame::{self, Tame};
use crate::behavior_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, EntityLevel, Event, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::start_attacking;
use crate::mob::brain::memory::{Val, WalkTarget};
use crate::mob::brain::sensors;
use crate::mob::brain::util::{self, Targeting};
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, OrderPolicy, RunningPolicy, Sensor, Status, Timed, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::interact::{self, HeldChange, Interactor, Outcome};
use crate::mob::path::PathType;
use crate::mob::random_pos::{self, Home};
use crate::mob::{self, Category, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{ValueAbsent, ValuePresent};

/// `Nautilus` (false) and `ZombieNautilus` (true).
pub struct Nautilus(pub bool);

pub static KIND: Nautilus = Nautilus(false);
pub static ZOMBIE: Nautilus = Nautilus(true);

static INFO: Info = Info {
    category: Category::WaterCreature,
    breathes_under_water: true,
    ..Info::animal("minecraft:nautilus", &[(MaxHealth, 15.0), (MovementSpeed, 1.0), (AttackDamage, 3.0), (KnockbackResistance, 0.30000001192092896)])
};

static ZOMBIE_INFO: Info = Info {
    category: Category::Monster,
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::animal("minecraft:zombie_nautilus", &[(MaxHealth, 15.0), (MovementSpeed, 1.100000023841858), (AttackDamage, 3.0), (KnockbackResistance, 0.30000001192092896)])
};

/// `Nautilus.NAUTILUS_TOTAL_AIR_SUPPLY` (the zombie one's is the shared 300 as well).
const AIR_SUPPLY: i32 = 300;
/// `ATTACK_TARGET_COOLDOWN` after a charge, and between two hunts of a wild one.
const TIME_BETWEEN_ATTACKS: i32 = 80;
const NON_PLAYER_ATTACKS: (i32, i32) = (2400, 3600);
/// `AbstractNautilus.DASH_COOLDOWN_TICKS`.
const DASH_COOLDOWN: i32 = 40;

#[derive(Clone, Debug)]
pub struct State {
    pub tame: Tame,
    /// `EquipmentSlot.SADDLE` and `BODY` (nautilus armor), and their drop chances.
    pub saddle: ItemStack,
    pub body: ItemStack,
    pub saddle_drop: f32,
    pub body_drop: f32,
    /// `dashCooldown` and `DATA_DASH`.
    dash_cooldown: i32,
    pub dashing: bool,
    /// `isUnderWater` as of the end of the last tick (for the sounds).
    under_water: bool,
    /// `Mob.homePosition` and `homeRadius`: where a tame one keeps to.
    home: Home,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("nautilus state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("nautilus state")
}

pub fn is_tame(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.tame.tame)
}

/// The tame state (see [`tame`]).
pub fn tame_of(m: &MobData) -> Option<&Tame> {
    ext::state::<State>(m).map(|s| &s.tame)
}

pub fn tame_of_mut(m: &mut MobData) -> Option<&mut Tame> {
    ext::state_mut::<State>(m).map(|s| &mut s.tame)
}

fn zombie(m: &MobData) -> bool {
    m.kind == MobKind::ZombieNautilus
}

/// The `entity.<nautilus or zombie_nautilus>.<what>` sound, with `baby_` for a nautilus calf.
fn snd(m: &MobData, what: &str) -> &'static str {
    let name = if zombie(m) {
        "zombie_nautilus"
    } else if m.baby() {
        "baby_nautilus"
    } else {
        "nautilus"
    };
    mob::sound_event(&format!("minecraft:entity.{name}.{what}"))
}

/// The sound with `_on_land` after it when the mob is not under water.
fn snd_wet(m: &MobData, what: &str) -> &'static str {
    if st(m).under_water { snd(m, what) } else { snd(m, &format!("{what}_on_land")) }
}

/// `playEatingSound`.
pub fn play_eating_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) {
    mob::make_sound(e, m, level, snd(m, "eat"));
}

// ---------------------------------------------------------------------- the brain

/// The selector of `NautilusAi.ATTACK_TARGET_CONDITIONS` (the world border is not modelled): armor
/// stands only with `mob_griefing`.
fn charge_selector(cx: &Cx, t: &mob::goals::Living) -> bool {
    cx.level.mob_griefing() || t.type_name != "minecraft:armor_stand"
}

/// `NautilusAi.isHostileTarget`: in the water and of `#nautilus_hostiles`.
fn is_hostile_target(cx: &mut Cx, id: i32) -> bool {
    let Some(t) = util::living(cx, id) else { return false };
    super::drowned::in_water(&*cx.level, &t) && mob::entity_type_tag(t.type_name, "minecraft:nautilus_hostiles")
}

/// `NautilusAi.findNearestValidAttackTarget`: not while breeding, out of the water, as a baby or
/// tame; the one it is angry at if that is in the water and attackable; else, when its attack
/// cooldown (2400 to 3600 ticks, started now) is over, half the time the closest visible
/// pufferfish in the water.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    if util::is_breeding(cx) || !cx.e.is_in_water() || cx.m.baby() || is_tame(cx.m) {
        return None;
    }
    if let Some(t) = brain::nether::living_from_uuid_memory(cx, Mem::AngryAt)
        && super::drowned::in_water(&*cx.level, &t)
        && util::is_entity_attackable_ignoring_los(cx, &t)
    {
        return Some(t.id);
    }
    if cx.b.mem.has(Mem::AttackTargetCooldown) {
        return None;
    }
    let cooldown = util::uniform(cx.rng(), NON_PLAYER_ATTACKS.0, NON_PLAYER_ATTACKS.1);
    cx.b.mem.set(Mem::AttackTargetCooldown, Val::Int(cooldown));
    if cx.rng().next_float() < 0.5 {
        return None;
    }
    util::find_closest_visible(cx, is_hostile_target)
}

/// `NautilusAi.setAngerTarget`: attackers it can attack make it angry for 400 ticks.
fn set_anger_target(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, attacker: i32) {
    let Some(t) = mob::goals::living(level, attacker) else { return };
    // `Sensor.isEntityAttackableIgnoringLineOfSight`.
    let range = m.attrs.value(FollowRange);
    if !mob::goals::targeting_ok(e, m, level, &t, true, range, true) {
        return;
    }
    let uuid = brain::nether::uuid_of(level, attacker);
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.erase(Mem::CantReachWalkTargetSince);
        b.st.mem.set_expiring(Mem::AngryAt, Val::Uuid(uuid), 400);
    }
}

/// `RandomStroll.getTargetSwimPos` for a mob with a home: swimmable spots at growing distances
/// (1, 3, 5, 6, 7, then 10 blocks across and 7 up), each further one in the direction of the
/// last, while the spots are in a fluid and inside the restriction.
fn target_swim_pos(cx: &mut Cx, home: Home) -> Option<Vec3> {
    const TIERS: [(i32, i32); 6] = [(1, 1), (3, 3), (5, 5), (6, 5), (7, 7), (10, 7)];
    let mut result: Option<Vec3> = None;
    let mut candidate: Option<Vec3> = None;
    for (h, v) in TIERS {
        candidate = match result {
            None => swimmable_pos(cx, h, v, home),
            Some(r) => {
                let p = cx.e.position();
                Some(p + (r - p).normalize().multiply(h as f64, v as f64, h as f64))
            }
        };
        let restricted = mob_restricted(cx.e, home, h as f64);
        match candidate {
            Some(c) => {
                let at = BlockPos::containing(c.x, c.y, c.z);
                let in_fluid = !crate::physics::fluid_state(cx.level.block(at)).is_empty();
                if in_fluid && !(restricted && !random_pos::within_home(home, at)) {
                    result = Some(c);
                } else {
                    return result;
                }
            }
            None => return result,
        }
    }
    candidate
}

/// `GoalUtils.mobRestricted(mob, h)` of a home.
fn mob_restricted(e: &Entity, home: Home, h: f64) -> bool {
    let Some((c, r)) = home else { return false };
    let d = r as f64 + h + 1.0;
    let (dx, dy, dz) = (c.x as f64 + 0.5 - e.x(), c.y as f64 + 0.5 - e.y(), c.z as f64 + 0.5 - e.z());
    dx * dx + dy * dy + dz * dz < d * d
}

/// `BehaviorUtils.getRandomSwimmablePos` with the mob's home.
fn swimmable_pos(cx: &mut Cx, h: i32, v: i32, home: Home) -> Option<Vec3> {
    let mut p = random_pos::default_pos_home(cx.e, cx.m, &*cx.level, h, v, home);
    let mut count = 0;
    while let Some(q) = p {
        let swimmable = crate::physics::fluid_state(cx.level.block(BlockPos::containing(q.x, q.y, q.z))).kind.is_water();
        if swimmable || count >= 10 {
            break;
        }
        count += 1;
        p = random_pos::default_pos_home(cx.e, cx.m, &*cx.level, h, v, home);
    }
    p
}

/// `RandomStroll.swim(speed)`.
fn swim_stroll(speed: f32) -> Box<dyn brain::Control> {
    shot("RandomStroll", &[(Mem::WalkTarget, ValueAbsent)], move |cx| {
        if !cx.e.is_in_water() {
            return false;
        }
        let home = st(cx.m).home;
        match target_swim_pos(cx, home) {
            Some(p) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, speed, 0))),
            None => cx.b.mem.erase(Mem::WalkTarget),
        }
        true
    })
}

/// `ChargeAttack(timeBetweenAttacks 80, ATTACK_TARGET_CONDITIONS, speed, knockback 2, max charge
/// distance 12, max detection distance 11, sound)`: runs at the target at `speed` for up to 60
/// ticks (60 to 60), turning to face it; whatever it bumps into on the way (that it may attack)
/// is hurt and thrown back, which ends the charge. A cooldown follows (`CHARGE_COOLDOWN_TICKS`)
/// and the attack target is dropped.
#[derive(Clone, Debug)]
struct ChargeAttack {
    time_between_attacks: i32,
    speed: f32,
    knockback_force: f32,
    max_charge_distance: f64,
    max_target_detection_distance: f64,
    charge_sound: &'static str,
    charge_velocity: Vec3,
    start_position: Vec3,
}

impl ChargeAttack {
    fn new(speed: f32, charge_sound: &'static str) -> Box<dyn brain::Control> {
        Timed::new(ChargeAttack {
            time_between_attacks: TIME_BETWEEN_ATTACKS,
            speed,
            knockback_force: 2.0,
            max_charge_distance: 12.0,
            max_target_detection_distance: 11.0,
            charge_sound,
            charge_velocity: Vec3::ZERO,
            start_position: Vec3::ZERO,
        })
    }

    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return false };
        if is_tame(cx.m) {
            return false;
        }
        if (cx.e.position() - self.start_position).length_sqr() >= self.max_charge_distance * self.max_charge_distance {
            return false;
        }
        if (target.pos - cx.e.position()).length_sqr() >= self.max_target_detection_distance * self.max_target_detection_distance {
            return false;
        }
        if !mob::has_line_of_sight_cached(cx.e, cx.m, &*cx.level, &target) {
            return false;
        }
        !cx.b.mem.has(Mem::ChargeCooldownTicks)
    }

    /// `dealKnockBack`: `causeExtraKnockback` of the speed (and effect) scaled force along the
    /// charger's facing; it slows down a little.
    fn deal_knockback(&self, cx: &mut Cx, target: &mob::goals::Living) {
        let speed_amp = crate::mob::effects::amplifier(cx.m, crate::effect::ids::speed()).map_or(0, |a| a + 1);
        let slow_amp = crate::mob::effects::amplifier(cx.m, crate::effect::ids::slowness()).map_or(0, |a| a + 1);
        let f = 0.25f32 * (speed_amp - slow_amp) as f32;
        let g = mob::mth::clamp(self.speed * cx.m.attrs.value(MovementSpeed) as f32, 0.2, 2.0) + f;
        let force = g * self.knockback_force;
        if force > 0.0 && !target.player {
            let rad = (cx.e.y_rot * 0.017453292) as f64;
            let (s, c) = (mob::mth::sin(rad) as f64, mob::mth::cos(rad) as f64);
            if let Some(o) = cx.level.entity_mut(target.id)
                && matches!(o.kind, crate::entity::EntityKind::Mob(_))
            {
                mob::knockback_entity(o, force as f64, s, -c);
            }
        } else if force > 0.0 && target.player {
            // A player's client owns its motion: it gets the push (`knockback` of the same
            // strength along the facing, less its knockback resistance of 0).
            let rad = (cx.e.y_rot * 0.017453292) as f64;
            let (s, c) = (mob::mth::sin(rad) as f64, mob::mth::cos(rad) as f64);
            let k = Vec3::new(s, 0.0, -c).normalize().scale(force as f64);
            cx.level.push(target.id, Vec3::new(-k.x, 0.4f64.min(force as f64), -k.z));
        }
        if force > 0.0 {
            cx.e.delta = cx.e.delta.multiply(0.6, 1.0, 0.6);
        }
    }
}

impl Behavior for ChargeAttack {
    fn name(&self) -> &'static str {
        "ChargeAttack"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::ChargeCooldownTicks, ValueAbsent), (Mem::AttackTarget, ValuePresent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::AttackTarget)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        ChargeAttack::can_still_use(self, cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        self.start_position = cx.e.position();
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return };
        let direction = (target.pos - cx.e.position()).normalize();
        self.charge_velocity = direction.scale(self.speed as f64);
        if ChargeAttack::can_still_use(self, cx) {
            mob::play_sound(cx.e, cx.m, cx.level, self.charge_sound, 1.0, 1.0);
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(target) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)) else { return };
        mob::mob_look_at(cx.e, &target, 360.0, 360.0);
        cx.e.delta = self.charge_velocity;
        let area = cx.e.bounding_box();
        let candidates = util::living_in_box(cx, &area);
        let combat = Targeting::combat();
        let mut hit = None;
        for id in candidates {
            let Some(l) = util::living(cx, id) else { continue };
            if combat.test(cx, &l) && charge_selector(cx, &l) {
                hit = Some(l);
                break;
            }
        }
        let Some(hit) = hit else { return };
        // (`hasPassenger(target)`: whoever rides it is spared.)
        if cx.e.passengers.contains(&hit.id) {
            return;
        }
        // `dealDamageToTarget`: `mobAttack` for its attack damage.
        let damage = cx.m.attrs.value(AttackDamage) as f32;
        let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(cx.e.id), direct: Some(cx.e.id), pos: Some(cx.e.position()), attacker_is_player: false };
        mob::hurt_living(cx.level, &hit, source, damage);
        self.deal_knockback(cx, &hit);
        self.stop(cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.set(Mem::ChargeCooldownTicks, Val::Int(self.time_between_attacks));
        cx.b.mem.erase(Mem::AttackTarget);
    }
    behavior_boilerplate!();
}

/// `NautilusAi.getActivities` / `ZombieNautilusAi.getActivities` with the sensors of
/// `Nautilus.BRAIN_PROVIDER`.
fn make_brain(zombie: bool, random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Adult { any_type: false }),
        Box::new(sensors::Players),
        Box::new(sensors::HurtBy),
        Box::new(sensors::Tempting::for_animal()),
    ];
    let mut core_behaviors: Vec<Box<dyn brain::Control>> = Vec::new();
    if !zombie {
        core_behaviors.push(AnimalPanic::new(1.6));
    }
    core_behaviors.extend([
        LookAtTargetSink::new(45, 90),
        MoveToTargetSink::new(),
        CountDownCooldownTicks::new(Mem::TemptationCooldownTicks),
        CountDownCooldownTicks::new(Mem::ChargeCooldownTicks),
        CountDownCooldownTicks::new(Mem::AttackTargetCooldown),
    ]);
    let core = ActivityData::create(Activity::Core, 0, core_behaviors);
    let tempt_speed: fn(&Cx) -> f32 = if zombie { |_| 0.9 } else { |_| 1.3 };
    let mut idle_behaviors: Vec<(i32, Box<dyn brain::Control>)> = Vec::new();
    if !zombie {
        idle_behaviors.push((1, AnimalMakeLove::new("minecraft:nautilus", 0.4, 2)));
    }
    idle_behaviors.push((if zombie { 1 } else { 2 }, FollowTemptation::with(tempt_speed, |cx| if cx.m.baby() { 2.5 } else { 3.5 }, false)));
    idle_behaviors.push((if zombie { 2 } else { 3 }, start_attacking(|_| true, find_nearest_valid_attack_target)));
    idle_behaviors.push((
        if zombie { 3 } else { 4 },
        Gate::new(
            "GateBehavior",
            &[(Mem::WalkTarget, ValueAbsent)],
            &[],
            OrderPolicy::Ordered,
            RunningPolicy::TryAll,
            vec![(swim_stroll(1.0), 2), (set_walk_target_from_look_target(1.0, 3), 3)],
        ),
    ));
    let idle = ActivityData::with_priorities(Activity::Idle, idle_behaviors);
    let (charge_speed, sound) = if zombie { (0.5, "minecraft:entity.zombie_nautilus.dash") } else { (0.6, "minecraft:entity.nautilus.dash") };
    let fight = ActivityData::full(
        Activity::Fight,
        vec![(0, ChargeAttack::new(charge_speed, sound))],
        &[(Mem::AttackTarget, ValuePresent), (Mem::TemptingPlayer, ValueAbsent), (Mem::BreedTarget, ValueAbsent), (Mem::ChargeCooldownTicks, ValueAbsent)],
        &[],
    );
    Brain::new(&[Mem::AngryAt, Mem::AttackTargetCooldown], sensors, vec![core, idle, fight], random)
}

// ---------------------------------------------------------------------- the kind

impl Nautilus {
    /// `checkRestriction`: a tame one that nobody leads or rides stays within a radius of the
    /// spot it was left at (re-set when it has moved too far or its saddle changed the radius).
    fn check_restriction(&self, e: &Entity, m: &mut MobData) {
        if !e.passengers.is_empty() || !is_tame(m) {
            return;
        }
        let radius = if !m.baby() && st(m).saddle.is_empty() { 32 } else { 16 };
        if let Some((home, r)) = st(m).home {
            // `getHomePosition().closerThan(blockPosition(), radius + 8)` and the same radius.
            let b = e.block_position();
            let d = ((home.x - b.x).pow(2) + (home.y - b.y).pow(2) + (home.z - b.z).pow(2)) as f64;
            let near = (radius + 8) as f64;
            if d < near * near && r == radius {
                return;
            }
        }
        st_mut(m).home = Some((e.block_position(), radius));
    }

    /// `AbstractNautilus.tick` after `Mob.tick`: the rider's breath effect, the dash timers and
    /// the draws of the bubbles it blows.
    fn tick_extras(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `applyEffects`: whoever rides it (a player) breathes under water.
        if let Some(&first) = e.passengers.first()
            && level.player(first).is_some()
        {
            let has = level.player_effect(first, "minecraft:breath_of_the_nautilus").is_some();
            let refresh = level.game_time() % 40 == 0;
            if !has || refresh {
                level.add_effect(first, "minecraft:breath_of_the_nautilus", 60, 0, None);
            }
        }
        let (dashing, cooldown) = (st(m).dashing, st(m).dash_cooldown);
        if dashing && cooldown < 35 {
            st_mut(m).dashing = false;
        }
        if st(m).dash_cooldown > 0 {
            st_mut(m).dash_cooldown -= 1;
            if st(m).dash_cooldown == 0 {
                mob::make_sound(e, m, level, snd_wet(m, "dash_ready"));
            }
        }
        if e.is_in_water() {
            // `spawnBubbles`: particles for the clients; the random draws are the server's too.
            let speed = e.delta.length();
            let probability = mob::mth::clamp_d(speed * 2.0, 0.15000000596046448, 1.0);
            if (e.random.next_float() as f64) < probability {
                let d = e.random.next_double() * 0.8 * (1.0 + speed);
                for _ in 0..3 {
                    let _ = (e.random.next_float() as f64 - 0.5) * d;
                }
            }
        }
    }
}

impl Kind for Nautilus {
    fn info(&self) -> &'static Info {
        if self.0 { &ZOMBIE_INFO } else { &INFO }
    }

    fn can_be_baby(&self) -> bool {
        !self.0
    }

    fn sun_protection_on_body(&self) -> bool {
        self.0
    }

    fn body_slot_mut<'a>(&self, m: &'a mut MobData) -> Option<&'a mut ItemStack> {
        ext::state_mut::<State>(m).map(|s| &mut s.body)
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        // `WaterBoundPathNavigation`; water costs nothing; 300 ticks of air.
        super::tame::set_malus(m, PathType::Water, 0.0);
        m.nav.water_bound = true;
        m.air_supply_max = AIR_SUPPLY;
        Some(Box::new(State {
            tame: Tame::default(),
            saddle: ItemStack::empty(),
            body: ItemStack::empty(),
            saddle_drop: 0.085,
            body_drop: 0.085,
            dash_cooldown: 0,
            dashing: false,
            under_water: false,
            home: None,
        }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(self.0, random))
    }

    /// `AbstractNautilus.isFood` for the grown tame and for calves (`#nautilus_food`).
    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:nautilus_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:nautilus_food")
    }

    /// The brain, the activity, `checkRestriction`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
        }
        self.check_restriction(e, m);
    }

    /// `SmoothSwimmingMoveControl(85, 10, 0.011, 0, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        super::axolotl::smooth_swim_move(e, m, 85, 10, 0.011, 0.0, true);
        true
    }

    /// `SmoothSwimmingLookControl(10)`.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        super::axolotl::smooth_swim_look(e, m, 10);
        true
    }

    /// `AbstractNautilus.travelInWater`: its own speed, a drag of 0.9, no gravity.
    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        mob::move_relative(e, m.speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        true
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    /// `getSwimSound` (the step sound is none: `playStepSound` does nothing).
    fn swim_sound(&self) -> Option<&'static str> {
        Some(mob::sound_event(if self.0 { "minecraft:entity.zombie_nautilus.swim" } else { "minecraft:entity.nautilus.swim" }))
    }

    fn swim_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(snd(m, "swim"))
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(snd_wet(m, "ambient")))
    }

    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(snd_wet(m, "hurt"))
    }

    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(snd_wet(m, "death"))
    }

    /// `handleAirSupply`: 300 ticks, then 2 damage a tick it is not in the water (`dryOut`).
    fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
        // (The zombie nautilus is undead: it breathes anywhere and never dries out.)
        if self.0 || m.no_ai {
            return;
        }
        if mob::is_alive(e, m) && !e.is_in_water() {
            e.air_supply = air_before - 1;
            if e.air_supply <= -20 {
                e.air_supply = 0;
                mob::hurt(e, m, level, DamageSource::of(DamageKind::DryOut), 2.0);
            }
        } else {
            e.air_supply = AIR_SUPPLY;
        }
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.tick_extras(e, m, level);
        st_mut(m).under_water = e.was_eye_in_water && e.is_in_water();
    }

    /// `AbstractNautilus.hurtServer`: whoever hurts it makes it angry (when it can attack them).
    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt && let Some(a) = source.attacker {
            set_anger_target(e, m, &*level, a);
        }
    }

    /// `AbstractNautilus.canBeAffected`: no poison.
    fn can_be_affected(&self, _m: &MobData, effect: &crate::effect::Effect, base: bool) -> bool {
        base && effect.id != crate::effect::ids::poison()
    }

    /// `TamableAnimal.canAttack`: never its owner.
    fn can_attack(&self, m: &MobData, level: &dyn EntityLevel, t: &mob::goals::Living) -> bool {
        !tame::owned_by(m, level, t.id)
    }

    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        Some(!is_tame(m))
    }

    /// `Nautilus.getBreedOffspring`: the baby of a tame parent is tame, with the same owner.
    fn breed_offspring(&self, _e: &mut Entity, m: &mut MobData, _partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        if is_tame(m) {
            let owner = st(m).tame.owner;
            let c = st_mut(child);
            c.tame.owner = owner;
            c.tame.tame = true;
        }
    }

    fn can_mate(&self, _m: &MobData, _partner: &MobData) -> bool {
        !self.0
    }

    /// `AbstractNautilus.finalizeSpawn`: the attack cooldown the brain starts with (a draw), then
    /// the animal's. The zombie one picks its variant first (one draw among the single best).
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        if self.0 {
            let _ = r.next_int_bounded(1);
            let warm = ctx.biome.is_some_and(|b| mob::species::biome_tag(b, "minecraft:spawns_coral_variant_zombie_nautilus"));
            let name = if warm { "minecraft:warm" } else { "minecraft:temperate" };
            if let Some(v) = kiln_data::synced_id("minecraft:zombie_nautilus_variant", name) {
                m.variant = v;
            }
        }
        let cooldown = util::uniform(r, NON_PLAYER_ATTACKS.0, NON_PLAYER_ATTACKS.1);
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.set(Mem::AttackTargetCooldown, Val::Int(cooldown));
        }
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
    }

    /// `checkNautilusSpawnRules`: in the sea between 25 and 5 blocks under the surface, with
    /// water below and above.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        let sea = view.sea_level();
        let min_y = sea - 25;
        Some(
            pos.y >= min_y
                && pos.y <= sea - 5
                && crate::physics::fluid_state(view.block(pos.below())).kind.is_water()
                && crate::blocks::block_name(view.block(pos.above())) == "minecraft:water",
        )
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }

    fn spawn_ignores_light(&self) -> bool {
        true
    }

    // ------------------------------------------------------------------ riding

    fn steerable_by(&self, m: &MobData, _rider: &PlayerView) -> bool {
        !st(m).saddle.is_empty()
    }

    /// `getRiddenRotation` and `tickRidden`: the rider's yaw (half the way each tick) and half its
    /// pitch.
    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &PlayerView) {
        let target_yaw = rider.yaw;
        let mut yaw = e.y_rot;
        let delta = mob::mth::wrap_degrees(target_yaw - yaw);
        yaw += delta * 0.5;
        e.y_rot = yaw % 360.0;
        e.x_rot = (rider.pitch * 0.5) % 360.0;
        m.y_head_rot = yaw;
        m.y_body_rot = yaw;
        e.y_rot_o = yaw;
    }

    /// `PASSENGER` attachment: 1.1375 up (a calf's 0.5).
    fn passenger_offset(&self, _e: &Entity, m: &MobData) -> Option<Vec3> {
        Some(Vec3::new(0.0, if m.baby() { 0.5 } else { 1.1375 }, 0.0))
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `AbstractNautilus.interact`: whoever touches it makes it stay.
        m.persistence_required = true;
        // A calf: `Animal.mobInteract` alone.
        if m.baby() {
            return Some(interact::animal_interact(e, m, level, who, stack));
        }
        let tame = is_tame(m);
        if tame && who.sneaking {
            // The inventory screen is not modelled.
            return Some(Outcome::success(HeldChange::None));
        }
        if !stack.is_empty() {
            // Untamed: pufferfish (or its bucket) tame it, one try in three.
            if !tame && mob::item_tag(stack.item(), "minecraft:nautilus_taming_items") {
                let held = use_player_item(stack);
                try_to_tame(e, m, level, who);
                return Some(Outcome::success(held));
            }
            if tame && self.is_food(stack.item()) && m.health < m.max_health() {
                // `feed(player, hand, stack, 2.0, 1.0)`: two hearts a point of nutrition.
                let held = use_player_item(stack);
                let nutrition = stack.get(kiln_item::keys::FOOD).map(|f| f.nutrition as f32);
                m.set_health(m.health + nutrition.map_or(1.0, |n| 2.0 * n));
                play_eating_sound(e, m, level);
                return Some(Outcome::success(held));
            }
            // The held item's own `interactLivingEntity`: a saddle and armor go on a tame, grown one.
            let usable = mob::is_alive(e, m) && tame;
            if usable && st(m).saddle.is_empty() && mob::item_name(stack) == "minecraft:saddle" {
                let mut one = stack.clone();
                one.set_count(1);
                st_mut(m).saddle = one;
                if !e.silent {
                    let sound = if st(m).under_water { "minecraft:entity.nautilus.saddle_underwater_equip" } else { "minecraft:entity.nautilus.saddle_equip" };
                    level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch: 1.0 });
                }
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            if usable && st(m).body.is_empty() && super::horse::equippable_in_slot(stack, kiln_item::component::EquipmentSlot::Body, e.type_name) {
                let mut one = stack.clone();
                one.set_count(1);
                let s = st_mut(m);
                s.body = one;
                s.body_drop = 2.0;
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
        }
        if tame && !who.sneaking && (stack.is_empty() || !self.is_food(stack.item())) && e.passengers.is_empty() {
            // `doPlayerRide`: a rider ends the home it kept.
            st_mut(m).home = None;
            let mut out = Outcome::success(HeldChange::None);
            out.ride = true;
            return Some(out);
        }
        // `Animal.mobInteract` (an untamed grown one has no food: the taming items are above).
        if !tame {
            return Some(Outcome::PASS);
        }
        Some(interact::animal_interact(e, m, level, who, stack))
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        let mut out = Vec::new();
        if !s.body.is_empty() {
            out.push((6, s.body.clone()));
        }
        if !s.saddle.is_empty() {
            out.push((7, s.saddle.clone()));
        }
        out
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = st_mut(m);
        vec![(std::mem::take(&mut s.body), s.body_drop), (std::mem::take(&mut s.saddle), s.saddle_drop)]
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let equipment = |key: &str, r: &mut Input| match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == key).and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let (saddle, body) = (equipment("saddle", r), equipment("body", r));
        let drop_chance = |r: &mut Input, key: &str| match r.get("drop_chances") {
            Some(Tag::Compound(dc)) => dc.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.as_f64()).map(|f| f as f32),
            _ => None,
        };
        let (saddle_drop, body_drop) = (drop_chance(r, "saddle"), drop_chance(r, "body"));
        let variant = r.get("variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:zombie_nautilus_variant", v));
        let mut tame_state = std::mem::take(&mut st_mut(m).tame);
        tame::load(&mut tame_state, r);
        let s = st_mut(m);
        s.tame = tame_state;
        if let Some(sd) = saddle {
            s.saddle = sd;
        }
        if let Some(b) = body {
            s.body = b;
        }
        if let Some(d) = saddle_drop {
            s.saddle_drop = d;
        }
        if let Some(d) = body_drop {
            s.body_drop = d;
        }
        if self.0
            && let Some(v) = variant
        {
            m.variant = v;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        tame::save(&s.tame, o);
        for (key, stack) in [("body", &s.body), ("saddle", &s.saddle)] {
            if stack.is_empty() {
                continue;
            }
            let entry = (key.to_owned(), stack.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        for (key, chance) in [("saddle", s.saddle_drop), ("body", s.body_drop)] {
            if chance != 0.085 {
                let entry = (key.to_owned(), Tag::Float(chance));
                match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                    Some((_, Tag::Compound(dc))) => dc.push(entry),
                    _ => o.put("drop_chances", Tag::Compound(vec![entry])),
                }
            }
        }
        if self.0
            && let Some(n) = super::wolf::synced_name("minecraft:zombie_nautilus_variant", m.variant)
        {
            o.put("variant", Tag::String(n.to_owned()));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        tame::entity_data(&s.tame, d);
        d.set(kiln_data::entities::data::abstract_nautilus::DASH, &DataValue::Boolean(s.dashing));
        if self.0 {
            d.set(kiln_data::entities::data::zombie_nautilus::VARIANT, &DataValue::Holder(m.variant));
        }
    }
}

/// `usePlayerItem`: a bucket of fish comes back as a water bucket, anything else is eaten.
fn use_player_item(stack: &ItemStack) -> HeldChange {
    if mob::item_tag(stack.item(), "minecraft:nautilus_bucket_food")
        && let Some(bucket) = ItemStack::of("minecraft:water_bucket", 1)
    {
        return HeldChange::Fill(bucket);
    }
    HeldChange::Consume(1)
}

/// `tryToTame`: one try in three takes; the navigation stops and the hearts (or the smoke) show.
fn try_to_tame(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor) {
    if e.random.next_int_bounded(3) == 0 {
        if let Some(p) = level.player(who.id) {
            // `tame(player)`: tame, owned by the player, the advancement.
            let s = st_mut(m);
            s.tame.tame = true;
            s.tame.owner = Some(p.uuid);
            let animal = crate::level::Seen::of_mob(e, m);
            level.emit(Event::Criterion { player: p.id, criterion: crate::level::Criterion::TameAnimal { animal } });
        }
        m.nav.stop();
        level.emit(Event::EntityEvent { entity: e.id, event: 7 });
    } else {
        level.emit(Event::EntityEvent { entity: e.id, event: 6 });
    }
    play_eating_sound(e, m, level);
}

/// `handleStartJump` on a saddled nautilus (the rider's jump key): the dash, with its sound and a
/// cooldown of 40 ticks (the dash's motion is the rider's client's).
pub fn start_jump(m: &mut MobData) -> Option<&'static str> {
    let s = ext::state::<State>(m)?;
    if s.saddle.is_empty() || s.dash_cooldown > 0 {
        return None;
    }
    let sound = snd_wet(m, "dash");
    let s = st_mut(m);
    s.dashing = true;
    s.dash_cooldown = DASH_COOLDOWN;
    Some(sound)
}
