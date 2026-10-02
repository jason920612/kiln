//! `TamableAnimal` (wolves, cats): the owner, the tame and sitting flags, teleporting to the
//! owner, and the goals tamed animals share (`SitWhenOrderedToGoal`, `FollowOwnerGoal`,
//! `OwnerHurtByTargetGoal`, `OwnerHurtTargetGoal`, `NonTameRandomTargetGoal`,
//! `TamableAnimal.TamableAnimalPanicGoal`).

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::ext::{self, CustomGoal};
use crate::mob::goals::{self, Living, MOVE, JUMP, LOOK, TARGET};
use crate::mob::mth::{next_int_between, reduced_tick_delay};
use crate::mob::path::{self, PathType};
use crate::mob::{MobData, MobKind, random_pos};
use crate::persist::{Input, Output, uuid_to_tag};
use kiln_data::entities::data;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

/// The `TamableAnimal` state.
#[derive(Clone, Debug, Default)]
pub struct Tame {
    /// `DATA_OWNERUUID_ID`.
    pub owner: Option<u128>,
    /// `DATA_FLAGS_ID` bit 4.
    pub tame: bool,
    /// `DATA_FLAGS_ID` bit 1 (`isInSittingPose`).
    pub sitting: bool,
    pub ordered_to_sit: bool,
}

/// The tame state of a tamable mob.
pub fn get(m: &MobData) -> Option<&Tame> {
    match m.kind {
        MobKind::Nautilus | MobKind::ZombieNautilus => super::nautilus::tame_of(m),
        MobKind::Wolf => ext::state::<super::wolf::State>(m).map(|s| &s.tame),
        MobKind::Cat => ext::state::<super::cat::State>(m).map(|s| &s.tame),
        MobKind::Parrot => ext::state::<super::parrot::State>(m).map(|s| &s.tame),
        _ => None,
    }
}

pub fn get_mut(m: &mut MobData) -> Option<&mut Tame> {
    match m.kind {
        MobKind::Nautilus | MobKind::ZombieNautilus => super::nautilus::tame_of_mut(m),
        MobKind::Wolf => ext::state_mut::<super::wolf::State>(m).map(|s| &mut s.tame),
        MobKind::Cat => ext::state_mut::<super::cat::State>(m).map(|s| &mut s.tame),
        MobKind::Parrot => ext::state_mut::<super::parrot::State>(m).map(|s| &mut s.tame),
        _ => None,
    }
}

pub fn is_tame(m: &MobData) -> bool {
    get(m).is_some_and(|t| t.tame)
}

pub fn ordered_to_sit(m: &MobData) -> bool {
    get(m).is_some_and(|t| t.ordered_to_sit)
}

pub fn set_ordered_to_sit(m: &mut MobData, on: bool) {
    if let Some(t) = get_mut(m) {
        t.ordered_to_sit = on;
    }
}

pub fn set_sitting(m: &mut MobData, on: bool) {
    if let Some(t) = get_mut(m) {
        t.sitting = on;
    }
}

/// `getOwner`: the owner player, if it is in the level.
pub fn owner(m: &MobData, level: &dyn EntityLevel) -> Option<PlayerView> {
    let u = get(m)?.owner?;
    level.player_by_uuid(u)
}

/// `isOwnedBy(player)`.
pub fn owned_by(m: &MobData, level: &dyn EntityLevel, player: i32) -> bool {
    let (Some(u), Some(p)) = (get(m).and_then(|t| t.owner), level.player(player)) else { return false };
    p.uuid == u
}

/// `tame(player)`: tame, owned by `player` (the type applies its side effects).
pub fn tame(m: &mut MobData, owner: u128) {
    if let Some(t) = get_mut(m) {
        t.tame = true;
        t.owner = Some(owner);
    }
}

fn dist_sqr(e: &Entity, p: Vec3) -> f64 {
    e.position().distance_to_sqr(p)
}

