//! `GoalSelector`, `WrappedGoal` and the goals of the mobs Kiln simulates.

use super::attributes::Attr;
use super::control::look_at;
use super::mth::{self, reduced_tick_delay};
use super::{MobData, MobKind, path, random_pos};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use kiln_javamath::random::RandomSource;

pub const MOVE: u8 = 1;
pub const LOOK: u8 = 2;
pub const JUMP: u8 = 4;
pub const TARGET: u8 = 8;

/// Which entities a target or look goal looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    Player,
    /// Players at most this many blocks above or below the mob (slimes, magma cubes, ghasts:
    /// `Math.abs(target.getY() - getY()) <= 4`).
    PlayerWithinDy(u8),
    /// A type Kiln does not simulate yet (turtles, ...): the search always comes back empty,
    /// but the goal still draws its randomness.
    Unsimulated,
    /// Mobs of these types (`getNearestEntity` over `getEntitiesOfClass` in the follow range box).
    Types(&'static [&'static str]),
    /// `Turtle.class` with `Turtle.BABY_ON_LAND_SELECTOR`: baby turtles out of the water.
    BabyTurtlesOnLand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeleeKind {
    Plain,
    Zombie,
    Spider,
}

#[derive(Clone, Debug)]
pub enum Goal {
    Float,
    Panic { speed: f64, pos: Vec3 },
    /// A goal that never starts (kept for its slot in the selector).
    Idle,
    /// `BreedGoal`: an animal in love walks to a partner of its type in love and breeds.
    Breed { speed: f64, partner: Option<i32>, love_time: i32 },
    /// `FollowParentGoal`: a baby walks after the nearest adult of its type.
    FollowParent { speed: f64, parent: Option<i32>, recalc: i32 },
    Tempt { speed: f64, calm_down: i32, player: Option<i32> },
    RandomStroll { speed: f64, interval: i32, check_no_action: bool, water_avoiding: Option<f32>, wanted: Vec3, force: bool },
    LookAtPlayer { dist: f32, probability: f32, look_at: Option<i32>, look_time: i32 },
    RandomLookAround { rel_x: f64, rel_z: f64, look_time: i32 },
    EatBlock { tick: i32 },
    Melee { kind: MeleeKind, speed: f64, follow_unseen: bool, path: Option<path::Path>, recalc: i32, next_attack: i32, last_can_use: i64, pathed: Vec3, raise_arm: i32 },
    RangedBow { speed: f64, interval_min: i32, radius_sqr: f32, attack_time: i32, see_time: i32, strafing_clockwise: bool, strafing_backwards: bool, strafing_time: i32 },
    Swell { target: Option<i32> },
    AvoidEntity,
    LeapAtTarget { yd: f32, target: Option<i32> },
    RestrictSun,
    FleeSun { speed: f64, wanted: Vec3 },
    /// `ZombieAttackTurtleEggGoal` (a `RemoveBlockGoal` for turtle eggs).
    RemoveTurtleEgg { next_start: i32, block: BlockPos, try_ticks: i32, max_stay: i32, reached: bool, since_reached: i32 },
    /// Goals whose start conditions Kiln's world never meets (villages, spears).
    Never,
    HurtByTarget { timestamp: i32, alert_others: bool, target_mob: Option<i32>, unseen: i32, unseen_memory: i32 },
    /// A goal of an extension type (see [`super::ext::CustomGoal`]).
    Custom(Box<dyn super::ext::CustomGoal>),
    NearestAttackable { wanted: Wanted, interval: i32, must_see: bool, target: Option<i32>, unseen: i32, spider: bool },
}

impl Goal {
    pub fn flags(&self) -> u8 {
        match self {
            Goal::Custom(c) => c.flags(),
            Goal::Float => JUMP,
            Goal::Panic { .. } | Goal::RandomStroll { .. } | Goal::Swell { .. } | Goal::FleeSun { .. } | Goal::AvoidEntity => MOVE,
            Goal::Tempt { .. } | Goal::Breed { .. } | Goal::Melee { .. } | Goal::RangedBow { .. } | Goal::RandomLookAround { .. } => MOVE | LOOK,
            Goal::LookAtPlayer { .. } => LOOK,
            Goal::EatBlock { .. } => MOVE | LOOK | JUMP,
            Goal::LeapAtTarget { .. } => JUMP | MOVE,
            Goal::RemoveTurtleEgg { .. } => MOVE | JUMP,
            Goal::HurtByTarget { .. } | Goal::NearestAttackable { .. } => TARGET,
            Goal::Idle | Goal::RestrictSun | Goal::Never | Goal::FollowParent { .. } => 0,
        }
    }

    pub(crate) fn every_tick(&self) -> bool {
        if let Goal::Custom(c) = self {
            return c.every_tick();
        }
        matches!(self, Goal::Float | Goal::RandomLookAround { .. } | Goal::Melee { .. } | Goal::RangedBow { .. } | Goal::Swell { .. })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Goal::Custom(c) => c.name(),
            Goal::Float => "float",
            Goal::Panic { .. } => "panic",
            Goal::Idle => "idle",
            Goal::Breed { .. } => "breed",
            Goal::FollowParent { .. } => "follow_parent",
            Goal::Tempt { .. } => "tempt",
            Goal::RandomStroll { .. } => "stroll",
            Goal::LookAtPlayer { .. } => "look_at_player",
            Goal::RandomLookAround { .. } => "look_around",
            Goal::EatBlock { .. } => "eat_block",
            Goal::Melee { .. } => "melee",
            Goal::RangedBow { .. } => "bow",
            Goal::Swell { .. } => "swell",
            Goal::AvoidEntity => "avoid",
            Goal::LeapAtTarget { .. } => "leap",
            Goal::RestrictSun => "restrict_sun",
            Goal::FleeSun { .. } => "flee_sun",
            Goal::RemoveTurtleEgg { .. } => "turtle_egg",
            Goal::Never => "never",
            Goal::HurtByTarget { .. } => "hurt_by",
            Goal::NearestAttackable { .. } => "nearest_attackable",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Wrapped {
    pub priority: i32,
    pub goal: Goal,
    pub running: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GoalSelector {
    pub goals: Vec<Wrapped>,
    /// The goal holding each flag (MOVE, LOOK, JUMP, TARGET).
    locked: [Option<usize>; 4],
    pub disabled: u8,
}

impl GoalSelector {
    pub fn add(&mut self, priority: i32, goal: Goal) {
        self.goals.push(Wrapped { priority, goal, running: false });
    }

    /// `GoalSelector.removeGoal`: the matching goals stop being available (a running one is
    /// simply gone: its `stop` is not called; the goals it removed here have none of note).
    pub fn remove_where(&mut self, f: impl Fn(&Goal) -> bool) {
        let mut shift = vec![0usize; self.goals.len()];
        let mut gone = vec![false; self.goals.len()];
        let mut removed = 0;
        for (i, w) in self.goals.iter().enumerate() {
            gone[i] = f(&w.goal);
            shift[i] = removed;
            if gone[i] {
                removed += 1;
            }
        }
        if removed == 0 {
            return;
        }
        for slot in self.locked.iter_mut() {
            *slot = slot.and_then(|j| if gone[j] { None } else { Some(j - shift[j]) });
        }
        let mut i = 0;
        self.goals.retain(|_| {
            i += 1;
            !gone[i - 1]
        });
    }

    pub fn set_control_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.disabled &= !flag;
        } else {
            self.disabled |= flag;
        }
    }

    pub fn running_names(&self) -> Vec<&'static str> {
        self.goals.iter().filter(|w| w.running).map(|w| w.goal.name()).collect()
    }

