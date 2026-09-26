//! Movement scripts. Bots walk on flat ground at the height the server teleported them to,
//! one step per 50 ms tick, and pick Move Player packets the way the vanilla client does.

use crate::proto::Move;
use serde::Serialize;
use std::f64::consts::{FRAC_PI_4, TAU};

/// Vanilla walking speed in blocks per second.
pub const WALK_SPEED: f64 = 4.317;
const TICKS_PER_SECOND: f64 = 20.0;
/// The vanilla client sends its position at least once per second even when standing still.
const POSITION_REMINDER_TICKS: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Behavior {
    /// Stand still.
    Idle,
    /// Random walk with a new heading every 2-6 s, staying within the radius of the origin.
    Walk,
    /// Walk around a circle of the given radius centred on the origin.
    Circle,
    /// Walk to the origin, then mill around within the radius.
    Crowd,
    /// Walk to a random point within the radius of the origin, then stand still.
    Spread,
}

impl Behavior {
    /// Radius used when none is configured.
    pub fn default_radius(self) -> f64 {
        match self {
            Behavior::Idle => 0.0,
            Behavior::Walk => 64.0,
            Behavior::Circle => 16.0,
            Behavior::Crowd => 6.0,
            Behavior::Spread => 256.0,
        }
    }
}

/// SplitMix64: small, fast and good enough for movement scripts.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn ticks(&mut self, lo_secs: f64, hi_secs: f64) -> u32 {
        (self.range(lo_secs, hi_secs) * TICKS_PER_SECOND) as u32
    }

    /// Uniform point in the disc of radius `r` around `c`.
    fn in_disc(&mut self, c: [f64; 2], r: f64) -> [f64; 2] {
        let d = r * self.unit().sqrt();
        let a = self.range(0.0, TAU);
        [c[0] + d * a.cos(), c[1] + d * a.sin()]
    }
}

#[derive(Debug, Clone)]
enum Plan {
    Idle,
    Walk { heading: f64, turn_in: u32 },
    Circle { dir: f64 },
    Crowd { target: Option<[f64; 2]>, pause: u32 },
    Spread { target: Option<[f64; 2]> },
}

/// A bot's position and movement script.
#[derive(Debug, Clone)]
pub struct Mover {
    rng: Rng,
    plan: Plan,
    /// Centre of the script, as (x, z).
    origin: [f64; 2],
    radius: f64,
    /// Blocks per tick.
    step: f64,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    sent_pos: [f64; 3],
    sent_rot: (f32, f32),
    reminder: u32,
}

impl Mover {
    /// `speed` is in blocks per second.
    pub fn new(behavior: Behavior, mut rng: Rng, origin: [f64; 2], radius: f64, speed: f64) -> Self {
        let plan = match behavior {
            Behavior::Idle => Plan::Idle,
            Behavior::Walk => Plan::Walk { heading: 0.0, turn_in: 0 },
            Behavior::Circle => Plan::Circle { dir: if rng.unit() < 0.5 { 1.0 } else { -1.0 } },
            Behavior::Crowd => Plan::Crowd { target: None, pause: 0 },
            Behavior::Spread => Plan::Spread { target: Some(rng.in_disc(origin, radius)) },
        };
        Self {
            rng,
            plan,
            origin,
            radius: radius.max(0.5),
            step: speed / TICKS_PER_SECOND,
            pos: [origin[0], 0.0, origin[1]],
            yaw: 0.0,
            pitch: 0.0,
            sent_pos: [origin[0], 0.0, origin[1]],
            sent_rot: (0.0, 0.0),
            reminder: 0,
        }
    }

    /// Adopts a position set by the server (a teleport that we then confirmed with it).
    pub fn teleported(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) {
        self.pos = pos;
        self.yaw = yaw;
        self.pitch = pitch;
        self.sent_pos = pos;
        self.sent_rot = (yaw, pitch);
        self.reminder = 0;
    }

