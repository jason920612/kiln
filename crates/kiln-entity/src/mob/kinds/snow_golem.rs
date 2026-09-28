//! Snow golem: throws snowballs (1.6 speed, 12 uncertainty, every 20 ticks within 10 blocks)
//! at monsters, leaves a trail of snow layers where it walks (mob griefing permitting),
//! melts (fire damage 1) where `snow_golem_melts` holds (deserts, savannas, badlands, the
//! Nether) and takes drowning damage from water and rain. Shears take its pumpkin off.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, LOOK, MOVE, Living, Wanted};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{self, DamageSource, MobData, item_name, path};
use crate::persist::{Input, Output};
use crate::projectile::Throwable;
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct SnowGolem;

pub static KIND: SnowGolem = SnowGolem;

static INFO: Info = Info { sounds: Some("snow_golem"), ambient_interval: 80, ..Info::misc("minecraft:snow_golem", &[(MaxHealth, 4.0), (MovementSpeed, 0.20000000298023224)]) };

/// `Enemy` types Kiln simulates (`NearestAttackableTargetGoal` with `ENEMY_SELECTOR`).
const ENEMIES: &[&str] = &[
    "minecraft:zombie",
    "minecraft:skeleton",
    "minecraft:creeper",
    "minecraft:spider",
    "minecraft:husk",
    "minecraft:stray",
    "minecraft:drowned",
    "minecraft:zombie_villager",
    "minecraft:zombified_piglin",
    "minecraft:wither_skeleton",
    "minecraft:enderman",
    "minecraft:endermite",
    "minecraft:shulker",
    "minecraft:witch",
    "minecraft:slime",
    "minecraft:magma_cube",
    "minecraft:phantom",
    "minecraft:ghast",
    "minecraft:blaze",
    "minecraft:piglin",
    "minecraft:hoglin",
];

#[derive(Clone, Debug)]
pub struct State {
    /// `DATA_PUMPKIN_ID` bit 16.
    pub pumpkin: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("snow golem state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("snow golem state")
}

impl Kind for SnowGolem {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State { pumpkin: true }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(RangedAttack { target: None, attack_time: -1, see_time: 0 })));
        g.add(2, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(1.0000001e-5), wanted: Vec3::ZERO, force: false });
        g.add(3, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(4, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        m.targets.add(1, Goal::NearestAttackable { wanted: Wanted::Types(ENEMIES), interval: reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false });
    }

    fn sensitive_to_water(&self) -> bool {
        true
    }

    /// The end of `SnowGolem.aiStep`: melting, then the trail of snow.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !mob::is_alive(e, m) {
            return;
        }
        if level.snow_golem_melts(e.position()) {
            mob::hurt(e, m, level, DamageSource::of(DamageKind::OnFire), 1.0);
        }
        if !level.mob_griefing() {
            return;
        }
        for i in 0..4 {
            let x = crate::math::floor(e.x() + ((i % 2 * 2 - 1) as f32 * 0.25) as f64);
            let y = crate::math::floor(e.y());
            let z = crate::math::floor(e.z() + ((i / 2 % 2 * 2 - 1) as f32 * 0.25) as f64);
            let pos = BlockPos::new(x, y, z);
            if kiln_data::blocks_types::is_air(level.block(pos)) && snow_survives(level, pos) {
                level.set_block(pos, kiln_data::blocks::default_state::SNOW, 3);
                level.emit(Event::GameEvent { event: "minecraft:block_place", pos: pos.center(), entity: Some(e.id) });
            }
        }
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if stack.is_empty() || item_name(stack) != "minecraft:shears" || !st(m).pumpkin {
            return Some(Outcome::PASS);
        }
        // `shear`: the pumpkin comes off and drops from the shearing loot table.
        st_mut(m).pumpkin = false;
        e.needs_sync = true;
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.snow_golem.shear", source: "players", volume: 1.0, pitch: 1.0 });
        level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
        let mut out = Outcome::success(HeldChange::Damage(1));
        out.shear = Some("minecraft:shearing/snow_golem".into());
        Some(out)
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let pumpkin = r.bool_or("Pumpkin", true);
        st_mut(m).pumpkin = pumpkin;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("Pumpkin", kiln_proto::nbt::Tag::Byte(st(m).pumpkin as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        if !st(m).pumpkin {
            d.set(data::snow_golem::PUMPKIN, &DataValue::Byte(0));
        }
    }
}

/// `SnowLayerBlock.canSurvive` for a new layer on top of `pos.below()`.
fn snow_survives(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    let below_pos = pos.below();
    let below = level.block(below_pos);
    match crate::blocks::block_name(below) {
        "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier" => return false,
        "minecraft:honey_block" | "minecraft:soul_sand" | "minecraft:mud" => return true,
        "minecraft:snow" => return kiln_data::blocks_types::block_of(below).property(below, "layers") == Some("8"),
        _ => {}
    }
    let (shape, _) = crate::collision::collision_shape(below, below_pos, &crate::collision::CollisionContext::EMPTY);
    shape.boxes().iter().any(|b| b.max_y >= 1.0 && b.min_x <= 0.0 && b.max_x >= 1.0 && b.min_z <= 0.0 && b.max_z >= 1.0)
}

/// `SnowGolem.performRangedAttack`: a snowball from the eyes toward the target's chest.
fn perform_ranged_attack(e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    let dx = t.pos.x - e.x();
    let dy = t.eye_y - 1.100000023841858;
    let dz = t.pos.z - e.z();
    let h = (dx * dx + dz * dz).sqrt() * 0.20000000298023224;
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
    let mut p = crate::projectile::new(id, 0, Throwable::Snowball, pos, Vec3::ZERO, Some(e.id), seed);
    if let EntityKind::Throwable(d) = &mut p.kind {
        d.item = ItemStack::of("minecraft:snowball", 1);
    }
    let py = p.y();
    mob::species::shoot(&mut p, dx, dy + h - py, dz, 1.6, 12.0);
    level.add_entity(p);
    if !e.silent {
        let pitch = 0.4 / (e.random.next_float() * 0.4 + 0.8);
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.snow_golem.shoot", source: "neutral", volume: 1.0, pitch });
    }
}

/// `RangedAttackGoal(this, 1.25, 20, 10)`.
#[derive(Clone, Debug)]
struct RangedAttack {
    target: Option<i32>,
    attack_time: i32,
    see_time: i32,
}

const RADIUS: f32 = 10.0;
const INTERVAL: i32 = 20;

impl CustomGoal for RangedAttack {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RangedAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match goals::target(m, level) {
            Some(t) if t.alive => {
                self.target = Some(t.id);
                true
            }
            _ => false,
        }
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
        if !(d > (RADIUS * RADIUS) as f64) && self.see_time >= 5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.25);
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            perform_ranged_attack(e, m, level, &t);
            self.attack_time = INTERVAL;
        } else if self.attack_time < 0 {
            self.attack_time = INTERVAL;
        }
    }
}
