//! Warden: listens for vibrations, grows angry at their sources (`AngerManagement`), sniffs
//! out nearby entities, roars at the one it is angry at, then hunts it with melee hits and
//! sonic booms through armor; pulses darkness; emerges from the ground when a shrieker
//! summons it and digs back down after a minute without a disturbance.
//!
//! Driven by the brain of `WardenAi` on [`crate::mob::brain`]: the activities emerge, dig,
//! roar, fight, investigate, sniff and idle (the first valid one wins, chosen at the end of
//! `customServerAiStep`), the memories `IS_EMERGING`, `DIG_COOLDOWN`, `ROAR_TARGET`,
//! `DISTURBANCE_LOCATION`, `SNIFF_COOLDOWN`, `SONIC_BOOM_*`, ... with their expiry, the
//! `WardenEntitySensor` next to the player sensor, and the behaviours in vanilla's priority
//! order. What stays in [`WardenState`] is what is not a memory in vanilla: the pose, the
//! `AngerManagement`, and the vibration listener (`VibrationSystem.Data` at its eyes, 16 blocks,
//! `#warden_can_listen`), ticked before the warden's tick like vanilla's.

use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat;
use crate::mob::brain::sensors::{self, Players};
use crate::mob::brain::util::{self, uniform};
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Control, Cx, Gate, Mem, Memories, Sensor, Status, Timed, Tracker, Val, shot};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{self, Living};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, control, path};
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use crate::vibration::{self, Ear, VibrationData};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{Registered, ValueAbsent, ValuePresent};

pub struct Warden;

pub static KIND: Warden = Warden;

static INFO: Info = Info {
    fire_immune: true,
    sounds: Some("warden"),
    ..Info::monster(
        "minecraft:warden",
        &[(MaxHealth, 500.0), (MovementSpeed, 0.30000001192092896), (KnockbackResistance, 1.0), (AttackKnockback, 1.5), (AttackDamage, 30.0), (FollowRange, 24.0)],
    )
};

/// `WardenAi` durations (`Mth.ceil` of 133.6, 100, 84, 83.2 and, for the sonic boom, 60 and 34).
pub const EMERGE_DURATION: i32 = 134;
pub const DIGGING_DURATION: i32 = 100;
pub const ROAR_DURATION: i32 = 84;
pub const SNIFFING_DURATION: i32 = 84;
pub const DIGGING_COOLDOWN: i32 = 1200;
const SONIC_BOOM_DURATION: i32 = 60;
const SONIC_BOOM_SOUND_DELAY: i32 = 34;
const ROAR_SOUND_DELAY: i32 = 25;
const MELEE_COOLDOWN: i32 = 18;
/// `AngerLevel` thresholds.
const AGITATED: i32 = 40;
const ANGRY: i32 = 80;
const MAX_ANGER: i32 = 150;

/// The warden's `Pose` (the entity data the client animates).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Standing,
    Emerging,
    Digging,
    Roaring,
    Sniffing,
}

impl Pose {
    fn id(self) -> i32 {
        use kiln_data::entities::pose;
        match self {
            Pose::Standing => pose::STANDING,
            Pose::Emerging => pose::EMERGING,
            Pose::Digging => pose::DIGGING,
            Pose::Roaring => pose::ROARING,
            Pose::Sniffing => pose::SNIFFING,
        }
    }
}

/// `AngerManagement`: anger per suspect (at most 150), sorted with the angry ones first, then
/// players, then by anger.
#[derive(Clone, Debug, Default)]
pub struct AngerManagement {
    /// (entity id, uuid, is a player, anger) in sorted order.
    pub suspects: Vec<(i32, u128, bool, i32)>,
    /// Anger at entities not loaded yet (saved by UUID).
    pub by_uuid: Vec<(u128, i32)>,
    pub highest: i32,
}

impl AngerManagement {
    fn anger_of(&self, id: i32) -> i32 {
        self.suspects.iter().find(|s| s.0 == id).map_or(0, |s| s.3)
    }

    /// `sortAndUpdateHighestAnger`.
    fn sort(&mut self) {
        self.suspects.sort_by(|a, b| {
            let (angry_a, angry_b) = (a.3 >= ANGRY, b.3 >= ANGRY);
            angry_b.cmp(&angry_a).then(b.2.cmp(&a.2)).then(b.3.cmp(&a.3))
        });
        self.highest = self.suspects.iter().map(|s| s.3).max().unwrap_or(0);
    }

    /// `increaseAnger`: the new anger.
    pub fn increase(&mut self, id: i32, uuid: u128, player: bool, amount: i32) -> i32 {
        let anger = match self.suspects.iter_mut().find(|s| s.0 == id) {
            Some(s) => {
                s.3 = (s.3 + amount).min(MAX_ANGER);
                s.3
            }
            None => {
                let saved = self.by_uuid.iter().position(|u| u.0 == uuid).map_or(0, |i| self.by_uuid.remove(i).1);
                let a = amount.min(MAX_ANGER) + saved;
                self.suspects.push((id, uuid, player, a));
                a
            }
        };
        self.sort();
        anger
    }

    /// `clearAnger`.
    pub fn clear(&mut self, id: i32) {
        self.suspects.retain(|s| s.0 != id);
        self.sort();
    }

