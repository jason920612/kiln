//! The nether group: what piglins, piglin brutes, hoglins and zoglins share. Sensors
//! (`NearestItemSensor`, `PiglinSpecificSensor`, `PiglinBruteSpecificSensor`,
//! `HoglinSpecificSensor`) and behaviours (`SetWalkTargetAwayFrom`, `CopyMemoryWithExpiry`,
//! `EraseMemoryIf`, `StopBeingAngryIfTargetDead`, `StartCelebratingIfTargetDead`, `InteractWith`,
//! `SetLookAndInteract`, `GoToTargetLocation`, `GoToWantedItem`, `BecomePassiveIfMemoryPresent`,
//! `StrollToPoi`, `StrollAroundPoi`, `Mount`, `DismountOrSkipMounting`, `CrossbowAttack`,
//! `InteractWithDoor`) in vanilla's order of conditions and random draws.

use super::memory::{GlobalPos, Tracker, Val, WalkTarget};
use super::util::{self, uniform};
use super::{Behavior, Control, Cx, Mem, Sensor, Shot, ShotBehavior, Status, Timed, shot};
use crate::behavior_boilerplate;
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{BlockPos, Vec3};
use crate::mob::goals::{self, Living};
use crate::mob::{self, MobData};
use crate::sensor_boilerplate;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};

use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- helpers

pub const PIGLIN: &str = "minecraft:piglin";
pub const PIGLIN_BRUTE: &str = "minecraft:piglin_brute";
pub const HOGLIN: &str = "minecraft:hoglin";
pub const ZOGLIN: &str = "minecraft:zoglin";
pub const PLAYER: &str = "minecraft:player";

/// `PiglinAi.isZombified`: zombified piglins and zoglins.
pub fn is_zombified_type(name: &str) -> bool {
    name == "minecraft:zombified_piglin" || name == ZOGLIN
}

/// The mob data of entity `id` (players and gone entities have none).
pub fn mob_data<'a>(cx: &'a Cx, id: i32) -> Option<&'a MobData> {
    mob::data(cx.level.entity(id)?)
}

/// `isBaby()` of the mob `id` (false for what is not a mob).
pub fn is_baby(cx: &Cx, id: i32) -> bool {
    mob_data(cx, id).is_some_and(|m| m.baby())
}

/// `goals::living` (`util::living`), also for the mob that is ticking.
pub fn living_now(cx: &Cx, id: i32) -> Option<Living> {
    living_or_ticking(&*cx.level, id)
}

/// `Level.getEntity(uuid)` for a living entity near this mob (players first).
pub fn living_by_uuid(cx: &Cx, uuid: u128) -> Option<Living> {
    if let Some(l) = TICKING.with(|t| t.borrow().as_ref().filter(|(_, u)| *u == uuid).map(|(l, _)| l.clone())) {
        return Some(l);
    }
    let area = cx.e.bounding_box().inflate(128.0, 128.0, 128.0);
    for p in cx.level.players_in(&area) {
        if p.uuid == uuid {
            return Some(goals::living_player(&p));
        }
    }
    for id in cx.level.entities_in(&area, EntityFilter::Living, cx.e.id) {
        if cx.level.entity(id).is_some_and(|o| o.uuid == uuid) {
            return goals::living(&*cx.level, id);
        }
    }
    None
}

/// `BehaviorUtils.getLivingEntityFromUUIDMemory`.
pub fn living_from_uuid_memory(cx: &Cx, mem: Mem) -> Option<Living> {
    let u = cx.b.mem.uuid(mem)?;
    living_by_uuid(cx, u)
}

/// The uuid of a living entity (players included).
pub fn uuid_of(level: &dyn EntityLevel, id: i32) -> u128 {
    match level.player(id) {
        Some(p) => p.uuid,
        None => match level.entity(id) {
            Some(e) => e.uuid,
            None => TICKING.with(|t| t.borrow().as_ref().filter(|(l, _)| l.id == id).map_or(0, |(_, u)| *u)),
        },
    }
}

/// `Entity.closerThan(entity, dist)`.
pub fn closer_than(cx: &Cx, other: Vec3, dist: f64) -> bool {
    cx.e.position().distance_to_sqr(other) < dist * dist
}

/// `Vec3i.closerToCenterThan(pos, dist)`.
pub fn closer_to_center(b: BlockPos, p: Vec3, dist: f64) -> bool {
    Vec3::new(b.x as f64 + 0.5, b.y as f64 + 0.5, b.z as f64 + 0.5).distance_to_sqr(p) < dist * dist
}

/// The dimension a global position is in (the level's, as far as the level tells: the nether has
/// `fast_lava`).
pub fn dimension_of(level: &dyn EntityLevel) -> &'static str {
    if level.fast_lava() { "minecraft:the_nether" } else { "minecraft:overworld" }
}

/// Item stack of the item entity `id`.
pub fn item_stack_of(level: &dyn EntityLevel, id: i32) -> Option<ItemStack> {
    match &level.entity(id)?.kind {
        crate::entity::EntityKind::Item(d) => Some(d.stack.clone()),
        _ => None,
    }
}

thread_local! {
    /// The mob whose brain is ticking now (its entity is out of the level): hits it lands reach
    /// mobs that look the attacker up, which the level cannot show.
    static TICKING: std::cell::RefCell<Option<(Living, u128)>> = const { std::cell::RefCell::new(None) };
}

/// `LivingEntity` view of a mob out of the level (`goals::living` for a mob in it).
pub fn living_of_mob(e: &crate::entity::Entity, m: &MobData) -> Living {
    Living {
        id: e.id,
        type_name: e.type_name,
        pos: e.position(),
        eye_y: e.eye_y(),
        alive: e.is_alive() && m.health > 0.0,
        player: false,
        creative: false,
        spectator: false,
        invulnerable: e.invulnerable,
        sneaking: false,
        invisible: mob::effects::invisible(m),
        armor_cover: mob::armor_cover(m),
        bb: e.bounding_box(),
    }
}