    /// Advances the script by one tick and returns the packet the vanilla client would send.
    pub fn tick(&mut self) -> Option<Move> {
        self.advance();
        self.reminder += 1;
        let [dx, dy, dz] = [0, 1, 2].map(|i| self.pos[i] - self.sent_pos[i]);
        let moved = dx * dx + dy * dy + dz * dz > 4.0e-8 || self.reminder >= POSITION_REMINDER_TICKS;
        let rotated = (self.yaw, self.pitch) != self.sent_rot;
        let m = match (moved, rotated) {
            (true, true) => Move::PosRot(self.pos, self.yaw, self.pitch),
            (true, false) => Move::Pos(self.pos),
            (false, true) => Move::Rot(self.yaw, self.pitch),
            (false, false) => return None,
        };
        if moved {
            self.sent_pos = self.pos;
            self.reminder = 0;
        }
        if rotated {
            self.sent_rot = (self.yaw, self.pitch);
        }
        Some(m)
    }

    fn advance(&mut self) {
        let here = [self.pos[0], self.pos[2]];
        match &mut self.plan {
            Plan::Idle => {}
            Plan::Walk { heading, turn_in } => {
                if *turn_in == 0 {
                    *heading = if dist(here, self.origin) > self.radius {
                        angle_to(here, self.origin) + self.rng.range(-FRAC_PI_4, FRAC_PI_4)
                    } else {
                        self.rng.range(0.0, TAU)
                    };
                    *turn_in = self.rng.ticks(2.0, 6.0);
                }
                *turn_in -= 1;
                let (dx, dz) = (heading.cos(), heading.sin());
                self.pos[0] += dx * self.step;
                self.pos[2] += dz * self.step;
                self.yaw = yaw_of(dx, dz);
            }
            Plan::Circle { dir } => {
                // Chase a point slightly ahead on the circle: converges onto it from anywhere.
                let dir = *dir;
                let theta =
                    if dist(here, self.origin) < 1e-6 { self.rng.range(0.0, TAU) } else { angle_to(self.origin, here) };
                let lead = (10.0 * self.step / self.radius).min(FRAC_PI_4);
                let a = theta + dir * lead;
                let to = [self.origin[0] + self.radius * a.cos(), self.origin[1] + self.radius * a.sin()];
                self.walk_toward(to);
            }
            Plan::Crowd { target, pause } => {
                if *pause > 0 {
                    *pause -= 1;
                    return;
                }
                let to = *target.get_or_insert_with(|| self.rng.in_disc(self.origin, self.radius));
                if self.walk_toward(to) {
                    self.plan = Plan::Crowd { target: None, pause: self.rng.ticks(0.5, 3.0) };
                }
            }
            Plan::Spread { target } => {
                if let Some(to) = *target
                    && self.walk_toward(to)
                {
                    self.plan = Plan::Spread { target: None };
                }
            }
        }
    }

    /// Takes one step toward `to`; returns whether it was reached.
    fn walk_toward(&mut self, to: [f64; 2]) -> bool {
        let (dx, dz) = (to[0] - self.pos[0], to[1] - self.pos[2]);
        let d = (dx * dx + dz * dz).sqrt();
        if d > 1e-9 {
            self.yaw = yaw_of(dx, dz);
        }
        if d <= self.step {
            self.pos[0] = to[0];
            self.pos[2] = to[1];
            return true;
        }
        self.pos[0] += dx / d * self.step;
        self.pos[2] += dz / d * self.step;
        false
    }
}