    /// `tick` (every 20 ticks): anger drops by one; suspects that left go.
    fn tick(&mut self, valid: impl Fn(i32) -> bool) {
        self.by_uuid.retain_mut(|u| {
            u.1 -= 1;
            u.1 > 0
        });
        self.suspects.retain_mut(|s| {
            if s.3 > 1 && valid(s.0) {
                s.3 -= 1;
                true
            } else {
                false
            }
        });
        self.sort();
    }

    /// `getActiveAnger(target)`.
    pub fn active_anger(&self, target: Option<i32>) -> i32 {
        match target {
            Some(t) => self.anger_of(t),
            None => self.highest,
        }
    }

    /// `getTopSuspect` with the warden's filter.
    fn top(&self, ok: impl Fn(i32) -> bool) -> Option<i32> {
        self.suspects.iter().map(|s| s.0).find(|&id| ok(id))
    }

    fn to_nbt(&self) -> Tag {
        let uuid = |u: u128| Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32]);
        let list = self
            .suspects
            .iter()
            .map(|s| (s.1, s.3))
            .chain(self.by_uuid.iter().copied())
            .map(|(u, a)| Tag::Compound(vec![("uuid".into(), uuid(u)), ("anger".into(), Tag::Int(a))]))
            .collect();
        Tag::Compound(vec![("suspects".into(), Tag::List(list))])
    }

    fn from_nbt(t: Option<&Tag>) -> AngerManagement {
        let mut a = AngerManagement::default();
        for s in t.and_then(|t| t.get("suspects")).and_then(Tag::as_list).unwrap_or(&[]) {
            let s = s.unwrap_list_element();
            let (Some(Tag::IntArray(u)), Some(anger)) = (s.get("uuid"), s.get("anger").and_then(Tag::as_i64)) else { continue };
            if u.len() == 4 && anger >= 0 {
                let uuid = u.iter().fold(0u128, |acc, &x| (acc << 32) | x as u32 as u128);
                a.by_uuid.push((uuid, anger as i32));
            }
        }
        a
    }
}

/// The warden's state that is not a brain memory: its pose, listener and anger.
#[derive(Clone, Debug, Default)]
pub struct WardenState {
    pub pose: Pose,
    pub vibration: VibrationData,
    pub anger: AngerManagement,
    /// `CLIENT_ANGER_LEVEL` as last synced.
    pub client_anger: i32,
    /// `doHurtTarget` landed a hit: its sonic boom cooldown is set once the brain is back.
    melee_hit: bool,
}

pub fn state(m: &MobData) -> Option<&WardenState> {
    ext::state::<WardenState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut WardenState> {
    ext::state_mut::<WardenState>(m)
}

fn digging_or_emerging(m: &MobData) -> bool {
    state(m).is_some_and(|s| matches!(s.pose, Pose::Digging | Pose::Emerging))
}

fn has_pose(m: &MobData, pose: Pose) -> bool {
    state(m).is_some_and(|s| s.pose == pose)
}

/// `Entity.setPose`: the entity data changes and the size follows (`refreshDimensions`).
fn set_pose(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, pose: Pose) {
    let changed = state_mut(m).is_some_and(|s| {
        let c = s.pose != pose;
        s.pose = pose;
        c
    });
    if changed {
        e.needs_sync = true;
        mob::refresh_dimensions_in(e, m, level);
    }
}

/// Runs `f` with the brain's memories while the brain is in the mob (outside of its tick).
fn with_mem<R>(m: &mut MobData, f: impl FnOnce(&mut MobData, &mut Memories) -> R) -> Option<R> {
    let mut b = m.brain.take()?;
    let r = f(m, &mut b.st.mem);
    m.brain = Some(b);
    Some(r)
}

/// `Warden.canTargetEntity` on a living view: not creative, a spectator, invulnerable, dead, an
/// armor stand or another warden.
fn targetable(t: &Living) -> bool {
    t.alive && !t.creative && !t.spectator && !t.invulnerable && !matches!(t.type_name, "minecraft:armor_stand" | "minecraft:warden")
}

/// `Warden.canTargetEntity`.
fn can_target(level: &dyn EntityLevel, id: i32) -> Option<Living> {
    let t = goals::living(level, id)?;
    targetable(&t).then_some(t)
}

/// `Mob.getTarget` (`getTargetFromBrain`): the attack target memory while it can be attacked.
fn get_target(mem: &Memories, level: &dyn EntityLevel) -> Option<i32> {
    mem.entity(Mem::AttackTarget).filter(|&t| can_target(level, t).is_some())
}

/// The mob's `getTarget` mirrored for the code that reads `MobData::target`.
fn sync_target(m: &mut MobData) {
    if let Some(b) = m.brain.as_ref() {
        m.target = b.st.mem.entity(Mem::AttackTarget);
    }
}

fn uuid_of(level: &dyn EntityLevel, id: i32) -> u128 {
    level.entity(id).map(|e| e.uuid).or_else(|| level.player(id).map(|p| p.uuid)).unwrap_or(0)
}

/// `getVoicePitch`.
fn voice_pitch(e: &mut Entity) -> f32 {
    (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0
}

fn sound(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume, pitch });
    }
}

/// `WardenAi.setDigCooldown`: refreshed only while present.
fn refresh_dig_cooldown(mem: &mut Memories) {
    if mem.has(Mem::DigCooldown) {
        mem.set_expiring(Mem::DigCooldown, Val::Unit, DIGGING_COOLDOWN as i64);
    }
}