/// Marks the mob as the one that ticks (`None`: nobody).
pub fn set_ticking(mob: Option<(&crate::entity::Entity, &MobData)>) {
    TICKING.with(|t| *t.borrow_mut() = mob.map(|(e, m)| (living_of_mob(e, m), e.uuid)));
}

/// `goals::living`, also for the mob that is ticking.
pub fn living_or_ticking(level: &dyn EntityLevel, id: i32) -> Option<Living> {
    goals::living(level, id).or_else(|| TICKING.with(|t| t.borrow().as_ref().filter(|(l, _)| l.id == id).map(|(l, _)| l.clone())))
}

/// A placeholder entity standing in a level's slot while its real entity is taken out.
pub fn marker() -> crate::entity::Entity {
    crate::entity::Entity::new("minecraft:marker", i32::MIN, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0)
}

/// Runs `f` as the mob `id` (its entity, data and brain taken out of the level for the duration):
/// what vanilla does to another mob's brain (`setAngerTarget(level, otherPiglin, ...)`) runs as
/// that mob. `None` when `id` is no brain mob (or is the mob itself).
pub fn as_mob<R>(cx: &mut Cx, id: i32, f: impl FnOnce(&mut Cx) -> R) -> Option<R> {
    if id == cx.e.id {
        return None;
    }
    let time = cx.time;
    let slot = cx.level.entity_mut(id)?;
    mob::data(slot)?;
    let mut be = std::mem::replace(slot, marker());
    let mut bm = mob::take(&mut be);
    let mut brain = bm.brain.take();
    let r = brain.as_mut().map(|b| {
        let mut c2 = Cx { e: &mut be, m: &mut bm, level: &mut *cx.level, b: &mut b.st, time };
        f(&mut c2)
    });
    bm.brain = brain;
    mob::put(&mut be, bm);
    if let Some(slot) = cx.level.entity_mut(id) {
        *slot = be;
    }
    r
}

/// `BehaviorBuilder.triggerIf(predicate, oneShot)`: the one-shot when the predicate holds (its
/// conditions are those of the one-shot).
pub fn trigger_if(pred: fn(&mut Cx) -> bool, inner: Box<dyn Control>) -> Box<dyn Control> {
    Box::new(TriggerIf { pred, inner })
}

#[derive(Clone)]
struct TriggerIf {
    pred: fn(&mut Cx) -> bool,
    inner: Box<dyn Control>,
}

impl std::fmt::Debug for TriggerIf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TriggerIf({:?})", self.inner)
    }
}

impl Control for TriggerIf {
    fn name(&self) -> &'static str {
        ""
    }
    fn running(&self) -> bool {
        false
    }
    fn required(&self, out: &mut Vec<Mem>) {
        self.inner.required(out);
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        (self.pred)(cx) && self.inner.try_start(cx)
    }
    fn tick_or_stop(&mut self, _cx: &mut Cx) {}
    fn do_stop(&mut self, _cx: &mut Cx) {}
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        self.inner.seed_gates(base, k);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// `UniformInt.sample(random)`.
pub fn sample(r: &mut dyn RandomSource, min: i32, max: i32) -> i32 {
    uniform(r, min, max)
}

/// `(min, max)` of `TimeUtil.rangeOfSeconds(a, b)`.
pub const fn seconds(a: i32, b: i32) -> (i32, i32) {
    (a * 20, b * 20)
}

// ---------------------------------------------------------------------------- sensors

/// `NearestItemSensor`: the closest item the mob wants to pick up, in sight, within 32 blocks.
#[derive(Clone, Debug)]
pub struct NearestItems {
    /// `Mob.wantsToPickUp(level, stack)`.
    pub wants: fn(&Cx, &ItemStack) -> bool,
}