/// `unableToMoveToOwner`.
pub fn unable_to_move_to_owner(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    // (`mayBeLeashed`: a lead on it, or a lead data waiting for its holder.)
    ordered_to_sit(m) || e.vehicle.is_some() || e.leash.is_some() || owner(m, level).is_some_and(|o| o.spectator)
}

/// `shouldTryTeleportToOwner`: 12 blocks or more away.
pub fn should_try_teleport_to_owner(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    owner(m, level).is_some_and(|o| dist_sqr(e, o.pos) >= 144.0)
}

/// `tryToTeleportToOwner`: up to ten tries at a spot 2 to 3 blocks from the owner.
pub fn try_to_teleport_to_owner(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let Some(o) = owner(m, level) else { return };
    let pos = BlockPos::containing(o.pos.x, o.pos.y, o.pos.z);
    for _ in 0..10 {
        let dx = next_int_between(&mut e.random, -3, 3);
        let dz = next_int_between(&mut e.random, -3, 3);
        if dx.abs() < 2 && dz.abs() < 2 {
            continue;
        }
        let dy = next_int_between(&mut e.random, -1, 1);
        let p = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
        if can_teleport_to(e, level, p, m.kind == MobKind::Parrot) {
            e.set_pos(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5));
            e.set_old_pos_and_rot();
            m.nav.stop();
            return;
        }
    }
}

/// `canTeleportTo`: a walkable spot, not on leaves, where the mob fits.
fn can_teleport_to(e: &Entity, level: &dyn EntityLevel, p: BlockPos, can_fly_to_owner: bool) -> bool {
    if path::path_type_static(level, p.x, p.y, p.z) != PathType::Walkable {
        return false;
    }
    // `instanceof LeavesBlock` (unless the animal `canFlyToOwner`: parrots).
    if !can_fly_to_owner && crate::blocks::block_name(level.block(p.below())).ends_with("_leaves") {
        return false;
    }
    let b = e.block_position();
    let bb = e.bounding_box().offset((p.x - b.x) as f64, (p.y - b.y) as f64, (p.z - b.z) as f64);
    crate::collision::no_collision(level, &e.collision_context(), e.id, &bb)
}

/// `setPathfindingMalus`.
pub fn set_malus(m: &mut MobData, t: PathType, v: f32) {
    match m.maluses.iter_mut().find(|(k, _)| *k == t) {
        Some(slot) => slot.1 = v,
        None => m.maluses.push((t, v)),
    }
}

/// `TamableAnimal.readAdditionalSaveData`; returns whether the taming side effects apply
/// (no owner: `setTame(false, true)`).
pub fn load(t: &mut Tame, r: &mut Input) -> bool {
    t.owner = r.uuid("Owner");
    t.tame = t.owner.is_some();
    t.ordered_to_sit = r.bool_or("Sitting", false);
    t.sitting = t.ordered_to_sit;
    !t.tame
}

pub fn save(t: &Tame, o: &mut Output) {
    if let Some(u) = t.owner {
        o.put("Owner", uuid_to_tag(u));
    }
    o.put("Sitting", Tag::Byte(t.ordered_to_sit as i8));
}

pub fn entity_data(t: &Tame, d: &mut EntityData) {
    let flags = (t.sitting as i8) | if t.tame { 4 } else { 0 };
    if flags != 0 {
        d.set(data::tamable_animal::FLAGS, &DataValue::Byte(flags));
    }
    if let Some(u) = t.owner {
        d.set(data::tamable_animal::OWNERUUID, &DataValue::OptionalEntityReference(Some(uuid::Uuid::from_u128(u))));
    }
}

/// `TargetingConditions.DEFAULT` (combat, line of sight, any range) against `id`.
fn can_attack_default(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, id: Option<i32>) -> Option<Living> {
    let t = goals::living(level, id?)?;
    goals::targeting_ok(e, m, level, &t, true, -1.0, true).then_some(t)
}