/// `SonicBoom.setCooldown`.
fn set_sonic_cooldown(mem: &mut Memories, ticks: i32) {
    mem.set_expiring(Mem::SonicBoomCooldown, Val::Unit, ticks as i64);
}

/// `Warden.getActiveAnger`.
fn active_anger(m: &MobData, mem: &Memories, level: &dyn EntityLevel) -> i32 {
    state(m).map_or(0, |s| s.anger.active_anger(get_target(mem, level)))
}

/// `Warden.getEntityAngryAt`: the top suspect when angry.
fn entity_angry_at(m: &MobData, mem: &Memories, level: &dyn EntityLevel) -> Option<i32> {
    let s = state(m)?;
    if s.anger.active_anger(get_target(mem, level)) < ANGRY {
        return None;
    }
    s.anger.top(|id| can_target(level, id).is_some())
}

/// `Warden.playListeningSound`.
fn play_listening_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel, mem: &Memories) {
    if has_pose(m, Pose::Roaring) {
        return;
    }
    let listening = if active_anger(m, mem, level) >= AGITATED { "minecraft:entity.warden.listening_angry" } else { "minecraft:entity.warden.listening" };
    let pitch = voice_pitch(e);
    sound(e, level, listening, 10.0, pitch);
}

/// `Warden.increaseAngerAt(entity, amount, playSound)`.
fn increase_anger_at(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &mut Memories, target: i32, amount: i32, play_sound: bool) {
    if m.no_ai || can_target(&*level, target).is_none() {
        return;
    }
    refresh_dig_cooldown(mem);
    let target_is_player = get_target(mem, &*level).is_some_and(|t| level.player(t).is_some());
    let player = level.player(target).is_some();
    let uuid = uuid_of(&*level, target);
    let Some(s) = state_mut(m) else { return };
    let anger = s.anger.increase(target, uuid, player, amount);
    if player && !target_is_player && anger >= ANGRY {
        mem.erase(Mem::AttackTarget);
    }
    if play_sound {
        play_listening_sound(e, m, level, mem);
    }
}

/// `WardenAi.setDisturbanceLocation`.
fn set_disturbance_location(m: &MobData, level: &dyn EntityLevel, mem: &mut Memories, pos: BlockPos) {
    if entity_angry_at(m, mem, level).is_some() || mem.has(Mem::AttackTarget) {
        return;
    }
    refresh_dig_cooldown(mem);
    mem.set_expiring(Mem::SniffCooldown, Val::Unit, 100);
    mem.set_expiring(Mem::LookTarget, Val::Look(Tracker::block(pos)), 100);
    mem.set_expiring(Mem::DisturbanceLocation, Val::Block(pos), 100);
    mem.erase(Mem::WalkTarget);
}

/// `Warden.setAttackTarget`.
fn set_attack_target(mem: &mut Memories, target: i32) {
    mem.erase(Mem::RoarTarget);
    mem.set(Mem::AttackTarget, Val::Entity(target));
    mem.erase(Mem::CantReachWalkTargetSince);
    set_sonic_cooldown(mem, 200);
}

/// `closerThan(target, xz, y)`.
fn closer_than(e: &Entity, t: Vec3, xz: f64, y: f64) -> bool {
    let (dx, dz) = (t.x - e.x(), t.z - e.z());
    dx * dx + dz * dz < xz * xz && (t.y - e.y()).abs() < y
}

fn block_of(t: &Living) -> BlockPos {
    BlockPos::containing(t.pos.x, t.pos.y, t.pos.z)
}

/// `VibrationSystem.Ticker.tick` for the warden (before its tick), then its listener for the
/// dispatcher.
fn tick_vibrations(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &mut Memories) {
    let heard = level.take_vibrations(e.id);
    let now = level.game_time();
    let eyes = Vec3::new(e.x(), e.eye_y(), e.z());
    let Some(s) = state_mut(m) else { return };
    for h in heard {
        // The listener's checks ran when it was heard; the selector takes it now.
        s.vibration.schedule(h.event, h.from, h.to, h.source, None, h.tick);
    }
    if s.vibration.current.is_none() && s.vibration.selector.current.is_none() {
        return;
    }
    let t = s.vibration.tick(now, eyes, vibration::travel_time);
    let eye_height = e.eye_height;
    for (from, ticks) in t.particles {
        level.vibration_particle(from, e.id, eye_height, ticks);
    }
    if !t.arrived {
        return;
    }
    let Some(info) = state(m).and_then(|s| s.vibration.current.clone()) else { return };
    on_receive(e, m, level, mem, &info);
    if let Some(s) = state_mut(m) {
        s.vibration.received();
    }
}