impl Sensor for NearestItems {
    fn name(&self) -> &'static str {
        "NearestItemSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleWantedItem]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let area = cx.e.bounding_box().inflate(32.0, 16.0, 32.0);
        let me = cx.e.position();
        let mut items: Vec<(f64, i32)> = cx.level.entities_in(&area, EntityFilter::Item, cx.e.id).into_iter().filter_map(|id| cx.level.entity(id).map(|o| (o.position().distance_to_sqr(me), id))).collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut found = None;
        for (d, id) in items {
            let Some(stack) = item_stack_of(&*cx.level, id) else { continue };
            if !(self.wants)(cx, &stack) || d >= 32.0 * 32.0 {
                continue;
            }
            let Some(o) = cx.level.entity(id) else { continue };
            let from = Vec3::new(cx.e.x(), cx.e.eye_y(), cx.e.z());
            let to = Vec3::new(o.x(), o.eye_y(), o.z());
            if to.distance_to_sqr(from).sqrt() <= 128.0 && !mob::clip_blocks(&*cx.level, from, to) {
                found = Some(id);
                break;
            }
        }
        cx.b.mem.set_opt(Mem::NearestVisibleWantedItem, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

/// `BlockPos.withinBoxByManhattanDistance(pos, 8, 4)` first hit of `pred` (`findBlocksInBox...
/// filterState(pred).findFirst()`).
pub fn find_nearest_repellent(cx: &Cx, pred: fn(u16) -> bool) -> Option<BlockPos> {
    let c = cx.e.block_position();
    crate::mob::kinds::turtle::within_manhattan(c, 8, 4, 8).find(|&p| pred(cx.level.block(p)))
}

/// `#minecraft:piglin_repellents` (a soul campfire only while lit).
pub fn is_piglin_repellent(state: u16) -> bool {
    let tagged = crate::mob::kinds::wolf::block_in_tag(state, "minecraft:piglin_repellents");
    if tagged && crate::blocks::block_name(state) == "minecraft:soul_campfire" {
        return block_prop(state, "lit") == Some("true");
    }
    tagged
}

/// `#minecraft:hoglin_repellents`.
pub fn is_hoglin_repellent(state: u16) -> bool {
    crate::mob::kinds::wolf::block_in_tag(state, "minecraft:hoglin_repellents")
}

/// `PiglinAi.isWearingSafeArmor` of a player (`piglin_safe_armor` in an armor slot).
pub fn wearing_safe_armor(cx: &Cx, id: i32) -> bool {
    cx.level.player(id).is_some_and(|p| p.piglin_safe_armor)
}

/// `PiglinAi.isLovedItem(item)`.
pub fn loved_item(item: i32) -> bool {
    mob::item_tag(item, "minecraft:piglin_loved")
}

/// `PiglinAi.isPlayerHoldingLovedItem`.
pub fn player_holding_loved_item(cx: &Cx, id: i32) -> bool {
    cx.level.player(id).is_some_and(|p| loved_item(p.main_hand) || loved_item(p.off_hand))
}

/// `PiglinSpecificSensor`.
#[derive(Clone, Debug)]
pub struct PiglinSpecific;

impl Sensor for PiglinSpecific {
    fn name(&self) -> &'static str {
        "PiglinSpecificSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[
            Mem::NearestVisibleLivingEntities,
            Mem::NearestLivingEntities,
            Mem::NearestVisibleNemesis,
            Mem::NearestTargetablePlayerNotWearingGold,
            Mem::NearestPlayerHoldingWantedItem,
            Mem::NearestVisibleHuntableHoglin,
            Mem::NearestVisibleBabyHoglin,
            Mem::NearestVisibleAdultPiglins,
            Mem::NearbyAdultPiglins,
            Mem::VisibleAdultPiglinCount,
            Mem::VisibleAdultHoglinCount,
            Mem::NearestRepellent,
        ]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let rep = find_nearest_repellent(cx, is_piglin_repellent);
        cx.b.mem.set_opt(Mem::NearestRepellent, rep.map(Val::Block));
        let mut nemesis: Option<i32> = None;
        let mut huntable: Option<i32> = None;
        let mut baby_hoglin: Option<i32> = None;
        let mut zombified: Option<i32> = None;
        let mut no_gold: Option<i32> = None;
        let mut holding: Option<i32> = None;
        let mut hoglins = 0;
        let mut adults: Vec<i32> = Vec::new();
        let visible = util::find_all_visible(cx, |_, _| true);
        for id in visible {
            let Some(l) = util::living(cx, id) else { continue };
            match l.type_name {
                HOGLIN => {
                    let baby = is_baby(cx, id);
                    if baby && baby_hoglin.is_none() {
                        baby_hoglin = Some(id);
                    } else if !baby {
                        hoglins += 1;
                        if huntable.is_none() && can_be_hunted(cx, id) {
                            huntable = Some(id);
                        }
                    }
                }
                PIGLIN_BRUTE => adults.push(id),
                PIGLIN => {
                    if !is_baby(cx, id) {
                        adults.push(id);
                    }
                }
                PLAYER => {
                    if no_gold.is_none() && !wearing_safe_armor(cx, id) && goals::can_attack(cx.m, &*cx.level, &l) {
                        no_gold = Some(id);
                    }
                    if holding.is_none() && !l.spectator && player_holding_loved_item(cx, id) {
                        holding = Some(id);
                    }
                }
                t => {
                    if nemesis.is_none() && (t == "minecraft:wither_skeleton" || t == "minecraft:wither") {
                        nemesis = Some(id);
                    } else if zombified.is_none() && is_zombified_type(t) {
                        zombified = Some(id);
                    }
                }
            }
        }
        let nearby = nearby_adult_piglins(cx);
        let m = &mut cx.b.mem;
        m.set_opt(Mem::NearestVisibleNemesis, nemesis.map(Val::Entity));
        m.set_opt(Mem::NearestVisibleHuntableHoglin, huntable.map(Val::Entity));
        m.set_opt(Mem::NearestVisibleBabyHoglin, baby_hoglin.map(Val::Entity));
        m.set_opt(Mem::NearestVisibleZombified, zombified.map(Val::Entity));
        m.set_opt(Mem::NearestTargetablePlayerNotWearingGold, no_gold.map(Val::Entity));
        m.set_opt(Mem::NearestPlayerHoldingWantedItem, holding.map(Val::Entity));
        m.set(Mem::NearbyAdultPiglins, Val::Entities(nearby));
        m.set(Mem::NearestVisibleAdultPiglins, Val::Entities(adults.clone()));
        m.set(Mem::VisibleAdultPiglinCount, Val::Int(adults.len() as i32));
        m.set(Mem::VisibleAdultHoglinCount, Val::Int(hoglins));
    }
    sensor_boilerplate!();
}

/// `Hoglin.canBeHunted`.
fn can_be_hunted(cx: &Cx, id: i32) -> bool {
    mob_data(cx, id).is_some_and(|m| !m.baby() && !crate::mob::kinds::hoglin::state(m).is_some_and(|s| s.cannot_be_hunted))
}

/// `PiglinAi.findNearbyAdultPiglins`: adult piglins and brutes of `NEAREST_LIVING_ENTITIES`.
pub fn nearby_adult_piglins(cx: &Cx) -> Vec<i32> {
    cx.b.mem
        .entities(Mem::NearestLivingEntities)
        .iter()
        .copied()
        .filter(|&id| cx.level.entity(id).is_some_and(|o| (o.type_name == PIGLIN || o.type_name == PIGLIN_BRUTE) && mob::data(o).is_some_and(|m| !m.baby())))
        .collect()
}

/// `PiglinBruteSpecificSensor`.
#[derive(Clone, Debug)]
pub struct PiglinBruteSpecific;

