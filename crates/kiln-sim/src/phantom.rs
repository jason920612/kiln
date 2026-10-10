//! What the server does with a player between its move packets (vanilla `ServerPlayer.doTick`).
//!
//! The client is authoritative for where a player is, but the server still ticks the player's
//! body like any living entity: `LivingEntity.travel` runs with no input (gravity and drag, a
//! ladder's grip, fluids, bounces off slime and beds), the body moves, and only afterwards the
//! connection puts the position back where the client said it is. What stays is everything
//! except the position: the server's idea of whether the player stands on the ground, its
//! fall distance and its own velocity, and the blocks the extra move passed through, whose
//! effects apply that tick (so a player falling into lava burns a tick before the client
//! reaches it). `jumpFromGround` gives that velocity a jump when the client leaves the ground
//! going up.

use crate::Player;
use kiln_entity::entity::{Entity, EntityKind, Movement};
use kiln_entity::level::{EntityFilter, EntityLevel, Event};
use kiln_entity::math::{Aabb, BlockPos, Vec3};
use kiln_entity::player::{PlayerData, TravelInput};
use kiln_javamath::random::LegacyRandom;
use kiln_region::CellSet;
use kiln_world::{Blocks, Cell};

/// The blocks around a player, read only (what its move touches).
pub(crate) struct PhantomLevel<'a> {
    cells: &'a CellSet<Cell>,
    rng: LegacyRandom,
    game_time: i64,
    min_y: i32,
    fast_lava: bool,
}

impl<'a> PhantomLevel<'a> {
    pub(crate) fn new(cells: &'a CellSet<Cell>, game_time: i64, min_y: i32, fast_lava: bool) -> Self {
        PhantomLevel { cells, rng: LegacyRandom::new(0), game_time, min_y, fast_lava }
    }
}

impl EntityLevel for PhantomLevel<'_> {
    fn block(&self, pos: BlockPos) -> u16 {
        self.cells.get_block(pos.x, pos.y, pos.z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR)
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.cells.get_block(pos.x, pos.y, pos.z).is_some()
    }

    fn set_block(&mut self, _pos: BlockPos, _state: u16, _flags: u32) -> bool {
        false
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.rng
    }

    fn game_time(&self) -> i64 {
        self.game_time
    }

    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn fast_lava(&self) -> bool {
        self.fast_lava
    }

    fn entities_in(&self, _area: &Aabb, _filter: EntityFilter, _exclude: i32) -> Vec<i32> {
        Vec::new()
    }

    fn entity_mut(&mut self, _id: i32) -> Option<&mut Entity> {
        None
    }

    fn entity(&self, _id: i32) -> Option<&Entity> {
        None
    }

    fn add_entity(&mut self, _entity: Entity) {}

    fn next_entity_id(&mut self) -> i32 {
        0
    }

    fn fresh_seed(&mut self) -> i64 {
        0
    }

    fn emit(&mut self, _event: Event) {}
}

/// One movement of the tick, as `Entity.Movement`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mv {
    pub from: Vec3,
    pub to: Vec3,
    /// The requested movement when the path follows it axis by axis.
    pub original: Option<Vec3>,
}

impl From<Movement> for Mv {
    fn from(m: Movement) -> Mv {
        Mv { from: m.from, to: m.to, original: m.axis_dependent_original }
    }
}

