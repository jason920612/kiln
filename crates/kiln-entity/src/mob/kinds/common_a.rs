//! Goal building blocks shared by the slice 3 "common mobs A" types (rabbits, polar bears,
//! turtles, foxes, pandas, bees, ...): `AvoidEntityGoal` over players or mob types, the
//! `MoveToBlockGoal` search, and a wrapper that gives a shared goal a vanilla subclass name.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::ext::CustomGoal;
use crate::mob::goals::{self, Goal, Living, MOVE};
use crate::mob::{MobData, MobKind, path, random_pos};
use kiln_javamath::random::RandomSource;

/// Which entities an [`AvoidEntityGoal`] runs from (vanilla's `avoidClass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Avoid {
    /// `Player.class` (not creative or spectator: `NO_CREATIVE_OR_SPECTATOR`).
    Players,
    /// Mobs of these types.
    Types(&'static [&'static str]),
    /// `Monster.class`.
    Monsters,
}

/// `instanceof Monster` for a mob type (slimes, ghasts, phantoms and shulkers are `Mob`s or
/// golems; hoglins are animals).
pub fn is_monster_class(type_name: &str) -> bool {
    let Some(k) = MobKind::by_name(type_name) else { return false };
    k.category() == crate::mob::Category::Monster
        && !matches!(k, MobKind::Slime | MobKind::MagmaCube | MobKind::Ghast | MobKind::Phantom | MobKind::Shulker | MobKind::Hoglin)
}

/// `AvoidEntityGoal`: runs (16 blocks away, 7 up or down) from the nearest entity of the class
/// within `max_dist`, sprinting while it is within 7 blocks.
#[derive(Clone, Debug)]
pub struct AvoidEntityGoal {
    pub name: &'static str,
    pub avoid: Avoid,
    pub max_dist: f32,
    pub walk: f64,
    pub sprint: f64,
    /// Extra `canUse` condition of the mob (`None`: always).
    pub gate: Option<fn(&MobData, &dyn EntityLevel) -> bool>,
    /// `avoidPredicate` on the entity to avoid.
    pub filter: Option<fn(&MobData, &dyn EntityLevel, &Living) -> bool>,
    pub to_avoid: Option<i32>,
    path: Option<path::Path>,
}

impl AvoidEntityGoal {
    pub fn new(name: &'static str, avoid: Avoid, max_dist: f32, walk: f64, sprint: f64) -> AvoidEntityGoal {
        AvoidEntityGoal { name, avoid, max_dist, walk, sprint, gate: None, filter: None, to_avoid: None, path: None }
    }

    pub fn gate(mut self, f: fn(&MobData, &dyn EntityLevel) -> bool) -> Self {
        self.gate = Some(f);
        self
    }

    pub fn filter(mut self, f: fn(&MobData, &dyn EntityLevel, &Living) -> bool) -> Self {
        self.filter = Some(f);
        self
    }

    /// `getNearestEntity(getEntitiesOfClass(class, box inflated (d, 3, d)), forCombat().range(d), x, y, z)`.
    pub fn find(&self, e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> Option<Living> {
        let d = self.max_dist as f64;
        let area = e.bounding_box().inflate(d, 3.0, d);
        let mut best: Option<(f64, Living)> = None;
        let mut consider = |t: Living, m: &mut MobData| {
            if let Some(f) = self.filter
                && !f(m, level, &t)
            {
                return;
            }
            if !goals::targeting_ok(e, m, level, &t, true, d, true) {
                return;
            }
            let dist = e.position().distance_to_sqr(t.pos);
            // Ties go to the lower id: the simulation's player list has no fixed order.
            if best.as_ref().is_none_or(|(b, o)| dist < *b || (dist == *b && t.player && t.id < o.id)) {
                best = Some((dist, t));
            }
        };
        match self.avoid {
            Avoid::Players => {
                for p in level.players_in(&area).iter() {
                    let h = if p.sneaking { 1.5 } else { 1.8 };
                    let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
                    if !pb.intersects(&area) || p.creative || p.spectator {
                        continue;
                    }
                    consider(goals::living_player(p), m);
                }
            }
            Avoid::Types(types) => {
                for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
                    let Some(t) = goals::living(level, id) else { continue };
                    if !t.player && types.contains(&t.type_name) {
                        consider(t, m);
                    }
                }
            }
            Avoid::Monsters => {
                for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
                    let Some(t) = goals::living(level, id) else { continue };
                    if !t.player && is_monster_class(t.type_name) {
                        consider(t, m);
                    }
                }
            }
        }
        best.map(|(_, t)| t)
    }
}