/// `Warden.VibrationUser.onReceiveVibration`.
fn on_receive(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &mut Memories, info: &vibration::VibrationInfo) {
    if m.is_dead_or_dying() {
        return;
    }
    let origin = BlockPos::containing(info.pos.x, info.pos.y, info.pos.z);
    mem.set_expiring(Mem::VibrationCooldown, Val::Unit, 40);
    level.emit(Event::EntityEvent { entity: e.id, event: 61 });
    let pitch = voice_pitch(e);
    sound(e, level, "minecraft:entity.warden.tendril_clicks", 5.0, pitch);
    let source = info.source.map(|s| s.id).filter(|&id| goals::living(&*level, id).is_some() || level.entity(id).is_some());
    let owner = info.source.and_then(|s| s.projectile_owner).filter(|&o| goals::living(&*level, o).is_some());
    let mut suspicious = origin;
    match owner {
        Some(o) => {
            let near = goals::living(&*level, o).is_some_and(|t| t.pos.distance_to_sqr(e.position()) < 30.0 * 30.0);
            if near {
                if mem.has(Mem::RecentProjectile) {
                    if let Some(t) = can_target(&*level, o) {
                        suspicious = block_of(&t);
                    }
                    increase_anger_at(e, m, level, mem, o, 35, true);
                } else {
                    increase_anger_at(e, m, level, mem, o, 10, true);
                }
            }
            mem.set_expiring(Mem::RecentProjectile, Val::Unit, 100);
        }
        None => {
            if let Some(src) = source {
                increase_anger_at(e, m, level, mem, src, 35, true);
            }
        }
    }
    if active_anger(m, mem, &*level) < ANGRY {
        let active = state(m).and_then(|s| s.anger.top(|id| can_target(&*level, id).is_some()));
        if owner.is_some() || active.is_none() || active == source {
            set_disturbance_location(m, &*level, mem, suspicious);
        }
    }
}

/// The warden's `DynamicGameEventListener` after its tick.
fn update_listener(e: &Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if e.is_removed() || m.is_dead_or_dying() {
        level.set_listener(e.id, None);
        return;
    }
    let Some(s) = state(m) else { return };
    let cooldown = m.brain.as_ref().is_some_and(|b| b.st.mem.has(Mem::VibrationCooldown));
    let ear = Ear {
        pos: Vec3::new(e.x(), e.eye_y(), e.z()),
        busy: s.vibration.current.is_some(),
        can_hear: !m.no_ai && !cooldown && !matches!(s.pose, Pose::Digging | Pose::Emerging),
    };
    level.set_listener(e.id, Some(ear));
}

// ---------------------------------------------------------------------------- sensors

/// `WardenEntitySensor`: `NearestLivingEntitySensor`, then the nearest targetable player within
/// its follow range, else the nearest other targetable living entity, as `NEAREST_ATTACKABLE`.
#[derive(Clone, Debug)]
struct WardenEntitySensor;

impl Sensor for WardenEntitySensor {
    fn name(&self) -> &'static str {
        "WardenEntitySensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestLivingEntities, Mem::NearestVisibleLivingEntities, Mem::NearestAttackable]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        sensors::NearestLivingEntities.do_tick(cx);
        let list = cx.b.mem.entities(Mem::NearestLivingEntities);
        let find = |want_player: bool| list.iter().copied().find(|&id| can_target(&*cx.level, id).is_some_and(|l| l.player == want_player));
        let found = find(true).or_else(|| find(false));
        cx.b.mem.set_opt(Mem::NearestAttackable, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

// ---------------------------------------------------------------------------- behaviours

/// `SetWardenLookTarget`: looks at the roar target or the disturbance (unless fighting).
fn set_warden_look_target() -> Box<dyn Control> {
    shot(
        "SetWardenLookTarget",
        &[(Mem::LookTarget, Registered), (Mem::DisturbanceLocation, Registered), (Mem::RoarTarget, Registered), (Mem::AttackTarget, ValueAbsent)],
        |cx| {
            let roar = cx.b.mem.entity(Mem::RoarTarget).and_then(|id| util::living(cx, id)).map(|l| block_of(&l));
            let Some(pos) = roar.or_else(|| cx.b.mem.block(Mem::DisturbanceLocation)) else { return false };
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::block(pos)));
            true
        },
    )
}

/// `Emerging`.
#[derive(Clone, Debug)]
struct Emerging;

impl Behavior for Emerging {
    fn name(&self) -> &'static str {
        "Emerging"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::IsEmerging, ValuePresent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)]
    }
    fn duration(&self) -> (i32, i32) {
        (EMERGE_DURATION, EMERGE_DURATION)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        set_pose(cx.e, cx.m, &*cx.level, Pose::Emerging);
        sound(cx.e, cx.level, "minecraft:entity.warden.emerge", 5.0, 1.0);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if has_pose(cx.m, Pose::Emerging) {
            set_pose(cx.e, cx.m, &*cx.level, Pose::Standing);
        }
    }
    behavior_boilerplate!();
}

/// `ForceUnmount`: gets off its vehicle before digging.
#[derive(Clone, Debug)]
struct ForceUnmount;

impl Behavior for ForceUnmount {
    fn name(&self) -> &'static str {
        "ForceUnmount"
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.vehicle.is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        if let Some(v) = cx.e.vehicle.take() {
            let id = cx.e.id;
            if let Some(ve) = cx.level.entity_mut(v) {
                crate::ride::remove_passenger(ve, id);
            }
            cx.e.needs_sync = true;
        }
    }
    behavior_boilerplate!();
}

/// `Digging`: sinks into the ground and is gone (`remove(DISCARDED)`).
#[derive(Clone, Debug)]
struct Digging;

impl Digging {
    fn remove(cx: &mut Cx) {
        if !cx.e.is_removed() {
            cx.e.discard();
            // `LivingEntity.remove`: `brain.clearMemories()`.
            cx.b.mem.clear_all();
        }
    }
}

