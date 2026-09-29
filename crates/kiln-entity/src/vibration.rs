//! Vibrations (`VibrationSystem`): game event frequencies and radii, the one vibration a
//! listener picks per tick (`VibrationSelector`), its travel to the listener
//! (`VibrationSystem.Data`, `Ticker`), occlusion by `#occludes_vibration_signals` blocks, and the
//! redstone strength sculk sensors give by distance.
//!
//! World-independent: the simulation's game event dispatcher (sculk sensors, shriekers) and the
//! warden drive it.

use crate::math::{BlockPos, Direction, Vec3};
use kiln_proto::nbt::Tag;
use std::sync::OnceLock;

/// `GameEvent.DEFAULT_NOTIFICATION_RADIUS`.
pub const DEFAULT_RADIUS: i32 = 16;

/// The `minecraft:game_event` entries, by protocol id.
fn events() -> &'static [&'static str] {
    kiln_data::builtin_entries("minecraft:game_event").unwrap_or(&[])
}

/// The registry's own `&'static str` for game event `name` (so vibrations can keep it).
pub fn intern(name: &str) -> Option<&'static str> {
    events().iter().find(|e| **e == name).copied()
}

/// `GameEvent.notificationRadius`.
pub fn notification_radius(event: &str) -> i32 {
    match event {
        "minecraft:jukebox_play" | "minecraft:jukebox_stop_play" => 10,
        "minecraft:shriek" => 32,
        _ => DEFAULT_RADIUS,
    }
}

/// `VibrationSystem.getGameEventFrequency` (0: not a vibration sculk sensors react to).
pub fn frequency(event: &str) -> i32 {
    let Some(name) = event.strip_prefix("minecraft:") else { return 0 };
    if let Some(n) = name.strip_prefix("resonate_") {
        return n.parse().unwrap_or(0);
    }
    match name {
        "step" | "swim" | "flap" => 1,
        "projectile_land" | "hit_ground" | "splash" | "bounce" => 2,
        "item_interact_finish" | "projectile_shoot" | "instrument_play" => 3,
        "entity_action" | "elytra_glide" | "unequip" => 4,
        "entity_dismount" | "equip" => 5,
        "entity_interact" | "shear" | "entity_mount" => 6,
        "entity_damage" => 7,
        "drink" | "eat" => 8,
        "container_close" | "block_close" | "block_deactivate" | "block_detach" => 9,
        "container_open" | "block_open" | "block_activate" | "block_attach" | "prime_fuse" | "note_block_play" => 10,
        "block_change" => 11,
        "block_destroy" | "fluid_pickup" => 12,
        "block_place" | "fluid_place" => 13,
        "entity_place" | "lightning_strike" | "teleport" => 14,
        "entity_die" | "explode" => 15,
        _ => 0,
    }
}

/// `VibrationSystem.getResonanceEventByFrequency`.
pub fn resonance_event(frequency: i32) -> &'static str {
    const NAMES: [&str; 15] = [
        "minecraft:resonate_1",
        "minecraft:resonate_2",
        "minecraft:resonate_3",
        "minecraft:resonate_4",
        "minecraft:resonate_5",
        "minecraft:resonate_6",
        "minecraft:resonate_7",
        "minecraft:resonate_8",
        "minecraft:resonate_9",
        "minecraft:resonate_10",
        "minecraft:resonate_11",
        "minecraft:resonate_12",
        "minecraft:resonate_13",
        "minecraft:resonate_14",
        "minecraft:resonate_15",
    ];
    NAMES[(frequency - 1).clamp(0, 14) as usize]
}

