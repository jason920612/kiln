//! Copper golem: walks between chests, taking up to 16 items from a copper chest and putting
//! them into a chest or trapped chest that already holds the same item (or none), oxidizes over
//! time (a waxed one does not; an axe scrapes it) and, fully oxidized, now and then stiffens
//! into a statue. A poppy given on its head by an iron golem shears off with shears.
//!
//! Driven by the brain of `CopperGolemAi` (core: panic, look and move sinks, doors, cooldowns;
//! idle: the chest transport, look at players, stroll or wait) with a ground navigation that
//! opens doors and plans 48 long, on [`crate::mob::brain`]. The sensors are the nearest living
//! entities and `HurtBy`.

use super::chest_access;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Direction};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::nether::InteractWithDoor;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Gate, Mem, Sensor, Status, Timed, Val};
use crate::mob::brain::sensors;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::path::PathType;
use crate::mob::{self, DamageSource, MobData};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct CopperGolem;

pub static KIND: CopperGolem = CopperGolem;

static INFO: Info = Info {
    ambient_interval: 120,
    ..Info::misc("minecraft:copper_golem", &[(MaxHealth, 12.0), (MovementSpeed, 0.20000000298023224), (StepHeight, 1.0)])
};

/// `WeatheringCopper.WeatherState` names, in order.
const WEATHER: [&str; 4] = ["unaffected", "exposed", "weathered", "oxidized"];
const OXIDIZED: u8 = 3;

/// `CopperGolem.UNSET_WEATHERING_TICK` and `IGNORE_WEATHERING_TICK` (waxed).
const UNSET_WEATHERING_TICK: i64 = -1;
const IGNORE_WEATHERING_TICK: i64 = -2;

/// `CopperGolemState` ordinals.
pub const IDLE: u8 = 0;
pub const GETTING_ITEM: u8 = 1;
pub const GETTING_NO_ITEM: u8 = 2;
pub const DROPPING_ITEM: u8 = 3;
pub const DROPPING_NO_ITEM: u8 = 4;

#[derive(Clone, Debug)]
pub struct State {
    /// `WeatheringCopper.WeatherState` ordinal.
    pub weather: u8,
    /// `CopperGolemState` ordinal.
    pub state: u8,
    pub next_weathering_tick: i64,
    /// `openedChestPos`: the chest it holds open.
    pub opened_chest: Option<BlockPos>,
    pub last_lightning: Option<u128>,
    /// The antenna (`EquipmentSlot.SADDLE`) and its drop chance.
    pub antenna: ItemStack,
    pub antenna_drop: f32,
}

impl Default for State {
    fn default() -> State {
        State { weather: 0, state: IDLE, next_weathering_tick: UNSET_WEATHERING_TICK, opened_chest: None, last_lightning: None, antenna: ItemStack::empty(), antenna_drop: 0.085 }
    }
}

pub fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("copper golem state")
}

pub fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("copper golem state")
}

/// `CopperGolemOxidationLevels`: the sounds by weather state (`kind`: spin, hurt, death, step).
fn oxidation_sound(weather: u8, kind: &str) -> &'static str {
    let name = match (weather, kind) {
        (0 | 1, "spin") => "minecraft:entity.copper_golem.spin",
        (0 | 1, "hurt") => "minecraft:entity.copper_golem.hurt",
        (0 | 1, "death") => "minecraft:entity.copper_golem.death",
        (0 | 1, _) => "minecraft:entity.copper_golem.step",
        (2, "spin") => "minecraft:entity.copper_golem_weathered.spin",
        (2, "hurt") => "minecraft:entity.copper_golem_weathered.hurt",
        (2, "death") => "minecraft:entity.copper_golem_weathered.death",
        (2, _) => "minecraft:entity.copper_golem_weathered.step",
        (_, "spin") => "minecraft:entity.copper_golem_oxidized.spin",
        (_, "hurt") => "minecraft:entity.copper_golem_oxidized.hurt",
        (_, "death") => "minecraft:entity.copper_golem_oxidized.death",
        _ => "minecraft:entity.copper_golem_oxidized.step",
    };
    name
}

/// What `level.getRandom()` hands the golem: the shared stream where there is one.
fn with_level_random<T>(m: &mut MobData, level: &mut dyn EntityLevel, f: impl FnOnce(&mut dyn RandomSource) -> T) -> T {
    match level.shared_ai_random() {
        Some(r) => f(r),
        None => f(&mut m.brain_random),
    }
}

impl CopperGolem {
    /// `CopperGolem.updateWeathering`.
    fn update_weathering(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let now = level.game_time();
        let next = st(m).next_weathering_tick;
        if next == IGNORE_WEATHERING_TICK {
            return;
        }
        if next == UNSET_WEATHERING_TICK {
            let d = with_level_random(m, level, |r| mob::mth::next_int_between(r, 504000, 552000));
            st_mut(m).next_weathering_tick = now + d as i64;
            return;
        }
        let weather = st(m).weather;
        let oxidized = weather == OXIDIZED;
        if now >= next && !oxidized {
            let new = weather + 1;
            st_mut(m).weather = new;
            if new == OXIDIZED {
                st_mut(m).next_weathering_tick = 0;
            } else {
                let d = with_level_random(m, level, |r| mob::mth::next_int_between(r, 504000, 552000));
                st_mut(m).next_weathering_tick = next + d as i64;
            }
        }
        if oxidized && Self::can_turn_to_statue(e, m, level) {
            Self::turn_to_statue(e, m, level);
        }
    }

