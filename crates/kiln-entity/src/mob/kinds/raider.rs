//! `Raider`, `PatrollingMonster` and `AbstractIllager`: what pillagers, vindicators, evokers,
//! illusioners, ravagers and witches share. Raid membership (the raid id and wave), patrols
//! (leaders with the ominous banner walking to a far target with their companions), the
//! raider goals (long distance patrol, fetching the leader's banner, walking to the raid,
//! through the village's homes, celebrating a lost raid, holding ground while patrolling) and
//! the goals illagers share (looking at other mobs, target goals that ignore raiders).
//!
//! The raid itself (`Raid`) is the simulation's: raiders see it through
//! [`EntityLevel::raid`] and tell it what happened with [`RaidEvent`]s.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event, PoiOccupancy, RaidEvent, RaidView};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr;
use crate::mob::ext::{self, CustomGoal};
use crate::mob::goals::{self, Goal, Living, LOOK, MOVE};
use crate::mob::mth::{self, reduced_tick_delay};
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, HEAD, path, random_pos};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `Raider` and `PatrollingMonster` state.
#[derive(Clone, Debug, Default)]
pub struct RaiderState {
    /// `raid` (by id; kept when the raid stops, as vanilla keeps its reference).
    pub raid: Option<i32>,
    pub wave: i32,
    pub can_join_raid: bool,
    pub ticks_outside_raid: i32,
    /// `patrolTarget`, `patrolLeader`, `patrolling`.
    pub patrol_target: Option<BlockPos>,
    pub patrol_leader: bool,
    pub patrolling: bool,
    /// `IS_CELEBRATING`.
    pub celebrating: bool,
}

/// The state of the illager types and the ravager: the raider's, plus each type's own.
#[derive(Clone, Debug, Default)]
pub struct IllagerState {
    pub raider: RaiderState,
    /// Pillager: `IS_CHARGING_CROSSBOW` and its inventory (5 slots, banners in raids).
    pub charging: bool,
    pub inventory: Vec<ItemStack>,
    /// Spellcasters: `spellCastingTickCount` and `currentSpell` (`IllagerSpell` id).
    pub spell_ticks: i32,
    pub spell: u8,
    /// Evoker: `wololoTarget`.
    pub wololo_target: Option<i32>,
    /// Vindicator: `isJohnny`.
    pub johnny: bool,
    /// Ravager: `attackTick`, `stunnedTick`, `roarTick`.
    pub attack_tick: i32,
    pub stunned_tick: i32,
    pub roar_tick: i32,
    /// Illusioner: `clientSideIllusionTicks` is client side; the offsets are not simulated.
    pub illusion_ticks: i32,
}

pub fn illager(m: &MobData) -> Option<&IllagerState> {
    ext::state::<IllagerState>(m)
}

pub fn illager_mut(m: &mut MobData) -> Option<&mut IllagerState> {
    ext::state_mut::<IllagerState>(m)
}

/// The raider part of a mob's state (illagers, ravagers, witches).
pub fn raider(m: &MobData) -> Option<&RaiderState> {
    if let Some(s) = illager(m) {
        return Some(&s.raider);
    }
    ext::state::<super::witch::WitchState>(m).map(|s| &s.raider)
}

pub fn raider_mut(m: &mut MobData) -> Option<&mut RaiderState> {
    if ext::state::<IllagerState>(m).is_some() {
        return illager_mut(m).map(|s| &mut s.raider);
    }
    ext::state_mut::<super::witch::WitchState>(m).map(|s| &mut s.raider)
}

/// `instanceof Raider`.
pub fn is_raider(kind: MobKind) -> bool {
    matches!(kind, MobKind::Pillager | MobKind::Vindicator | MobKind::Evoker | MobKind::Illusioner | MobKind::Ravager | MobKind::Witch)
}

/// `instanceof AbstractIllager`.
pub fn is_illager(kind: MobKind) -> bool {
    matches!(kind, MobKind::Pillager | MobKind::Vindicator | MobKind::Evoker | MobKind::Illusioner)
}

/// `getCurrentRaid` as the level sees it (`None` without a raid, or once the raid is gone).
pub fn current_raid<'a>(m: &MobData, level: &'a dyn EntityLevel) -> Option<&'a RaidView> {
    raider(m)?.raid.and_then(|id| level.raid(id))
}

/// `hasActiveRaid`.
pub fn has_active_raid(m: &MobData, level: &dyn EntityLevel) -> bool {
    current_raid(m, level).is_some_and(|r| r.active)
}

