//! Snow golem (`SnowGolem`, an `AbstractGolem`): built from two snow blocks under a carved
//! pumpkin (the building is the simulation's), it pelts monsters with snowballs, leaves a trail
//! of snow where snow can lie, melts in hot biomes and in water or rain, and can be sheared of
//! its pumpkin.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, LOOK, Living, MOVE, Wanted};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{self, Category, DamageSource, MobData, path};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct SnowGolem;

pub static KIND: SnowGolem = SnowGolem;

static INFO: Info = Info { ambient_interval: 120, ..Info::misc("minecraft:snow_golem", &[(MaxHealth, 4.0), (MovementSpeed, 0.20000000298023224)]) };

#[derive(Clone, Debug)]
pub struct State {
    /// `DATA_PUMPKIN_ID` (bit 4: wearing its pumpkin).
    pub flags: i8,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("snow golem state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("snow golem state")
}

pub fn has_pumpkin(m: &MobData) -> bool {
    st(m).flags & 16 != 0
}

/// Entity types that are an `Enemy` (the monster category).
pub fn enemies() -> &'static [&'static str] {
    static LIST: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| mob::ALL_KINDS.iter().filter(|k| k.category() == Category::Monster).map(|k| k.type_name()).collect())
}

impl Kind for SnowGolem {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State { flags: 16 }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(RangedAttackGoal::new(1.25, 20, 20, 10.0))));
        g.add(2, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(1.0000001e-5), wanted: Vec3::ZERO, force: false });
        g.add(3, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(4, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        m.targets.add(
            1,
            Goal::NearestAttackable { wanted: Wanted::Types(enemies()), interval: reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false },
        );
    }

    fn sensitive_to_water(&self) -> bool {
        true
    }

    /// `SnowGolem.aiStep` after `Mob.aiStep`: melting in hot places, then the snow trail.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if level.snow_golem_melts(e.position()) {
            mob::hurt(e, m, level, DamageSource::of(DamageKind::OnFire), 1.0);
        }
        if !level.mob_griefing() {
            return;
        }
        let snow = kiln_data::blocks::default_state::SNOW;
        for i in 0..4 {
            let x = crate::math::floor(e.x() + ((i % 2 * 2 - 1) as f32 * 0.25) as f64);
            let y = crate::math::floor(e.y());
            let z = crate::math::floor(e.z() + ((i / 2 % 2 * 2 - 1) as f32 * 0.25) as f64);
            let p = BlockPos::new(x, y, z);
            if kiln_data::blocks_types::is_air(level.block(p)) && snow_survives(level, p) {
                level.set_block(p, snow, 3);
                level.emit(Event::GameEvent { event: "minecraft:block_place", pos: Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5), entity: Some(e.id) });
            }
        }
    }

    fn can_attack(&self, _m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        t.type_name != "minecraft:ghast"
    }

    /// Shears take the pumpkin off (dropping it at the eyes).
    fn ready_for_shearing(&self, m: &MobData) -> bool {
        has_pumpkin(m)
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if stack.is_empty() || mob::item_name(stack) != "minecraft:shears" || !has_pumpkin(m) {
            return Some(Outcome::PASS);
        }
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.snow_golem.shear", source: "players", volume: 1.0, pitch: 1.0 });
        st_mut(m).flags &= !16;
        let p = e.position();
        level.emit(Event::ShearLoot { entity: e.id, table: "minecraft:shearing/snow_golem".into(), pos: Vec3::new(p.x, p.y + e.eye_height as f64 - 1.0, p.z) });
        level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
        Some(Outcome::success(HeldChange::Damage(1)))
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let pumpkin = r.bool_or("Pumpkin", true);
        let s = st_mut(m);
        s.flags = if pumpkin { s.flags | 16 } else { s.flags & !16 };
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("Pumpkin", Tag::Byte(has_pumpkin(m) as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::snow_golem::PUMPKIN, &DataValue::Byte(st(m).flags));
    }
}