impl CustomGoal for AvoidEntityGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Some(g) = self.gate
            && !g(m, level)
        {
            return false;
        }
        let Some(t) = self.find(e, m, level) else {
            self.to_avoid = None;
            return false;
        };
        self.to_avoid = Some(t.id);
        let Some(away) = random_pos::default_pos_away(e, m, level, 16, 7, t.pos) else { return false };
        if t.pos.distance_to_sqr(away) < t.pos.distance_to_sqr(e.position()) {
            return false;
        }
        // (`pathNav` is the mob's own navigation, taken when the goal was made: a rider plans
        // with its own, whose steps then move its mount.)
        self.path = path::own_nav(m, |m| path::create_path(e, m, level, BlockPos::containing(away.x, away.y, away.z), 0));
        self.path.is_some()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let p = self.path.take();
        path::own_nav(m, |m| path::move_to_path(e, m, level, p, self.walk));
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.to_avoid = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.to_avoid.and_then(|id| goals::living(level, id)) else { return };
        m.nav_mut().speed_modifier = if e.position().distance_to_sqr(t.pos) < 49.0 { self.sprint } else { self.walk };
    }
}

/// `MoveToBlockGoal.findNearestBlock`: the nearest valid block in rings around the mob.
pub fn find_nearest_block(e: &Entity, range: i32, vrange: i32, vstart: i32, mut valid: impl FnMut(BlockPos) -> bool) -> Option<BlockPos> {
    let o = e.block_position();
    let mut dy = vstart;
    while dy <= vrange {
        for r in 0..range {
            let mut dx = 0;
            while dx <= r {
                let mut dz = if dx < r && dx > -r { r } else { 0 };
                while dz <= r {
                    let p = o.offset(dx, dy - 1, dz);
                    if valid(p) {
                        return Some(p);
                    }
                    dz = if dz > 0 { -dz } else { 1 - dz };
                }
                dx = if dx > 0 { -dx } else { 1 - dx };
            }
        }
        dy = if dy > 0 { -dy } else { 1 - dy };
    }
    None
}

/// The shared state of a `MoveToBlockGoal` subclass.
#[derive(Clone, Debug, Default)]
pub struct MoveToBlock {
    pub speed: f64,
    pub range: i32,
    pub vrange: i32,
    pub vstart: i32,
    pub next_start: i32,
    pub try_ticks: i32,
    pub max_stay: i32,
    pub block: BlockPos,
    pub reached: bool,
}

impl MoveToBlock {
    pub fn new(speed: f64, range: i32, vrange: i32) -> MoveToBlock {
        MoveToBlock { speed, range, vrange, ..Default::default() }
    }

    /// `canUse` with the default `nextStartTick` (`reducedTickDelay(200 + nextInt(200))`).
    pub fn can_use(&mut self, e: &mut Entity, valid: impl FnMut(BlockPos) -> bool) -> bool {
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = crate::mob::mth::reduced_tick_delay(200 + e.random.next_int_bounded(200));
        self.find(e, valid)
    }

    pub fn find(&mut self, e: &Entity, valid: impl FnMut(BlockPos) -> bool) -> bool {
        match find_nearest_block(e, self.range, self.vrange, self.vstart, valid) {
            Some(p) => {
                self.block = p;
                true
            }
            None => false,
        }
    }

    /// `canContinueToUse` without the validity test.
    pub fn in_time(&self) -> bool {
        self.try_ticks >= -self.max_stay && self.try_ticks <= 1200
    }