fn raid_over(m: &MobData, level: &dyn EntityLevel) -> bool {
    current_raid(m, level).is_some_and(|r| r.over)
}

/// `canBeLeader` (ravagers never lead).
pub fn can_be_leader(kind: MobKind) -> bool {
    kind != MobKind::Ravager
}

/// `Raids.canJoinRaid`.
pub fn can_join_raid(e: &Entity, m: &MobData) -> bool {
    mob::is_alive(e, m) && raider(m).is_some_and(|r| r.can_join_raid) && m.no_action_time <= 2400
}

/// `Raid.joinRaid(level, wave, raider, null, true)` on the raider's side: it takes the raid and
/// the wave; the raid learns it through a [`RaidEvent::Joined`].
pub fn join_raid(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, raid: i32, wave: i32) {
    if let Some(r) = raider_mut(m) {
        r.raid = Some(raid);
        r.wave = wave;
        r.can_join_raid = true;
        r.ticks_outside_raid = 0;
    }
    level.emit(Event::Raid(RaidEvent::Joined { raid, entity: e.id, wave }));
}

// ---------------------------------------------------------------------- the ominous banner

const OMINOUS_LAYERS: [(&str, &str); 8] = [
    ("minecraft:rhombus", "cyan"),
    ("minecraft:stripe_bottom", "light_gray"),
    ("minecraft:stripe_center", "gray"),
    ("minecraft:border", "light_gray"),
    ("minecraft:stripe_middle", "black"),
    ("minecraft:half_horizontal", "light_gray"),
    ("minecraft:circle", "light_gray"),
    ("minecraft:border", "black"),
];

/// `Raid.getOminousBannerInstance`: a white banner with the eight illager layers, its item name
/// and uncommon rarity, the patterns hidden from the tooltip.
pub fn ominous_banner() -> ItemStack {
    let layers = OMINOUS_LAYERS
        .iter()
        .map(|(p, c)| Tag::Compound(vec![("pattern".into(), Tag::String((*p).into())), ("color".into(), Tag::String((*c).into()))]))
        .collect();
    let components = Tag::Compound(vec![
        ("minecraft:banner_patterns".into(), Tag::List(layers)),
        (
            "minecraft:tooltip_display".into(),
            Tag::Compound(vec![("hidden_components".into(), Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]))]),
        ),
        ("minecraft:item_name".into(), Tag::Compound(vec![("translate".into(), Tag::String("block.minecraft.ominous_banner".into()))])),
        ("minecraft:rarity".into(), Tag::String("uncommon".into())),
    ]);
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:white_banner".into())),
        ("count".into(), Tag::Int(1)),
        ("components".into(), components),
    ]);
    ItemStack::from_nbt(&tag).unwrap_or_else(|_| ItemStack::of("minecraft:white_banner", 1).unwrap_or_else(ItemStack::empty))
}

/// `ItemStack.matches(stack, ominousBanner)`.
pub fn is_ominous_banner(stack: &ItemStack) -> bool {
    !stack.is_empty() && stack.count() == 1 && stack.is_same_item_same_components(&ominous_banner())
}

/// `Raider.isCaptain`: a patrol leader wearing the ominous banner.
pub fn is_captain(m: &MobData) -> bool {
    raider(m).is_some_and(|r| r.patrol_leader) && is_ominous_banner(&m.equipment[HEAD])
}

// ---------------------------------------------------------------------- hooks

/// `Raider.aiStep` before `Monster.aiStep`: a raider that may join raids looks for one every
/// second; one fighting a player or golem in a raid stays busy.
pub fn ai_step_before(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if mob::is_alive(e, m) && raider(m).is_some_and(|r| r.can_join_raid) {
        match raider(m).and_then(|r| r.raid) {
            None => {
                if level.game_time() % 20 == 0
                    && let Some((id, wave)) = level.raid_at(e.block_position()).map(|r| (r.id, r.groups_spawned))
                    && can_join_raid(e, m)
                {
                    join_raid(e, m, level, id, wave);
                }
            }
            Some(_) => {
                if let Some(t) = goals::target(m, level)
                    && matches!(t.type_name, "minecraft:player" | "minecraft:iron_golem")
                {
                    m.no_action_time = 0;
                }
            }
        }
    }
}

/// `Raider.die`: the raid loses the raider (and its wave's leader); a player killer becomes a
/// hero of the village.
pub fn die(e: &Entity, m: &MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
    let Some(r) = raider(m) else { return };
    let Some(raid) = r.raid else { return };
    if level.raid(raid).is_none() {
        return;
    }
    let hero = source.attacker.filter(|a| level.player(*a).is_some());
    level.emit(Event::Raid(RaidEvent::Died { raid, entity: e.id, wave: r.wave, leader: r.patrol_leader, hero }));
}