fn vec(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

impl Player {
    /// The server's body of this player, set to what the player is now.
    fn phantom_in(&mut self, level: &dyn EntityLevel) -> Box<Entity> {
        let mut e = self.phantom.take().unwrap_or_else(|| {
            let mut e = kiln_entity::player::new(self.entity_id, self.uuid.as_u128(), vec(self.pos), 1.8, 0.6);
            e.first_tick = false;
            Box::new(e)
        });
        let (w, h, eye) = self.dimensions();
        e.width = w;
        e.height = h;
        e.eye_height = eye;
        e.set_pos(vec(self.pos));
        e.on_ground = self.on_ground;
        e.fall_distance = self.fall_distance;
        e.delta = vec(self.server_delta);
        e.shift_key_down = self.sneaking;
        e.main_supporting_block_pos = self.main_supporting_block;
        e.stuck_speed_multiplier = vec(self.stuck_speed);
        e.no_physics = false;
        e.was_touching_water = self.was_touching_water;
        e.y_rot = self.rot[0];
        let walks = self.walks_on_powder_snow();
        if let EntityKind::Player(d) = &mut e.kind {
            *d = PlayerData { flying: false, walks_on_powder_snow: walks, spectator: false };
        }
        let _ = level;
        e
    }

    /// Takes the body's state back (not its position unless `moved`).
    fn phantom_out(&mut self, mut e: Box<Entity>, moved: bool) {
        if moved {
            self.pos = [e.x(), e.y(), e.z()];
        }
        self.on_ground = e.on_ground;
        // The body's own ground state hid the client's report; the support goes with it.
        self.main_supporting_block = e.main_supporting_block_pos;
        if e.fall_distance == 0.0 && self.fall_distance != 0.0 {
            self.reset_fall_distance();
        } else {
            self.fall_distance = e.fall_distance;
        }
        self.server_delta = [e.delta.x, e.delta.y, e.delta.z];
        self.stuck_speed = [e.stuck_speed_multiplier.x, e.stuck_speed_multiplier.y, e.stuck_speed_multiplier.z];
        self.movements.extend(e.drain_movements().into_iter().map(Mv::from));
        self.phantom = Some(e);
    }

    /// `LivingEntity.travel` for the server's body of this player (see the module docs). The
    /// player's position is left where the body went; the caller puts it back after the
    /// tick's block effects.
    pub(crate) fn phantom_travel(&mut self, cells: &CellSet<Cell>, game_time: i64, min_y: i32, fast_lava: bool) {
        // Spectators, flying players, gliders, riders and the dead are not moved this way.
        if self.game_mode == 3 || self.flying || self.fall_flying || self.vehicle.is_some() || self.dead || self.sleep.pos.is_some() {
            self.server_delta = [0.0; 3];
            return;
        }
        let mut level = PhantomLevel::new(cells, game_time, min_y, fast_lava);
        let mut e = self.phantom_in(&level);
        // `Entity.baseTick`'s fluid update (currents push the body).
        e.update_fluid_interaction(&mut level);
        let t = TravelInput {
            gravity: self.attribute(crate::combat::GRAVITY),
            slow_falling: self.has_effect("minecraft:slow_falling"),
            levitation: self.effect_amplifier("minecraft:levitation"),
            dolphins_grace: self.has_effect("minecraft:dolphins_grace"),
            friction_modifier: 1.0,
            air_drag_modifier: 1.0,
            water_efficiency: self.attribute(crate::combat::WATER_MOVEMENT_EFFICIENCY) as f32,
            sprinting: self.sprinting,
        };
        kiln_entity::player::travel(&mut level, &mut e, &t);
        self.phantom_out(e, true);
    }

    /// `Player.updatePlayerPose` where the server's body stands.
    pub(crate) fn update_pose(&mut self, cells: &CellSet<Cell>, game_time: i64, min_y: i32) {
        let level = PhantomLevel::new(cells, game_time, min_y, false);
        let (pos, id, ctx) = (self.pos, self.entity_id, self.collision_context());
        let fits = |pose: i32| {
            let (w, h, _) = crate::pose::dimensions_of(pose);
            let half = (w / 2.0) as f64;
            let bb = Aabb::new(pos[0] - half, pos[1], pos[2] - half, pos[0] + half, pos[1] + h as f64, pos[2] + half).deflate_all(1.0e-7);
            kiln_entity::collision::no_collision(&level, &ctx, id, &bb)
        };
        self.update_player_pose(&fits);
    }

    /// `ServerPlayer.jumpFromGround`: the server's velocity gets the jump (and a sprinting
    /// player's push) when the client leaves the ground going up.
    pub(crate) fn server_jump(&mut self, from: [f64; 3], cells: &CellSet<Cell>, game_time: i64, min_y: i32) {
        let level = PhantomLevel::new(cells, game_time, min_y, false);
        // The jump happens where the player was before the move.
        let moved_to = std::mem::replace(&mut self.pos, from);
        let e = self.phantom_in(&level);
        self.pos = moved_to;
        // `getJumpPower`: the attribute times the block's jump factor, plus jump boost.
        let at = |p: BlockPos| level.block(p);
        let factor = {
            let f = kiln_entity::physics::block_factors(at(e.block_position())).jump;
            let below = kiln_entity::physics::block_factors(at(e.block_pos_below_that_affects_movement(&level))).jump;
            if f as f64 == 1.0 { below } else { f }
        };
        let boost = self.effect_amplifier("minecraft:jump_boost").map_or(0.0f32, |a| 0.1f32 * (a + 1) as f32);
        let power = self.attribute(crate::combat::JUMP_STRENGTH) as f32 * factor + boost;
        if power > 1.0e-5 {
            self.server_delta[1] = (power as f64).max(self.server_delta[1]);
            if self.sprinting {
                let f = self.rot[0] * 0.017453292;
                self.server_delta[0] += (-kiln_entity::mob::mth::sin(f as f64) * 0.2) as f64;
                self.server_delta[2] += (kiln_entity::mob::mth::cos(f as f64) * 0.2) as f64;
            }
        }
        self.phantom = Some(e);
    }

    /// The server's `player.move(PLAYER, delta)` for a move packet (`player::server_move` on the
    /// server's body, from where the body stood before the packet): collisions clamp the
    /// request, a body held by cobwebs, berry bushes or powder snow moves a fraction of it and
    /// its velocity is gone. The path it took is the movement of the tick the blocks are judged
    /// by. Returns where the body ended (the connection compares it with the client's claim: "moved wrongly"); the
    /// player's ground state is the body's now, the client's report (`setOnGroundWithMovement`) is the caller's to
    /// put back once it takes the move.
    pub(crate) fn server_packet_move(&mut self, from: [f64; 3], d: [f64; 3], was_on_ground: bool, cells: &CellSet<Cell>, game_time: i64, min_y: i32, fast_lava: bool) -> [f64; 3] {
        let mut level = PhantomLevel::new(cells, game_time, min_y, fast_lava);
        let (to, on_ground) = (self.pos, self.on_ground);
        self.pos = from;
        self.on_ground = was_on_ground;
        let mut e = self.phantom_in(&level);
        self.pos = to;
        self.on_ground = on_ground;
        if self.game_mode == 3 {
            if let EntityKind::Player(data) = &mut e.kind {
                data.spectator = true;
            }
        }
        kiln_entity::player::server_move(&mut level, &mut e, vec(d));
        let end = [e.x(), e.y(), e.z()];
        // Everything but the position stays with the body.
        self.phantom_out(e, false);
        end
    }
}