    /// `start`: `moveMobToBlock` and the stay limit.
    pub fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
        let b = self.block;
        path::move_to(e, m, level, b.x as f64 + 0.5, (b.y + 1) as f64, b.z as f64 + 0.5, self.speed);
        self.try_ticks = 0;
        let inner = e.random.next_int_bounded(1200);
        self.max_stay = e.random.next_int_bounded(inner + 1200) + 1200;
    }

    /// `tick` with the move target `target` and `acceptedDistance` `accepted`.
    pub fn tick_to(&mut self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, target: BlockPos, accepted: f64) {
        let c = Vec3::new(target.x as f64 + 0.5, target.y as f64 + 0.5, target.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= accepted * accepted {
            self.reached = false;
            self.try_ticks += 1;
            if self.try_ticks % 40 == 0 {
                path::move_to(e, m, level, target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5, self.speed);
            }
        } else {
            self.reached = true;
            self.try_ticks -= 1;
        }
    }

    pub fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
        let t = self.block.above();
        self.tick_to(e, m, level, t, 1.0);
    }
}

/// A shared goal under a vanilla subclass name, with the subclass's extra condition and tick
/// (`RabbitPanicGoal`, `FoxPanicGoal`, `PolarBearPanicGoal`, ...).
#[derive(Clone, Debug)]
pub struct Named {
    pub name: &'static str,
    pub inner: Goal,
    /// Extra `canUse` condition, checked first.
    pub gate: Option<fn(&Entity, &MobData, &dyn EntityLevel) -> bool>,
    /// Extra `canContinueToUse` condition, checked first.
    pub keep: Option<fn(&Entity, &MobData, &dyn EntityLevel) -> bool>,
    /// Extra `canUse` condition checked after the inner goal's (`super.canUse() && ...`).
    pub post_gate: Option<fn(&Entity, &MobData, &dyn EntityLevel) -> bool>,
    /// Run after the inner goal's tick.
    pub after_tick: Option<fn(&Goal, &mut Entity, &mut MobData, &mut dyn EntityLevel)>,
    /// Run after the inner goal's start.
    pub after_start: Option<fn(&mut Goal, &mut Entity, &mut MobData, &mut dyn EntityLevel)>,
}

impl Named {
    pub fn new(name: &'static str, inner: Goal) -> Named {
        Named { name, inner, gate: None, keep: None, post_gate: None, after_tick: None, after_start: None }
    }
    pub fn gate(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel) -> bool) -> Self {
        self.gate = Some(f);
        self
    }
    pub fn post_gate(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel) -> bool) -> Self {
        self.post_gate = Some(f);
        self
    }
    pub fn keep(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel) -> bool) -> Self {
        self.keep = Some(f);
        self
    }
    pub fn after_tick(mut self, f: fn(&Goal, &mut Entity, &mut MobData, &mut dyn EntityLevel)) -> Self {
        self.after_tick = Some(f);
        self
    }
    pub fn after_start(mut self, f: fn(&mut Goal, &mut Entity, &mut MobData, &mut dyn EntityLevel)) -> Self {
        self.after_start = Some(f);
        self
    }
    pub fn boxed(self) -> Goal {
        Goal::Custom(Box::new(self))
    }
}

impl CustomGoal for Named {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn every_tick(&self) -> bool {
        self.inner.every_tick()
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Some(g) = self.gate
            && !g(e, m, level)
        {
            return false;
        }
        goals::can_use(&mut self.inner, e, m, level) && self.post_gate.is_none_or(|g| g(e, m, level))
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Some(g) = self.keep
            && !g(e, m, level)
        {
            return false;
        }
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
        if let Some(f) = self.after_start {
            f(&mut self.inner, e, m, level);
        }
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
        if let Some(f) = self.after_tick {
            f(&self.inner, e, m, level);
        }
    }
}