/// `Raider.removeWhenFarAway` / `PatrollingMonster.removeWhenFarAway`: never while in a raid;
/// a patrolling raider only when farther than 128 blocks.
pub fn remove_when_far_away(m: &MobData, dist_sqr: f64) -> bool {
    let Some(r) = raider(m) else { return true };
    if r.raid.is_some() {
        return false;
    }
    !r.patrolling || dist_sqr > 16384.0
}

/// `Raider.finalizeSpawn` and `PatrollingMonster.finalizeSpawn` (before `Monster`'s): witches
/// spawned naturally never join raids; outside patrols, raids and structures 6% become patrol
/// leaders with the ominous banner.
pub fn finalize_spawn(m: &mut MobData, r: &mut dyn RandomSource, group: &GroupData) {
    let witch = m.kind == MobKind::Witch;
    let kind = m.kind;
    let Some(st) = raider_mut(m) else { return };
    st.can_join_raid = !witch || !group.natural;
    if !group.patrol && !group.event && !group.structure && r.next_float() < 0.06 && can_be_leader(kind) {
        st.patrol_leader = true;
    }
    let leader = st.patrol_leader;
    if group.patrol {
        st.patrolling = true;
    }
    if leader {
        m.equipment[HEAD] = ominous_banner();
        m.drop_chances[HEAD] = 2.0;
    }
}

/// `Raider` and `PatrollingMonster` `readAdditionalSaveData`.
pub fn load(m: &mut MobData, r: &mut Input) {
    let target = match r.get("patrol_target") {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
        _ => None,
    };
    let leader = r.bool_or("PatrolLeader", false);
    let patrolling = r.bool_or("Patrolling", false);
    let wave = r.int_or("Wave", 0);
    let can_join = r.bool_or("CanJoinRaid", false);
    let raid = r.num("RaidId").map(|v| v as i32);
    let Some(st) = raider_mut(m) else { return };
    st.patrol_target = target;
    st.patrol_leader = leader;
    st.patrolling = patrolling;
    st.wave = wave;
    st.can_join_raid = can_join;
    st.raid = raid;
}

/// `Raider` and `PatrollingMonster` `addAdditionalSaveData`.
pub fn save(m: &MobData, o: &mut Output) {
    let Some(st) = raider(m) else { return };
    if let Some(p) = st.patrol_target {
        o.put("patrol_target", Tag::IntArray(vec![p.x, p.y, p.z]));
    }
    o.put("PatrolLeader", Tag::Byte(st.patrol_leader as i8));
    o.put("Patrolling", Tag::Byte(st.patrolling as i8));
    o.put("Wave", Tag::Int(st.wave));
    o.put("CanJoinRaid", Tag::Byte(st.can_join_raid as i8));
    if let Some(id) = st.raid {
        o.put("RaidId", Tag::Int(id));
    }
}

/// `PatrollingMonster.setPatrolLeader`.
pub fn set_patrol_leader(m: &mut MobData, leader: bool) {
    if let Some(st) = raider_mut(m) {
        st.patrol_leader = leader;
        st.patrolling = true;
    }
}

/// `PatrollingMonster.findPatrolTarget`: somewhere within 500 blocks.
pub fn find_patrol_target(e: &mut Entity, m: &mut MobData) {
    let dx = -500 + e.random.next_int_bounded(1000);
    let dz = -500 + e.random.next_int_bounded(1000);
    let p = e.block_position().offset(dx, 0, dz);
    if let Some(st) = raider_mut(m) {
        st.patrol_target = Some(p);
        st.patrolling = true;
    }
}

/// `AbstractIllager.canAttack`: never a baby villager.
pub fn illager_can_attack(level: &dyn EntityLevel, t: &Living) -> bool {
    !(is_villager_type(t.type_name) && is_baby(level, t.id))
}

pub fn is_villager_type(name: &str) -> bool {
    matches!(name, "minecraft:villager" | "minecraft:wandering_trader")
}

pub fn is_baby(level: &dyn EntityLevel, id: i32) -> bool {
    level.entity(id).and_then(mob::data).is_some_and(MobData::baby)
}

/// `AbstractIllager.considersEntityAsAlly`: other `#illager_friends` (teams are not simulated).
pub fn illager_ally(type_name: &str) -> bool {
    mob::entity_type_tag(type_name, "minecraft:illager_friends")
}

