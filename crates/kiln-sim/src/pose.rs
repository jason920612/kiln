//! The player's pose (vanilla `Player.updatePlayerPose`, `Avatar.updateSwimming`).
//!
//! The pose decides the hit box and the eye height (1.8 standing, 1.5 crouching, 0.6 swimming,
//! crawling, gliding and spinning, 0.2 asleep) and what the other players see. The server works
//! it out itself at the end of every player tick, whatever the client does: a player sprinting
//! under water swims; one whose head would be inside a ceiling crouches, and when even that does
//! not fit crawls. The shift key is separate (`sneaking`), and the pose follows it a tick later.

use crate::Player;
use kiln_data::entities::pose;

/// `EntityDimensions` of a pose: width, height and eye height.
pub(crate) fn dimensions_of(p: i32) -> (f32, f32, f32) {
    match p {
        pose::CROUCHING => (0.6, 1.5, 1.27),
        pose::SWIMMING | pose::FALL_FLYING | pose::SPIN_ATTACK => (0.6, 0.6, 0.4),
        pose::SLEEPING => (0.2, 0.2, 0.2),
        _ => (0.6, 1.8, 1.62),
    }
}

impl Player {
    /// `isCrouching`: the pose, not the key.
    pub(crate) fn is_crouching(&self) -> bool {
        self.pose == pose::CROUCHING
    }

    /// The shift key goes down or up together with the pose (what the vectors' harness does
    /// with `setShiftKeyDown` and `setPose`; the server's tick would settle it a tick later).
    pub(crate) fn set_shift_key(&mut self, on: bool) {
        self.sneaking = on;
        self.pose = if on { pose::CROUCHING } else { pose::STANDING };
        self.crouch_attr = on;
    }

    /// `Avatar.updateSwimming` (and `Player.updateSwimming`: a flying player never swims):
    /// sprinting in water; to start, with the eyes under it and water at the feet.
    pub(crate) fn update_swimming(&mut self, in_water: bool, under_water: bool, water_at_feet: bool) {
        let passenger = self.vehicle.is_some();
        let swimming = if self.flying {
            false
        } else if self.swimming {
            self.sprinting && in_water && !passenger
        } else {
            self.sprinting && under_water && !passenger && water_at_feet
        };
        if swimming != self.swimming {
            self.swimming = swimming;
            self.meta_dirty = true;
            self.self_meta_dirty = true;
        }
    }

    /// `getDesiredPose`.
    fn desired_pose(&self) -> i32 {
        if self.sleep.pos.is_some() {
            pose::SLEEPING
        } else if self.swimming {
            pose::SWIMMING
        } else if self.fall_flying {
            pose::FALL_FLYING
        } else if self.spin_pose {
            pose::SPIN_ATTACK
        } else if self.sneaking && !self.flying {
            pose::CROUCHING
        } else {
            pose::STANDING
        }
    }

    /// `Player.updatePlayerPose`; `fits` tells whether the player's box in a pose touches no
    /// collision (`canPlayerFitWithinBlocksAndEntitiesWhen`).
    pub(crate) fn update_player_pose(&mut self, fits: &dyn Fn(i32) -> bool) {
        let desired = self.desired_pose();
        // (Every pose but sleeping has a box that holds the swimming one: where the desired pose fits, so does that,
        // and one look at the blocks answers both.)
        let direct = desired != pose::SLEEPING && self.game_mode != 3 && self.vehicle.is_none() && fits(desired);
        if !direct && !fits(pose::SWIMMING) {
            return;
        }
        let new = if direct || self.game_mode == 3 || self.vehicle.is_some() || fits(desired) {
            desired
        } else if fits(pose::CROUCHING) {
            pose::CROUCHING
        } else {
            pose::SWIMMING
        };
        if new != self.pose {
            self.pose = new;
            self.meta_dirty = true;
            self.self_meta_dirty = true;
        }
    }
}