impl Sensor for PiglinBruteSpecific {
    fn name(&self) -> &'static str {
        "PiglinBruteSpecificSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleLivingEntities, Mem::NearestVisibleNemesis, Mem::NearbyAdultPiglins]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let nemesis = util::find_closest_visible(cx, |cx, id| util::living(cx, id).is_some_and(|l| l.type_name == "minecraft:wither_skeleton" || l.type_name == "minecraft:wither"));
        let nearby = nearby_adult_piglins(cx);
        cx.b.mem.set_opt(Mem::NearestVisibleNemesis, nemesis.map(Val::Entity));
        cx.b.mem.set(Mem::NearbyAdultPiglins, Val::Entities(nearby));
    }
    sensor_boilerplate!();
}

/// `HoglinSpecificSensor`.
#[derive(Clone, Debug)]
pub struct HoglinSpecific;

impl Sensor for HoglinSpecific {
    fn name(&self) -> &'static str {
        "HoglinSpecificSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[
            Mem::NearestVisibleLivingEntities,
            Mem::NearestRepellent,
            Mem::NearestVisibleAdultPiglin,
            Mem::NearestVisibleAdultHoglins,
            Mem::VisibleAdultPiglinCount,
            Mem::VisibleAdultHoglinCount,
        ]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let rep = find_nearest_repellent(cx, is_hoglin_repellent);
        cx.b.mem.set_opt(Mem::NearestRepellent, rep.map(Val::Block));
        crate::mob::kinds::hoglin::remember_repellent(cx.m, rep);
        let mut piglin: Option<i32> = None;
        let mut piglins = 0;
        let mut hoglins: Vec<i32> = Vec::new();
        let visible = util::find_all_visible(cx, |cx, id| !is_baby(cx, id) && cx.level.entity(id).is_some_and(|o| o.type_name == PIGLIN || o.type_name == HOGLIN));
        for id in visible {
            let Some(o) = cx.level.entity(id) else { continue };
            if o.type_name == PIGLIN {
                piglins += 1;
                if piglin.is_none() {
                    piglin = Some(id);
                }
            }
            if o.type_name == HOGLIN {
                hoglins.push(id);
            }
        }
        let m = &mut cx.b.mem;
        m.set_opt(Mem::NearestVisibleAdultPiglin, piglin.map(Val::Entity));
        m.set(Mem::NearestVisibleAdultHoglins, Val::Entities(hoglins.clone()));
        m.set(Mem::VisibleAdultPiglinCount, Val::Int(piglins));
        m.set(Mem::VisibleAdultHoglinCount, Val::Int(hoglins.len() as i32));
    }
    sensor_boilerplate!();
}

// ---------------------------------------------------------------------------- entry tables

/// A `'static` entry list `[(WALK_TARGET, Registered), (mem, ValuePresent)]` per memory.
fn walk_and_present(mem: Mem) -> &'static [(Mem, Status)] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<[(Mem, Status); 2]>> = OnceLock::new();
    let t = TABLE.get_or_init(|| Mem::ALL.iter().map(|&m| [(Mem::WalkTarget, Registered), (m, ValuePresent)]).collect());
    &t[mem as usize]
}

/// `[(from, ValuePresent), (to, ValueAbsent)]` for the memory copies the group uses.
fn copy_entry(from: Mem, to: Mem) -> &'static [(Mem, Status)] {
    match (from, to) {
        (Mem::NearestVisibleNemesis, Mem::AvoidTarget) => &[(Mem::NearestVisibleNemesis, ValuePresent), (Mem::AvoidTarget, ValueAbsent)],
        (Mem::NearestVisibleZombified, Mem::AvoidTarget) => &[(Mem::NearestVisibleZombified, ValuePresent), (Mem::AvoidTarget, ValueAbsent)],
        (Mem::NearestVisibleBabyHoglin, Mem::RideTarget) => &[(Mem::NearestVisibleBabyHoglin, ValuePresent), (Mem::RideTarget, ValueAbsent)],
        _ => panic!("CopyMemoryWithExpiry {from:?} -> {to:?}: add its entry conditions"),
    }
}

// ---------------------------------------------------------------------------- behaviours

/// `CopyMemoryWithExpiry.create(predicate, from, to, duration)`: the value of `from` goes to
/// `to` for a random time.
#[derive(Clone)]
pub struct CopyMemoryWithExpiry {
    pub pred: fn(&mut Cx) -> bool,
    pub from: Mem,
    pub to: Mem,
    pub duration: (i32, i32),
}

impl std::fmt::Debug for CopyMemoryWithExpiry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CopyMemoryWithExpiry({:?} -> {:?})", self.from, self.to)
    }
}

impl CopyMemoryWithExpiry {
    pub fn new(pred: fn(&mut Cx) -> bool, from: Mem, to: Mem, duration: (i32, i32)) -> Box<dyn Control> {
        Shot::new(CopyMemoryWithExpiry { pred, from, to, duration })
    }
}

impl ShotBehavior for CopyMemoryWithExpiry {
    fn name(&self) -> &'static str {
        "CopyMemoryWithExpiry"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        copy_entry(self.from, self.to)
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        if !(self.pred)(cx) {
            return false;
        }
        let Some(v) = cx.b.mem.get(self.from).cloned() else { return false };
        let ttl = sample(cx.rng(), self.duration.0, self.duration.1) as i64;
        cx.b.mem.set_expiring(self.to, v, ttl);
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `EraseMemoryIf.create(predicate, memory)`.
pub fn erase_memory_if(pred: fn(&mut Cx) -> bool, mem: Mem) -> Box<dyn Control> {
    Shot::new(EraseMemoryIf { pred, mem })
}

#[derive(Clone)]
struct EraseMemoryIf {
    pred: fn(&mut Cx) -> bool,
    mem: Mem,
}

impl std::fmt::Debug for EraseMemoryIf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EraseMemoryIf({:?})", self.mem)
    }
}