/// `Raider.getCelebrateSound`.
pub fn celebrate_sound(kind: MobKind) -> &'static str {
    let name = match kind {
        MobKind::Illusioner => "minecraft:entity.illusioner.ambient",
        k => return mob::sound_event(&format!("minecraft:entity.{}.celebrate", k.short_name())),
    };
    mob::sound_event(name)
}

// ---------------------------------------------------------------------- raider goals

/// `Raider.registerGoals` with `PatrollingMonster`'s before it.
pub fn register_raider_goals(m: &mut MobData) {
    let g = &mut m.goals;
    g.add(4, Goal::Custom(Box::new(LongDistancePatrolGoal { speed: 0.7, leader_speed: 0.595, cooldown_until: -1 })));
    g.add(1, Goal::Custom(Box::new(ObtainRaidLeaderBannerGoal { unreachable: Vec::new(), path: None, banner: None })));
    g.add(3, Goal::Custom(Box::new(PathfindToRaidGoal { recruitment_tick: 0 })));
    g.add(4, Goal::Custom(Box::new(RaiderMoveThroughVillageGoal { speed: 1.05, distance: 1, poi: BlockPos::default(), visited: Vec::new(), stuck: false })));
    g.add(5, Goal::Custom(Box::new(RaiderCelebration)));
}

/// `PatrollingMonster.LongDistancePatrolGoal`.
#[derive(Clone, Debug)]
pub struct LongDistancePatrolGoal {
    speed: f64,
    leader_speed: f64,
    cooldown_until: i64,
}

impl LongDistancePatrolGoal {
    /// `findPatrolCompanions`: patrolling monsters within 16 that can join a patrol.
    fn companions(e: &Entity, level: &dyn EntityLevel) -> Vec<i32> {
        let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
        level
            .entities_in(&area, EntityFilter::Living, e.id)
            .into_iter()
            .filter(|&id| {
                level.entity(id).and_then(mob::data).is_some_and(|om| raider(om).is_some() && !has_active_raid(om, level))
            })
            .collect()
    }

    fn move_randomly(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, speed: f64) -> bool {
        let dx = -8 + e.random.next_int_bounded(16);
        let dz = -8 + e.random.next_int_bounded(16);
        let p = e.block_position().offset(dx, 0, dz);
        let y = level.motion_blocking_no_leaves_height(p.x, p.z);
        path::move_to(e, m, level, p.x as f64, y as f64, p.z as f64, speed)
    }
}

impl CustomGoal for LongDistancePatrolGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LongDistancePatrolGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let cooling = level.game_time() < self.cooldown_until;
        let Some(r) = raider(m) else { return false };
        r.patrolling && goals::target(m, level).is_none() && e.passengers.is_empty() && r.patrol_target.is_some() && !cooling
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(r) = raider(m).cloned() else { return };
        if !m.nav_ref().is_done() {
            return;
        }
        let companions = Self::companions(e, level);
        let Some(target) = r.patrol_target else { return };
        let centre = |p: BlockPos, v: Vec3| {
            let (dx, dy, dz) = (p.x as f64 + 0.5 - v.x, p.y as f64 + 0.5 - v.y, p.z as f64 + 0.5 - v.z);
            dx * dx + dy * dy + dz * dz
        };
        if r.patrolling && companions.is_empty() {
            if let Some(st) = raider_mut(m) {
                st.patrolling = false;
            }
        } else if r.patrol_leader && centre(target, e.position()) < 100.0 {
            find_patrol_target(e, m);
        } else {
            let long = Vec3::new(target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5);
            let me = e.position();
            let d = me - long;
            // `Vec3.yRot(90.0F)`: 90 radians, as vanilla passes it.
            let (c, s) = (mth::cos(90.0) as f64, mth::sin(90.0) as f64);
            let rot = Vec3::new(d.x * c + d.z * s, d.y, d.z * c - d.x * s);
            let long = rot.scale(0.4) + long;
            let mv = (long - me).normalize().scale(10.0) + me;
            let p = BlockPos::containing(mv.x, mv.y, mv.z);
            let p = p.at_y(level.motion_blocking_no_leaves_height(p.x, p.z));
            let speed = if r.patrol_leader { self.leader_speed } else { self.speed };
            if !path::move_to(e, m, level, p.x as f64, p.y as f64, p.z as f64, speed) {
                Self::move_randomly(e, m, level, self.speed);
                self.cooldown_until = level.game_time() + 200;
            } else if r.patrol_leader {
                for id in companions {
                    if let Some(om) = level.entity_mut(id).and_then(mob::data_mut)
                        && let Some(st) = raider_mut(om)
                    {
                        st.patrol_target = Some(p);
                        st.patrolling = true;
                    }
                }
            }
        }
    }
}