/// Shorthands for the shared goals with their usual arguments.
pub fn stroll(speed: f64) -> Goal {
    Goal::RandomStroll { speed, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false }
}
pub fn look(dist: f32) -> Goal {
    Goal::LookAtPlayer { dist, probability: 0.02, look_at: None, look_time: 0 }
}
pub fn look_around() -> Goal {
    Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 }
}
pub fn tempt(speed: f64) -> Goal {
    Goal::Tempt { speed, calm_down: 0, player: None }
}
pub fn panic(speed: f64) -> Goal {
    Goal::Panic { speed, pos: Vec3::ZERO }
}
pub fn breed(speed: f64) -> Goal {
    Goal::Breed { speed, partner: None, love_time: 0 }
}
pub fn follow_parent(speed: f64) -> Goal {
    Goal::FollowParent { speed, parent: None, recalc: 0 }
}
pub fn melee(speed: f64, follow_unseen: bool) -> Goal {
    Goal::Melee {
        kind: goals::MeleeKind::Plain,
        speed,
        follow_unseen,
        path: None,
        recalc: 0,
        next_attack: 0,
        last_can_use: 0,
        pathed: Vec3::ZERO,
        raise_arm: 0,
    }
}
pub fn hurt_by(alert: bool) -> Goal {
    Goal::HurtByTarget { timestamp: 0, alert_others: alert, target_mob: None, unseen: 0, unseen_memory: 60 }
}
pub fn nearest(wanted: goals::Wanted, interval: i32, must_see: bool) -> Goal {
    Goal::NearestAttackable { wanted, interval: crate::mob::mth::reduced_tick_delay(interval), must_see, target: None, unseen: 0, spider: false }
}

/// `Entity.playSound(sound, volume, pitch)` for a mob: skipped when silent (the pitch was drawn
/// by the caller either way).
pub fn play(e: &Entity, m: &MobData, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(crate::level::Event::Sound { pos: e.position(), sound, source: m.kind.sound_source(), volume, pitch });
    }
}

/// The usual random voice pitch `(nextFloat - nextFloat) * 0.2 + 1`.
pub fn voice(e: &mut Entity) -> f32 {
    (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0
}

/// `PanicGoal` with the panic-causing damage tag chosen per mob (polar bear cubs panic at any
/// `panic_causes`, adults only at `panic_environmental_causes`).
#[derive(Clone, Debug)]
pub struct PanicGoal {
    pub name: &'static str,
    pub speed: f64,
    pub tag: fn(&MobData) -> &'static str,
    pub pos: Vec3,
}

impl PanicGoal {
    pub fn new(name: &'static str, speed: f64, tag: fn(&MobData) -> &'static str) -> PanicGoal {
        PanicGoal { name, speed, tag, pos: Vec3::ZERO }
    }
}

impl CustomGoal for PanicGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let tag = (self.tag)(m);
        if !m.last_damage_source(level.game_time()).is_some_and(|s| s.kind.is_tag(tag)) {
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
        !m.nav_ref().is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.pos.x, self.pos.y, self.pos.z, self.speed);
    }
}

/// `NearestAttackableTargetGoal` with the knobs subclasses turn: the class (`Avoid` reused as
/// the target class), the random interval, line of sight, a follow distance scale
/// (`getFollowDistance` overrides), a condition checked before the random draw and a
/// `TargetingConditions` selector.
#[derive(Clone, Debug)]
pub struct NearestTargetGoal {
    pub name: &'static str,
    pub class: Avoid,
    /// `reducedTickDelay(randomInterval)`.
    pub interval: i32,
    pub must_see: bool,
    pub range_scale: f64,
    pub gate: Option<fn(&Entity, &MobData, &dyn EntityLevel) -> bool>,
    pub selector: Option<fn(&Entity, &MobData, &dyn EntityLevel, &Living) -> bool>,
    /// Runs after a target was found (`canUse` additions after `super.canUse()`).
    pub after: Option<fn(&Entity, &MobData, &dyn EntityLevel) -> bool>,
    pub target: Option<i32>,
    unseen: i32,
}

