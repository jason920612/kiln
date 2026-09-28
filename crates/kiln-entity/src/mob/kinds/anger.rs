//! `NeutralMob` (wolves, iron golems): a persistent anger target with an end time, refreshed
//! while the mob has a target, and the target goal that goes after the player it is angry at.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::mob::attributes::Attr;
use crate::mob::ext::{self, CustomGoal};
use crate::mob::goals::{self, Living, TARGET};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{MobData, MobKind};
use kiln_javamath::random::RandomSource;

#[derive(Clone, Debug)]
pub struct Anger {
    /// `getPersistentAngerEndTime` (-1: none).
    pub end: i64,
    /// `getPersistentAngerTarget` (by network id; not saved).
    pub target: Option<i32>,
}

impl Default for Anger {
    fn default() -> Anger {
        Anger { end: -1, target: None }
    }
}

pub fn get(m: &MobData) -> Option<&Anger> {
    match m.kind {
        MobKind::Wolf => ext::state::<super::wolf::State>(m).map(|s| &s.anger),
        MobKind::IronGolem => ext::state::<super::iron_golem::State>(m).map(|s| &s.anger),
        _ => None,
    }
}

pub fn get_mut(m: &mut MobData) -> Option<&mut Anger> {
    match m.kind {
        MobKind::Wolf => ext::state_mut::<super::wolf::State>(m).map(|s| &mut s.anger),
        MobKind::IronGolem => ext::state_mut::<super::iron_golem::State>(m).map(|s| &mut s.anger),
        _ => None,
    }
}

/// `isAngry`.
pub fn is_angry(m: &MobData, level: &dyn EntityLevel) -> bool {
    let end = get(m).map_or(-1, |a| a.end);
    end > 0 && end - level.game_time() > 0
}

/// `stopBeingAngry`.
pub fn stop_being_angry(m: &mut MobData) {
    m.last_hurt_by_mob = None;
    if let Some(a) = get_mut(m) {
        a.target = None;
        a.end = -1;
    }
    m.target = None;
}

/// `startPersistentAngerTimer` (20 to 39 seconds for both types).
fn start_timer(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let t = 400 + e.random.next_int_bounded(381);
    if let Some(a) = get_mut(m) {
        a.end = level.game_time() + t as i64;
    }
}

fn valid_player_target(level: &dyn EntityLevel, t: &Living) -> bool {
    t.player && !t.creative && !t.spectator && level.difficulty() != 0
}

/// `updatePersistentAnger(level, true)`.
pub fn update_persistent_anger(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let anger_ref = get(m).and_then(|a| a.target);
    if let Some(u) = m.target.and_then(|id| goals::living(level, id))
        && !u.alive
        && anger_ref == Some(u.id)
        && !u.player
    {
        stop_being_angry(m);
        return;
    }
    let target = goals::target(m, level);
    if let Some(t) = target {
        if anger_ref != Some(t.id)
            && let Some(a) = get_mut(m)
        {
            a.target = Some(t.id);
        }
        start_timer(e, m, level);
    }
    if anger_ref.is_some() && !is_angry(m, level) && target.is_none_or(|t| !valid_player_target(level, &t)) {
        stop_being_angry(m);
    }
    if let Some(p) = anger_ref.and_then(|id| level.player(id))
        && (p.creative || p.spectator || level.difficulty() == 0)
    {
        stop_being_angry(m);
    }
}

/// `NearestAttackableTargetGoal<Player>(10, mustSee, isAngryAt)`: the player the mob is angry at.
#[derive(Clone, Debug, Default)]
pub struct AngryAtPlayerGoal {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for AngryAtPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NearestAttackableTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_int_bounded(reduced_tick_delay(10)) != 0 {
            return false;
        }
        let range = m.attrs.value(Attr::FollowRange);
        let angry_at = get(m).and_then(|a| a.target);
        self.target = goals::nearest_player(e, m, level, true, range, true, |p| angry_at == Some(p.id)).map(|p| p.id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, self.target, true, &mut self.unseen, 60)
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