/// `Raider.ObtainRaidLeaderBannerGoal`: a raider of a wave without a live leader walks to a
/// dropped ominous banner and picks it up.
#[derive(Clone, Debug)]
pub struct ObtainRaidLeaderBannerGoal {
    /// `unreachableBannerCache`: banner entity id → game time it may be tried again.
    unreachable: Vec<(i32, i64)>,
    path: Option<path::Path>,
    banner: Option<i32>,
}

impl ObtainRaidLeaderBannerGoal {
    fn cannot_pick_up(m: &MobData, level: &dyn EntityLevel) -> bool {
        let Some(raid) = current_raid(m, level).filter(|r| r.active) else { return true };
        if raid.over || !can_be_leader(m.kind) || is_ominous_banner(&m.equipment[HEAD]) {
            return true;
        }
        let wave = raider(m).map_or(0, |r| r.wave);
        raid.leader(wave).is_some_and(|id| level.entity(id).is_some_and(|l| l.is_alive() && mob::data(l).is_some_and(|lm| lm.health > 0.0)))
    }
}

impl CustomGoal for ObtainRaidLeaderBannerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ObtainRaidLeaderBannerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if Self::cannot_pick_up(m, level) {
            return false;
        }
        let now = level.game_time();
        let range = m.attrs.value(Attr::FollowRange);
        let area = e.bounding_box().inflate(range, 8.0, range);
        let mut temp = Vec::new();
        for id in level.entities_in(&area, EntityFilter::Item, e.id) {
            let Some(item) = level.entity(id) else { continue };
            let EntityKind::Item(d) = &item.kind else { continue };
            if d.pickup_delay > 0 || !item.is_alive() || !is_ominous_banner(&d.stack) {
                continue;
            }
            let until = self.unreachable.iter().find(|(b, _)| *b == id).map_or(i64::MIN, |(_, t)| *t);
            if now < until {
                temp.push((id, until));
                continue;
            }
            let at = item.block_position();
            let p = path::create_path_to_entity(e, m, level, at, 1);
            if p.as_ref().is_some_and(|p| p.reached) {
                self.path = p;
                self.banner = Some(id);
                return true;
            }
            temp.push((id, now + 600));
        }
        self.unreachable = temp;
        false
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let (Some(b), Some(_)) = (self.banner, self.path.as_ref()) else { return false };
        if level.entity(b).is_none_or(|i| i.is_removed()) {
            return false;
        }
        !m.nav_ref().is_done() && !Self::cannot_pick_up(m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let p = self.path.clone();
        path::move_to_path(e, m, level, p, 1.149999976158142);
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.path = None;
        self.banner = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(b) = self.banner else { return };
        let Some(item) = level.entity(b) else { return };
        if item.position().distance_to_sqr(e.position()) < 1.414 * 1.414 {
            pick_up_banner(e, m, level, b);
        }
    }
}

/// `Raider.pickUpItem` of an ominous banner in an active raid without a leader: the raider
/// wears it and leads its wave.
pub fn pick_up_banner(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, item: i32) {
    let Some(raid) = current_raid(m, level).filter(|r| r.active).cloned() else { return };
    let wave = raider(m).map_or(0, |r| r.wave);
    if raid.leader(wave).is_some() {
        return;
    }
    let Some(stack) = level.entity(item).and_then(|i| match &i.kind {
        EntityKind::Item(d) => Some(d.stack.clone()),
        _ => None,
    }) else {
        return;
    };
    if !is_ominous_banner(&stack) {
        return;
    }
    let current = m.equipment[HEAD].clone();
    if !current.is_empty() && (e.random.next_float() - 0.1).max(0.0) < m.drop_chances[HEAD] {
        mob::spawn_at_location(e, level, current);
    }
    m.equipment[HEAD] = stack;
    m.drop_chances[HEAD] = 2.0;
    if let Some(i) = level.entity_mut(item) {
        i.discard();
    }
    set_patrol_leader(m, true);
    level.emit(Event::Raid(RaidEvent::Leader { raid: raid.id, wave, entity: e.id }));
}

/// `PathfindToRaidGoal`: a raider away from the village walks toward the raid's center and
/// recruits the raiders it meets.
#[derive(Clone, Debug)]
pub struct PathfindToRaidGoal {
    recruitment_tick: i32,
}