    /// `canTurnToStatue`: in a spot of air, 0.58 percent a tick.
    fn can_turn_to_statue(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        kiln_data::blocks_types::is_air(level.block(e.block_position())) && with_level_random(m, level, |r| r.next_float()) <= 0.0058
    }

    /// `turnToStatue`: the block (a random pose, facing as the golem does), the golem gone, its
    /// preserved equipment dropped.
    fn turn_to_statue(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let pos = e.block_position();
        let pose = e.random.next_int_bounded(4) as u8;
        // `Direction.fromYRot`.
        let facing = match (((e.y_rot as f64) / 90.0 + 0.5).floor() as i32) & 3 {
            0 => Direction::South,
            1 => Direction::West,
            2 => Direction::North,
            _ => Direction::East,
        };
        let name = e.extra.iter().find(|(k, _)| k == "CustomName").map(|(_, v)| v.clone());
        level.emit(Event::CopperGolemStatue { pos, pose, facing, name });
        Self::drop_preserved_equipment(e, m, level);
        e.discard();
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.copper_golem_become_statue", source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        if crate::leash::is_leashed(e) {
            if level.entity_drops() {
                crate::leash::drop_leash(e, Some(m), level);
            } else {
                crate::leash::remove_leash(e, Some(m), level);
            }
        }
    }

    /// `LivingEntity.dropPreservedEquipment`: whatever has a drop chance above one.
    pub fn drop_preserved_equipment(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        for i in 0..6 {
            if m.drop_chances[i] > 1.0 && !m.equipment[i].is_empty() {
                let stack = std::mem::replace(&mut m.equipment[i], ItemStack::empty());
                mob::spawn_at_location(e, level, stack);
            }
        }
        let s = st_mut(m);
        if s.antenna_drop > 1.0 && !s.antenna.is_empty() {
            let stack = std::mem::replace(&mut s.antenna, ItemStack::empty());
            mob::spawn_at_location(e, level, stack);
        }
    }
}