impl Behavior for Digging {
    fn name(&self) -> &'static str {
        "Digging"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent)]
    }
    fn duration(&self) -> (i32, i32) {
        (DIGGING_DURATION, DIGGING_DURATION)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.e.on_ground || cx.e.is_in_water() || cx.e.is_in_lava()
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        !cx.e.is_removed()
    }
    fn start(&mut self, cx: &mut Cx) {
        if cx.e.on_ground {
            set_pose(cx.e, cx.m, &*cx.level, Pose::Digging);
            sound(cx.e, cx.level, "minecraft:entity.warden.dig", 5.0, 1.0);
        } else {
            sound(cx.e, cx.level, "minecraft:entity.warden.agitated", 5.0, 1.0);
            Digging::remove(cx);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        Digging::remove(cx);
    }
    behavior_boilerplate!();
}

/// `SetRoarTarget(getEntityAngryAt)`.
fn set_roar_target() -> Box<dyn Control> {
    shot("SetRoarTarget", &[(Mem::RoarTarget, ValueAbsent), (Mem::AttackTarget, ValueAbsent), (Mem::CantReachWalkTargetSince, Registered)], |cx| {
        let Some(t) = entity_angry_at(cx.m, &cx.b.mem, &*cx.level).filter(|&t| can_target(&*cx.level, t).is_some()) else { return false };
        cx.b.mem.set(Mem::RoarTarget, Val::Entity(t));
        cx.b.mem.erase(Mem::CantReachWalkTargetSince);
        true
    })
}

/// `TryToSniff`.
fn try_to_sniff() -> Box<dyn Control> {
    shot(
        "TryToSniff",
        &[
            (Mem::IsSniffing, Registered),
            (Mem::WalkTarget, Registered),
            (Mem::SniffCooldown, ValueAbsent),
            (Mem::NearestAttackable, ValuePresent),
            (Mem::DisturbanceLocation, ValueAbsent),
        ],
        |cx| {
            cx.b.mem.set(Mem::IsSniffing, Val::Unit);
            let cooldown = uniform(cx.rng(), 100, 200);
            cx.b.mem.set_expiring(Mem::SniffCooldown, Val::Unit, cooldown as i64);
            cx.b.mem.erase(Mem::WalkTarget);
            set_pose(cx.e, cx.m, &*cx.level, Pose::Sniffing);
            true
        },
    )
}

/// `Sniffing`: 84 ticks of sniffing, then anger at the nearest attackable if it is close.
#[derive(Clone, Debug)]
struct Sniffing;

impl Behavior for Sniffing {
    fn name(&self) -> &'static str {
        "Sniffing"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::IsSniffing, ValuePresent),
            (Mem::AttackTarget, ValueAbsent),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::LookTarget, Registered),
            (Mem::NearestAttackable, Registered),
            (Mem::DisturbanceLocation, Registered),
            (Mem::SniffCooldown, Registered),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (SNIFFING_DURATION, SNIFFING_DURATION)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        sound(cx.e, cx.level, "minecraft:entity.warden.sniff", 5.0, 1.0);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if has_pose(cx.m, Pose::Sniffing) {
            set_pose(cx.e, cx.m, &*cx.level, Pose::Standing);
        }
        cx.b.mem.erase(Mem::IsSniffing);
        let Some(t) = cx.b.mem.entity(Mem::NearestAttackable).and_then(|id| can_target(&*cx.level, id)) else { return };
        if closer_than(cx.e, t.pos, 6.0, 20.0) {
            increase_anger_at(cx.e, cx.m, cx.level, &mut cx.b.mem, t.id, 35, true);
        }
        if !cx.b.mem.has(Mem::DisturbanceLocation) {
            set_disturbance_location(cx.m, &*cx.level, &mut cx.b.mem, block_of(&t));
        }
    }
    behavior_boilerplate!();
}

/// `Roar`: 84 ticks turned to the roar target, then it becomes the attack target.
#[derive(Clone, Debug)]
struct Roar;

impl Behavior for Roar {
    fn name(&self) -> &'static str {
        "Roar"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::RoarTarget, ValuePresent), (Mem::AttackTarget, ValueAbsent), (Mem::RoarSoundCooldown, Registered), (Mem::RoarSoundDelay, Registered)]
    }
    fn duration(&self) -> (i32, i32) {
        (ROAR_DURATION, ROAR_DURATION)
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set_expiring(Mem::RoarSoundDelay, Val::Unit, ROAR_SOUND_DELAY as i64);
        cx.b.mem.erase(Mem::WalkTarget);
        let Some(t) = cx.b.mem.entity(Mem::RoarTarget) else { return };
        util::look_at_entity(cx, t);
        set_pose(cx.e, cx.m, &*cx.level, Pose::Roaring);
        increase_anger_at(cx.e, cx.m, cx.level, &mut cx.b.mem, t, 20, false);
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.b.mem.has(Mem::RoarSoundDelay) || cx.b.mem.has(Mem::RoarSoundCooldown) {
            return;
        }
        cx.b.mem.set_expiring(Mem::RoarSoundCooldown, Val::Unit, (ROAR_DURATION - ROAR_SOUND_DELAY) as i64);
        sound(cx.e, cx.level, "minecraft:entity.warden.roar", 3.0, 1.0);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if has_pose(cx.m, Pose::Roaring) {
            set_pose(cx.e, cx.m, &*cx.level, Pose::Standing);
        }
        if let Some(t) = cx.b.mem.entity(Mem::RoarTarget) {
            set_attack_target(&mut cx.b.mem, t);
        }
        cx.b.mem.erase(Mem::RoarTarget);
    }
    behavior_boilerplate!();
}