    pub fn is_running(&self, f: impl Fn(&Goal) -> bool) -> bool {
        self.goals.iter().any(|w| w.running && f(&w.goal))
    }
}

/// `GoalSelector.tick`.
pub fn tick(sel: &mut GoalSelector, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    for i in 0..sel.goals.len() {
        if sel.goals[i].running && (sel.goals[i].goal.flags() & sel.disabled != 0 || !can_continue(&mut sel.goals[i].goal, e, m, level)) {
            stop_goal(sel, i, e, m, level);
        }
    }
    for slot in sel.locked.iter_mut() {
        if slot.is_some_and(|i| !sel.goals[i].running) {
            *slot = None;
        }
    }
    for i in 0..sel.goals.len() {
        let w = &sel.goals[i];
        if w.running || w.goal.flags() & sel.disabled != 0 {
            continue;
        }
        let flags = w.goal.flags();
        let priority = w.priority;
        let replaceable = (0..4).filter(|b| flags & (1 << b) != 0).all(|b| match sel.locked[b] {
            None => priority < i32::MAX,
            Some(j) => interruptable(&sel.goals[j].goal) && priority < sel.goals[j].priority,
        });
        if !replaceable || !can_use(&mut sel.goals[i].goal, e, m, level) {
            continue;
        }
        for b in 0..4 {
            if flags & (1 << b) != 0 {
                if let Some(j) = sel.locked[b] {
                    stop_goal(sel, j, e, m, level);
                }
                sel.locked[b] = Some(i);
            }
        }
        if !sel.goals[i].running {
            sel.goals[i].running = true;
            start(&mut sel.goals[i].goal, e, m, level);
        }
    }
    tick_running(sel, e, m, level, true);
}

fn stop_goal(sel: &mut GoalSelector, i: usize, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if sel.goals[i].running {
        sel.goals[i].running = false;
        stop(&mut sel.goals[i].goal, e, m, level);
    }
}

/// `GoalSelector.tickRunningGoals`.
pub fn tick_running(sel: &mut GoalSelector, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, all: bool) {
    for w in sel.goals.iter_mut() {
        if w.running && (all || w.goal.every_tick()) {
            tick_goal(&mut w.goal, e, m, level);
        }
    }
}

fn interruptable(g: &Goal) -> bool {
    match g {
        Goal::Custom(c) => c.interruptable(),
        _ => true,
    }
}

// ---------------------------------------------------------------------- targets

/// A living entity as goals see it.
#[derive(Clone, Copy, Debug)]
pub struct Living {
    pub id: i32,
    /// The entity type (`minecraft:player` for players).
    pub type_name: &'static str,
    pub pos: Vec3,
    pub eye_y: f64,
    pub alive: bool,
    pub player: bool,
    pub creative: bool,
    pub spectator: bool,
    pub invulnerable: bool,
    pub sneaking: bool,
    pub invisible: bool,
    pub armor_cover: f32,
    pub bb: crate::math::Aabb,
}

impl Living {
    pub(crate) fn block_pos(&self) -> BlockPos {
        BlockPos::containing(self.pos.x, self.pos.y, self.pos.z)
    }

    fn dist_sqr(&self, x: f64, y: f64, z: f64) -> f64 {
        let (a, b, c) = (self.pos.x - x, self.pos.y - y, self.pos.z - z);
        a * a + b * b + c * c
    }

    /// `canBeSeenByAnyone`.
    fn seen_by_anyone(&self) -> bool {
        !self.spectator && self.alive
    }

    /// `canBeSeenAsEnemy` (creative players are invulnerable).
    fn seen_as_enemy(&self) -> bool {
        !self.invulnerable && !self.creative && self.seen_by_anyone()
    }
}

pub fn living(level: &dyn EntityLevel, id: i32) -> Option<Living> {
    if let Some(p) = level.player(id) {
        return Some(living_player(&p));
    }
    living_entity(level, id)
}

/// A player as a [`Living`] target, from its view (no lookup).
pub fn living_player(p: &crate::level::PlayerView) -> Living {
    {
        let id = p.id;
        let h = if p.sneaking { 1.5 } else { 1.8 };
        return Living {
            id,
            type_name: "minecraft:player",
            pos: p.pos,
            eye_y: p.pos.y + p.eye_height as f64,
            alive: p.alive,
            player: true,
            creative: p.creative,
            spectator: p.spectator,
            invulnerable: p.creative,
            sneaking: p.sneaking,
            invisible: p.invisible,
            armor_cover: p.armor_cover,
            bb: crate::math::Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3),
        };
    }
}