impl ShotBehavior for EraseMemoryIf {
    fn name(&self) -> &'static str {
        "EraseMemoryIf"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        super::behaviors::cooldown_entry(self.mem)
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        if (self.pred)(cx) {
            cx.b.mem.erase(self.mem);
            return true;
        }
        false
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `SetWalkTargetAwayFrom.pos(memory, speed, desiredDistance, walkTargetSetEvenIfAlready)` and
/// `.entity(..)`: walks away from a block or an entity that is closer than the distance.
#[derive(Clone, Debug)]
pub struct SetWalkTargetAwayFrom {
    pub mem: Mem,
    pub speed: f32,
    pub distance: i32,
    pub override_walk_target: bool,
}

impl SetWalkTargetAwayFrom {
    pub fn pos(mem: Mem, speed: f32, distance: i32, override_walk_target: bool) -> Box<dyn Control> {
        Shot::new(SetWalkTargetAwayFrom { mem, speed, distance, override_walk_target })
    }

    pub fn entity(mem: Mem, speed: f32, distance: i32, override_walk_target: bool) -> Box<dyn Control> {
        Shot::new(SetWalkTargetAwayFrom { mem, speed, distance, override_walk_target })
    }

    fn target_pos(&self, cx: &Cx) -> Option<Vec3> {
        match cx.b.mem.get(self.mem)? {
            // `Vec3.atBottomCenterOf`.
            Val::Block(p) => Some(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5)),
            Val::Pos(g) => Some(Vec3::new(g.pos.x as f64 + 0.5, g.pos.y as f64, g.pos.z as f64 + 0.5)),
            Val::Entity(id) => cx.level.entity(*id).map(|e| e.position()).or_else(|| cx.level.player(*id).map(|p| p.pos)),
            _ => None,
        }
    }
}

impl ShotBehavior for SetWalkTargetAwayFrom {
    fn name(&self) -> &'static str {
        "SetWalkTargetAwayFrom"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        walk_and_present(self.mem)
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let existing = cx.b.mem.walk_target();
        if existing.is_some() && !self.override_walk_target {
            return false;
        }
        let me = cx.e.position();
        let Some(away) = self.target_pos(cx) else { return false };
        if !(me.distance_to_sqr(away) < (self.distance as f64) * (self.distance as f64)) {
            return false;
        }
        if let Some(w) = existing
            && w.speed == self.speed
        {
            let cur = util::tracker_pos(cx, &w.target).unwrap_or(me);
            let d1 = cur - me;
            let d2 = away - me;
            if d1.x * d2.x + d1.y * d2.y + d1.z * d2.z < 0.0 {
                return false;
            }
        }
        for _ in 0..10 {
            if let Some(p) = crate::mob::random_pos::land_pos_away(cx.e, cx.m, &*cx.level, 16, 7, away) {
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(p, self.speed, 0)));
                break;
            }
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `StopBeingAngryIfTargetDead.create()`.
pub fn stop_being_angry_if_target_dead() -> Box<dyn Control> {
    shot("StopBeingAngryIfTargetDead", &[(Mem::AngryAt, ValuePresent)], |cx| {
        if let Some(l) = living_from_uuid_memory(cx, Mem::AngryAt)
            && is_dead_or_dying(cx, l.id)
            && (!l.player || cx.level.forgive_dead_players())
        {
            cx.b.mem.erase(Mem::AngryAt);
        }
        true
    })
}

/// `LivingEntity.isDeadOrDying`.
pub fn is_dead_or_dying(cx: &Cx, id: i32) -> bool {
    match cx.level.player(id) {
        Some(p) => !p.alive,
        None => cx.level.entity(id).is_none_or(|e| mob::data(e).is_none_or(|m| m.is_dead_or_dying())),
    }
}

/// `StartCelebratingIfTargetDead.create(duration, wantsToDance)`.
pub fn start_celebrating_if_target_dead(duration: i32, wants_to_dance: fn(&mut Cx, &Living) -> bool) -> Box<dyn Control> {
    shot(
        "StartCelebratingIfTargetDead",
        &[(Mem::AttackTarget, ValuePresent), (Mem::AngryAt, Registered), (Mem::CelebrateLocation, ValueAbsent), (Mem::Dancing, Registered)],
        move |cx| {
            let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
            if !is_dead_or_dying(cx, id) {
                return false;
            }
            let Some(t) = util::living(cx, id) else { return false };
            if wants_to_dance(cx, &t) {
                cx.b.mem.set_expiring(Mem::Dancing, Val::Bool(true), duration as i64);
            }
            let pos = BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
            cx.b.mem.set_expiring(Mem::CelebrateLocation, Val::Block(pos), duration as i64);
            if !t.player || cx.level.forgive_dead_players() {
                cx.b.mem.erase(Mem::AttackTarget);
                cx.b.mem.erase(Mem::AngryAt);
            }
            true
        },
    )
}

/// `InteractWith.of(type, range, memory, speed, closeEnough)`: picks the closest visible entity
/// of `type` within `range` to talk to.
pub fn interact_with(entity_type: &'static str, range: i32, speed: f32, close_enough: i32) -> Box<dyn Control> {
    let r2 = (range * range) as f64;
    shot(
        "InteractWith",
        &[(Mem::InteractionTarget, Registered), (Mem::LookTarget, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent)],
        move |cx| {
            if util::find_closest_visible(cx, |cx, id| util::living(cx, id).is_some_and(|l| l.type_name == entity_type)).is_none() {
                return false;
            }
            let found = util::find_closest_visible(cx, |cx, id| {
                util::living(cx, id).is_some_and(|l| cx.e.position().distance_to_sqr(l.pos) <= r2 && l.type_name == entity_type)
            });
            if let Some(id) = found {
                cx.b.mem.set(Mem::InteractionTarget, Val::Entity(id));
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity(id, false), speed, close_enough }));
            }
            true
        },
    )
}