/// `SonicBoom`: charges for 34 ticks, then a boom through armor at a target within 15 blocks.
#[derive(Clone, Debug)]
struct SonicBoom;

impl Behavior for SonicBoom {
    fn name(&self) -> &'static str {
        "SonicBoom"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::AttackTarget, ValuePresent),
            (Mem::SonicBoomCooldown, ValueAbsent),
            (Mem::SonicBoomSoundCooldown, Registered),
            (Mem::SonicBoomSoundDelay, Registered),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (SONIC_BOOM_DURATION, SONIC_BOOM_DURATION)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id)).is_some_and(|t| closer_than(cx.e, t.pos, 15.0, 20.0))
    }
    fn can_still_use(&mut self, _cx: &mut Cx) -> bool {
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        cx.b.mem.set_expiring(Mem::AttackCoolingDown, Val::Bool(true), SONIC_BOOM_DURATION as i64);
        cx.b.mem.set_expiring(Mem::SonicBoomSoundDelay, Val::Unit, SONIC_BOOM_SOUND_DELAY as i64);
        let id = cx.e.id;
        cx.level.emit(Event::EntityEvent { entity: id, event: 62 });
        sound(cx.e, cx.level, "minecraft:entity.warden.sonic_charge", 3.0, 1.0);
    }
    fn tick(&mut self, cx: &mut Cx) {
        let target = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id));
        if let Some(t) = &target {
            control::look_at(cx.m, t.pos.x, t.pos.y, t.pos.z);
        }
        if cx.b.mem.has(Mem::SonicBoomSoundDelay) || cx.b.mem.has(Mem::SonicBoomSoundCooldown) {
            return;
        }
        cx.b.mem.set_expiring(Mem::SonicBoomSoundCooldown, Val::Unit, (SONIC_BOOM_DURATION - SONIC_BOOM_SOUND_DELAY) as i64);
        let Some(t) = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| can_target(&*cx.level, id)).filter(|t| closer_than(cx.e, t.pos, 15.0, 20.0)) else { return };
        // `EntityAttachment.WARDEN_CHEST` (0, 1.6, 0).
        let from = cx.e.position().add(0.0, 1.6, 0.0);
        let d = Vec3::new(t.pos.x, t.eye_y, t.pos.z) - from;
        let n = d.normalize();
        let steps = crate::math::floor(d.length()) + 7;
        for i in 1..steps {
            cx.level.particle("minecraft:sonic_boom", from + n.scale(i as f64));
        }
        sound(cx.e, cx.level, "minecraft:entity.warden.sonic_boom", 3.0, 1.0);
        let source = DamageSource { kind: DamageKind::SonicBoom, attacker: Some(cx.e.id), direct: Some(cx.e.id), pos: Some(cx.e.position()), attacker_is_player: false };
        if mob::hurt_living(cx.level, &t, source, 10.0) {
            let resistance = cx.level.entity(t.id).and_then(mob::data).map_or(0.0, |o| o.attrs.value(KnockbackResistance));
            let (v, h) = (0.5 * (1.0 - resistance), 2.5 * (1.0 - resistance));
            if let Some(o) = cx.level.entity_mut(t.id) {
                o.delta = o.delta.add(n.x * h, n.y * v, n.z * h);
                o.needs_sync = true;
            }
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        set_sonic_cooldown(&mut cx.b.mem, 40);
    }
    behavior_boilerplate!();
}

/// `GoToTargetLocation(DISTURBANCE_LOCATION, 2, 0.7)`: walks to within two blocks of a nearby
/// spot next to the disturbance.
fn go_to_disturbance() -> Box<dyn Control> {
    shot(
        "GoToTargetLocation",
        &[(Mem::DisturbanceLocation, ValuePresent), (Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)],
        |cx| {
            let Some(pos) = cx.b.mem.block(Mem::DisturbanceLocation) else { return false };
            let here = cx.e.block_position();
            let closer = util::dist_sqr_pos(pos, here) < 4.0;
            if !closer {
                let dx = cx.rng().next_int_bounded(3) - 1;
                let dz = cx.rng().next_int_bounded(3) - 1;
                util::set_walk_and_look(cx, Tracker::block(pos.offset(dx, 0, dz)), 0.7, 2);
            }
            true
        },
    )
}

/// `WardenAi.DIG_COOLDOWN_SETTER`.
fn dig_cooldown_setter() -> Box<dyn Control> {
    shot("", &[(Mem::DigCooldown, Registered)], |cx| {
        refresh_dig_cooldown(&mut cx.b.mem);
        true
    })
}

/// The stop condition of `StopAttackingIfTargetInvalid` in the fight: not angry any more, or the
/// target can no longer be targeted.
fn fight_stop(cx: &mut Cx, t: &Living) -> bool {
    active_anger(cx.m, &cx.b.mem, &*cx.level) < ANGRY || can_target(&*cx.level, t.id).is_none()
}

/// `WardenAi.onTargetInvalid`.
fn on_target_invalid(cx: &mut Cx, id: i32) {
    if can_target(&*cx.level, id).is_none()
        && let Some(s) = state_mut(cx.m)
    {
        s.anger.clear(id);
    }
    refresh_dig_cooldown(&mut cx.b.mem);
}