fn living_entity(level: &dyn EntityLevel, id: i32) -> Option<Living> {
    let e = level.entity(id)?;
    let crate::entity::EntityKind::Mob(m) = &e.kind else { return None };
    Some(Living {
        id,
        type_name: e.type_name,
        pos: e.position(),
        eye_y: e.eye_y(),
        alive: e.is_alive() && m.health > 0.0,
        player: false,
        creative: false,
        spectator: false,
        invulnerable: e.invulnerable,
        sneaking: false,
        invisible: super::effects::invisible(m),
        armor_cover: super::armor_cover(m),
        bb: e.bounding_box(),
    })
}

/// `Mob.getTarget`: the target unless it became invalid (`asValidTarget`).
pub fn target(m: &MobData, level: &dyn EntityLevel) -> Option<Living> {
    let t = living(level, m.target?)?;
    if t.player && (t.creative || t.spectator) {
        return None;
    }
    if !can_attack(m, level, &t) {
        return None;
    }
    Some(t)
}

/// `LivingEntity.canAttack` (and the type's own vetoes).
pub fn can_attack(m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    if t.player && level.difficulty() == 0 {
        return false;
    }
    if let Some(k) = m.kind.ext()
        && !k.can_attack(m, level, t)
    {
        return false;
    }
    // `Axolotl.canBeSeenAsEnemy`: not while playing dead.
    if t.type_name == "minecraft:axolotl" && level.entity(t.id).and_then(super::data).is_some_and(super::kinds::axolotl::is_playing_dead) {
        return false;
    }
    t.seen_as_enemy()
}

/// `TargetingConditions.test` for a mob.
pub fn targeting_ok(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, t: &Living, combat: bool, range: f64, los: bool) -> bool {
    if t.id == e.id || !t.seen_by_anyone() {
        return false;
    }
    if combat && !can_attack(m, level, t) {
        return false;
    }
    if range > 0.0 {
        let mut vis = 1.0;
        if t.sneaking {
            vis *= 0.8;
        }
        if t.invisible {
            vis *= 0.7 * t.armor_cover.max(0.1) as f64;
        }
        let vis = mth::clamp_d(vis, 0.0, 10.0);
        let d = (range * vis).max(2.0);
        if e.position().distance_to_sqr(t.pos) > d * d {
            return false;
        }
    }
    if los && !super::has_line_of_sight_cached(e, m, level, t) {
        return false;
    }
    true
}

/// The players that can be within `range` of the mob (from the level's player grid, in the order
/// of `players()`), instead of every player of the level.
pub fn players_around(e: &Entity, level: &dyn EntityLevel, range: f64) -> Vec<crate::level::PlayerView> {
    // A range of zero or less has no limit (`targeting_ok`); one under two blocks still sees two.
    if !range.is_finite() || range <= 0.0 {
        return level.players().to_vec();
    }
    let r = range.max(2.0) + 1.0;
    level.players_in(&crate::math::Aabb::new(e.x() - r, e.y() - r - 2.0, e.z() - r, e.x() + r, e.eye_y() + r, e.z() + r))
}

/// `getNearestPlayer(conditions, mob, x, eyeY, z)`.
pub fn nearest_player(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, combat: bool, range: f64, los: bool, filter: impl Fn(&crate::level::PlayerView) -> bool) -> Option<Living> {
    let mut best: Option<(f64, Living)> = None;
    for p in players_around(e, level, range).iter() {
        if !filter(p) {
            continue;
        }
        let t = living_player(p);
        if !targeting_ok(e, m, level, &t, combat, range, los) {
            continue;
        }
        let d = t.dist_sqr(e.x(), e.eye_y(), e.z());
        if best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, t));
        }
    }
    best.map(|(_, t)| t)
}

/// `NearestAttackableTargetGoal<Player>.findTarget`: the nearest player passing the combat
/// conditions and the type's selector ([`super::ext::Kind::player_target_ok`]).
fn nearest_attackable_player(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, range: f64, filter: impl Fn(&crate::level::PlayerView) -> bool) -> Option<Living> {
    let k = m.kind.ext();
    let mut best: Option<(f64, Living)> = None;
    for p in players_around(e, level, range).iter() {
        if !filter(p) {
            continue;
        }
        let t = living_player(p);
        if k.is_some_and(|k| !k.player_target_ok(e, m, level, &t)) {
            continue;
        }
        if !targeting_ok(e, m, level, &t, true, range, true) {
            continue;
        }
        let d = t.dist_sqr(e.x(), e.eye_y(), e.z());
        if best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, t));
        }
    }
    best.map(|(_, t)| t)
}

/// `NearestAttackableTargetGoal.findTarget` for mob types: the nearest (to the eyes) mob of
/// `types` in the box `range` around (4 up and down) that passes the combat conditions.
pub fn nearest_mob(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, range: f64, must_see: bool, types: &[&str]) -> Option<i32> {
    nearest_mob_where(e, m, level, range, must_see, types, |_| true)
}

