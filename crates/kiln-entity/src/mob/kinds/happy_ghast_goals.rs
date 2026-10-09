//! The goals of a grown happy ghast that are its own.

use super::happy_ghast::on_still_timeout;
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::Vec3;
use crate::mob::attributes::Attr;
use crate::mob::control::Operation;
use crate::mob::ext::CustomGoal;
use crate::mob::goals::{self, LOOK, MOVE};
use crate::mob::{self, MobData};
use kiln_javamath::random::RandomSource;

/// `TemptGoal.ForNonPathfinders(this, 1.0, HappyGhast::isFood, false, 7.0)`: toward a player holding food, by the move
/// control, stopping 7 blocks short.
#[derive(Clone, Debug)]
pub struct TemptForNonPathfinders {
    pub speed: f64,
    pub stop_distance: f64,
    pub player: Option<i32>,
    pub calm_down: i32,
}

impl TemptForNonPathfinders {
    pub fn new(speed: f64, stop_distance: f64) -> TemptForNonPathfinders {
        TemptForNonPathfinders { speed, stop_distance, player: None, calm_down: 0 }
    }

    /// `shouldFollow`: food in either hand.
    fn follows(m: &MobData, p: &crate::level::PlayerView) -> bool {
        let food = |item: i32| item != 0 && m.kind.ext().is_some_and(|k| k.is_food(item));
        food(p.main_hand) || food(p.off_hand)
    }
}

impl CustomGoal for TemptForNonPathfinders {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ForNonPathfinders"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        // `forNonCombat().ignoreLineOfSight().range(TEMPT_RANGE)` and the food selector.
        let range = m.attrs.value(Attr::TemptRange);
        let kind = m.kind;
        let found = goals::nearest_player(e, m, &*level, false, range, false, |p| {
            let food = |item: i32| item != 0 && kind.ext().is_some_and(|k| k.is_food(item));
            food(p.main_hand) || food(p.off_hand)
        });
        self.player = found.map(|t| t.id);
        self.player.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level)
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.player = None;
        // `stopNavigation`: the move control waits.
        m.mov.operation = Operation::Wait;
        self.calm_down = mob::mth::reduced_tick_delay(100);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return };
        m.look.set_look_at(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z, (m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
        // (Distance to the player's feet.)
        if e.position().distance_to_sqr(p.pos) < self.stop_distance * self.stop_distance {
            m.mov.operation = Operation::Wait;
        } else {
            // `navigateTowards`: a random point on the line to the player's eyes.
            let eye = Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z);
            let r = e.random.next_double();
            let to_eye = Vec3::new(eye.x - e.x(), eye.y - e.y(), eye.z - e.z()).scale(r);
            m.mov.set_wanted_position(to_eye.x + e.x(), to_eye.y + e.y(), to_eye.z + e.z(), self.speed);
        }
    }
}

/// Whether the happy ghast stays still (for the float goal's gate).
pub fn float_gate(_e: &Entity, m: &MobData, _level: &dyn EntityLevel) -> bool {
    !on_still_timeout(m)
}

#[allow(dead_code)]
fn unused(m: &MobData, p: &crate::level::PlayerView) -> bool {
    TemptForNonPathfinders::follows(m, p)
}