impl Kind for CopperGolem {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructor: a ground navigation that opens doors and plans 48 long, persistent,
    /// fire and the damaging neighbours avoided.
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.required_path_length = 48.0;
        m.nav.can_open_doors = true;
        m.persistence_required = true;
        m.maluses.retain(|(t, _)| !matches!(t, PathType::FireInNeighbor | PathType::DamagingInNeighbor | PathType::Fire));
        m.maluses.push((PathType::FireInNeighbor, 16.0));
        m.maluses.push((PathType::DamagingInNeighbor, 16.0));
        m.maluses.push((PathType::Fire, -1.0));
        Some(Box::new(State::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    /// The brain, then the constructor's first transport cooldown (a draw of 60 to 99).
    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        let mut b = make_brain(random);
        let cooldown = random.next_int_bounded(40) + 60;
        b.st.mem.set(Mem::TransportItemsCooldownTicks, Val::Int(cooldown));
        Some(b)
    }

    /// The brain, then `CopperGolemAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Idle]);
        }
    }

    /// `CopperGolem.tick`: the weathering clock.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !e.is_removed() {
            Self::update_weathering(e, m, level);
        }
    }

    /// `actuallyHurt`: it stops what it was doing.
    fn actually_hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) {
        st_mut(m).state = IDLE;
    }

    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(oxidation_sound(st(m).weather, "hurt"))
    }

    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(oxidation_sound(st(m).weather, "death"))
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    /// `thunderHit`: a struck golem is scraped one stage back, once per bolt.
    fn after_thunder_hit(&self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, bolt: i32) {
        let Some(uuid) = level.entity(bolt).map(|b| b.uuid) else { return };
        let s = st_mut(m);
        if s.last_lightning != Some(uuid) {
            s.last_lightning = Some(uuid);
            if s.weather != 0 {
                s.next_weathering_tick = UNSET_WEATHERING_TICK;
                s.weather -= 1;
            }
        }
    }

    /// `mobInteract`: an empty hand takes what it holds, shears take the antenna, honeycomb
    /// waxes, an axe scrapes the wax or one stage of weathering off. Then the held item's own
    /// `interactLivingEntity` (nothing here).
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let name = if stack.is_empty() { "" } else { mob::item_name(stack) };
        if stack.is_empty() {
            let held = m.equipment[mob::MAINHAND].clone();
            if !held.is_empty() {
                let at = level.player(who.id).map_or(e.position(), |p| p.pos);
                super::allay::throw_item(e, level, held, at, (0.30000001192092896, 0.30000001192092896, 0.30000001192092896), 0.3);
                m.equipment[mob::MAINHAND] = ItemStack::empty();
                return Some(Outcome::success(HeldChange::None));
            }
        }
        let axe = !stack.is_empty() && mob::item_tag(stack.item(), "minecraft:axes");
        if name == "minecraft:shears" && !st(m).antenna.is_empty() && mob::item_tag(st(m).antenna.item(), "minecraft:shearable_from_copper_golem") {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.copper_golem.shear", source: "players", volume: 1.0, pitch: 1.0 });
            let antenna = std::mem::replace(&mut st_mut(m).antenna, ItemStack::empty());
            mob::spawn_at_location_offset(e, level, antenna, 1.5);
            level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        let block = e.block_position();
        if name == "minecraft:honeycomb" && st(m).next_weathering_tick != IGNORE_WEATHERING_TICK {
            level.emit(Event::LevelEvent { event: 3003, pos: block, data: 0 });
            level.emit(Event::Sound { pos: block.center(), sound: "minecraft:item.honeycomb.wax_on", source: "block", volume: 1.0, pitch: 1.0 });
            st_mut(m).next_weathering_tick = IGNORE_WEATHERING_TICK;
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if axe && st(m).next_weathering_tick == IGNORE_WEATHERING_TICK {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.axe.scrape", source: "neutral", volume: 1.0, pitch: 1.0 });
            level.emit(Event::LevelEvent { event: 3004, pos: block, data: 0 });
            st_mut(m).next_weathering_tick = UNSET_WEATHERING_TICK;
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        if axe && st(m).weather != 0 {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.axe.scrape", source: "neutral", volume: 1.0, pitch: 1.0 });
            level.emit(Event::LevelEvent { event: 3005, pos: block, data: 0 });
            let s = st_mut(m);
            s.next_weathering_tick = UNSET_WEATHERING_TICK;
            s.weather -= 1;
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        None
    }

    /// `dropEquipment`: `dropPreservedEquipment`.
    fn drop_equipment(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        Self::drop_preserved_equipment(e, m, level);
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        if s.antenna.is_empty() { Vec::new() } else { vec![(7, s.antenna.clone())] }
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = st_mut(m);
        vec![(std::mem::take(&mut s.antenna), s.antenna_drop)]
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let next = r.get("next_weather_age").and_then(Tag::as_i64).unwrap_or(UNSET_WEATHERING_TICK);
        let weather = r.get("weather_state").and_then(Tag::as_str).and_then(|s| WEATHER.iter().position(|w| *w == s.trim_start_matches("minecraft:"))).unwrap_or(0) as u8;
        let antenna = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let antenna_drop = match r.get("drop_chances") {
            Some(Tag::Compound(dc)) => dc.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| v.as_f64()).map(|f| f as f32),
            _ => None,
        };
        let s = st_mut(m);
        s.next_weathering_tick = next;
        s.weather = weather;
        if let Some(a) = antenna {
            s.antenna = a;
        }
        if let Some(d) = antenna_drop {
            s.antenna_drop = d;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("next_weather_age", Tag::Long(s.next_weathering_tick));
        o.put("weather_state", Tag::String(WEATHER[s.weather as usize].into()));
        if !s.antenna.is_empty() {
            let entry = ("saddle".to_owned(), s.antenna.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        if s.antenna_drop != 0.085 {
            let entry = ("saddle".to_owned(), Tag::Float(s.antenna_drop));
            match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                Some((_, Tag::Compound(dc))) => dc.push(entry),
                _ => o.put("drop_chances", Tag::Compound(vec![entry])),
            }
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::copper_golem::WEATHER_STATE, &DataValue::Enum(s.weather as i32));
        d.set(data::copper_golem::COPPER_GOLEM_STATE, &DataValue::Enum(s.state as i32));
    }
}

/// `CopperGolemAi.getActivities` and the sensors of `CopperGolem.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![Box::new(sensors::NearestLivingEntities), Box::new(sensors::HurtBy)];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            AnimalPanic::new(1.5),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            InteractWithDoor::new(),
            CountDownCooldownTicks::new(Mem::GazeCooldownTicks),
            CountDownCooldownTicks::new(Mem::TransportItemsCooldownTicks),
        ],
    );
    let idle = ActivityData::create(
        Activity::Idle,
        0,
        vec![
            Timed::new(super::copper_golem_ai::TransportItemsBetweenContainers::new()),
            SetEntityLookTargetSometimes::new(Some("minecraft:player"), 6.0, (40, 80)),
            Gate::run_one_when(
                &[(Mem::WalkTarget, Status::ValueAbsent), (Mem::TransportItemsCooldownTicks, Status::ValuePresent)],
                vec![(stroll(1.0, StrollKind::LandRange { h: 2, v: 2 }), 1), (DoNothing::new(30, 60), 1)],
            ),
        ],
    );
    Brain::new(&[Mem::GazeCooldownTicks], sensors, vec![core, idle], random)
}