impl NearestTargetGoal {
    pub fn new(name: &'static str, class: Avoid, interval: i32, must_see: bool) -> NearestTargetGoal {
        NearestTargetGoal {
            name,
            class,
            interval: crate::mob::mth::reduced_tick_delay(interval),
            must_see,
            range_scale: 1.0,
            gate: None,
            selector: None,
            after: None,
            target: None,
            unseen: 0,
        }
    }
    pub fn gate(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel) -> bool) -> Self {
        self.gate = Some(f);
        self
    }
    pub fn selector(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel, &Living) -> bool) -> Self {
        self.selector = Some(f);
        self
    }
    pub fn after(mut self, f: fn(&Entity, &MobData, &dyn EntityLevel) -> bool) -> Self {
        self.after = Some(f);
        self
    }
    pub fn scale(mut self, s: f64) -> Self {
        self.range_scale = s;
        self
    }
    pub fn boxed(self) -> Goal {
        Goal::Custom(Box::new(self))
    }

    fn follow(&self, m: &MobData) -> f64 {
        m.attrs.value(crate::mob::attributes::Attr::FollowRange) * self.range_scale
    }

    /// `findTarget`.
    pub fn find(&self, e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> Option<i32> {
        let range = self.follow(m);
        let (x, y, z) = (e.x(), e.eye_y(), e.z());
        let mut best: Option<(f64, i32)> = None;
        let mut consider = |t: Living, m: &mut MobData| {
            if let Some(f) = self.selector
                && !f(e, m, level, &t)
            {
                return;
            }
            if !goals::targeting_ok(e, m, level, &t, true, range, self.must_see) {
                return;
            }
            let (dx, dy, dz) = (t.pos.x - x, t.pos.y - y, t.pos.z - z);
            let d = dx * dx + dy * dy + dz * dz;
            if best.is_none_or(|(b, o)| d < b || (d == b && t.player && t.id < o)) {
                best = Some((d, t.id));
            }
        };
        match self.class {
            Avoid::Players => {
                for p in goals::players_around(e, level, range).iter() {
                    consider(goals::living_player(p), m);
                }
            }
            Avoid::Types(types) => {
                let area = e.bounding_box().inflate(range, range, range);
                for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
                    let Some(t) = goals::living(level, id) else { continue };
                    if !t.player && types.contains(&t.type_name) {
                        consider(t, m);
                    }
                }
            }
            Avoid::Monsters => {
                let area = e.bounding_box().inflate(range, range, range);
                for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
                    let Some(t) = goals::living(level, id) else { continue };
                    if !t.player && is_monster_class(t.type_name) {
                        consider(t, m);
                    }
                }
            }
        }
        best.map(|(_, id)| id)
    }
}

/// `TargetGoal.canContinueToUse` with follow distance `follow`.
#[allow(clippy::too_many_arguments)]
pub fn continue_target(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, target_mob: Option<i32>, must_see: bool, unseen: &mut i32, memory: i32, follow: f64) -> bool {
    let id = m.target.or(target_mob);
    let Some(t) = id.and_then(|id| goals::living(level, id)) else { return false };
    if !goals::can_attack(m, level, &t) {
        return false;
    }
    if e.position().distance_to_sqr(t.pos) > follow * follow {
        return false;
    }
    if must_see {
        if crate::mob::has_line_of_sight_cached(e, m, level, &t) {
            *unseen = 0;
        } else {
            *unseen += 1;
            if *unseen > crate::mob::mth::reduced_tick_delay(memory) {
                return false;
            }
        }
    }
    crate::mob::set_target(e, m, Some(t.id));
    true
}

impl CustomGoal for NearestTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        goals::TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Some(g) = self.gate
            && !g(e, m, level)
        {
            return false;
        }
        if self.interval > 0 && e.random.next_int_bounded(self.interval) != 0 {
            return false;
        }
        self.target = self.find(e, m, level);
        if self.target.is_none() {
            return false;
        }
        self.after.is_none_or(|f| f(e, m, level))
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let follow = self.follow(m);
        continue_target(e, m, level, self.target, self.must_see, &mut self.unseen, 60, follow)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, self.target);
        self.unseen = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, None);
        self.target = None;
    }
}