impl CustomGoal for PathfindToRaidGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PathfindToRaidGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_none() && e.passengers.is_empty() && has_active_raid(m, level) && !raid_over(m, level) && !level.is_village(e.block_position())
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        has_active_raid(m, level) && !raid_over(m, level) && !level.is_village(e.block_position())
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(raid) = current_raid(m, level).filter(|r| r.active).cloned() else { return };
        if e.tick_count > self.recruitment_tick {
            self.recruitment_tick = e.tick_count + 20;
            recruit_nearby(e, level, &raid);
        }
        if m.nav_ref().is_done() {
            let c = raid.center;
            let to = Vec3::new(c.x as f64 + 0.5, c.y as f64, c.z as f64 + 0.5);
            if let Some(p) = random_pos::default_pos_towards(e, m, level, 15, 4, to, std::f32::consts::FRAC_PI_2 as f64) {
                path::move_to(e, m, level, p.x, p.y, p.z, 1.0);
            }
        }
    }
}

/// `PathfindToRaidGoal.recruitNearby`: raiders within 16 without an active raid join it.
fn recruit_nearby(e: &Entity, level: &mut dyn EntityLevel, raid: &RaidView) {
    if !raid.active {
        return;
    }
    let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
    let recruits: Vec<i32> = level
        .entities_in(&area, EntityFilter::Living, e.id)
        .into_iter()
        .filter(|&id| {
            level.entity(id).is_some_and(|o| mob::data(o).is_some_and(|om| raider(om).is_some() && !has_active_raid(om, level) && can_join_raid(o, om)))
        })
        .collect();
    for id in recruits {
        let Some(o) = level.entity_mut(id) else { continue };
        if let Some(om) = mob::data_mut(o)
            && let Some(r) = raider_mut(om)
        {
            r.raid = Some(raid.id);
            r.wave = raid.groups_spawned;
            r.can_join_raid = true;
            r.ticks_outside_raid = 0;
        }
        level.emit(Event::Raid(RaidEvent::Joined { raid: raid.id, entity: id, wave: raid.groups_spawned }));
    }
}

/// `Raider.RaiderMoveThroughVillageGoal`: in a raid, raiders without a target walk from home to
/// home (beds), remembering the last few they reached.
#[derive(Clone, Debug)]
pub struct RaiderMoveThroughVillageGoal {
    speed: f64,
    distance: i32,
    poi: BlockPos,
    visited: Vec<BlockPos>,
    stuck: bool,
}

fn closer_to_center(p: BlockPos, v: Vec3, dist: f64) -> bool {
    let (dx, dy, dz) = (p.x as f64 + 0.5 - v.x, p.y as f64 + 0.5 - v.y, p.z as f64 + 0.5 - v.z);
    dx * dx + dy * dy + dz * dz < dist * dist
}

impl CustomGoal for RaiderMoveThroughVillageGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RaiderMoveThroughVillageGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.visited.len() > 2 {
            self.visited.remove(0);
        }
        if !(has_active_raid(m, level) && !raid_over(m, level)) {
            return false;
        }
        // `PoiManager.getRandom(HOME, notVisited, ANY, pos, 48, random)`.
        let mut homes = level.poi_in_range(&["minecraft:home"], e.block_position(), 48, PoiOccupancy::Any);
        let n = homes.len();
        for i in (2..=n).rev() {
            let j = e.random.next_int_bounded(i as i32) as usize;
            homes.swap(i - 1, j);
        }
        let Some(p) = homes.into_iter().find(|p| !self.visited.contains(p)) else { return false };
        self.poi = p;
        goals::target(m, level).is_none()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.nav_ref().is_done() {
            return false;
        }
        goals::target(m, level).is_none() && !closer_to_center(self.poi, e.position(), e.width as f64 + self.distance as f64) && !self.stuck
    }
    fn stop(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        if closer_to_center(self.poi, e.position(), self.distance as f64) {
            self.visited.push(self.poi);
        }
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.no_action_time = 0;
        let p = self.poi;
        path::move_to(e, m, level, p.x as f64, p.y as f64, p.z as f64, self.speed);
        self.stuck = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !m.nav_ref().is_done() {
            return;
        }
        let p = self.poi;
        let to = Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
        let next = random_pos::default_pos_towards(e, m, level, 16, 7, to, std::f32::consts::PI as f64 / 10.0)
            .or_else(|| random_pos::default_pos_towards(e, m, level, 8, 7, to, std::f32::consts::FRAC_PI_2 as f64));
        match next {
            Some(n) => {
                path::move_to(e, m, level, n.x, n.y, n.z, self.speed);
            }
            None => self.stuck = true,
        }
    }
}