/// [`nearest_mob`] with a selector on the candidates.
pub fn nearest_mob_where(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, range: f64, must_see: bool, types: &[&str], selector: impl Fn(i32) -> bool) -> Option<i32> {
    let area = e.bounding_box().inflate(range, 4.0, range);
    let mut best: Option<(f64, i32)> = None;
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        let Some(t) = living(level, id) else { continue };
        if t.player || !types.contains(&t.type_name) || !selector(id) {
            continue;
        }
        if !targeting_ok(e, m, level, &t, true, range, must_see) {
            continue;
        }
        let d = t.dist_sqr(e.x(), e.eye_y(), e.z());
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

// ---------------------------------------------------------------------- goal logic

fn nav_done(m: &MobData) -> bool {
    m.nav_ref().is_done()
}

pub(crate) fn can_use(g: &mut Goal, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    let every = g.every_tick();
    let adj = |t: i32| if every { t } else { reduced_tick_delay(t) };
    match g {
        Goal::Custom(c) => c.can_use(e, m, level),
        Goal::Float => (e.fluid_height_water() > fluid_jump_threshold(e)) || e.is_in_lava(),
        Goal::Panic { pos, .. } => {
            if !should_panic(m, level) {
                return false;
            }
            // `lookForWater` when burning: Kiln's mobs panic to a random position instead
            // (fire is rare for animals).
            match random_pos::default_pos(e, m, level, 5, 4) {
                Some(p) => {
                    *pos = p;
                    true
                }
                None => false,
            }
        }
        Goal::Idle | Goal::Never | Goal::AvoidEntity => false,
        Goal::Breed { partner, .. } => {
            if m.in_love <= 0 {
                return false;
            }
            *partner = free_partner(e, m, level);
            partner.is_some()
        }
        Goal::FollowParent { parent, .. } => {
            if m.age >= 0 {
                return false;
            }
            let area = e.bounding_box().inflate(8.0, 4.0, 8.0);
            let mut best: Option<(f64, i32)> = None;
            for id in level.entities_in(&area, crate::level::EntityFilter::Living, i32::MIN) {
                let Some(o) = level.entity(id) else { continue };
                let crate::entity::EntityKind::Mob(om) = &o.kind else { continue };
                if om.kind != m.kind || om.age < 0 {
                    continue;
                }
                let d = e.position().distance_to_sqr(o.position());
                if best.is_some_and(|(b, _)| d > b) {
                    continue;
                }
                best = Some((d, id));
            }
            match best {
                Some((d, id)) if d >= 9.0 => {
                    *parent = Some(id);
                    true
                }
                _ => false,
            }
        }
        Goal::Tempt { calm_down, player, .. } => {
            if *calm_down > 0 {
                *calm_down -= 1;
                return false;
            }
            let range = m.attrs.value(Attr::TemptRange);
            let kind = m.kind;
            let found = nearest_player(e, m, level, false, range, false, |p| kind.tempted_by(p.main_hand) || kind.tempted_by(p.off_hand));
            *player = found.map(|p| p.id);
            player.is_some()
        }
        Goal::RandomStroll { interval, check_no_action, water_avoiding, wanted, force, .. } => {
            // (`RandomStrollGoal.canUse`: a mount with a steering rider does not wander.)
            if super::has_controlling_passenger(e, m, level) {
                return false;
            }
            if !*force {
                if *check_no_action && m.no_action_time >= 100 {
                    return false;
                }
                if e.random.next_int_bounded(reduced_tick_delay(*interval)) != 0 {
                    return false;
                }
            }
            let p = match water_avoiding {
                Some(prob) => {
                    if e.is_in_water() {
                        random_pos::land_pos(e, m, level, 15, 7).or_else(|| random_pos::default_pos(e, m, level, 10, 7))
                    } else if e.random.next_float() >= *prob {
                        random_pos::land_pos(e, m, level, 10, 7)
                    } else {
                        random_pos::default_pos(e, m, level, 10, 7)
                    }
                }
                None => random_pos::default_pos(e, m, level, 10, 7),
            };
            match p {
                Some(p) => {
                    *wanted = p;
                    *force = false;
                    true
                }
                None => false,
            }
        }
        Goal::LookAtPlayer { dist, probability, look_at: la, .. } => {
            if e.random.next_float() >= *probability {
                return false;
            }
            if let Some(t) = m.target {
                *la = Some(t);
            }
            let found = nearest_player(e, m, level, false, *dist as f64, true, |_| true);
            *la = found.map(|p| p.id);
            la.is_some()
        }
        Goal::RandomLookAround { .. } => e.random.next_float() < 0.02,
        Goal::EatBlock { .. } => {
            let bound = adj(if m.baby() { 50 } else { 1000 });
            if e.random.next_int_bounded(bound) != 0 {
                return false;
            }
            let p = e.block_position();
            crate::blocks::has_tag(level.block(p), crate::blocks::Tag::EdibleForSheep) || crate::blocks::block_name(level.block(p.below())) == "minecraft:grass_block"
        }
        Goal::Melee { kind, last_can_use, path, .. } => {
            if *kind == MeleeKind::Spider && m.is_vehicle {
                return false;
            }
            let now = level.game_time();
            if now - *last_can_use < 20 {
                return false;
            }
            *last_can_use = now;
            let Some(t) = target(m, level) else { return false };
            if !t.alive {
                return false;
            }
            *path = path::create_path_to_entity(e, m, level, t.block_pos(), 0);
            path.is_some() || super::within_melee_range(e, m, &t)
        }
        Goal::RangedBow { .. } => target(m, level).is_some() && m.holding_bow(),
        Goal::Swell { .. } => {
            let t = target(m, level);
            m.swell_dir() > 0 || t.is_some_and(|t| t.alive && e.position().distance_to_sqr(t.pos) < 9.0)
        }
        Goal::LeapAtTarget { target: tg, .. } => {
            let Some(t) = target(m, level) else {
                *tg = None;
                return false;
            };
            *tg = Some(t.id);
            let d = e.position().distance_to_sqr(t.pos);
            if d < 4.0 || d > 16.0 {
                return false;
            }
            if !e.on_ground {
                return false;
            }
            e.random.next_int_bounded(reduced_tick_delay(5)) == 0
        }
        Goal::RestrictSun => level.is_bright_outside() && m.equipment[super::HEAD].is_empty(),
        Goal::FleeSun { wanted, .. } => {
            if m.target.is_some() || !level.is_bright_outside() || !e.is_on_fire() || !level.can_see_sky(e.block_position()) {
                return false;
            }
            if !m.equipment[super::HEAD].is_empty() {
                return false;
            }
            let base = e.block_position();
            for _ in 0..10 {
                let dx = e.random.next_int_bounded(20) - 10;
                let dy = e.random.next_int_bounded(6) - 3;
                let dz = e.random.next_int_bounded(20) - 10;
                let p = base.offset(dx, dy, dz);
                if !level.can_see_sky(p) && super::walk_target_value(m, level, p) < 0.0 {
                    *wanted = Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
                    return true;
                }
            }
            false
        }
        Goal::RemoveTurtleEgg { next_start, block, .. } => {
            if !level.mob_griefing() {
                return false;
            }
            if *next_start > 0 {
                *next_start -= 1;
                return false;
            }
            match find_turtle_egg(e, level) {
                Some(p) => {
                    *block = p;
                    *next_start = reduced_tick_delay(20);
                    true
                }
                None => {
                    *next_start = reduced_tick_delay(200 + e.random.next_int_bounded(200));
                    false
                }
            }
        }
        Goal::HurtByTarget { timestamp, .. } => {
            let (ts, by) = (m.last_hurt_by_mob_timestamp, m.last_hurt_by_mob);
            if ts == *timestamp {
                return false;
            }
            let Some(by) = by else { return false };
            let Some(t) = living(level, by) else { return false };
            // `HURT_BY_TARGETING`: combat, ignoring line of sight and invisibility; and
            // `TargetGoal.canAttack`: within the mob's home (a led mob keeps near its holder).
            targeting_ok(e, m, level, &t, true, -1.0, false) && super::random_pos::within_home(m.home, t.block_pos())
        }
        Goal::NearestAttackable { wanted, interval, target: tg, spider, .. } => {
            if *spider && super::light_magic_value(e, level) >= 0.5 {
                return false;
            }
            if *interval > 0 && e.random.next_int_bounded(*interval) != 0 {
                return false;
            }
            let range = m.attrs.value(Attr::FollowRange);
            *tg = match wanted {
                Wanted::Player => nearest_attackable_player(e, m, level, range, |_| true).map(|p| p.id),
                Wanted::PlayerWithinDy(dy) => {
                    let (y, dy) = (e.y(), *dy as f64);
                    nearest_attackable_player(e, m, level, range, |p| (p.pos.y - y).abs() <= dy).map(|p| p.id)
                }
                Wanted::Unsimulated => None,
                Wanted::Types(types) => nearest_mob(e, m, level, range, true, types),
                Wanted::BabyTurtlesOnLand => {
                    let on_land = |id: i32| level.entity(id).is_some_and(|o| !o.is_in_water() && super::data(o).is_some_and(|om| om.baby()));
                    nearest_mob_where(e, m, level, range, true, &["minecraft:turtle"], on_land)
                }
            };
            tg.is_some()
        }
    }
}

pub(crate) fn can_continue(g: &mut Goal, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    match g {
        Goal::Custom(c) => c.can_continue(e, m, level),
        Goal::Panic { .. } | Goal::FleeSun { .. } => !nav_done(m),
        Goal::RandomStroll { .. } => !nav_done(m),
        Goal::Tempt { .. } => can_use(g, e, m, level),
        Goal::Breed { partner, love_time, .. } => {
            let Some(p) = partner.and_then(|id| level.entity(id)) else { return false };
            let Some(pm) = super::data(p) else { return false };
            p.is_alive() && pm.health > 0.0 && pm.in_love > 0 && *love_time < 60 && !pm.goals.is_running(|g| matches!(g, Goal::Panic { .. }))
        }
        Goal::FollowParent { parent, .. } => {
            if m.age >= 0 {
                return false;
            }
            let Some(p) = parent.and_then(|id| living(level, id)) else { return false };
            if !p.alive {
                return false;
            }
            let d = e.position().distance_to_sqr(p.pos);
            !(d < 9.0) && !(d > 256.0)
        }
        Goal::LookAtPlayer { dist, look_at, look_time, .. } => {
            let Some(t) = look_at.and_then(|id| living(level, id)) else { return false };
            t.alive && e.position().distance_to_sqr(t.pos) <= (*dist * *dist) as f64 && *look_time > 0
        }
        Goal::RandomLookAround { look_time, .. } => *look_time >= 0,
        Goal::EatBlock { tick } => *tick > 0,
        Goal::Melee { kind, follow_unseen, .. } => {
            if *kind == MeleeKind::Spider && light_ok_for_spider_to_stop(e, level) && e.random.next_int_bounded(100) == 0 {
                super::set_target(e, m, None);
                return false;
            }
            let Some(t) = target(m, level) else { return false };
            if !t.alive {
                return false;
            }
            if !*follow_unseen {
                return !nav_done(m);
            }
            !(t.player && (t.spectator || t.creative))
        }
        Goal::RangedBow { .. } => (can_use(g, e, m, level) || !nav_done(m)) && m.holding_bow(),
        Goal::LeapAtTarget { .. } => !e.on_ground,
        Goal::RemoveTurtleEgg { try_ticks, max_stay, block, .. } => {
            *try_ticks >= -*max_stay && *try_ticks <= 1200 && is_turtle_egg_target(level, *block)
        }
        Goal::HurtByTarget { target_mob, unseen, unseen_memory, .. } => {
            continue_target(e, m, level, *target_mob, true, unseen, *unseen_memory)
        }
        // `NearestAttackableTargetGoal` never sets `targetMob`: once the mob's target is cleared
        // (a guardian's beam fired) the goal stops.
        Goal::NearestAttackable { must_see, unseen, .. } => continue_target(e, m, level, None, *must_see, unseen, 60),
        _ => can_use(g, e, m, level),
    }
}

fn light_ok_for_spider_to_stop(e: &Entity, level: &dyn EntityLevel) -> bool {
    super::light_magic_value(e, level) >= 0.5
}

/// `TargetGoal.canContinueToUse`.
pub fn continue_target(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, target_mob: Option<i32>, must_see: bool, unseen: &mut i32, memory: i32) -> bool {
    let id = m.target.or(target_mob);
    let Some(t) = id.and_then(|id| living(level, id)) else { return false };
    if !can_attack(m, level, &t) {
        return false;
    }
    let follow = m.attrs.value(Attr::FollowRange);
    if e.position().distance_to_sqr(t.pos) > follow * follow {
        return false;
    }
    if must_see {
        if super::has_line_of_sight_cached(e, m, level, &t) {
            *unseen = 0;
        } else {
            *unseen += 1;
            if *unseen > reduced_tick_delay(memory) {
                return false;
            }
        }
    }
    super::set_target(e, m, Some(t.id));
    true
}

pub(crate) fn start(g: &mut Goal, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let every = g.every_tick();
    let adj = |t: i32| if every { t } else { reduced_tick_delay(t) };
    match g {
        Goal::Custom(c) => c.start(e, m, level),
        Goal::Panic { speed, pos } => {
            path::move_to(e, m, level, pos.x, pos.y, pos.z, *speed);
        }
        Goal::RandomStroll { speed, wanted, .. } => {
            path::move_to(e, m, level, wanted.x, wanted.y, wanted.z, *speed);
        }
        Goal::FleeSun { speed, wanted } => {
            path::move_to(e, m, level, wanted.x, wanted.y, wanted.z, *speed);
        }
        Goal::LookAtPlayer { look_time, .. } => *look_time = adj(40 + e.random.next_int_bounded(40)),
        Goal::RandomLookAround { rel_x, rel_z, look_time } => {
            let d = std::f64::consts::TAU * e.random.next_double();
            *rel_x = kiln_javamath::trig::cos(d);
            *rel_z = kiln_javamath::trig::sin(d);
            *look_time = 20 + e.random.next_int_bounded(20);
        }
        Goal::EatBlock { tick } => {
            *tick = adj(40);
            level.emit(Event::EntityEvent { entity: e.id, event: 10 });
            m.nav_mut().stop();
        }
        Goal::Melee { speed, path, recalc, next_attack, raise_arm, .. } => {
            let p = path.take();
            path::move_to_path(e, m, level, p, *speed);
            m.set_aggressive(true);
            *recalc = 0;
            *next_attack = 0;
            *raise_arm = 0;
        }
        Goal::RangedBow { .. } => m.set_aggressive(true),
        Goal::Swell { target: t } => {
            m.nav_mut().stop();
            *t = m.target;
        }
        Goal::LeapAtTarget { yd, target: tg } => {
            let Some(t) = tg.and_then(|id| living(level, id)) else { return };
            let v = e.delta;
            let mut d = Vec3::new(t.pos.x - e.x(), 0.0, t.pos.z - e.z());
            if d.length_sqr() > 1.0e-7 {
                d = d.normalize().scale(0.4) + v.scale(0.2);
            }
            e.delta = Vec3::new(d.x, *yd as f64, d.z);
        }
        Goal::RestrictSun => m.nav_mut().avoid_sun = true,
        Goal::RemoveTurtleEgg { block, try_ticks, max_stay, .. } => {
            let b = *block;
            path::move_to(e, m, level, b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, 1.0);
            *try_ticks = 0;
            let inner = e.random.next_int_bounded(1200) + 1200;
            *max_stay = e.random.next_int_bounded(inner) + 1200;
        }
        Goal::HurtByTarget { timestamp, alert_others, target_mob, unseen, unseen_memory } => {
            let by = m.last_hurt_by_mob;
            super::set_target(e, m, by);
            *target_mob = m.target;
            *timestamp = m.last_hurt_by_mob_timestamp;
            *unseen_memory = 300;
            if *alert_others {
                alert_others_of_kind(e, m, level);
            }
            *unseen = 0;
        }
        Goal::NearestAttackable { target: tg, unseen, .. } => {
            super::set_target(e, m, *tg);
            *unseen = 0;
        }
        Goal::Tempt { .. } => {}
        Goal::FollowParent { recalc, .. } => *recalc = 0,
        _ => {}
    }
}

pub(crate) fn stop(g: &mut Goal, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    match g {
        Goal::Custom(c) => c.stop(_e, m, level),
        Goal::RandomStroll { .. } => m.nav_mut().stop(),
        Goal::Tempt { calm_down, player, .. } => {
            *player = None;
            m.nav_mut().stop();
            *calm_down = reduced_tick_delay(100);
        }
        Goal::LookAtPlayer { look_at, .. } => *look_at = None,
        Goal::Breed { partner, love_time, .. } => {
            *partner = None;
            *love_time = 0;
        }
        Goal::FollowParent { parent, .. } => *parent = None,
        Goal::EatBlock { tick } => *tick = 0,
        Goal::Melee { .. } => {
            // `NO_CREATIVE_OR_SPECTATOR.test(target)` fails for a missing target too.
            let keep = m.target.and_then(|id| living(level, id)).is_some_and(|t| !(t.player && (t.creative || t.spectator)));
            if !keep {
                super::set_target(_e, m, None);
            }
            m.set_aggressive(false);
            m.nav_mut().stop();
        }
        Goal::RangedBow { see_time, attack_time, .. } => {
            m.set_aggressive(false);
            *see_time = 0;
            *attack_time = -1;
            m.stop_using_item();
        }
        Goal::Swell { target } => *target = None,
        Goal::RestrictSun => m.nav_mut().avoid_sun = false,
        Goal::RemoveTurtleEgg { .. } => _e.fall_distance = 1.0,
        Goal::HurtByTarget { target_mob, .. } | Goal::NearestAttackable { target: target_mob, .. } => {
            super::set_target(_e, m, None);
            *target_mob = None;
        }
        _ => {}
    }
}

pub(crate) fn tick_goal(g: &mut Goal, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let every = g.every_tick();
    let adj = |t: i32| if every { t } else { reduced_tick_delay(t) };
    match g {
        Goal::Custom(c) => c.tick(e, m, level),
        Goal::Float => {
            if e.random.next_float() < 0.8 {
                m.jump.jump = true;
            }
        }
        Goal::Tempt { speed, player, .. } => {
            let Some(p) = player.and_then(|id| living(level, id)) else { return };
            let (hs, hx) = ((m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
            m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, hs, hx);
            if e.position().distance_to_sqr(p.pos) < 2.5 * 2.5 {
                m.nav_mut().stop();
            } else {
                path::move_to_entity(e, m, level, p.block_pos(), *speed);
            }
        }
        Goal::Breed { speed, partner, love_time } => {
            let Some(p) = partner.and_then(|id| living(level, id)) else { return };
            let max_x = m.max_head_x_rot() as f32;
            m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, 10.0, max_x);
            path::move_to_entity(e, m, level, p.block_pos(), *speed);
            *love_time += 1;
            if *love_time >= adj(60) && e.position().distance_to_sqr(p.pos) < 9.0 {
                super::breed::spawn_child(e, m, level, p.id);
            }
        }
        Goal::FollowParent { speed, parent, recalc } => {
            *recalc -= 1;
            if *recalc > 0 {
                return;
            }
            *recalc = adj(10);
            let Some(p) = parent.and_then(|id| living(level, id)) else { return };
            path::move_to_entity(e, m, level, p.block_pos(), *speed);
        }
        Goal::LookAtPlayer { look_at: la, look_time, .. } => {
            let Some(t) = la.and_then(|id| living(level, id)) else { return };
            if !t.alive {
                return;
            }
            look_at(m, t.pos.x, t.eye_y, t.pos.z);
            *look_time -= 1;
        }
        Goal::RandomLookAround { rel_x, rel_z, look_time } => {
            *look_time -= 1;
            look_at(m, e.x() + *rel_x, e.eye_y(), e.z() + *rel_z);
        }
        Goal::EatBlock { tick } => {
            *tick = (*tick - 1).max(0);
            if *tick != adj(4) {
                return;
            }
            let p = e.block_position();
            if crate::blocks::has_tag(level.block(p), crate::blocks::Tag::EdibleForSheep) {
                if level.mob_griefing() {
                    level.destroy_block(p, false);
                }
                super::species::ate(e, m, level);
            } else if crate::blocks::block_name(level.block(p.below())) == "minecraft:grass_block" {
                if level.mob_griefing() {
                    let grass = kiln_data::blocks::default_state::GRASS_BLOCK;
                    level.emit(Event::LevelEvent { event: 2001, pos: p.below(), data: grass as i32 });
                    level.set_block(p.below(), kiln_data::blocks::default_state::DIRT, 2);
                }
                super::species::ate(e, m, level);
            }
        }
        Goal::Melee { kind, speed, follow_unseen, recalc, next_attack, pathed, raise_arm, .. } => {
            let Some(t) = target(m, level) else { return };
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
            *recalc = (*recalc - 1).max(0);
            if (*follow_unseen || super::has_line_of_sight_cached(e, m, level, &t))
                && *recalc <= 0
                && ((pathed.x == 0.0 && pathed.y == 0.0 && pathed.z == 0.0)
                    || t.dist_sqr(pathed.x, pathed.y, pathed.z) >= 1.0
                    || e.random.next_float() < 0.05)
            {
                *pathed = t.pos;
                *recalc = 4 + e.random.next_int_bounded(7);
                let d = e.position().distance_to_sqr(t.pos);
                if d > 1024.0 {
                    *recalc += 10;
                } else if d > 256.0 {
                    *recalc += 5;
                }
                // `moveTo(target, 0, speed)`: no path (an airborne mob makes none) leaves the one it has.
                let p = path::create_path_to_entity(e, m, level, t.block_pos(), 0);
                if !(p.is_some() && path::move_to_path(e, m, level, p, *speed)) {
                    *recalc += 15;
                }
                *recalc = adj(*recalc);
            }
            *next_attack = (*next_attack - 1).max(0);
            if *next_attack <= 0 && super::within_melee_range(e, m, &t) && super::has_line_of_sight_cached(e, m, level, &t) {
                *next_attack = adj(20);
                m.swing = true;
                super::do_hurt_target(e, m, level, &t);
            }
            if *kind == MeleeKind::Zombie {
                *raise_arm += 1;
                let aggressive = *raise_arm >= 5 && *next_attack < adj(20) / 2;
                m.set_aggressive(aggressive);
            }
        }
        Goal::RangedBow { speed, interval_min, radius_sqr, attack_time, see_time, strafing_clockwise, strafing_backwards, strafing_time } => {
            let Some(t) = target(m, level) else { return };
            let d = e.position().distance_to_sqr(t.pos);
            let sees = super::has_line_of_sight_cached(e, m, level, &t);
            if sees != (*see_time > 0) {
                *see_time = 0;
            }
            if sees {
                *see_time += 1;
            } else {
                *see_time -= 1;
            }
            if d <= *radius_sqr as f64 && *see_time >= 20 {
                m.nav_mut().stop();
                *strafing_time += 1;
            } else {
                path::move_to_entity(e, m, level, t.block_pos(), *speed);
                *strafing_time = -1;
            }
            if *strafing_time >= 20 {
                if (e.random.next_float() as f64) < 0.3 {
                    *strafing_clockwise = !*strafing_clockwise;
                }
                if (e.random.next_float() as f64) < 0.3 {
                    *strafing_backwards = !*strafing_backwards;
                }
                *strafing_time = 0;
            }
            if *strafing_time > -1 {
                if d > (*radius_sqr * 0.75) as f64 {
                    *strafing_backwards = false;
                } else if d < (*radius_sqr * 0.25) as f64 {
                    *strafing_backwards = true;
                }
                m.mov_mut().strafe(if *strafing_backwards { -0.5 } else { 0.5 }, if *strafing_clockwise { 0.5 } else { -0.5 });
                // (`getControlledVehicle() instanceof Mob`: the mount turns to the target too.)
                if let Some(c) = m.mount.as_mut() {
                    super::mob_look_at(&mut c.e, &t, 30.0, 30.0);
                }
                super::mob_look_at(e, &t, 30.0, 30.0);
            } else {
                m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
            }
            if m.using_item.is_some() {
                if !sees && *see_time < -60 {
                    m.stop_using_item();
                } else if sees {
                    let ticks = m.ticks_using_item();
                    if ticks >= 20 {
                        m.stop_using_item();
                        super::species::perform_ranged_attack(e, m, level, &t, bow_power(ticks));
                        *attack_time = *interval_min;
                    }
                }
            } else {
                *attack_time -= 1;
                if *attack_time <= 0 && *see_time >= -60 {
                    m.start_using_item();
                }
            }
        }
        Goal::Swell { target: tg } => {
            let t = tg.and_then(|id| living(level, id));
            let dir = match t {
                Some(t) if t.alive && e.position().distance_to_sqr(t.pos) <= 49.0 && super::has_line_of_sight_cached(e, m, level, &t) => 1,
                _ => -1,
            };
            m.set_swell_dir(dir);
        }
        Goal::RemoveTurtleEgg { block, try_ticks, reached, since_reached, .. } => {
            let target = block.above();
            let c = Vec3::new(target.x as f64 + 0.5, target.y as f64 + 0.5, target.z as f64 + 0.5);
            if c.distance_to_sqr(e.position()) >= 1.14 * 1.14 {
                *reached = false;
                *try_ticks += 1;
                if *try_ticks % 40 == 0 {
                    path::move_to(e, m, level, target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5, 1.0);
                }
            } else {
                *reached = true;
                *try_ticks -= 1;
            }
            let here = e.block_position();
            let egg = [here, here.below(), here.offset(-1, 0, 0), here.offset(1, 0, 0), here.offset(0, 0, -1), here.offset(0, 0, 1), here.offset(0, -2, 0)]
                .into_iter()
                .find(|p| crate::blocks::block_name(level.block(*p)) == "minecraft:turtle_egg");
            if *reached && let Some(egg) = egg {
                if *since_reached > 0 {
                    e.delta = Vec3::new(e.delta.x, 0.3, e.delta.z);
                    for _ in 0..3 {
                        e.random.next_float();
                    }
                }
                if *since_reached % 2 == 0 {
                    e.delta = Vec3::new(e.delta.x, -0.3, e.delta.z);
                }
                if *since_reached > 60 {
                    level.set_block(egg, 0, 3);
                    for _ in 0..60 {
                        e.random.next_double();
                    }
                }
                *since_reached += 1;
            }
        }
        _ => {}
    }
}

/// `BowItem.getPowerForTime`.
fn bow_power(ticks: i32) -> f32 {
    let f = ticks as f32 / 20.0;
    let f = (f * f + f * 2.0) / 3.0;
    f.min(1.0)
}

/// `Entity.getFluidJumpThreshold`.
fn fluid_jump_threshold(e: &Entity) -> f64 {
    if (e.eye_height as f64) < 0.4 { 0.0 } else { 0.4 }
}

/// `PanicGoal.shouldPanic`: hurt by a `panic_causes` damage type in the last 40 ticks.
pub fn should_panic(m: &MobData, level: &dyn EntityLevel) -> bool {
    m.last_damage_source(level.game_time()).is_some_and(|s| s.kind.is_tag("minecraft:panic_causes"))
}

/// `BreedGoal.getFreePartner`: the nearest animal of the same type within 8 blocks that can
/// mate (both in love) and is not panicking.
pub fn free_partner(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<i32> {
    let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
    let mut best: Option<(f64, i32)> = None;
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        let Some(o) = level.entity(id) else { continue };
        let crate::entity::EntityKind::Mob(om) = &o.kind else { continue };
        // `PARTNER_TARGETING`: non-combat, 8 blocks, no line of sight needed.
        // (`getEntitiesOfClass(animal.getClass())`: a llama also finds trader llamas.)
        let class = om.kind == m.kind || (m.kind == MobKind::Llama && om.kind == MobKind::TraderLlama);
        if !class || !o.is_alive() || om.health <= 0.0 || e.position().distance_to_sqr(o.position()) > 64.0 {
            continue;
        }
        if !(m.in_love > 0 && om.in_love > 0) || om.goals.is_running(|g| matches!(g, Goal::Panic { .. })) {
            continue;
        }
        if let Some(k) = m.kind.ext()
            && !k.can_mate(m, om)
        {
            continue;
        }
        let d = e.position().distance_to_sqr(o.position());
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

/// `HurtByTargetGoal.alertOthers`: mobs of the same type nearby without a target.
fn alert_others_of_kind(e: &Entity, m: &MobData, level: &mut dyn EntityLevel) {
    let Some(attacker) = m.last_hurt_by_mob else { return };
    let r = m.attrs.value(Attr::FollowRange);
    let p = e.position();
    let area = crate::math::Aabb::new(p.x, p.y, p.z, p.x + 1.0, p.y + 1.0, p.z + 1.0).inflate(r, 10.0, r);
    // `getEntitiesOfClass(mob.getClass())`: zombies alert every kind of zombie; zombies and
    // drowned leave zombified piglins alone (`setAlertOthers(ZombifiedPiglin.class)`).
    let kind = m.kind;
    let same = |o: MobKind| {
        let class = o == kind || (kind == MobKind::Zombie && o.is_zombie());
        let ignored = o == MobKind::ZombifiedPiglin && kind != MobKind::ZombifiedPiglin;
        class && !ignored
    };
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        let alert = matches!(level.entity(id).map(|o| &o.kind), Some(crate::entity::EntityKind::Mob(om)) if same(om.kind) && om.target.is_none());
        if alert {
            super::set_target_of(level, id, Some(attacker));
        }
    }
}

fn is_turtle_egg_target(level: &dyn EntityLevel, p: BlockPos) -> bool {
    level.is_loaded(p)
        && crate::blocks::block_name(level.block(p)) == "minecraft:turtle_egg"
        && kiln_data::blocks_types::is_air(level.block(p.above()))
        && kiln_data::blocks_types::is_air(level.block(p.above().above()))
}

/// `MoveToBlockGoal.findNearestBlock` (range 24, vertical 3) for turtle eggs.
fn find_turtle_egg(e: &Entity, level: &dyn EntityLevel) -> Option<BlockPos> {
    let (range, vrange) = (24, 3);
    let base = e.block_position();
    let mut k = 0;
    while k <= vrange {
        for l in 0..range {
            let mut m = 0;
            while m <= l {
                let mut n = if m < l && m > -l { l } else { 0 };
                while n <= l {
                    let p = base.offset(m, k - 1, n);
                    if is_turtle_egg_target(level, p) {
                        return Some(p);
                    }
                    n = if n > 0 { -n } else { 1 - n };
                }
                m = if m > 0 { -m } else { 1 - m };
            }
        }
        k = if k > 0 { -k } else { 1 - k };
    }
    None
}

impl MobKind {
    /// The items a `TemptGoal` follows (the type's food tags).
    pub fn tempted_by(self, item: i32) -> bool {
        if item <= 0 {
            return false;
        }
        let tag = match self {
            MobKind::Pig => {
                if Some(item) == kiln_data::builtin_id("minecraft:item", "minecraft:carrot_on_a_stick") {
                    return true;
                }
                "minecraft:pig_food"
            }
            MobKind::Cow => "minecraft:cow_food",
            MobKind::Sheep => "minecraft:sheep_food",
            MobKind::Chicken => "minecraft:chicken_food",
            _ => return self.ext().is_some_and(|k| k.tempted_by(item)),
        };
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:item")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&item))
    }
}
