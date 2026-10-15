//! Elytra flights: a bot that flies like a player gliding on elytra with firework rockets, in a
//! straight line at a constant speed (a rocket-boosted glide holds about 33 blocks per second),
//! high enough to clear the terrain ahead, and measures how far ahead of it the server has the
//! terrain ready.
//!
//! The bot sets itself up with commands (it needs operator permission): it moves to its start
//! in the flight's dimension, puts on an elytra, takes rockets, steps off into the air and starts
//! gliding with the Start Fall Flying player command, as the client does when jump is pressed in
//! the air. From then on it sends one position per client tick and uses a rocket every two
//! seconds, which the server spawns as a firework attached to the player.
//!
//! Measured while flying (after `warmup`): for every tick, the distance along the flight line to
//! the first chunk that has not arrived yet (capped at the view), and for every chunk that came
//! into view, how long it took to arrive.

use crate::metrics::{Shared, Traffic};
use crate::proto::{self, Move};
use crate::Out;
use kiln_proto::{DecodeError, Reader};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Blocks per second of a rocket-boosted glide (measured with the 26.3 client in wp65).
pub const FLY_SPEED: f64 = 32.8;
/// Ticks between rockets (wp65's real flights used one every 2 s).
const ROCKET_TICKS: u64 = 40;
/// Ticks between topping up the rockets and the elytra (a flight outlasts 64 rockets and an
/// elytra's 432 s of durability).
const RESUPPLY_TICKS: u64 = 20 * 60;
/// Clearance kept above the highest surface ahead.
const CLEARANCE: f64 = 12.0;
/// How far ahead (blocks) the terrain is looked at for the altitude.
const LOOK_AHEAD: f64 = 64.0;
/// Fastest climb and descent, blocks per tick.
const CLIMB: f64 = 1.0;
const SINK: f64 = 0.4;
/// Overworld bottom (heightmaps count from there).
const OVERWORLD_MIN_Y: i32 = -64;

/// Where and how a flight goes.
#[derive(Debug, Clone)]
pub struct FlightPlan {
    /// `minecraft:overworld`, `minecraft:the_nether`, ...
    pub dim: String,
    /// Start (x, z).
    pub start: [f64; 2],
    /// Direction of flight in the x/z plane (unit).
    pub heading: [f64; 2],
    /// Blocks per second.
    pub speed: f64,
    /// Lowest altitude; the bot climbs over terrain that reaches higher.
    pub altitude: f64,
    /// Seconds of flight before the measurements count.
    pub warmup: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Commands sent; waiting to stand at the start with terrain around.
    Setup,
    /// In the air at the start: one falling move, then Start Fall Flying.
    Launch(u8),
    Flying,
}

/// A flying bot's state.
pub(crate) struct Flight {
    plan: FlightPlan,
    stage: Stage,
    entity_id: i32,
    pub pos: [f64; 3],
    yaw: f32,
    /// View radius in chunks (the smaller of ours and the server's).
    radius: i32,
    /// Chunks that arrived and were not forgotten, with the highest surface (overworld).
    chunks: HashMap<(i32, i32), i32>,
    /// Chunks in view that have not arrived, with when they came into view.
    want: HashMap<(i32, i32), Instant>,
    center: Option<(i32, i32)>,
    setup_at: Option<Instant>,
    flying_since: Option<Instant>,
    ticks: u64,
    sequence: i32,
    /// After a dimension change the client reports its level loaded again.
    pub needs_loaded: bool,
    terrain_follow: bool,
}

impl Flight {
    pub(crate) fn new(plan: FlightPlan, entity_id: i32, radius: i32) -> Self {
        let yaw = (-plan.heading[0]).atan2(plan.heading[1]).to_degrees() as f32;
        let terrain_follow = plan.dim == "minecraft:overworld";
        Flight {
            plan,
            stage: Stage::Setup,
            entity_id,
            pos: [0.0; 3],
            yaw,
            radius: radius.max(2),
            chunks: HashMap::new(),
            want: HashMap::new(),
            center: None,
            setup_at: None,
            flying_since: None,
            ticks: 0,
            sequence: 0,
            needs_loaded: false,
            terrain_follow,
        }
    }