/// `SetLookAndInteract.create(type, distance)`.
pub fn set_look_and_interact(entity_type: &'static str, distance: i32) -> Box<dyn Control> {
    let r2 = (distance * distance) as f64;
    shot("SetLookAndInteract", &[(Mem::LookTarget, Registered), (Mem::InteractionTarget, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent)], move |cx| {
        let found = util::find_closest_visible(cx, |cx, id| util::living(cx, id).is_some_and(|l| cx.e.position().distance_to_sqr(l.pos) <= r2 && l.type_name == entity_type));
        let Some(id) = found else { return false };
        cx.b.mem.set(Mem::InteractionTarget, Val::Entity(id));
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
        true
    })
}

/// `GoToTargetLocation.create(memory, closeEnough, speed)`.
pub fn go_to_target_location(mem: Mem, close_enough: i32, speed: f32) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::CelebrateLocation => &[(Mem::CelebrateLocation, ValuePresent), (Mem::AttackTarget, ValueAbsent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered)],
        _ => panic!("GoToTargetLocation over {mem:?}: add its entry conditions"),
    };
    shot("GoToTargetLocation", entry, move |cx| {
        let Some(pos) = cx.b.mem.block(mem) else { return false };
        let me = cx.e.block_position();
        let closer = util::dist_sqr_pos(pos, me) < (close_enough as f64) * (close_enough as f64);
        if !closer {
            // `getNearbyPos`: two draws of `nextInt(3) - 1` from the level's random.
            let r = cx.rng();
            let dx = r.next_int_bounded(3) - 1;
            let dz = r.next_int_bounded(3) - 1;
            util::set_walk_and_look(cx, Tracker::block(pos.offset(dx, 0, dz)), speed, close_enough);
        }
        true
    })
}

/// `GoToWantedItem.create(predicate, speed, overrideWalkTarget, maxDistToWalk)`.
pub fn go_to_wanted_item(pred: fn(&Cx) -> bool, speed: f32, override_walk_target: bool, max_dist: i32) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = if override_walk_target {
        &[(Mem::LookTarget, Registered), (Mem::WalkTarget, Registered), (Mem::NearestVisibleWantedItem, ValuePresent), (Mem::ItemPickupCooldownTicks, Registered)]
    } else {
        &[(Mem::LookTarget, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::NearestVisibleWantedItem, ValuePresent), (Mem::ItemPickupCooldownTicks, Registered)]
    };
    shot("GoToWantedItem", entry, move |cx| {
        let Some(item) = cx.b.mem.entity(Mem::NearestVisibleWantedItem) else { return false };
        let Some(pos) = cx.level.entity(item).map(|e| e.position()) else { return false };
        if !cx.b.mem.has(Mem::ItemPickupCooldownTicks) && pred(cx) && closer_than(cx, pos, max_dist as f64) && cx.m.can_pick_up_loot {
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(item, true)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity(item, false), speed, close_enough: 0 }));
            return true;
        }
        false
    })
}

/// `BecomePassiveIfMemoryPresent.create(memory, pacifyDuration)`.
pub fn become_passive_if_memory_present(mem: Mem, duration: i32) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::NearestRepellent => &[(Mem::AttackTarget, Registered), (Mem::Pacified, ValueAbsent), (Mem::NearestRepellent, ValuePresent)],
        _ => panic!("BecomePassiveIfMemoryPresent over {mem:?}: add its entry conditions"),
    };
    shot("BecomePassiveIfMemoryPresent", entry, move |cx| {
        cx.b.mem.set_expiring(Mem::Pacified, Val::Bool(true), duration as i64);
        cx.b.mem.erase(Mem::AttackTarget);
        true
    })
}

/// `StrollToPoi.create(memory, speed, closeEnough, maxDistanceFromPoi)`.
#[derive(Clone, Debug)]
pub struct StrollToPoi {
    pub mem: Mem,
    pub speed: f32,
    pub close_enough: i32,
    pub max_distance: i32,
    next: i64,
}

impl StrollToPoi {
    pub fn new(mem: Mem, speed: f32, close_enough: i32, max_distance: i32) -> Box<dyn Control> {
        Shot::new(StrollToPoi { mem, speed, close_enough, max_distance, next: 0 })
    }
}

impl ShotBehavior for StrollToPoi {
    fn name(&self) -> &'static str {
        "StrollToPoi"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        match self.mem {
            Mem::Home => &[(Mem::WalkTarget, Registered), (Mem::Home, ValuePresent)],
            _ => panic!("StrollToPoi over {:?}: add its entry conditions", self.mem),
        }
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(home) = cx.b.mem.global_pos(self.mem).cloned() else { return false };
        if &*home.dim != dimension_of(&*cx.level) || !closer_to_center(home.pos, cx.e.position(), self.max_distance as f64) {
            return false;
        }
        if cx.time > self.next {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::block(home.pos, self.speed, self.close_enough)));
            self.next = cx.time + 80;
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `StrollAroundPoi.create(memory, speed, maxDistanceFromPoi)`.
#[derive(Clone, Debug)]
pub struct StrollAroundPoi {
    pub mem: Mem,
    pub speed: f32,
    pub max_distance: i32,
    next: i64,
}

impl StrollAroundPoi {
    pub fn new(mem: Mem, speed: f32, max_distance: i32) -> Box<dyn Control> {
        Shot::new(StrollAroundPoi { mem, speed, max_distance, next: 0 })
    }
}