/// `wantsToAttack(target, owner)` of the type.
fn wants_to_attack(m: &MobData, level: &dyn EntityLevel, t: &Living, owner: &PlayerView) -> bool {
    match m.kind {
        MobKind::Wolf => super::wolf::wants_to_attack(level, t, owner),
        _ => true,
    }
}

// ---------------------------------------------------------------------- goals

/// `SitWhenOrderedToGoal`.
#[derive(Clone, Debug)]
pub struct SitWhenOrderedToGoal;

impl CustomGoal for SitWhenOrderedToGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SitWhenOrderedToGoal"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let ordered = ordered_to_sit(m);
        if !ordered && !is_tame(m) {
            return false;
        }
        if e.is_in_water() || !e.on_ground {
            return false;
        }
        let Some(o) = owner(m, level) else { return true };
        if dist_sqr(e, o.pos) < 144.0 && o.last_hurt_by_mob.is_some() {
            return false;
        }
        ordered
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        ordered_to_sit(m)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
        set_sitting(m, true);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_sitting(m, false);
    }
}

/// `FollowOwnerGoal`.
#[derive(Clone, Debug)]
pub struct FollowOwnerGoal {
    pub speed: f64,
    pub start_distance: f32,
    pub stop_distance: f32,
    owner: Option<i32>,
    recalc: i32,
    old_water_cost: f32,
}

impl FollowOwnerGoal {
    pub fn new(speed: f64, start_distance: f32, stop_distance: f32) -> FollowOwnerGoal {
        FollowOwnerGoal { speed, start_distance, stop_distance, owner: None, recalc: 0, old_water_cost: 0.0 }
    }
}

impl CustomGoal for FollowOwnerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FollowOwnerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(o) = owner(m, level) else { return false };
        if unable_to_move_to_owner(e, m, level) {
            return false;
        }
        if dist_sqr(e, o.pos) < (self.start_distance * self.start_distance) as f64 {
            return false;
        }
        self.owner = Some(o.id);
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.nav.is_done() || unable_to_move_to_owner(e, m, level) {
            return false;
        }
        let Some(o) = self.owner.and_then(|id| goals::living(level, id)) else { return false };
        !(dist_sqr(e, o.pos) <= (self.stop_distance * self.stop_distance) as f64)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.recalc = 0;
        self.old_water_cost = path::malus(m, PathType::Water);
        set_malus(m, PathType::Water, 0.0);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.owner = None;
        m.nav.stop();
        set_malus(m, PathType::Water, self.old_water_cost);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let teleport = should_try_teleport_to_owner(e, m, level);
        let Some(o) = self.owner.and_then(|id| goals::living(level, id)) else { return };
        if !teleport {
            let max_x = m.max_head_x_rot() as f32;
            m.look.set_look_at(o.pos.x, o.eye_y, o.pos.z, 10.0, max_x);
        }
        self.recalc -= 1;
        if self.recalc > 0 {
            return;
        }
        self.recalc = reduced_tick_delay(10);
        if teleport {
            try_to_teleport_to_owner(e, m, level);
        } else {
            let p = BlockPos::containing(o.pos.x, o.pos.y, o.pos.z);
            path::move_to_entity(e, m, level, p, self.speed);
        }
    }
}

/// `OwnerHurtByTargetGoal` (`hurt_by`) and `OwnerHurtTargetGoal` (not `hurt_by`): the mob the
/// owner was last hurt by, or last hurt, becomes the target.
#[derive(Clone, Debug)]
pub struct OwnerTargetGoal {
    pub hurt_by: bool,
    candidate: Option<i32>,
    timestamp: i32,
    target_mob: Option<i32>,
    unseen: i32,
}

impl OwnerTargetGoal {
    pub fn new(hurt_by: bool) -> OwnerTargetGoal {
        OwnerTargetGoal { hurt_by, candidate: None, timestamp: 0, target_mob: None, unseen: 0 }
    }
}