/// `Raider.RaiderCelebration`: raiders of a lost raid without a target cheer and jump.
#[derive(Clone, Debug)]
pub struct RaiderCelebration;

impl CustomGoal for RaiderCelebration {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RaiderCelebration"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        mob::is_alive(e, m) && goals::target(m, level).is_none() && current_raid(m, level).is_some_and(|r| r.loss)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if let Some(r) = raider_mut(m) {
            r.celebrating = true;
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if let Some(r) = raider_mut(m) {
            r.celebrating = false;
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !e.silent && e.random.next_int_bounded(reduced_tick_delay(100)) == 0 {
            mob::make_sound(e, m, level, celebrate_sound(m.kind));
        }
        if e.vehicle.is_none() && e.random.next_int_bounded(reduced_tick_delay(50)) == 0 {
            m.jump.jump = true;
        }
    }
}

/// `Raider.HoldGroundAttackGoal`: a patrolling raider that spots a target stops, rallies the
/// raiders around it to the same target and only charges once it comes close.
#[derive(Clone, Debug)]
pub struct HoldGroundAttackGoal {
    pub hostile_radius_sqr: f32,
}

/// Raiders within 8 of `e` (`getNearbyEntities(Raider.class, shoutTargeting, ...)`).
fn shout_targets(e: &Entity, level: &dyn EntityLevel) -> Vec<i32> {
    let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
    level
        .entities_in(&area, EntityFilter::Living, e.id)
        .into_iter()
        .filter(|&id| {
            level.entity(id).is_some_and(|o| {
                mob::data(o).is_some_and(|om| is_raider(om.kind) && om.health > 0.0) && o.position().distance_to_sqr(e.position()) <= 64.0
            })
        })
        .collect()
}

impl CustomGoal for HoldGroundAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "HoldGroundAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(r) = raider(m) else { return false };
        let by_player = m.last_hurt_by_mob.is_some_and(|id| level.player(id).is_some());
        r.raid.is_none() && r.patrolling && goals::target(m, level).is_some() && !m.aggressive && !by_player
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.nav_mut().stop();
        let t = goals::target(m, level).map(|t| t.id);
        for id in shout_targets(e, level) {
            mob::set_target_of(level, id, t);
        }
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level).map(|t| t.id) else { return };
        for id in shout_targets(e, level) {
            mob::set_target_of(level, id, Some(t));
            if let Some(om) = level.entity_mut(id).and_then(mob::data_mut) {
                om.set_aggressive(true);
            }
        }
        m.set_aggressive(true);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        if e.position().distance_to_sqr(t.pos) > self.hostile_radius_sqr as f64 {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
            if e.random.next_int_bounded(50) == 0 {
                play_ambient_sound(e, m, level);
            }
        } else {
            m.set_aggressive(true);
        }
    }
}

/// `Mob.playAmbientSound`.
pub fn play_ambient_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if let Some(s) = m.kind.ambient_sound() {
        mob::make_sound(e, m, level, s);
    }
}

// ---------------------------------------------------------------------- shared goals

/// `LookAtPlayerGoal(this, Mob.class, dist)`: now and then looks at the nearest visible mob.
#[derive(Clone, Debug)]
pub struct LookAtMobGoal {
    pub dist: f32,
    pub probability: f32,
    look_at: Option<i32>,
    look_time: i32,
}

impl LookAtMobGoal {
    pub fn new(dist: f32) -> LookAtMobGoal {
        LookAtMobGoal { dist, probability: 0.02, look_at: None, look_time: 0 }
    }
}

impl CustomGoal for LookAtMobGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_float() >= self.probability {
            return false;
        }
        if let Some(t) = m.target {
            self.look_at = Some(t);
        }
        let d = self.dist as f64;
        let area = e.bounding_box().inflate(d, 3.0, d);
        let eye = Vec3::new(e.x(), e.eye_y(), e.z());
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            let Some(t) = goals::living(level, id) else { continue };
            if t.player || !goals::targeting_ok(e, m, level, &t, false, d, true) {
                continue;
            }
            let dd = t.pos.distance_to_sqr(eye);
            if best.is_none_or(|(b, _)| dd < b) {
                best = Some((dd, id));
            }
        }
        self.look_at = best.map(|(_, id)| id);
        self.look_at.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return false };
        t.alive && e.position().distance_to_sqr(t.pos) <= (self.dist * self.dist) as f64 && self.look_time > 0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time = reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_at = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return };
        if !t.alive {
            return;
        }
        crate::mob::control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
        self.look_time -= 1;
    }
}