/// `HurtByTargetGoal.alertOthers` for a type: mobs of the same type in the follow range box
/// (10 up and down) without a target take on the attacker, when `accept` lets them.
pub fn alert_others(e: &Entity, m: &MobData, level: &mut dyn EntityLevel, accept: fn(&MobData) -> bool) {
    let Some(attacker) = m.last_hurt_by_mob else { return };
    let r = m.attrs.value(crate::mob::attributes::Attr::FollowRange);
    let p = e.position();
    let area = Aabb::new(p.x, p.y, p.z, p.x + 1.0, p.y + 1.0, p.z + 1.0).inflate(r, 10.0, r);
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        let ok = matches!(level.entity(id).map(|o| &o.kind), Some(crate::entity::EntityKind::Mob(om)) if om.kind == m.kind && om.target.is_none() && accept(om));
        if ok {
            crate::mob::set_target_of(level, id, Some(attacker));
        }
    }
}

/// `MeleeAttackGoal` whose `checkAndPerformAttack` is the type's (`attack`: gets the goal's
/// attack cooldown, the adjusted 20-tick reset, and the target). The rest is the shared goal.
#[derive(Clone, Debug)]
pub struct MeleeGoal {
    pub name: &'static str,
    pub inner: Goal,
    pub attack: fn(&mut i32, i32, &mut Entity, &mut MobData, &mut dyn EntityLevel, &Living),
    pub on_stop: Option<fn(&mut Entity, &mut MobData)>,
    /// Extra `canUse` condition, checked first.
    pub gate: Option<fn(&MobData) -> bool>,
    pub on_start: Option<fn(&mut MobData)>,
}

impl MeleeGoal {
    pub fn new(name: &'static str, speed: f64, follow_unseen: bool, attack: fn(&mut i32, i32, &mut Entity, &mut MobData, &mut dyn EntityLevel, &Living)) -> MeleeGoal {
        MeleeGoal { name, inner: melee(speed, follow_unseen), attack, on_stop: None, gate: None, on_start: None }
    }
}

/// The shared `checkAndPerformAttack`: swing and hit once the cooldown ran out, in reach and in
/// sight.
pub fn plain_attack(next: &mut i32, reset: i32, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    if *next <= 0 && crate::mob::within_melee_range(e, m, t) && crate::mob::has_line_of_sight_cached(e, m, level, t) {
        *next = reset;
        m.swing = true;
        crate::mob::do_hurt_target(e, m, level, t);
    }
}

impl CustomGoal for MeleeGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.gate.is_some_and(|g| !g(m)) {
            return false;
        }
        goals::can_use(&mut self.inner, e, m, level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(f) = self.on_start {
            f(m);
        }
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(f) = self.on_stop {
            f(e, m);
        }
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Goal::Melee { speed, follow_unseen, recalc, next_attack, pathed, .. } = &mut self.inner else { return };
        let Some(t) = goals::target(m, level) else { return };
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        *recalc = (*recalc - 1).max(0);
        let sees = *follow_unseen || crate::mob::has_line_of_sight_cached(e, m, level, &t);
        if sees && *recalc <= 0 && ((pathed.x == 0.0 && pathed.y == 0.0 && pathed.z == 0.0) || t.pos.distance_to_sqr(*pathed) >= 1.0 || e.random.next_float() < 0.05) {
            *pathed = t.pos;
            *recalc = 4 + e.random.next_int_bounded(7);
            let d = e.position().distance_to_sqr(t.pos);
            if d > 1024.0 {
                *recalc += 10;
            } else if d > 256.0 {
                *recalc += 5;
            }
            let p = path::create_path_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 0);
            if !(p.is_some() && path::move_to_path(e, m, level, p, *speed)) {
                *recalc += 15;
            }
        }
        *next_attack = (*next_attack - 1).max(0);
        (self.attack)(next_attack, 20, e, m, level, &t);
    }
}