/// Game event tags as bit sets over protocol ids (there are fewer than 128 events).
fn event_tags() -> &'static [(&'static str, u128)] {
    static TAGS: OnceLock<Vec<(&'static str, u128)>> = OnceLock::new();
    TAGS.get_or_init(|| {
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:game_event")
            .map_or(&[][..], |(_, t)| *t)
            .iter()
            .map(|&(name, ids)| (name, ids.iter().filter(|&&i| i < 128).fold(0u128, |m, &i| m | 1 << i)))
            .collect()
    })
}

/// `Holder<GameEvent>.is(tag)` (`minecraft:vibrations`, `minecraft:warden_can_listen`, ...).
pub fn in_tag(event: &str, tag: &str) -> bool {
    let Some(id) = events().iter().position(|e| *e == event).filter(|&i| i < 128) else { return false };
    event_tags().iter().find(|(t, _)| *t == tag).is_some_and(|(_, m)| m & (1 << id) != 0)
}

/// Block tags vibrations consult, per state (bit 0 `#occludes_vibration_signals`, bit 1
/// `#dampens_vibrations`, bit 2 `#vibration_resonators`).
fn block_flags(state: u16) -> u8 {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    let flags = FLAGS.get_or_init(|| {
        let states = kiln_data::blocks::STATE_COUNT as usize;
        let mut out = vec![0u8; states];
        let tags = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == "minecraft:block").map_or(&[][..], |(_, t)| *t);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for (bit, tag) in ["minecraft:occludes_vibration_signals", "minecraft:dampens_vibrations", "minecraft:vibration_resonators"].iter().enumerate() {
            let ids = tags.iter().find(|(t, _)| t == tag).map_or(&[][..], |(_, ids)| *ids);
            for &id in ids {
                if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                    for f in &mut out[info.first as usize..=info.last as usize] {
                        *f |= 1 << bit;
                    }
                }
            }
        }
        out
    });
    flags.get(state as usize).copied().unwrap_or(0)
}

/// `#minecraft:occludes_vibration_signals` (wool).
pub fn occludes(state: u16) -> bool {
    block_flags(state) & 1 != 0
}

/// `#minecraft:dampens_vibrations` (wool and carpets).
pub fn dampens(state: u16) -> bool {
    block_flags(state) & 2 != 0
}

/// `#minecraft:vibration_resonators` (amethyst blocks).
pub fn resonator(state: u16) -> bool {
    block_flags(state) & 4 != 0
}

/// `VibrationSystem.getRedstoneStrengthForDistance`.
pub fn redstone_strength_for_distance(distance: f32, radius: i32) -> i32 {
    let scale = 15.0 / radius as f64;
    1.max(15 - crate::math::floor(scale * distance as f64))
}

/// `Listener.distanceBetweenInBlocks`.
pub fn distance_between_in_blocks(a: BlockPos, b: BlockPos) -> f32 {
    let (dx, dy, dz) = ((a.x - b.x) as f64, (a.y - b.y) as f64, (a.z - b.z) as f64);
    (dx * dx + dy * dy + dz * dz).sqrt() as f32
}

/// `User.calculateTravelTimeInTicks` (the default: a block a tick).
pub fn travel_time(distance: f32) -> i32 {
    crate::math::floor(distance as f64)
}

/// `Listener.isOccluded`: every one of the six nudged starts sees a
/// `#occludes_vibration_signals` block on the way (`isBlockInLine`).
pub fn is_occluded(block: &dyn Fn(BlockPos) -> u16, origin: Vec3, dest: Vec3) -> bool {
    let center = |v: Vec3| Vec3::new(v.x.floor() + 0.5, v.y.floor() + 0.5, v.z.floor() + 0.5);
    let (from, to) = (center(origin), center(dest));
    // `1.0E-5F` widened to a double.
    const NUDGE: f64 = 9.999999747378752e-6;
    Direction::ALL.iter().all(|&d| {
        let start = from.relative(d, NUDGE);
        crate::clip::traverse_blocks(start, to, |p| occludes(block(p)).then_some(())).is_some()
    })
}

/// The entity behind a game event as listeners judge it (`GameEvent.Context.sourceEntity`),
/// taken when the event happened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventSource {
    pub id: i32,
    pub uuid: u128,
    pub type_name: &'static str,
    pub spectator: bool,
    /// `isSteppingCarefully` (a sneaking player).
    pub stepping_carefully: bool,
    /// `dampensVibrations` (an item entity of `#dampens_vibrations`).
    pub dampens: bool,
    /// `SculkShriekerBlockEntity.tryGetPlayer`: the player the entity stands for (itself, its
    /// controlling rider, the owner of its projectile or dropped item).
    pub player: Option<i32>,
    /// A projectile's owner (`VibrationInfo.projectileOwnerUuid`).
    pub projectile_owner: Option<i32>,
    /// A `LivingEntity` (wardens get angry at it).
    pub living: bool,
    /// A living entity no mob may target (`EntitySelector.NO_CREATIVE_OR_SPECTATOR` fails,
    /// invulnerable, or dying).
    pub untargetable: bool,
    /// The entity's position when it made the event.
    pub pos: Vec3,
}