/// `ActivityData.create(activity, start, behaviours, memory)`: the activity needs the memory
/// and erases it when it stops.
fn activity_for(activity: Activity, start: i32, behaviors: Vec<Box<dyn Control>>, memory: Mem) -> ActivityData {
    let pairs = behaviors.into_iter().enumerate().map(|(i, b)| (start + i as i32, b)).collect();
    ActivityData::full(activity, pairs, &[(memory, ValuePresent)], &[memory])
}

/// `Warden.BRAIN_PROVIDER` with `WardenAi.getActivities`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![Box::new(Players), Box::new(WardenEntitySensor)];
    let core = ActivityData::create(Activity::Core, 0, vec![Swim::new(0.8), set_warden_look_target(), LookAtTargetSink::new(45, 90), MoveToTargetSink::new()]);
    let emerge = activity_for(Activity::Emerge, 5, vec![Timed::new(Emerging)], Mem::IsEmerging);
    let dig = ActivityData::with_conditions(
        Activity::Dig,
        vec![(0, Timed::new(ForceUnmount)), (1, Timed::new(Digging))],
        &[(Mem::RoarTarget, ValueAbsent), (Mem::DigCooldown, ValueAbsent)],
    );
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![
            set_roar_target(),
            try_to_sniff(),
            Gate::run_one_when(&[(Mem::IsSniffing, ValueAbsent)], vec![(stroll(0.5, StrollKind::Land { avoid_water: false }), 2), (DoNothing::new(30, 60), 1)]),
        ],
    );
    let roar = activity_for(Activity::Roar, 10, vec![Timed::new(Roar)], Mem::RoarTarget);
    let fight = activity_for(
        Activity::Fight,
        10,
        vec![
            dig_cooldown_setter(),
            combat::stop_attacking_if_target_invalid(fight_stop, on_target_invalid, false),
            set_entity_look_target(|cx, id| cx.b.mem.entity(Mem::AttackTarget) == Some(id), 24.0),
            combat::set_walk_target_from_attack_target_if_out_of_reach(|_| 1.2),
            Timed::new(SonicBoom),
            combat::melee_attack(MELEE_COOLDOWN),
        ],
        Mem::AttackTarget,
    );
    let investigate = activity_for(Activity::Investigate, 5, vec![set_roar_target(), go_to_disturbance()], Mem::DisturbanceLocation);
    let sniff = activity_for(Activity::Sniff, 5, vec![set_roar_target(), Timed::new(Sniffing)], Mem::IsSniffing);
    Brain::new(
        &[Mem::NearestVisibleNemesis, Mem::RecentProjectile, Mem::TouchCooldown, Mem::VibrationCooldown],
        sensors,
        vec![core, emerge, dig, idle, roar, fight, investigate, sniff],
        random,
    )
}

/// `WardenAi.updateActivity`.
fn update_activity(m: &mut MobData) {
    if let Some(b) = m.brain.as_mut() {
        b.st.set_active_activity_to_first_valid(&[
            Activity::Emerge,
            Activity::Dig,
            Activity::Roar,
            Activity::Fight,
            Activity::Investigate,
            Activity::Sniff,
            Activity::Idle,
        ]);
    }
}

