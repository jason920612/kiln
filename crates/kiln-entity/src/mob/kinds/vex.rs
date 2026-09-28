//! Vex: a small flying spirit an evoker summons. Passes through blocks, drifts to random spots
//! around where it appeared, charges its (or its evoker's) target and starves after its limited
//! life.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::control::Operation;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Wanted, MOVE};
use crate::mob::kinds::raider;
use crate::mob::mth::{self, reduced_tick_delay};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Vex;

pub static KIND: Vex = Vex;

static INFO: Info = Info { sounds: Some("vex"), fire_immune: true, ..Info::monster("minecraft:vex", &[(MaxHealth, 14.0), (AttackDamage, 4.0)]) };

#[derive(Clone, Debug, Default)]
pub struct VexState {
    /// The summoning evoker: its network id while known, and its UUID.
    pub owner: Option<i32>,
    pub owner_uuid: Option<u128>,
    /// `boundOrigin`.
    pub bound_origin: Option<BlockPos>,
    /// `hasLimitedLife` and `limitedLifeTicks`.
    pub limited_life: Option<i32>,
    /// `DATA_FLAGS_ID` bit 1 (`isCharging`).
    pub charging: bool,
}

fn st(m: &MobData) -> &VexState {
    ext::state::<VexState>(m).expect("vex state")
}

fn st_mut(m: &mut MobData) -> &mut VexState {
    ext::state_mut::<VexState>(m).expect("vex state")
}

/// The vex's owner (`getOwner`), for its allies.
pub fn owner(m: &MobData) -> Option<i32> {
    ext::state::<VexState>(m).and_then(|s| s.owner)
}

/// `setOwner`, `setBoundOrigin` and `setLimitedLife` of a vex being summoned.
pub fn bind(m: &mut MobData, owner: i32, owner_uuid: u128, origin: BlockPos, life: i32) {
    let s = st_mut(m);
    s.owner = Some(owner);
    s.owner_uuid = (owner_uuid != 0).then_some(owner_uuid);
    s.bound_origin = Some(origin);
    s.limited_life = Some(life);
}

impl Kind for Vex {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(VexState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(4, Goal::Custom(Box::new(ChargeAttackGoal)));
        g.add(8, Goal::Custom(Box::new(RandomMoveGoal)));
        g.add(9, Goal::LookAtPlayer { dist: 3.0, probability: 1.0, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(8.0))));
        let t = &mut m.targets;
        t.add(1, raider::hurt_by_ignoring_raiders());
        t.add(2, Goal::Custom(Box::new(CopyOwnerTargetGoal { unseen: 0 })));
        t.add(3, super::zombie::nearest(Wanted::Player, true));
    }

    /// `Vex.tick` before `super.tick()`: no physics while it ticks.
    fn pre_tick(&self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        e.no_physics = true;
    }

    /// `Vex.tick` after `super.tick()`: no gravity from now on; the limited life takes 1 a second
    /// once it ran out.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        e.no_physics = false;
        e.no_gravity = true;
        let hurt = match st_mut(m).limited_life.as_mut() {
            Some(t) => {
                *t -= 1;
                if *t <= 0 {
                    *t = 20;
                    true
                } else {
                    false
                }
            }
            None => false,
        };
        if hurt {
            mob::hurt(e, m, level, DamageSource::of(DamageKind::Starve), 1.0);
        }
    }

    /// `VexMoveControl.tick`: flies straight at the wanted position, facing its target (or where
    /// it goes).
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.mov.operation != Operation::MoveTo {
            return true;
        }
        let [wx, wy, wz] = m.mov.wanted;
        let d = Vec3::new(wx - e.x(), wy - e.y(), wz - e.z());
        let len = d.length();
        let b = e.bounding_box();
        let size = (b.x_size() + b.y_size() + b.z_size()) / 3.0;
        if len < size {
            m.mov.operation = Operation::Wait;
            e.delta = e.delta.scale(0.5);
        } else {
            e.delta = e.delta + d.scale(m.mov.speed_modifier * 0.05 / len);
            let yaw = match goals::target(m, level) {
                None => -(mth::atan2(e.delta.x, e.delta.z) as f32) * (180.0f32 / std::f32::consts::PI),
                Some(t) => -(mth::atan2(t.pos.x - e.x(), t.pos.z - e.z()) as f32) * (180.0f32 / std::f32::consts::PI),
            };
            e.y_rot = yaw;
            m.y_body_rot = yaw;
        }
        true
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        m.equipment[MAINHAND] = ItemStack::of("minecraft:iron_sword", 1).unwrap_or_else(ItemStack::empty);
        m.drop_chances[MAINHAND] = 0.0;
        super::zombie::populate_enchantments(m, r, ctx);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let bound = match r.get("bound_pos") {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        };
        let life = r.num("life_ticks").map(|v| v as i32);
        let owner = r.uuid("owner");
        let s = st_mut(m);
        s.bound_origin = bound;
        s.limited_life = life;
        s.owner_uuid = owner;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        if let Some(p) = s.bound_origin {
            o.put("bound_pos", Tag::IntArray(vec![p.x, p.y, p.z]));
        }
        if let Some(t) = s.limited_life {
            o.put("life_ticks", Tag::Int(t));
        }
        if let Some(u) = s.owner_uuid {
            o.put("owner", crate::persist::uuid_to_tag(u));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::vex::FLAGS, &DataValue::Byte(st(m).charging as i8));
    }

    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(super::evoker::experience_with_equipment(e, m, 3))
    }
}

