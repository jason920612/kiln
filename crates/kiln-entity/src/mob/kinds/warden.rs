//! Warden: listens for vibrations, grows angry at their sources (`AngerManagement`), sniffs
//! out nearby entities, roars at the one it is angry at, then hunts it with melee hits and
//! sonic booms through armor; pulses darkness; emerges from the ground when a shrieker
//! summons it and digs back down after a minute without a disturbance.
//!
//! Vanilla drives wardens with a `Brain` (`WardenAi`: emerge, dig, roar, fight, investigate,
//! sniff and idle activities, first valid wins). Kiln runs the same activities from explicit
//! state: the memories are counters (`DIG_COOLDOWN`, `VIBRATION_COOLDOWN`, `SNIFF_COOLDOWN`,
//! `ROAR_TARGET`, `DISTURBANCE_LOCATION`, ...), the behaviours run for their vanilla durations
//! (emerging 134 ticks, digging 100, roaring 84, sniffing 84, a sonic boom 60), and the
//! warden's own vibration listener (`VibrationSystem.Data` at its eyes, 16 blocks,
//! `#warden_can_listen`) is ticked before its tick like vanilla's. Movement goes through
//! Kiln's navigation (walking to the disturbance at 0.7, to the target at 1.2, strolling at
//! 0.5) rather than the brain's walk target sink, and the sensors look every 20 ticks.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, control, mth, path, random_pos};
use crate::persist::{Input, Output};
use crate::vibration::{self, Ear, VibrationData};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

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

/// `WardenAi` durations.
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

/// A behaviour that runs for a set time (`Behavior` with its duration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    Emerging,
    Digging,
    Roar,
    Sniffing,
    SonicBoom,
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