/// Minecraft yaw in degrees for a direction in the x/z plane: 0 faces +z, 90 faces -x.
fn yaw_of(dx: f64, dz: f64) -> f32 {
    (-dx).atan2(dz).to_degrees() as f32
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Angle (radians, in the x/z plane) of the direction from `a` to `b`.
fn angle_to(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[1] - a[1]).atan2(b[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    const Y: f64 = -60.0;

    fn mover(b: Behavior, radius: f64) -> Mover {
        let mut m = Mover::new(b, Rng::new(7), [0.0, 0.0], radius, WALK_SPEED);
        m.teleported([0.0, Y, 0.0], 0.0, 0.0);
        m
    }

    fn run(m: &mut Mover, ticks: usize) -> Vec<Option<Move>> {
        (0..ticks).map(|_| m.tick()).collect()
    }

    fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
        (a[0] - b[0]).hypot(a[2] - b[2])
    }

    #[test]
    fn idle_sends_only_a_position_reminder_every_second() {
        let mut m = mover(Behavior::Idle, 0.0);
        let sent = run(&mut m, 60);
        let reminders: Vec<usize> = sent.iter().enumerate().filter(|(_, p)| p.is_some()).map(|(i, _)| i).collect();
        assert_eq!(reminders, [19, 39, 59]);
        assert!(sent.iter().flatten().all(|p| *p == Move::Pos([0.0, Y, 0.0])));
    }

    #[test]
    fn walkers_move_at_walking_speed_on_flat_ground_and_stay_near_the_origin() {
        let mut m = mover(Behavior::Walk, 20.0);
        let mut prev = m.pos;
        let mut turns = 0;
        for _ in 0..20 * 120 {
            let p = m.tick().expect("a walker moves every tick");
            let pos = match p {
                Move::Pos(pos) => pos,
                Move::PosRot(pos, ..) => {
                    turns += 1;
                    pos
                }
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(pos[1], Y);
            let d = horizontal(pos, prev);
            assert!((d - WALK_SPEED / 20.0).abs() < 1e-9, "step {d}");
            prev = pos;
            // Out of bounds, walkers turn back at the next heading change (at most 6 s later).
            assert!(horizontal(pos, [0.0, Y, 0.0]) < 20.0 + 6.0 * WALK_SPEED);
        }
        // Rotation is only sent when the heading changes, every 2-6 s.
        assert!((20..=61).contains(&turns), "{turns} turns");
    }

    #[test]
    fn yaw_faces_the_direction_of_travel() {
        let mut m = mover(Behavior::Spread, 0.0);
        m.plan = Plan::Spread { target: Some([0.0, 10.0]) }; // +z is yaw 0
        m.tick();
        assert_eq!(m.yaw, 0.0);
        m.plan = Plan::Spread { target: Some([-10.0, m.pos[2]]) }; // -x is yaw 90
        m.tick();
        assert_eq!(m.yaw, 90.0);
    }

    #[test]
    fn circlers_converge_onto_the_circle() {
        let mut m = mover(Behavior::Circle, 16.0);
        run(&mut m, 20 * 10);
        for _ in 0..20 * 30 {
            m.tick();
            let r = horizontal(m.pos, [0.0, Y, 0.0]);
            assert!((15.0..=16.0).contains(&r), "radius {r}");
        }
    }

    #[test]
    fn crowd_gathers_within_the_radius_of_a_distant_origin() {
        let mut m = Mover::new(Behavior::Crowd, Rng::new(3), [50.0, 0.0], 6.0, WALK_SPEED);
        m.teleported([0.0, Y, 0.0], 0.0, 0.0);
        run(&mut m, 20 * 15);
        for _ in 0..20 * 30 {
            m.tick();
            assert!(horizontal(m.pos, [50.0, Y, 0.0]) <= 6.0 + 1e-9);
        }
    }

    #[test]
    fn spreaders_stop_at_a_target_within_the_radius() {
        let mut m = mover(Behavior::Spread, 30.0);
        run(&mut m, 20 * 20);
        let rest = m.pos;
        assert!(horizontal(rest, [0.0, Y, 0.0]) <= 30.0);
        let later = run(&mut m, 40);
        assert_eq!(m.pos, rest);
        assert_eq!(later.iter().flatten().count(), 2, "only position reminders once arrived");
    }
}