/// `SnowLayerBlock.canSurvive` for one layer: not on ice or barriers, on honey and soul sand,
/// else on a full top face.
fn snow_survives(level: &dyn EntityLevel, p: BlockPos) -> bool {
    let below = level.block(p.below());
    match crate::blocks::block_name(below) {
        "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier" => false,
        "minecraft:honey_block" | "minecraft:soul_sand" => true,
        _ => {
            let (shape, _) = crate::collision::collision_shape(below, p.below(), &crate::collision::CollisionContext::EMPTY);
            shape.boxes().iter().any(|b| b.max_y >= 1.0 && b.min_x <= 0.0 && b.max_x >= 1.0 && b.min_z <= 0.0 && b.max_z >= 1.0)
        }
    }
}

/// `SnowGolem.performRangedAttack`: a snowball from the eyes toward the target's eyes.
fn snow_attack(e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    perform_ranged_attack(e, level, t);
}

fn perform_ranged_attack(e: &mut Entity, level: &mut dyn EntityLevel, t: &Living) {
    let dx = t.pos.x - e.x();
    let dy = t.eye_y - 1.100000023841858;
    let dz = t.pos.z - e.z();
    let yo = (dx * dx + dz * dz).sqrt() * 0.20000000298023224;
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
    let mut ball = crate::projectile::new(id, 0, crate::projectile::Throwable::Snowball, pos, Vec3::ZERO, Some(e.id), seed);
    if let crate::entity::EntityKind::Throwable(d) = &mut ball.kind {
        d.item = ItemStack::of("minecraft:snowball", 1);
    }
    let y = ball.y();
    mob::species::shoot(&mut ball, dx, dy + yo - y, dz, 1.6, 12.0);
    ball.set_old_pos_and_rot();
    level.add_entity(ball);
    let pitch = 0.4 / (e.random.next_float() * 0.4 + 0.8);
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.snow_golem.shoot", source: "neutral", volume: 1.0, pitch });
    }
}

/// `RangedAttackGoal`: closes in until it sees the target within `radius`, then attacks every
/// `interval_min` to `interval_max` ticks (by the distance).
#[derive(Clone, Debug)]
pub struct RangedAttackGoal {
    name: &'static str,
    /// `RangedAttackMob.performRangedAttack`.
    attack: fn(&mut Entity, &mut MobData, &mut dyn EntityLevel, &Living),
    speed: f64,
    interval_min: i32,
    interval_max: i32,
    radius: f32,
    target: Option<i32>,
    attack_time: i32,
    see_time: i32,
}

impl RangedAttackGoal {
    pub fn new(speed: f64, interval_min: i32, interval_max: i32, radius: f32) -> RangedAttackGoal {
        RangedAttackGoal { name: "RangedAttackGoal", attack: snow_attack, speed, interval_min, interval_max, radius, target: None, attack_time: -1, see_time: 0 }
    }

    /// A `RangedAttackGoal` of another mob: `attack` is its `performRangedAttack`.
    pub fn with_attack(name: &'static str, speed: f64, interval_min: i32, interval_max: i32, radius: f32, attack: fn(&mut Entity, &mut MobData, &mut dyn EntityLevel, &Living)) -> RangedAttackGoal {
        RangedAttackGoal { name, attack, speed, interval_min, interval_max, radius, target: None, attack_time: -1, see_time: 0 }
    }
}

impl CustomGoal for RangedAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
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
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level) || (self.target.and_then(|id| goals::living(level, id)).is_some_and(|t| t.alive) && !m.nav.is_done())
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.target = None;
        self.see_time = 0;
        self.attack_time = -1;
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
        let radius_sqr = (self.radius * self.radius) as f64;
        if !(d > radius_sqr) && self.see_time >= 5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), self.speed);
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        self.attack_time -= 1;
        let (lo, hi) = (self.interval_min as f32, self.interval_max as f32);
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = d.sqrt() as f32 / self.radius;
            (self.attack)(e, m, level, &t);
            self.attack_time = floor_f(dist * (hi - lo) + lo);
        } else if self.attack_time < 0 {
            let f = d.sqrt() / self.radius as f64;
            self.attack_time = crate::math::floor(crate::math::lerp(f, lo as f64, hi as f64));
        }
    }
}

fn floor_f(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}