impl Kind for Warden {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        for (t, v) in [
            (path::PathType::UnpassableRail, 0.0),
            (path::PathType::Damaging, 8.0),
            (path::PathType::PowderSnow, 8.0),
            (path::PathType::Lava, 8.0),
            (path::PathType::Fire, 0.0),
            (path::PathType::FireInNeighbor, 0.0),
        ] {
            m.maluses.retain(|(p, _)| *p != t);
            m.maluses.push((t, v));
        }
        Some(Box::new(WardenState::default()))
    }

    /// No goals: the brain does it all (its `Swim` included).
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let persistent = m.persistence_required || e.vehicle.is_some();
        with_mem(m, |m, mem| {
            tick_vibrations(e, m, level, mem);
            if persistent {
                refresh_dig_cooldown(mem);
            }
        });
        sync_target(m);
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        update_listener(e, m, level);
    }

    /// `Warden.customServerAiStep`: the brain, the darkness pulse, the anger's decay, then
    /// `WardenAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if state_mut(m).is_some_and(|s| std::mem::take(&mut s.melee_hit))
            && let Some(b) = m.brain.as_mut()
        {
            set_sonic_cooldown(&mut b.st.mem, 40);
        }
        if (e.tick_count + e.id) % 120 == 0 {
            level.darkness_around(e.position(), 20.0);
        }
        if e.tick_count % 20 == 0 {
            let valid: Vec<i32> = state(m).map_or(Vec::new(), |s| s.anger.suspects.iter().map(|x| x.0).filter(|&id| can_target(&*level, id).is_some()).collect());
            let target = m.brain.as_ref().and_then(|b| get_target(&b.st.mem, &*level));
            if let Some(s) = state_mut(m) {
                s.anger.tick(|id| valid.contains(&id));
                let anger = s.anger.active_anger(target);
                if anger != s.client_anger {
                    s.client_anger = anger;
                    e.needs_sync = true;
                }
            }
        }
        update_activity(m);
        sync_target(m);
    }

    /// `Warden$1$1.distance`: `distanceToXZ`.
    fn path_distance_xz(&self) -> bool {
        true
    }

    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        (digging_or_emerging(m) && !kind.is_tag("minecraft:bypasses_invulnerability")) || kind.is_tag("minecraft:is_fire")
    }

    /// `Warden.canAttack` is `canTargetEntity`.
    fn can_attack(&self, _m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        targetable(t)
    }

    /// `hurtServer`: whoever hurt it makes it angry (and, if it has no target, its target).
    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, _hurt: bool) {
        if m.no_ai || digging_or_emerging(m) {
            return;
        }
        let Some(a) = source.attacker else { return };
        with_mem(m, |m, mem| {
            increase_anger_at(e, m, level, mem, a, ANGRY + 20, false);
            let direct = source.direct.is_none() || source.direct == source.attacker;
            if !mem.has(Mem::AttackTarget)
                && let Some(t) = goals::living(&*level, a)
                && (direct || t.pos.distance_to_sqr(e.position()) < 25.0)
            {
                set_attack_target(mem, a);
            }
        });
        sync_target(m);
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        let pitch = voice_pitch(e);
        sound(e, level, "minecraft:entity.warden.attack_impact", 10.0, pitch);
        if let Some(s) = state_mut(m) {
            s.melee_hit = true;
        }
        let hit = mob::do_hurt_target_base(e, m, level, t);
        Some(hit)
    }

    /// `Warden.doPush`: a touch makes it angry (35) and look where the toucher stands.
    fn do_push_mut(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, other: i32) {
        if m.no_ai {
            return;
        }
        with_mem(m, |m, mem| {
            if mem.has(Mem::TouchCooldown) {
                return;
            }
            mem.set_expiring(Mem::TouchCooldown, Val::Unit, 20);
            increase_anger_at(e, m, level, mem, other, 35, true);
            if let Some(l) = goals::living(&*level, other) {
                set_disturbance_location(m, &*level, mem, block_of(&l));
            }
        });
        sync_target(m);
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.set_expiring(Mem::DigCooldown, Val::Unit, DIGGING_COOLDOWN as i64);
        }
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let anger = AngerManagement::from_nbt(r.get("anger"));
        let vibration = VibrationData::from_nbt(r.get("listener"));
        if let Some(s) = state_mut(m) {
            s.anger = anger;
            s.vibration = vibration;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(s) = state(m) else { return };
        o.put("anger", s.anger.to_nbt());
        o.put("listener", s.vibration.to_nbt());
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let Some(s) = state(m) else { return };
        d.set(kiln_data::entities::data::warden::CLIENT_ANGER_LEVEL, &DataValue::Int(s.client_anger));
        if m.health > 0.0 && s.pose != Pose::Standing {
            d.set(kiln_data::entities::data::entity::POSE, &DataValue::Pose(s.pose.id()));
        }
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if digging_or_emerging(m) { (base.0, 1.0, base.2.min(1.0 * 0.85)) } else { base }
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let s = state(m)?;
        if matches!(s.pose, Pose::Roaring | Pose::Digging | Pose::Emerging) {
            return Some(None);
        }
        let anger = s.anger.active_anger(m.target);
        Some(Some(if anger >= ANGRY {
            "minecraft:entity.warden.angry"
        } else if anger >= AGITATED {
            "minecraft:entity.warden.agitated"
        } else {
            "minecraft:entity.warden.ambient"
        }))
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(5)
    }
}

/// `SpawnUtil.trySpawnMob(WARDEN, TRIGGERED, ...)`: `finalizeSpawn` with the triggered reason
/// (emerging from the ground for 134 ticks, the agitated sound) on a new warden.
pub fn emerge(e: &mut Entity) {
    if let Some(m) = mob::data_mut(e) {
        if let Some(s) = state_mut(m) {
            s.pose = Pose::Emerging;
        }
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.set_expiring(Mem::IsEmerging, Val::Unit, EMERGE_DURATION as i64);
        }
    }
    // `setPose` sizes it for the ground it comes out of.
    let kind = std::mem::replace(&mut e.kind, crate::entity::EntityKind::MobTicking { gravity: 0.08 });
    if let crate::entity::EntityKind::Mob(m) = &kind {
        mob::refresh_dimensions(e, m);
    }
    e.kind = kind;
    e.needs_sync = true;
}

/// The area a warden checks for another before a shrieker warns (`hasNearbyWarden`).
pub fn nearby_box(pos: BlockPos) -> Aabb {
    let c = pos.center();
    Aabb::new(c.x - 24.0, c.y - 24.0, c.z - 24.0, c.x + 24.0, c.y + 24.0, c.z + 24.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anger_sorts_angry_players_first() {
        let mut a = AngerManagement::default();
        a.increase(1, 11, false, 50);
        a.increase(2, 22, true, 30);
        assert_eq!(a.suspects[0].0, 2, "players before others when neither is angry");
        a.increase(1, 11, false, 40);
        assert_eq!(a.suspects[0].0, 1, "the angry one first");
        assert_eq!(a.highest, 90);
        a.increase(1, 11, false, 100);
        assert_eq!(a.anger_of(1), MAX_ANGER);
        a.tick(|_| true);
        assert_eq!(a.anger_of(1), MAX_ANGER - 1);
        a.clear(1);
        // The tick before took one point off the player's 30.
        assert_eq!(a.highest, 29);
        let back = AngerManagement::from_nbt(Some(&a.to_nbt()));
        assert_eq!(back.by_uuid, vec![(22, 29)]);
    }
}