impl EventSource {
    /// A player as the source of a game event.
    pub fn player(id: i32, uuid: u128, pos: Vec3, sneaking: bool, spectator: bool, creative: bool) -> EventSource {
        EventSource {
            id,
            uuid,
            type_name: "minecraft:player",
            spectator,
            stepping_carefully: sneaking,
            dampens: false,
            player: Some(id),
            projectile_owner: None,
            living: true,
            untargetable: spectator || creative,
            pos,
        }
    }
}

/// The source of a game event made by entity `e` (`GameEvent.Context.of(e)`).
pub fn source_of(e: &crate::Entity, level: &dyn crate::EntityLevel) -> EventSource {
    use crate::EntityKind;
    let view = level.player(e.id);
    let owner = match &e.kind {
        EntityKind::Throwable(t) => t.owner,
        EntityKind::Arrow(a) => a.owner,
        _ => None,
    };
    let (living, untargetable) = match &e.kind {
        EntityKind::Mob(m) => (true, e.invulnerable || m.health <= 0.0),
        EntityKind::MobTicking { .. } => (true, e.invulnerable),
        EntityKind::Other { .. } | EntityKind::Player(_) => (true, view.map_or(e.invulnerable, |v| v.creative || v.spectator || !v.alive)),
        _ => (false, false),
    };
    let dampens = match &e.kind {
        // `ItemEntity.dampensVibrations`.
        EntityKind::Item(i) => crate::mob::item_tag(i.stack.item(), "minecraft:dampens_vibrations"),
        // `Warden.dampensVibrations`.
        _ => e.type_name == "minecraft:warden",
    };
    // `SculkShriekerBlockEntity.tryGetPlayer`.
    let player = if view.is_some() {
        Some(e.id)
    } else if let Some(p) = e.passengers.first().and_then(|&p| level.player(p)).filter(|p| crate::mob::data(e).is_some_and(|m| m.kind.ext().is_some_and(|k| k.steerable_by(m, p)))) {
        Some(p.id)
    } else if let Some(o) = owner.filter(|&o| level.player(o).is_some()) {
        Some(o)
    } else if let EntityKind::Item(i) = &e.kind {
        i.thrower.and_then(|u| level.players().iter().find(|p| p.uuid == u).map(|p| p.id))
    } else {
        None
    };
    EventSource {
        id: e.id,
        uuid: e.uuid,
        type_name: e.type_name,
        spectator: view.is_some_and(|v| v.spectator),
        stepping_carefully: view.map_or(e.shift_key_down, |v| v.sneaking),
        dampens,
        player,
        projectile_owner: owner,
        living,
        untargetable,
        pos: e.position(),
    }
}

/// A vibration a warden heard (see [`crate::EntityLevel::take_vibrations`]): the event, where
/// it came from, where the warden listened from, its source and the game time.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub event: &'static str,
    pub from: Vec3,
    pub to: Vec3,
    pub source: Option<EventSource>,
    pub tick: i64,
}

/// A warden's listener as the dispatcher sees it (see [`crate::EntityLevel::set_listener`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ear {
    /// `EntityPositionSource`: the warden's eyes.
    pub pos: Vec3,
    /// `Data.getCurrentVibration() != null`.
    pub busy: bool,
    /// The first half of `Warden.VibrationUser.canReceiveVibration` (AI on, alive, no
    /// vibration cooldown, not digging or emerging).
    pub can_hear: bool,
}

/// `GameEvent.Context`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Context {
    pub source: Option<EventSource>,
    /// The block the event is about (a `#dampens_vibrations` block makes it no vibration).
    pub affected_state: Option<u16>,
}

/// What `User.isValidVibration` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    Valid,
    Invalid,
    /// A sneaking player stepped carefully past a listener that awards `avoid_vibration`.
    Avoided { player: i32 },
}