    fn measuring(&self) -> bool {
        self.flying_since.is_some_and(|t| t.elapsed() >= self.plan.warmup)
    }

    /// The first position the server gave: the bot asks to be taken to its start.
    pub(crate) fn spawned(&mut self, out: &mut Out, counts: &mut Traffic, pos: [f64; 3]) {
        self.pos = pos;
        let [x, z] = self.plan.start;
        let y = self.plan.altitude;
        for c in [
            format!("execute in {} run tp @s {x:.1} {y:.1} {z:.1} {:.1} 0", self.plan.dim, self.yaw),
            "item replace entity @s armor.chest with minecraft:elytra".to_string(),
            "give @s minecraft:firework_rocket 64".to_string(),
        ] {
            out.send(|b| proto::chat_command(b, &c));
            counts.commands += 1;
        }
        self.setup_at = Some(Instant::now());
    }

    /// Respawn (a dimension change): the old level's chunks are gone.
    pub(crate) fn respawned(&mut self) {
        self.needs_loaded = true;
        self.chunks.clear();
        self.want.clear();
        self.center = None;
    }

    /// A position from the server: the start, or a correction.
    pub(crate) fn teleported(&mut self, pos: [f64; 3], counts: &mut Traffic) {
        if self.stage == Stage::Flying {
            counts.fly_corrections += 1;
        }
        self.pos = pos;
        self.view_moved();
    }

    /// A Level Chunk With Light packet body.
    pub(crate) fn on_chunk(&mut self, body: &[u8], shared: &Shared) -> Result<(), DecodeError> {
        let mut r = Reader::new(body);
        let (x, z) = (r.i32()?, r.i32()?);
        let mut top = i32::MIN;
        if self.terrain_follow {
            for _ in 0..r.varint()? {
                let kind = r.varint()?;
                let n = r.len()?;
                // MOTION_BLOCKING, packed 9 bits per column in the overworld (7 to a long).
                for _ in 0..n {
                    let l = r.i64()? as u64;
                    if kind == 4 {
                        for i in 0..7 {
                            top = top.max(((l >> (i * 9)) & 0x1ff) as i32);
                        }
                    }
                }
            }
        }
        self.chunks.insert((x, z), if top == i32::MIN { i32::MIN } else { top + OVERWORLD_MIN_Y });
        if let Some(t) = self.want.remove(&(x, z))
            && self.measuring()
        {
            shared.fly_chunk_latency.add(t.elapsed());
        }
        Ok(())
    }

    pub(crate) fn on_forget_chunk(&mut self, x: i32, z: i32) {
        self.chunks.remove(&(x, z));
    }

    /// The bot's chunk changed: the chunks that came into view start their clocks.
    fn view_moved(&mut self) {
        let c = chunk_of(self.pos);
        if self.center == Some(c) {
            return;
        }
        self.center = Some(c);
        let r = self.radius;
        let now = Instant::now();
        self.want.retain(|&(x, z), _| (x - c.0).abs() <= r && (z - c.1).abs() <= r);
        for x in c.0 - r..=c.0 + r {
            for z in c.1 - r..=c.1 + r {
                if !self.chunks.contains_key(&(x, z)) {
                    self.want.entry((x, z)).or_insert(now);
                }
            }
        }
    }