/// The warden's state: its listener, anger and brain memories (ticks left; 0: absent).
#[derive(Clone, Debug, Default)]
pub struct WardenState {
    pub pose: Pose,
    pub vibration: VibrationData,
    pub anger: AngerManagement,
    pub running: Option<(Running, i32)>,
    pub emerging: i32,
    /// `DIG_COOLDOWN` (only refreshed while present: once it runs out the warden digs away).
    pub dig_cooldown: i32,
    pub vibration_cooldown: i32,
    pub touch_cooldown: i32,
    pub recent_projectile: i32,
    pub sniff_cooldown: i32,
    pub sniffing: bool,
    pub disturbance: Option<(BlockPos, i32)>,
    pub roar_target: Option<i32>,
    pub roar_sound_delay: i32,
    pub roar_sound_cooldown: i32,
    pub sonic_boom_cooldown: i32,
    pub sonic_boom_sound_delay: i32,
    pub sonic_boom_sound_cooldown: i32,
    pub attack_cooling_down: i32,
    /// `NEAREST_ATTACKABLE` from the entity sensor.
    pub nearest_attackable: Option<i32>,
    /// Idle `DoNothing` ticks left.
    pub idle: i32,
    /// `CLIENT_ANGER_LEVEL` as last synced.
    pub client_anger: i32,
    /// An entity that pushed into the warden (`doPush`), for the brain's next tick.
    pub touched: Option<i32>,
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

/// `Warden.canTargetEntity`: a living entity that is not creative, a spectator, invulnerable,
/// dead, an armor stand or another warden.
fn can_target(level: &dyn EntityLevel, id: i32) -> Option<Living> {
    let t = goals::living(level, id)?;
    let ok = t.alive && !t.creative && !t.spectator && !t.invulnerable && !matches!(t.type_name, "minecraft:armor_stand" | "minecraft:warden");
    ok.then_some(t)
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

fn set_pose(e: &mut Entity, s: &mut WardenState, pose: Pose) {
    if s.pose != pose {
        s.pose = pose;
        e.needs_sync = true;
    }
}

/// `WardenAi.setDigCooldown`: refreshed only while present.
fn refresh_dig_cooldown(s: &mut WardenState) {
    if s.dig_cooldown > 0 {
        s.dig_cooldown = DIGGING_COOLDOWN;
    }
}

/// `Warden.getEntityAngryAt`: the top suspect when angry.
fn angry_at(m: &MobData, level: &dyn EntityLevel) -> Option<i32> {
    let s = state(m)?;
    if s.anger.active_anger(m.target) < ANGRY {
        return None;
    }
    s.anger.top(|id| can_target(level, id).is_some())
}

/// `Warden.increaseAngerAt(entity, amount, playSound)`.
fn increase_anger(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, target: i32, amount: i32, play_sound: bool) {
    if m.no_ai || can_target(level, target).is_none() {
        return;
    }
    let player = level.player(target).is_some();
    let uuid = uuid_of(level, target);
    let target_is_player = m.target.is_some_and(|t| level.player(t).is_some());
    let Some(s) = state_mut(m) else { return };
    refresh_dig_cooldown(s);
    let anger = s.anger.increase(target, uuid, player, amount);
    let pose = s.pose;
    if player && !target_is_player && anger >= ANGRY {
        m.target = None;
    }
    if play_sound && pose != Pose::Roaring {
        let active = state(m).map_or(0, |s| s.anger.active_anger(m.target));
        let listening = if active >= AGITATED { "minecraft:entity.warden.listening_angry" } else { "minecraft:entity.warden.listening" };
        let pitch = voice_pitch(e);
        sound(e, level, listening, 10.0, pitch);
    }
}

/// `WardenAi.setDisturbanceLocation`.
fn set_disturbance(m: &mut MobData, level: &dyn EntityLevel, pos: BlockPos) {
    if angry_at(m, level).is_some() || m.target.is_some() {
        return;
    }
    if let Some(s) = state_mut(m) {
        refresh_dig_cooldown(s);
        s.sniff_cooldown = 100;
        s.disturbance = Some((pos, 100));
    }
    control::look_at(m, pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    m.nav.stop();
}

/// `Warden.setAttackTarget`.
fn set_attack_target(m: &mut MobData, target: i32) {
    m.target = Some(target);
    if let Some(s) = state_mut(m) {
        s.roar_target = None;
        s.sonic_boom_cooldown = 200;
    }
}

/// `VibrationSystem.Ticker.tick` for the warden (before its tick), then its listener for the
/// dispatcher.
fn tick_vibrations(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
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
    on_receive(e, m, level, &info);
    if let Some(s) = state_mut(m) {
        s.vibration.received();
    }
}

/// `Warden.VibrationUser.onReceiveVibration`.
fn on_receive(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, info: &vibration::VibrationInfo) {
    if m.is_dead_or_dying() {
        return;
    }
    let origin = BlockPos::containing(info.pos.x, info.pos.y, info.pos.z);
    if let Some(s) = state_mut(m) {
        s.vibration_cooldown = 40;
    }
    level.emit(Event::EntityEvent { entity: e.id, event: 61 });
    let pitch = voice_pitch(e);
    sound(e, level, "minecraft:entity.warden.tendril_clicks", 5.0, pitch);
    let source = info.source.map(|s| s.id).filter(|&id| goals::living(level, id).is_some() || level.entity(id).is_some());
    let owner = info.source.and_then(|s| s.projectile_owner).filter(|&o| goals::living(level, o).is_some());
    let mut suspicious = origin;
    match owner {
        Some(o) => {
            let near = goals::living(level, o).is_some_and(|t| t.pos.distance_to_sqr(e.position()) < 30.0 * 30.0);
            if near {
                if state(m).is_some_and(|s| s.recent_projectile > 0) {
                    if let Some(t) = can_target(level, o) {
                        suspicious = BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
                    }
                    increase_anger(e, m, level, o, 35, true);
                } else {
                    increase_anger(e, m, level, o, 10, true);
                }
            }
            if let Some(s) = state_mut(m) {
                s.recent_projectile = 100;
            }
        }
        None => {
            if let Some(src) = source {
                increase_anger(e, m, level, src, 35, true);
            }
        }
    }
    let angry = state(m).is_some_and(|s| s.anger.active_anger(m.target) >= ANGRY);
    if !angry {
        let active = state(m).and_then(|s| s.anger.top(|id| can_target(level, id).is_some()));
        if owner.is_some() || active.is_none() || active == source {
            set_disturbance(m, level, suspicious);
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
    let ear = Ear {
        pos: Vec3::new(e.x(), e.eye_y(), e.z()),
        busy: s.vibration.current.is_some(),
        can_hear: !m.no_ai && s.vibration_cooldown == 0 && !matches!(s.pose, Pose::Digging | Pose::Emerging),
    };
    level.set_listener(e.id, Some(ear));
}

/// `WardenEntitySensor`: the nearest targetable player within 16 blocks (24 up and down, as
/// `NearestLivingEntitySensor` looks), else the nearest other targetable living entity.
fn sense(e: &Entity, level: &dyn EntityLevel) -> Option<i32> {
    let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
    let mut best: Option<(bool, f64, i32)> = None;
    let mut consider = |id: i32, pos: Vec3, player: bool| {
        if can_target(level, id).is_none() {
            return;
        }
        let d = pos.distance_to_sqr(e.position());
        // Players first, then the nearest.
        let better = match best {
            None => true,
            Some((bp, bd, _)) => (player && !bp) || (player == bp && d < bd),
        };
        if better {
            best = Some((player, d, id));
        }
    };
    for p in level.players() {
        if area.contains(p.pos) {
            consider(p.id, p.pos, true);
        }
    }
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        if level.player(id).is_some() {
            continue;
        }
        if let Some(o) = level.entity(id) {
            let pos = o.position();
            consider(id, pos, false);
        }
    }
    best.map(|b| b.2)
}

/// `closerThan(target, xz, y)`.
fn closer_than(e: &Entity, t: Vec3, xz: f64, y: f64) -> bool {
    let (dx, dz) = (t.x - e.x(), t.z - e.z());
    dx * dx + dz * dz < xz * xz && (t.y - e.y()).abs() < y
}

/// One tick of the warden's activities (`WardenAi.updateActivity` picks the first valid of
/// emerge, dig, roar, fight, investigate, sniff and idle).
fn brain(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    // Memories run out.
    if let Some(s) = state_mut(m) {
        for c in [
            &mut s.emerging,
            &mut s.dig_cooldown,
            &mut s.vibration_cooldown,
            &mut s.touch_cooldown,
            &mut s.recent_projectile,
            &mut s.sniff_cooldown,
            &mut s.roar_sound_delay,
            &mut s.roar_sound_cooldown,
            &mut s.sonic_boom_cooldown,
            &mut s.sonic_boom_sound_delay,
            &mut s.sonic_boom_sound_cooldown,
            &mut s.attack_cooling_down,
            &mut s.idle,
        ] {
            *c = (*c - 1).max(0);
        }
        if let Some((_, t)) = &mut s.disturbance {
            *t -= 1;
            if *t <= 0 {
                s.disturbance = None;
            }
        }
    }
    if let Some(t) = state_mut(m).and_then(|s| s.touched.take()) {
        increase_anger(e, m, level, t, 35, true);
        if let Some(l) = goals::living(level, t) {
            set_disturbance(m, level, BlockPos::containing(l.pos.x, l.pos.y, l.pos.z));
        }
    }
    // Sensors.
    if (e.tick_count + e.id) % 20 == 0 {
        let near = sense(e, level);
        if let Some(s) = state_mut(m) {
            s.nearest_attackable = near;
        }
    }
    let running = state(m).and_then(|s| s.running);
    if let Some((r, left)) = running {
        tick_running(e, m, level, r, left);
        return;
    }
    let Some(s) = state(m) else { return };
    // Emerge.
    if s.emerging > 0 {
        start(e, m, level, Running::Emerging);
        return;
    }
    // Dig: no roar target, no dig cooldown, no attack target, not walking.
    if s.roar_target.is_none() && s.dig_cooldown == 0 && m.target.is_none() && m.nav.is_done() {
        if e.on_ground || e.is_in_water() || e.is_in_lava() {
            start(e, m, level, Running::Digging);
            return;
        }
    }
    // Roar.
    if s.roar_target.is_some() && m.target.is_none() {
        start(e, m, level, Running::Roar);
        return;
    }
    if m.target.is_some() {
        fight(e, m, level);
        return;
    }
    // Investigate, sniff and idle look for someone to roar at first (`SetRoarTarget`).
    if let Some(t) = angry_at(m, level) {
        if let Some(s) = state_mut(m) {
            s.roar_target = Some(t);
        }
        return;
    }
    if let Some((pos, _)) = state(m).and_then(|s| s.disturbance) {
        // `GoToTargetLocation(DISTURBANCE_LOCATION, 2, 0.7)`.
        let c = Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5);
        if e.position().distance_to_sqr(c) > 2.0 * 2.0 {
            if m.nav.is_done() {
                path::move_to(e, m, level, c.x, c.y, c.z, 0.7);
            }
        } else {
            m.nav.stop();
        }
        return;
    }
    if state(m).is_some_and(|s| s.sniffing) {
        start(e, m, level, Running::Sniffing);
        return;
    }
    // Idle: `TryToSniff`, then stroll or stand.
    let s = state(m).expect("warden state");
    if s.sniff_cooldown == 0 && s.nearest_attackable.is_some() {
        let cooldown = 100 + level.random().next_int_bounded(101);
        if let Some(s) = state_mut(m) {
            s.sniffing = true;
            s.sniff_cooldown = cooldown;
            set_pose(e, s, Pose::Sniffing);
        }
        m.nav.stop();
        return;
    }
    if s.idle == 0 && m.nav.is_done() {
        // `RunOne`: stroll (weight 2) or do nothing for 30 to 60 ticks (weight 1).
        if e.random.next_int_bounded(3) < 2 {
            if let Some(p) = random_pos::land_pos(e, m, level, 10, 7) {
                path::move_to(e, m, level, p.x, p.y, p.z, 0.5);
            }
        } else {
            let t = 30 + e.random.next_int_bounded(31);
            if let Some(s) = state_mut(m) {
                s.idle = t;
            }
        }
    }
}

/// A behaviour starts.
fn start(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, r: Running) {
    let duration = match r {
        Running::Emerging => EMERGE_DURATION,
        Running::Digging => DIGGING_DURATION,
        Running::Roar => ROAR_DURATION,
        Running::Sniffing => SNIFFING_DURATION,
        Running::SonicBoom => SONIC_BOOM_DURATION,
    };
    m.nav.stop();
    match r {
        Running::Emerging => {
            if let Some(s) = state_mut(m) {
                set_pose(e, s, Pose::Emerging);
            }
            sound(e, level, "minecraft:entity.warden.emerge", 5.0, 1.0);
        }
        Running::Digging => {
            if !e.on_ground {
                // In a liquid: it gives up at once and goes.
                sound(e, level, "minecraft:entity.warden.agitated", 5.0, 1.0);
                e.discard();
                return;
            }
            if let Some(s) = state_mut(m) {
                set_pose(e, s, Pose::Digging);
            }
            sound(e, level, "minecraft:entity.warden.dig", 5.0, 1.0);
        }
        Running::Roar => {
            let target = state(m).and_then(|s| s.roar_target);
            if let Some(s) = state_mut(m) {
                s.roar_sound_delay = ROAR_SOUND_DELAY;
                set_pose(e, s, Pose::Roaring);
            }
            if let Some(t) = target {
                if let Some(l) = goals::living(level, t) {
                    control::look_at(m, l.pos.x, l.eye_y, l.pos.z);
                }
                increase_anger(e, m, level, t, 20, false);
            }
        }
        Running::Sniffing => sound(e, level, "minecraft:entity.warden.sniff", 5.0, 1.0),
        Running::SonicBoom => {
            if let Some(s) = state_mut(m) {
                s.attack_cooling_down = SONIC_BOOM_DURATION;
                s.sonic_boom_sound_delay = SONIC_BOOM_SOUND_DELAY;
            }
            level.emit(Event::EntityEvent { entity: e.id, event: 62 });
            sound(e, level, "minecraft:entity.warden.sonic_charge", 3.0, 1.0);
        }
    }
    if let Some(s) = state_mut(m) {
        s.running = Some((r, duration));
    }
}

/// A running behaviour's tick, and its end.
fn tick_running(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, r: Running, left: i32) {
    match r {
        Running::Roar => {
            let s = state(m).expect("warden state");
            if s.roar_sound_delay == 0 && s.roar_sound_cooldown == 0 {
                if let Some(s) = state_mut(m) {
                    s.roar_sound_cooldown = ROAR_DURATION - ROAR_SOUND_DELAY;
                }
                sound(e, level, "minecraft:entity.warden.roar", 3.0, 1.0);
            }
        }
        Running::SonicBoom => tick_sonic_boom(e, m, level),
        _ => {}
    }
    let left = left - 1;
    if left > 0 {
        if let Some(s) = state_mut(m) {
            s.running = Some((r, left));
        }
        return;
    }
    if let Some(s) = state_mut(m) {
        s.running = None;
    }
    match r {
        Running::Emerging => {
            if let Some(s) = state_mut(m)
                && s.pose == Pose::Emerging
            {
                set_pose(e, s, Pose::Standing);
            }
        }
        Running::Digging => e.discard(),
        Running::Roar => {
            let target = state(m).and_then(|s| s.roar_target);
            if let Some(s) = state_mut(m)
                && s.pose == Pose::Roaring
            {
                set_pose(e, s, Pose::Standing);
            }
            match target {
                Some(t) => set_attack_target(m, t),
                None => {
                    if let Some(s) = state_mut(m) {
                        s.roar_target = None;
                    }
                }
            }
        }
        Running::Sniffing => {
            if let Some(s) = state_mut(m) {
                if s.pose == Pose::Sniffing {
                    set_pose(e, s, Pose::Standing);
                }
                s.sniffing = false;
            }
            let near = state(m).and_then(|s| s.nearest_attackable).and_then(|id| can_target(level, id));
            if let Some(t) = near {
                if closer_than(e, t.pos, 6.0, 20.0) {
                    increase_anger(e, m, level, t.id, 35, true);
                }
                if state(m).is_some_and(|s| s.disturbance.is_none()) {
                    set_disturbance(m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z));
                }
            }
        }
        Running::SonicBoom => {
            if let Some(s) = state_mut(m) {
                s.sonic_boom_cooldown = 40;
            }
        }
    }
}

/// The fight activity: `StopAttackingIfTargetInvalid`, following the target, `SonicBoom` and
/// `MeleeAttack(18)`.
fn fight(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(tid) = m.target else { return };
    if let Some(s) = state_mut(m) {
        // `DIG_COOLDOWN_SETTER`.
        refresh_dig_cooldown(s);
    }
    let target = can_target(level, tid);
    let angry = state(m).is_some_and(|s| s.anger.active_anger(Some(tid)) >= ANGRY);
    let Some(t) = target.filter(|_| angry) else {
        // `onTargetInvalid`.
        if target.is_none()
            && let Some(s) = state_mut(m)
        {
            s.anger.clear(tid);
        }
        if let Some(s) = state_mut(m) {
            refresh_dig_cooldown(s);
        }
        m.target = None;
        m.nav.stop();
        return;
    };
    control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
    let reach = mob::within_melee_range(e, &t);
    // `SetWalkTargetFromAttackTargetIfTargetOutOfReach(1.2)`.
    if reach && mob::has_line_of_sight_cached(e, m, level, &t) {
        m.nav.stop();
    } else if m.nav.is_done() || e.tick_count % 10 == 0 {
        path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.2);
    }
    let s = state(m).expect("warden state");
    if s.sonic_boom_cooldown == 0 && closer_than(e, t.pos, 15.0, 20.0) {
        start(e, m, level, Running::SonicBoom);
        return;
    }
    if s.attack_cooling_down == 0 && reach && mob::has_line_of_sight_cached(e, m, level, &t) {
        m.swing = true;
        if let Some(s) = state_mut(m) {
            s.attack_cooling_down = MELEE_COOLDOWN;
        }
        mob::do_hurt_target(e, m, level, &t);
    }
}

/// `SonicBoom.tick`: after 34 ticks the boom: particles along the line, the sound, 10 damage
/// through armor and a push away.
fn tick_sonic_boom(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let target = m.target.and_then(|t| goals::living(level, t));
    if let Some(t) = &target {
        control::look_at(m, t.pos.x, t.pos.y, t.pos.z);
    }
    let s = state(m).expect("warden state");
    if s.sonic_boom_sound_delay > 0 || s.sonic_boom_sound_cooldown > 0 {
        return;
    }
    if let Some(s) = state_mut(m) {
        s.sonic_boom_sound_cooldown = SONIC_BOOM_DURATION - SONIC_BOOM_SOUND_DELAY;
    }
    let Some(t) = target.and_then(|t| can_target(level, t.id)) else { return };
    if !closer_than(e, t.pos, 15.0, 20.0) {
        return;
    }
    // `EntityAttachment.WARDEN_CHEST` (0, 1.6, 0).
    let from = e.position().add(0.0, 1.6, 0.0);
    let d = Vec3::new(t.pos.x, t.eye_y, t.pos.z) - from;
    let n = d.normalize();
    let steps = crate::math::floor(d.length()) + 7;
    for i in 1..steps {
        level.particle("minecraft:sonic_boom", from + n.scale(i as f64));
    }
    sound(e, level, "minecraft:entity.warden.sonic_boom", 3.0, 1.0);
    let source = DamageSource { kind: DamageKind::SonicBoom, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    if mob::hurt_living(level, &t, source, 10.0) {
        let resistance = level.entity(t.id).and_then(mob::data).map_or(0.0, |o| o.attrs.value(KnockbackResistance));
        let (v, h) = (0.5 * (1.0 - resistance), 2.5 * (1.0 - resistance));
        if let Some(o) = level.entity_mut(t.id) {
            o.delta = o.delta.add(n.x * h, n.y * v, n.z * h);
            o.needs_sync = true;
        }
    }
}

impl Kind for Warden {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(WardenState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        // The brain's `Swim(0.8)`; everything else is [`brain`].
        m.goals.add(0, Goal::Float);
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
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        tick_vibrations(e, m, level);
        if m.persistence_required
            && let Some(s) = state_mut(m)
        {
            refresh_dig_cooldown(s);
        }
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        update_listener(e, m, level);
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !m.no_ai {
            brain(e, m, level);
        }
        if (e.tick_count + e.id) % 120 == 0 {
            level.darkness_around(e.position(), 20.0);
        }
        if e.tick_count % 20 == 0 {
            let valid: Vec<i32> = state(m).map_or(Vec::new(), |s| s.anger.suspects.iter().map(|x| x.0).filter(|&id| can_target(level, id).is_some()).collect());
            let target = m.target;
            if let Some(s) = state_mut(m) {
                s.anger.tick(|id| valid.contains(&id));
                let anger = s.anger.active_anger(target);
                if anger != s.client_anger {
                    s.client_anger = anger;
                    e.needs_sync = true;
                }
            }
        }
        m.set_aggressive(m.target.is_some());
    }

    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        (digging_or_emerging(m) && !kind.is_tag("minecraft:bypasses_invulnerability")) || kind.is_tag("minecraft:is_fire")
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, _hurt: bool) {
        if m.no_ai || digging_or_emerging(m) {
            return;
        }
        let Some(a) = source.attacker else { return };
        increase_anger(e, m, level, a, ANGRY + 20, false);
        let direct = source.direct.is_none() || source.direct == source.attacker;
        if m.target.is_none()
            && let Some(t) = goals::living(level, a)
            && (direct || t.pos.distance_to_sqr(e.position()) < 25.0)
        {
            set_attack_target(m, a);
        }
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        let pitch = voice_pitch(e);
        sound(e, level, "minecraft:entity.warden.attack_impact", 10.0, pitch);
        if let Some(s) = state_mut(m) {
            s.sonic_boom_cooldown = 40;
        }
        let hit = mob::do_hurt_target_base(e, m, level, t);
        if hit {
            // `causeExtraKnockback` with the attack knockback of 1.5.
            let strength = m.attrs.value(AttackKnockback) as f32 * 0.5;
            let yaw = (e.y_rot * 0.017453292) as f64;
            let (s, c) = (mth::sin(yaw), mth::cos(yaw));
            if let Some(o) = level.entity_mut(t.id) {
                let v = Vec3::new(s as f64, 0.0, -(c as f64)).normalize().scale(strength as f64);
                o.delta = Vec3::new(o.delta.x / 2.0 - v.x, if o.on_ground { 0.4f64.min(o.delta.y / 2.0 + strength as f64) } else { o.delta.y }, o.delta.z / 2.0 - v.z);
                o.needs_sync = true;
            }
        }
        Some(hit)
    }

    fn do_push(&self, _e: &mut Entity, m: &mut MobData, _level: &dyn EntityLevel, other: i32) {
        // `Warden.doPush`: a touch makes it angry (35) and look where the toucher stands; the
        // anger needs the mutable level, so the brain applies it on its next tick.
        if m.no_ai {
            return;
        }
        if let Some(s) = state_mut(m)
            && s.touch_cooldown == 0
        {
            s.touch_cooldown = 20;
            s.touched = Some(other);
        }
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        if let Some(s) = state_mut(m) {
            s.dig_cooldown = DIGGING_COOLDOWN;
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
    let Some(m) = mob::data_mut(e) else { return };
    if let Some(s) = state_mut(m) {
        s.pose = Pose::Emerging;
        s.emerging = EMERGE_DURATION;
    }
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
        assert_eq!(a.highest, 30);
        let back = AngerManagement::from_nbt(Some(&a.to_nbt()));
        assert_eq!(back.by_uuid, vec![(22, 29)]);
    }
}