/// `User.isValidVibration` for a listener hearing `#listenable` events.
pub fn is_valid_vibration(event: &str, ctx: &Context, listenable: &str, can_trigger_avoid: bool) -> Validity {
    if !in_tag(event, listenable) {
        return Validity::Invalid;
    }
    if let Some(s) = &ctx.source {
        if s.spectator {
            return Validity::Invalid;
        }
        if s.stepping_carefully && in_tag(event, "minecraft:ignore_vibrations_sneaking") {
            return match s.player.filter(|&p| can_trigger_avoid && p == s.id) {
                Some(player) => Validity::Avoided { player },
                None => Validity::Invalid,
            };
        }
        if s.dampens {
            return Validity::Invalid;
        }
    }
    if ctx.affected_state.is_some_and(dampens) { Validity::Invalid } else { Validity::Valid }
}

/// `VibrationInfo`: a vibration on its way.
#[derive(Clone, Debug, PartialEq)]
pub struct VibrationInfo {
    pub event: &'static str,
    pub distance: f32,
    pub pos: Vec3,
    /// The entity that made it (`None` after loading: only its UUID is saved).
    pub source: Option<EventSource>,
    pub source_uuid: Option<u128>,
    pub owner_uuid: Option<u128>,
}

impl VibrationInfo {
    pub fn new(event: &'static str, distance: f32, pos: Vec3, source: Option<EventSource>, owner_uuid: Option<u128>) -> VibrationInfo {
        VibrationInfo { event, distance, pos, source, source_uuid: source.map(|s| s.uuid), owner_uuid }
    }

    fn to_nbt(&self) -> Tag {
        let mut f = vec![
            ("game_event".to_string(), Tag::String(self.event.to_string())),
            ("distance".to_string(), Tag::Float(self.distance)),
            ("pos".to_string(), Tag::List(vec![Tag::Double(self.pos.x), Tag::Double(self.pos.y), Tag::Double(self.pos.z)])),
        ];
        if let Some(u) = self.source_uuid {
            f.push(("source".into(), uuid_tag(u)));
        }
        if let Some(u) = self.owner_uuid {
            f.push(("projectile_owner".into(), uuid_tag(u)));
        }
        Tag::Compound(f)
    }

    fn from_nbt(t: &Tag) -> Option<VibrationInfo> {
        let event = intern(t.get("game_event")?.as_str()?)?;
        let distance = t.get("distance")?.as_f64()? as f32;
        let p = t.get("pos")?.as_list()?;
        let c = |i: usize| p.get(i).map(Tag::unwrap_list_element).and_then(Tag::as_f64);
        let pos = Vec3::new(c(0)?, c(1)?, c(2)?);
        if !(distance >= 0.0) {
            return None;
        }
        Some(VibrationInfo { event, distance, pos, source: None, source_uuid: t.get("source").and_then(uuid_of), owner_uuid: t.get("projectile_owner").and_then(uuid_of) })
    }
}

fn uuid_tag(u: u128) -> Tag {
    Tag::IntArray(vec![(u >> 96) as i32, (u >> 64) as i32, (u >> 32) as i32, u as i32])
}

fn uuid_of(t: &Tag) -> Option<u128> {
    match t {
        Tag::IntArray(v) if v.len() == 4 => Some(v.iter().fold(0u128, |a, &x| (a << 32) | x as u32 as u128)),
        _ => None,
    }
}

/// `VibrationSelector`: of the vibrations reaching a listener in one tick, the closest (then
/// the highest frequency, then the first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VibrationSelector {
    pub current: Option<(VibrationInfo, i64)>,
}

impl VibrationSelector {
    /// `addCandidate`.
    pub fn add_candidate(&mut self, v: VibrationInfo, tick: i64) {
        let replace = match &self.current {
            None => true,
            Some((prev, t)) => {
                *t == tick && (v.distance < prev.distance || (v.distance <= prev.distance && frequency(v.event) > frequency(prev.event)))
            }
        };
        if replace {
            self.current = Some((v, tick));
        }
    }

    /// `chosenCandidate`: the vibration picked in an earlier tick.
    pub fn chosen(&self, time: i64) -> Option<&VibrationInfo> {
        self.current.as_ref().filter(|(_, t)| *t < time).map(|(v, _)| v)
    }

    /// `startOver`.
    pub fn start_over(&mut self) {
        self.current = None;
    }
}

/// `VibrationSystem.Data`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VibrationData {
    pub current: Option<VibrationInfo>,
    pub travel_time: i32,
    pub selector: VibrationSelector,
    /// `reloadVibrationParticle`: set on load, so viewers see the vibration still travelling.
    pub reload_particle: bool,
}