/// `Vex.VexChargeAttackGoal`: now and then rushes at its target's eyes and hits it on contact.
#[derive(Clone, Debug)]
struct ChargeAttackGoal;

impl CustomGoal for ChargeAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "VexChargeAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = goals::target(m, level) else { return false };
        if !t.alive || m.mov.has_wanted() || e.random.next_int_bounded(reduced_tick_delay(7)) != 0 {
            return false;
        }
        e.position().distance_to_sqr(t.pos) > 4.0
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        m.mov.has_wanted() && st(m).charging && goals::target(m, level).is_some_and(|t| t.alive)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(t) = goals::target(m, level) {
            m.mov.set_wanted_position(t.pos.x, t.eye_y, t.pos.z, 1.0);
        }
        st_mut(m).charging = true;
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event("minecraft:entity.vex.charge"), source: "hostile", volume: 1.0, pitch: 1.0 });
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).charging = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        if e.bounding_box().intersects(&t.bb) {
            mob::do_hurt_target(e, m, level, &t);
            st_mut(m).charging = false;
        } else if e.position().distance_to_sqr(t.pos) < 9.0 {
            m.mov.set_wanted_position(t.pos.x, t.eye_y, t.pos.z, 1.0);
        }
    }
}

/// `Vex.VexRandomMoveGoal`: drifts to an empty spot within 7 of its bound origin.
#[derive(Clone, Debug)]
struct RandomMoveGoal;

impl CustomGoal for RandomMoveGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "VexRandomMoveGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.mov.has_wanted() && e.random.next_int_bounded(reduced_tick_delay(7)) == 0
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let origin = st(m).bound_origin.unwrap_or_else(|| e.block_position());
        for _ in 0..3 {
            let dx = e.random.next_int_bounded(15) - 7;
            let dy = e.random.next_int_bounded(11) - 5;
            let dz = e.random.next_int_bounded(15) - 7;
            let p = origin.offset(dx, dy, dz);
            if kiln_data::blocks_types::is_air(level.block(p)) {
                let (x, y, z) = (p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
                m.mov.set_wanted_position(x, y, z, 0.25);
                if goals::target(m, level).is_none() {
                    m.look.set_look_at(x, y, z, 180.0, 20.0);
                }
                break;
            }
        }
    }
}

/// `Vex.VexCopyOwnerTargetGoal`: takes its evoker's target.
#[derive(Clone, Debug)]
struct CopyOwnerTargetGoal {
    unseen: i32,
}

fn owner_target(m: &MobData, level: &dyn EntityLevel) -> Option<i32> {
    let o = level.entity(owner(m)?)?;
    let om = mob::data(o)?;
    let t = goals::living(level, om.target?)?;
    // `copyOwnerTargeting`: non-combat, seen by anyone.
    (t.alive && !t.spectator).then_some(t.id)
}

impl CustomGoal for CopyOwnerTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "VexCopyOwnerTargetGoal"
    }
    /// `TargetGoal` sets no flags: the nearest player goal still looks alongside it.
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        owner_target(m, level).is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, None, false, &mut self.unseen, 60)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let t = owner_target(m, level);
        mob::set_target(e, m, t);
        self.unseen = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, None);
    }
}