impl ShotBehavior for StrollAroundPoi {
    fn name(&self) -> &'static str {
        "StrollAroundPoi"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        match self.mem {
            Mem::Home => &[(Mem::WalkTarget, Registered), (Mem::Home, ValuePresent)],
            _ => panic!("StrollAroundPoi over {:?}: add its entry conditions", self.mem),
        }
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(home) = cx.b.mem.global_pos(self.mem).cloned() else { return false };
        if &*home.dim != dimension_of(&*cx.level) || !closer_to_center(home.pos, cx.e.position(), self.max_distance as f64) {
            return false;
        }
        if cx.time > self.next {
            let p = crate::mob::random_pos::land_pos(cx.e, cx.m, &*cx.level, 8, 6);
            match p {
                Some(v) => cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(v, self.speed, 1))),
                None => cx.b.mem.erase(Mem::WalkTarget),
            }
            self.next = cx.time + 180;
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

/// `Mount.create(speed)`.
pub fn mount(speed: f32) -> Box<dyn Control> {
    shot("Mount", &[(Mem::LookTarget, Registered), (Mem::WalkTarget, ValueAbsent), (Mem::RideTarget, ValuePresent)], move |cx| {
        if cx.e.vehicle.is_some() {
            return false;
        }
        let Some(v) = cx.b.mem.entity(Mem::RideTarget) else { return false };
        let Some(pos) = cx.level.entity(v).map(|e| e.position()) else { return false };
        if closer_than(cx, pos, 1.0) {
            start_riding(cx, v);
        } else {
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(v, true)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity(v, false), speed, close_enough: 1 }));
        }
        true
    })
}

/// `Piglin.startRiding` over the level: a baby piglin on a hoglin climbs on the top of the
/// stack (at most three).
pub fn start_riding(cx: &mut Cx, vehicle: i32) -> bool {
    let mut target = vehicle;
    if cx.m.baby() && cx.level.entity(vehicle).is_some_and(|e| e.type_name == HOGLIN) {
        let mut left = 3;
        loop {
            let Some(e) = cx.level.entity(target) else { break };
            match e.passengers.first() {
                Some(&p) if left > 1 => {
                    target = p;
                    left -= 1;
                }
                _ => break,
            }
        }
    }
    let Some(v) = cx.level.entity_mut(target) else { return false };
    crate::ride::start_riding(cx.e, v, false)
}

/// `Entity.stopRiding`.
pub fn stop_riding(cx: &mut Cx) {
    if let Some(v) = cx.e.vehicle.take()
        && let Some(ve) = cx.level.entity_mut(v)
    {
        crate::ride::remove_passenger(ve, cx.e.id);
    }
}

/// `DismountOrSkipMounting.create(maxDistance, wantsToStopRiding)`.
pub fn dismount_or_skip_mounting(max_distance: i32, wants_to_stop: fn(&Cx, i32) -> bool) -> Box<dyn Control> {
    shot("DismountOrSkipMounting", &[(Mem::RideTarget, Registered)], move |cx| {
        let vehicle = cx.e.vehicle;
        let ride = cx.b.mem.entity(Mem::RideTarget);
        if vehicle.is_none() && ride.is_none() {
            return false;
        }
        let v = vehicle.or(ride).unwrap();
        let valid = cx.level.entity(v).is_some_and(|e| {
            mob::data(e).is_some_and(|m| !m.is_dead_or_dying()) && !e.is_removed() && cx.e.position().distance_to_sqr(e.position()) < (max_distance as f64) * (max_distance as f64)
        });
        if !valid || wants_to_stop(cx, v) {
            stop_riding(cx);
            cx.b.mem.erase(Mem::RideTarget);
            return true;
        }
        false
    })
}

// ---------------------------------------------------------------------------- doors

/// `InteractWithDoor.create()`: opens the doors on the path and closes those it passed. Only the
/// opening and closing of doors is simulated.
#[derive(Clone, Debug, Default)]
pub struct InteractWithDoor {
    last_node: Option<(i32, i32, i32)>,
    cooldown: i32,
}

impl InteractWithDoor {
    pub fn new() -> Box<dyn Control> {
        Shot::new(InteractWithDoor::default())
    }
}

fn is_door(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::mob::kinds::wolf::block_in_tag(level.block(p), "minecraft:mob_interactable_doors")
}

/// A property of a block state, by name.
pub fn block_prop(state: u16, name: &str) -> Option<&'static str> {
    kiln_data::blocks_types::block_of(state).property(state, name)
}

/// `DoorBlock.setOpen`: both halves of the door at `pos`.
fn set_door_open(level: &mut dyn EntityLevel, pos: BlockPos, open: bool) -> bool {
    let s = level.block(pos);
    let info = kiln_data::blocks_types::block_of(s);
    let v = if open { "true" } else { "false" };
    let Some(ns) = info.with_property(s, "open", v) else { return false };
    let other = if block_prop(s, "half") == Some("lower") { pos.above() } else { pos.below() };
    level.set_block(pos, ns, 10);
    let os = level.block(other);
    if kiln_data::blocks_types::block_of(os).name == info.name
        && let Some(nos) = kiln_data::blocks_types::block_of(os).with_property(os, "open", v)
    {
        level.set_block(other, nos, 10);
    }
    true
}