/// What one `Ticker.tick` did before the vibration (if any) arrives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ticked {
    /// Vibration particles to send: (where, ticks left to the listener).
    pub particles: Vec<(Vec3, i32)>,
    /// The current vibration arrived: the caller runs `receiveVibration` (and on success
    /// [`VibrationData::received`]).
    pub arrived: bool,
    /// `onDataChanged` (before the arrival).
    pub changed: bool,
}

impl VibrationData {
    /// `Listener.handleGameEvent` after the user's checks: the vibration from `origin`
    /// becomes a candidate (`scheduleVibration`).
    pub fn schedule(&mut self, event: &'static str, origin: Vec3, dest: Vec3, source: Option<EventSource>, owner_uuid: Option<u128>, tick: i64) {
        let distance = origin.distance_to_sqr(dest).sqrt() as f32;
        self.selector.add_candidate(VibrationInfo::new(event, distance, origin, source, owner_uuid), tick);
    }

    /// `Ticker.tick` up to the arrival: picks the last tick's candidate (with its particle),
    /// resends the particle after loading, and counts the travel down. `travel` is the user's
    /// `calculateTravelTimeInTicks`; `dest` where the listener is now.
    pub fn tick(&mut self, now: i64, dest: Vec3, travel: impl Fn(f32) -> i32) -> Ticked {
        let mut out = Ticked::default();
        if self.current.is_none()
            && let Some(v) = self.selector.chosen(now).cloned()
        {
            self.travel_time = travel(v.distance);
            out.particles.push((v.pos, self.travel_time));
            out.changed = true;
            self.current = Some(v);
            self.selector.start_over();
        }
        let Some(v) = &self.current else { return out };
        let mut changed = self.travel_time > 0;
        if self.reload_particle {
            // `tryReloadVibrationParticle`: from where the vibration would be by now.
            let (origin, left, initial) = (v.pos, self.travel_time, travel(v.distance));
            let alpha = 1.0 - left as f64 / initial as f64;
            let lerp = |a: f64, b: f64| a + alpha * (b - a);
            out.particles.push((Vec3::new(lerp(origin.x, dest.x), lerp(origin.y, dest.y), lerp(origin.z, dest.z)), left));
            self.reload_particle = false;
        }
        self.travel_time = (self.travel_time - 1).max(0);
        if self.travel_time <= 0 {
            out.arrived = true;
            changed = false;
        }
        out.changed |= changed;
        out
    }

    /// `receiveVibration` succeeded: the listener is free again.
    pub fn received(&mut self) {
        self.current = None;
    }

    /// `Data.CODEC` (under `listener`).
    pub fn to_nbt(&self) -> Tag {
        let mut f = Vec::new();
        if let Some(v) = &self.current {
            f.push(("event".to_string(), v.to_nbt()));
        }
        let mut sel = Vec::new();
        if let Some((v, _)) = &self.selector.current {
            sel.push(("event".to_string(), v.to_nbt()));
        }
        sel.push(("tick".to_string(), Tag::Long(self.selector.current.as_ref().map_or(-1, |(_, t)| *t))));
        f.push(("selector".to_string(), Tag::Compound(sel)));
        f.push(("event_delay".to_string(), Tag::Int(self.travel_time)));
        Tag::Compound(f)
    }