    /// One client tick.
    pub(crate) fn tick(&mut self, out: &mut Out, counts: &mut Traffic, shared: &Shared) {
        self.ticks += 1;
        match self.stage {
            Stage::Setup => {
                let [x, z] = self.plan.start;
                let there = (self.pos[0] - x).abs() < 2.0 && (self.pos[2] - z).abs() < 2.0;
                let waited = self.setup_at.is_some_and(|t| t.elapsed() > Duration::from_secs(3));
                if there && waited && !self.needs_loaded && self.chunks.contains_key(&chunk_of(self.pos)) {
                    self.stage = Stage::Launch(0);
                }
            }
            Stage::Launch(n) => {
                // Off the ground: a small drop, then the wings open (the server checks that the
                // player is in the air).
                self.pos[1] -= 0.08;
                let m = Move::PosRot(self.pos, self.yaw, 0.0);
                out.send(|b| proto::move_player(b, m, false));
                if n >= 2 {
                    out.send(|b| proto::player_command(b, self.entity_id, proto::START_FALL_FLYING));
                    self.stage = Stage::Flying;
                    self.flying_since = Some(Instant::now());
                    shared.arrived.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    self.stage = Stage::Launch(n + 1);
                }
            }
            Stage::Flying => self.fly(out, counts, shared),
        }
    }

    fn fly(&mut self, out: &mut Out, counts: &mut Traffic, shared: &Shared) {
        let step = self.plan.speed / 20.0;
        let [hx, hz] = self.plan.heading;
        self.pos[0] += hx * step;
        self.pos[2] += hz * step;
        // Altitude: over the highest surface known ahead, climbing and sinking at glide rates.
        let mut target = self.plan.altitude;
        if self.terrain_follow {
            let mut d = 0.0;
            while d <= LOOK_AHEAD {
                let p = [self.pos[0] + hx * d, 0.0, self.pos[2] + hz * d];
                for dx in [-8.0, 0.0, 8.0] {
                    if let Some(&top) = self.chunks.get(&chunk_of([p[0] + dx * hz, 0.0, p[2] - dx * hx])) {
                        target = target.max(top as f64 + CLEARANCE);
                    }
                }
                d += 8.0;
            }
        }
        let dy = (target - self.pos[1]).clamp(-SINK, CLIMB);
        self.pos[1] += dy;
        let pitch = if dy > 0.1 { -30.0 } else { 5.0 };
        let m = Move::PosRot(self.pos, self.yaw, pitch);
        out.send(|b| proto::move_player(b, m, false));
        if self.ticks % ROCKET_TICKS == 0 {
            self.sequence += 1;
            let (seq, yaw) = (self.sequence, self.yaw);
            out.send(|b| proto::use_item(b, seq, yaw, pitch));
            counts.rockets += 1;
        }
        if self.ticks % RESUPPLY_TICKS == 0 {
            for c in ["give @s minecraft:firework_rocket 32", "item replace entity @s armor.chest with minecraft:elytra"] {
                out.send(|b| proto::chat_command(b, c));
                counts.commands += 1;
            }
        }
        self.view_moved();
        if self.measuring() {
            let ahead = self.ahead();
            counts.fly_ticks += 1;
            counts.fly_dm += (step * 10.0) as u64;
            if ahead < 32 {
                counts.fly_short_ticks += 1;
            }
            if !self.chunks.contains_key(&chunk_of(self.pos)) {
                counts.fly_missing_ticks += 1;
            }
            shared.fly_ahead.add(Duration::from_millis(ahead as u64));
        }
    }

    /// Blocks from the bot along its heading to the first chunk that has not arrived, up to the
    /// edge of the view.
    fn ahead(&self) -> u32 {
        let [hx, hz] = self.plan.heading;
        let reach = (self.radius as f64 + 0.5) * 16.0;
        let mut d = 0.0;
        while d < reach {
            let p = [self.pos[0] + hx * d, 0.0, self.pos[2] + hz * d];
            if !self.chunks.contains_key(&chunk_of(p)) {
                return d as u32;
            }
            d += 2.0;
        }
        reach as u32
    }
}

fn chunk_of(p: [f64; 3]) -> (i32, i32) {
    ((p[0].floor() as i32) >> 4, (p[2].floor() as i32) >> 4)
}