impl ShotBehavior for InteractWithDoor {
    fn name(&self) -> &'static str {
        "InteractWithDoor"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::Path, ValuePresent), (Mem::DoorsToClose, Registered), (Mem::NearestLivingEntities, Registered)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        let Some(path) = cx.m.nav.path.as_ref() else { return false };
        if path.next == 0 || path.is_done() {
            return false;
        }
        let next = path.nodes.get(path.next).map(|n| (n.x, n.y, n.z));
        if self.last_node == next {
            self.cooldown = 20;
        } else {
            self.cooldown -= 1;
            if self.cooldown > 0 {
                return false;
            }
        }
        self.last_node = next;
        let prev = path.nodes.get(path.next - 1).map(|n| BlockPos::new(n.x, n.y, n.z));
        let nxt = path.nodes.get(path.next).map(|n| BlockPos::new(n.x, n.y, n.z));
        let mut opened = Vec::new();
        for p in [prev, nxt].into_iter().flatten() {
            if is_door(&*cx.level, p) {
                let s = cx.level.block(p);
                if block_prop(s, "open") != Some("true") && set_door_open(cx.level, p, true) {
                    opened.push(p);
                }
            }
        }
        if !opened.is_empty() {
            let mut doors = cx.b.mem.positions(Mem::DoorsToClose).to_vec();
            let dim = dimension_of(&*cx.level);
            for p in opened {
                let g = GlobalPos::new(dim, p);
                if !doors.contains(&g) {
                    doors.push(g);
                }
            }
            cx.b.mem.set(Mem::DoorsToClose, Val::Positions(doors));
        }
        // Doors passed and no longer on the way close again once the mob has moved off.
        let mut doors = cx.b.mem.positions(Mem::DoorsToClose).to_vec();
        if !doors.is_empty() {
            let me = cx.e.position();
            let pn = prev;
            let nn = nxt;
            let mut keep = Vec::new();
            for g in doors.drain(..) {
                if pn == Some(g.pos) || nn == Some(g.pos) {
                    keep.push(g);
                    continue;
                }
                if !closer_to_center(g.pos, me, 3.0) {
                    continue;
                }
                if is_door(&*cx.level, g.pos) && block_prop(cx.level.block(g.pos), "open") == Some("true") {
                    // Another mob of the type coming through the door keeps it open.
                    let coming = cx
                        .b
                        .mem
                        .entities(Mem::NearestLivingEntities)
                        .iter()
                        .filter_map(|&id| cx.level.entity(id))
                        .filter(|o| o.type_name == cx.e.type_name && closer_to_center(g.pos, o.position(), 2.0))
                        .any(|o| {
                            mob::data(o).and_then(|m| m.nav.path.as_ref()).is_some_and(|pp| {
                                !pp.is_done() && pp.next > 0 && {
                                    let a = pp.nodes.get(pp.next - 1).map(|n| BlockPos::new(n.x, n.y, n.z));
                                    let b = pp.nodes.get(pp.next).map(|n| BlockPos::new(n.x, n.y, n.z));
                                    a == Some(g.pos) || b == Some(g.pos)
                                }
                            })
                        });
                    if !coming {
                        set_door_open(cx.level, g.pos, false);
                    } else {
                        keep.push(g);
                    }
                }
            }
            if keep.len() != cx.b.mem.positions(Mem::DoorsToClose).len() {
                cx.b.mem.set(Mem::DoorsToClose, Val::Positions(keep));
            }
        }
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

// ---------------------------------------------------------------------------- crossbow

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrossbowState {
    Uncharged,
    Charging,
    Charged,
    ReadyToAttack,
}

/// `CrossbowAttack`: charges the crossbow, waits 1 to 2 seconds and shoots.
#[derive(Clone, Debug)]
pub struct CrossbowAttack {
    state: CrossbowState,
    attack_delay: i32,
}

impl CrossbowAttack {
    pub fn new() -> Box<dyn Control> {
        Timed::new(CrossbowAttack { state: CrossbowState::Uncharged, attack_delay: 0 })
    }

    fn target(cx: &Cx) -> Option<Living> {
        cx.b.mem.entity(Mem::AttackTarget).and_then(|id| util::living(cx, id))
    }
}

impl Behavior for CrossbowAttack {
    fn name(&self) -> &'static str {
        "CrossbowAttack"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Registered), (Mem::AttackTarget, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (1200, 1200)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        let Some(t) = Self::target(cx) else { return false };
        crate::mob::kinds::pillager::holding_crossbow(cx.m) && util::can_see(cx, t.id) && super::combat::within_attack_range(cx, &t, 0)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.b.mem.has(Mem::AttackTarget) && self.check_extra_start(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = Self::target(cx) else { return };
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(t.id, true)));
        match self.state {
            CrossbowState::Uncharged => {
                cx.m.start_using_item();
                self.state = CrossbowState::Charging;
                crate::mob::kinds::piglin::set_charging_crossbow(cx.m, true);
            }
            CrossbowState::Charging => {
                if cx.m.using_item.is_none() {
                    self.state = CrossbowState::Uncharged;
                }
                let ticks = cx.m.ticks_using_item();
                let stack = if cx.m.using_item.is_some() { crossbow_stack(cx.m) } else { None };
                if let Some(stack) = stack
                    && ticks >= crate::mob::kinds::pillager::charge_duration(&stack)
                {
                    crate::mob::kinds::pillager::release_crossbow(cx.e, cx.m, cx.level);
                    self.state = CrossbowState::Charged;
                    self.attack_delay = 20 + cx.e.random.next_int_bounded(20);
                    crate::mob::kinds::piglin::set_charging_crossbow(cx.m, false);
                }
            }
            CrossbowState::Charged => {
                self.attack_delay -= 1;
                if self.attack_delay == 0 {
                    self.state = CrossbowState::ReadyToAttack;
                }
            }
            CrossbowState::ReadyToAttack => {
                crate::mob::kinds::pillager::perform_crossbow_attack(cx.e, cx.m, cx.level, &t);
                self.state = CrossbowState::Uncharged;
            }
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        if cx.m.using_item.is_some() {
            cx.m.stop_using_item();
        }
        if crate::mob::kinds::pillager::holding_crossbow(cx.m) {
            crate::mob::kinds::piglin::set_charging_crossbow(cx.m, false);
        }
    }
    behavior_boilerplate!();
}

/// The crossbow the mob holds (`ProjectileUtil.getWeaponHoldingHand`).
fn crossbow_stack(m: &MobData) -> Option<ItemStack> {
    [mob::MAINHAND, mob::OFFHAND].iter().map(|&i| &m.equipment[i]).find(|s| mob::item_name(s) == "minecraft:crossbow").cloned()
}

/// A fresh throwaway random (`RandomSource.createThreadLocalInstance(seed)`).
pub fn thread_local_random(seed: i64) -> LegacyRandom {
    LegacyRandom::new(seed)
}