    /// Reads `Data.CODEC`; a missing or broken one is a fresh listener.
    pub fn from_nbt(t: Option<&Tag>) -> VibrationData {
        let Some(t) = t.filter(|t| matches!(t, Tag::Compound(_))) else { return VibrationData::default() };
        let selector = t.get("selector").map_or(VibrationSelector::default(), |s| {
            let tick = s.get("tick").and_then(Tag::as_i64).unwrap_or(-1);
            VibrationSelector { current: s.get("event").and_then(VibrationInfo::from_nbt).map(|v| (v, tick)) }
        });
        VibrationData {
            current: t.get("event").and_then(VibrationInfo::from_nbt),
            travel_time: t.get("event_delay").and_then(Tag::as_i64).map_or(0, |v| v.max(0) as i32),
            selector,
            reload_particle: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(event: &'static str, distance: f32) -> VibrationInfo {
        VibrationInfo::new(event, distance, Vec3::ZERO, None, None)
    }

    #[test]
    fn frequencies_and_tags() {
        assert_eq!(frequency("minecraft:step"), 1);
        assert_eq!(frequency("minecraft:block_place"), 13);
        assert_eq!(frequency("minecraft:resonate_7"), 7);
        assert_eq!(frequency("minecraft:shriek"), 0);
        assert!(in_tag("minecraft:step", "minecraft:vibrations"));
        assert!(!in_tag("minecraft:shriek", "minecraft:vibrations"));
        assert!(in_tag("minecraft:shriek", "minecraft:warden_can_listen"));
        assert!(in_tag("minecraft:sculk_sensor_tendrils_clicking", "minecraft:shrieker_can_listen"));
        assert!(in_tag("minecraft:step", "minecraft:ignore_vibrations_sneaking"));
        assert_eq!(notification_radius("minecraft:shriek"), 32);
        assert!(occludes(kiln_data::blocks::default_state::WHITE_WOOL));
        assert!(dampens(kiln_data::blocks::default_state::WHITE_CARPET));
        assert!(resonator(kiln_data::blocks::default_state::AMETHYST_BLOCK));
        assert!(!occludes(kiln_data::blocks::default_state::STONE));
    }

    #[test]
    fn redstone_by_distance() {
        assert_eq!(redstone_strength_for_distance(0.0, 8), 15);
        assert_eq!(redstone_strength_for_distance(1.0, 8), 14);
        assert_eq!(redstone_strength_for_distance(7.9, 8), 1);
        assert_eq!(redstone_strength_for_distance(8.0, 8), 1);
        assert_eq!(redstone_strength_for_distance(8.0, 16), 8);
    }

    #[test]
    fn the_selector_keeps_the_closest_of_a_tick() {
        let mut s = VibrationSelector::default();
        s.add_candidate(info("minecraft:step", 5.0), 10);
        s.add_candidate(info("minecraft:eat", 5.0), 10);
        assert_eq!(s.current.as_ref().unwrap().0.event, "minecraft:eat");
        s.add_candidate(info("minecraft:explode", 6.0), 10);
        assert_eq!(s.current.as_ref().unwrap().0.event, "minecraft:eat");
        s.add_candidate(info("minecraft:step", 1.0), 11);
        assert_eq!(s.current.as_ref().unwrap().0.event, "minecraft:eat", "a later tick does not replace");
        assert!(s.chosen(10).is_none());
        assert!(s.chosen(11).is_some());
    }

    #[test]
    fn data_ticks_travel_and_round_trips() {
        let mut d = VibrationData::default();
        d.schedule("minecraft:step", Vec3::new(0.5, 0.5, 3.5), Vec3::new(0.5, 0.5, 0.5), None, None, 5);
        assert_eq!(d.tick(5, Vec3::ZERO, travel_time), Ticked::default());
        let t = d.tick(6, Vec3::ZERO, travel_time);
        assert_eq!(t.particles, vec![(Vec3::new(0.5, 0.5, 3.5), 3)]);
        assert!(!t.arrived);
        let back = VibrationData::from_nbt(Some(&d.to_nbt()));
        assert_eq!(back.current, d.current);
        assert_eq!(back.travel_time, 2);
        assert!(back.reload_particle);
        assert!(!d.tick(7, Vec3::ZERO, travel_time).arrived);
        assert!(d.tick(8, Vec3::ZERO, travel_time).arrived);
    }

    #[test]
    fn wool_walls_occlude() {
        let wool = kiln_data::blocks::default_state::WHITE_WOOL;
        let wall = |p: BlockPos| if p.x == 2 && (-1..=1).contains(&p.y) && (-1..=1).contains(&p.z) { wool } else { 0 };
        assert!(is_occluded(&wall, Vec3::new(0.5, 0.5, 0.5), Vec3::new(4.5, 0.5, 0.5)));
        assert!(!is_occluded(&|_| 0, Vec3::new(0.5, 0.5, 0.5), Vec3::new(4.5, 0.5, 0.5)));
        // Every nudged start is within 1e-5 of the centre, so one block on the line is enough.
        let one = |p: BlockPos| if p == BlockPos::new(2, 0, 0) { wool } else { 0 };
        assert!(is_occluded(&one, Vec3::new(0.5, 0.5, 0.5), Vec3::new(4.5, 0.5, 0.5)));
    }
}