impl CustomGoal for OwnerTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        if self.hurt_by { "OwnerHurtByTargetGoal" } else { "OwnerHurtTargetGoal" }
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !is_tame(m) || ordered_to_sit(m) {
            return false;
        }
        let Some(o) = owner(m, level) else { return false };
        let (who, ts) = if self.hurt_by {
            if !o.hurt_recently {
                return false;
            }
            (o.last_hurt_by_mob, o.last_hurt_by_mob_time)
        } else {
            (o.last_hurt_mob, o.last_hurt_mob_time)
        };
        self.candidate = who;
        if ts == self.timestamp {
            return false;
        }
        let Some(t) = can_attack_default(e, m, level, who) else { return false };
        wants_to_attack(m, level, &t, &o)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, self.target_mob, false, &mut self.unseen, 60)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.target = self.candidate;
        if let Some(o) = owner(m, level) {
            self.timestamp = if self.hurt_by { o.last_hurt_by_mob_time } else { o.last_hurt_mob_time };
        }
        self.unseen = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
        self.target_mob = None;
    }
}

/// `NonTameRandomTargetGoal`: a `NearestAttackableTargetGoal` (interval 10) for mobs of
/// `types` (empty: a type Kiln does not simulate) while the animal is not tame.
#[derive(Clone, Debug)]
pub struct NonTameRandomTargetGoal {
    pub types: &'static [&'static str],
    pub must_see: bool,
    target: Option<i32>,
    unseen: i32,
}

impl NonTameRandomTargetGoal {
    pub fn new(types: &'static [&'static str], must_see: bool) -> NonTameRandomTargetGoal {
        NonTameRandomTargetGoal { types, must_see, target: None, unseen: 0 }
    }
}

impl CustomGoal for NonTameRandomTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NonTameRandomTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if is_tame(m) {
            return false;
        }
        if e.random.next_int_bounded(reduced_tick_delay(10)) != 0 {
            return false;
        }
        let range = m.attrs.value(crate::mob::attributes::Attr::FollowRange);
        self.target = if self.types.is_empty() { None } else { goals::nearest_mob(e, m, level, range, true, self.types) };
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        // `targetConditions.test(mob, target)`: combat, the follow range, line of sight.
        let range = m.attrs.value(crate::mob::attributes::Attr::FollowRange);
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return false };
        goals::targeting_ok(e, m, level, &t, true, range, true)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = self.target;
        self.unseen = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
        self.target = None;
    }
}

/// `TamableAnimal.TamableAnimalPanicGoal`: a `PanicGoal` for the damage types of `tag` that
/// teleports to a far owner while running.
#[derive(Clone, Debug)]
pub struct TamableAnimalPanicGoal {
    pub speed: f64,
    pub tag: &'static str,
    /// The goal's class (`AbstractHorse.MountPanicGoal` is the same `PanicGoal`).
    pub name: &'static str,
    pos: Vec3,
}

impl TamableAnimalPanicGoal {
    pub fn new(speed: f64, tag: &'static str) -> TamableAnimalPanicGoal {
        TamableAnimalPanicGoal { speed, tag, name: "TamableAnimalPanicGoal", pos: Vec3::ZERO }
    }

    /// A plain `PanicGoal` subclass named `name` (no owner to teleport to).
    pub fn named(name: &'static str, speed: f64, tag: &'static str) -> TamableAnimalPanicGoal {
        TamableAnimalPanicGoal { speed, tag, name, pos: Vec3::ZERO }
    }
}

impl CustomGoal for TamableAnimalPanicGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !m.last_damage_source(level.game_time()).is_some_and(|s| s.kind.is_tag(self.tag)) {
            return false;
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
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !unable_to_move_to_owner(e, m, level) && should_try_teleport_to_owner(e, m, level) {
            try_to_teleport_to_owner(e, m, level);
        }
    }
}