/// A shared goal under a vanilla name with changes around it: `HurtByTargetGoal(this,
/// Raider.class)` ignores hurts from raiders; `NearestAttackableTargetGoal.setUnseenMemoryTicks`
/// remembers an unseen target longer.
#[derive(Clone, Debug)]
pub struct Wrapped {
    name: &'static str,
    inner: Goal,
    /// `toIgnoreDamage` is `Raider`.
    ignore_raiders: bool,
    /// `unseenMemoryTicks` for a nearest attackable target goal.
    memory: i32,
}

/// `HurtByTargetGoal(this, Raider.class).setAlertOthers()`.
pub fn hurt_by_ignoring_raiders() -> Goal {
    Goal::Custom(Box::new(Wrapped {
        name: "HurtByTargetGoal",
        inner: Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 },
        ignore_raiders: true,
        memory: 60,
    }))
}

/// `HurtByTargetGoal(this, Raider.class)` without alerting others (witches).
pub fn hurt_by_ignoring_raiders_alone() -> Goal {
    Goal::Custom(Box::new(Wrapped {
        name: "HurtByTargetGoal",
        inner: Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 },
        ignore_raiders: true,
        memory: 60,
    }))
}

/// `NearestAttackableTargetGoal(...).setUnseenMemoryTicks(memory)`.
pub fn nearest_with_memory(wanted: goals::Wanted, must_see: bool, memory: i32) -> Goal {
    Goal::Custom(Box::new(Wrapped {
        name: "NearestAttackableTargetGoal",
        inner: Goal::NearestAttackable { wanted, interval: reduced_tick_delay(10), must_see, target: None, unseen: 0, spider: false },
        ignore_raiders: false,
        memory,
    }))
}

impl CustomGoal for Wrapped {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.ignore_raiders
            && m.last_hurt_by_mob_timestamp != hurt_timestamp(&self.inner)
            && let Some(by) = m.last_hurt_by_mob
            && level.entity(by).and_then(mob::data).is_some_and(|om| is_raider(om.kind))
        {
            return false;
        }
        goals::can_use(&mut self.inner, e, m, level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Goal::NearestAttackable { must_see, target, unseen, .. } = &mut self.inner {
            return goals::continue_target(e, m, level, *target, *must_see, unseen, self.memory);
        }
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

fn hurt_timestamp(g: &Goal) -> i32 {
    match g {
        Goal::HurtByTarget { timestamp, .. } => *timestamp,
        _ => 0,
    }
}

/// `AvoidEntityGoal<Creaking>(this, Creaking.class, 8.0F, walk, sprint)` of the illagers.
pub fn avoid_creaking(walk: f64, sprint: f64) -> Goal {
    Goal::Custom(Box::new(super::common_a::AvoidEntityGoal::new("AvoidEntityGoal", super::common_a::Avoid::Types(&["minecraft:creaking"]), 8.0, walk, sprint)))
}

/// Knockback of `LivingEntity.knockback` on entity `id` (a mob, or a player's stand-in) from
/// `strength` toward (`dx`, `dz`), for hits outside the mob tick.
pub fn knockback_other(level: &mut dyn EntityLevel, id: i32, strength: f64, dx: f64, dz: f64) {
    let Some(o) = level.entity_mut(id) else { return };
    if matches!(o.kind, EntityKind::Mob(_)) {
        mob::knockback_entity(o, strength, dx, dz);
        return;
    }
    // A player's stand-in: its client applies the push.
    let v = o.delta;
    let mut dx = dx;
    let mut dz = dz;
    while dx * dx + dz * dz < 1.0e-5 {
        dx = (o.random.next_double() - o.random.next_double()) * 0.01;
        dz = (o.random.next_double() - o.random.next_double()) * 0.01;
    }
    let k = Vec3::new(dx, 0.0, dz).normalize().scale(strength);
    let y = if o.on_ground { 0.4f64.min(v.y / 2.0 + strength) } else { v.y };
    o.delta = Vec3::new(v.x / 2.0 - k.x, y, v.z / 2.0 - k.z);
    o.needs_sync = true;
}

/// `Entity.push(x, y, z)` on entity `id`.
pub fn push_other(level: &mut dyn EntityLevel, id: i32, x: f64, y: f64, z: f64) {
    if let Some(o) = level.entity_mut(id) {
        o.delta = o.delta.add(x, y, z);
        o.needs_sync = true;
    }
}

/// The area around a mob a `getEntitiesOfClass` inflate by `r` covers.
pub fn around(e: &Entity, r: f64) -> Aabb {
    e.bounding_box().inflate(r, r, r)
}
