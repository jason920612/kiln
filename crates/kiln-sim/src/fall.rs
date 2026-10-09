//! Landing for players (vanilla `ServerPlayer.doCheckFallDamage`, `Entity.checkFallDamage`,
//! `Block.fallOn` and its overrides, `Player.causeFallDamage` and `LivingEntity.calculateFallDamage`).
//!
//! The client says where it is and whether it stands on the ground; the server adds the move's
//! downward part to the fall distance (not while the player is in water, where it is reset), and
//! when the move ended on the ground, the block under the feet (`getOnPosLegacy`: 0.2 below,
//! found through the supporting block) decides what the landing does:
//!
//! - most blocks take the distance times `1 - fallDistanceReduction` (beds 0.5);
//! - hay bales and honey blocks pass it on with a multiplier of 0.2, slime blocks with 0 (and a
//!   sneaking player takes nothing at all), a pointed dripstone tip pointing up with the distance
//!   plus 2.5 and a multiplier of 2 as `stalagmite` damage, powder snow with none;
//! - the damage is `floor((distance + 1e-6 - safe_fall_distance) * multiplier *
//!   fall_damage_multiplier)`, dealt as `fall` damage when above 0 (creative and spectator
//!   players, who may fly, take none).

use crate::Player;
use crate::health::{Cause, DamageCtx};
use kiln_entity::blocks::{Kind, Tag, block_name, has_tag, kind};
use kiln_entity::collision::{CollisionContext, find_supporting_block_in};
use kiln_entity::math::{Aabb, BlockPos, Vec3};
use kiln_javamath::random::RandomSource;

/// A block reader (`None`: the chunk is not loaded).
pub(crate) type Blocks<'a> = &'a dyn Fn(BlockPos) -> Option<u16>;

/// `EntityEvent.HONEY_BLOCK_SLIDE`-style landing event sent with a honey landing.
const HONEY_LANDING_EVENT: u8 = 54;

/// `level.clip(ClipContext(from, to, FALLDAMAGE_RESETTING, WATER, entity))` hits something.
fn clip_fall_damage_resetting(blocks: Blocks, from: Vec3, to: Vec3) -> bool {
    kiln_entity::clip::traverse_blocks(from, to, |pos| {
        let state = blocks(pos).unwrap_or(0);
        let block_hit = has_tag(state, Tag::FallDamageResetting)
            && kiln_entity::clip::shape_clips(kiln_entity::physics::block_shape(), from, to, pos);
        let f = kiln_entity::physics::fluid_state(state);
        let fluid_hit = f.kind.is_water() && {
            let at = |p: BlockPos| blocks(p).unwrap_or(0);
            let h = crate::hazards::fluid_height(&at, pos, &f);
            let shape = kiln_entity::shape::Shape::from_box(&Aabb::new(0.0, 0.0, 0.0, 1.0, h as f64, 1.0));
            shape.is_some_and(|s| kiln_entity::clip::shape_clips(&s, from, to, pos))
        };
        (block_hit || fluid_hit).then_some(())
    })
    .is_some()
}

/// A change a player's own tick asks its region for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BlockEdit {
    /// `destroyBlock(pos, false)` (burning in powder snow).
    Destroy(BlockPos),
    /// `FarmBlock.turnToDirt` (a landing on farmland).
    Dirt(BlockPos),
    /// Frost walker boots: `ReplaceDisk` of frosted ice around `pos`, centred on the block under the feet
    /// (`origin`), `radius` blocks out.
    FrostWalker { origin: BlockPos, radius: i32, pos: [f64; 3] },
}

impl Player {
    /// The tick's resets of the fall distance (`Player.aiStep`: flying; `LivingEntity.aiStep`:
    /// slow falling and levitation; climbing a ladder is the server body's, see [`crate::phantom`]).
    pub(crate) fn tick_fall_resets(&mut self, _block: crate::hazards::BlockAt) {
        if self.flying {
            self.reset_fall_distance();
        }
        if self.has_effect("minecraft:slow_falling") || self.has_effect("minecraft:levitation") {
            self.reset_fall_distance();
        }
    }

    /// `EntityCollisionContext.of(player)`.
    pub(crate) fn collision_context(&self) -> CollisionContext {
        CollisionContext {
            descending: self.sneaking,
            entity_bottom: self.pos[1],
            placement: false,
            always_collide_with_fluid: false,
            has_entity: true,
            fall_distance: self.fall_distance,
            falling_block: false,
            walks_on_powder_snow: self.walks_on_powder_snow(),
            stands_on_lava: false,
        }
    }

    /// `PowderSnowBlock.canEntityWalkOnPowderSnow` for a player: leather boots.
    pub(crate) fn walks_on_powder_snow(&self) -> bool {
        kiln_item::registry::ITEM
            .id("minecraft:leather_boots")
            .is_some_and(|id| self.worn(kiln_item::component::EquipmentSlot::Feet).item() == id)
    }

    /// `Entity.checkSupportingBlock`.
    pub(crate) fn check_supporting_block(&mut self, blocks: Blocks, on_ground: bool, movement: Option<[f64; 3]>) {
        if !on_ground {
            self.on_ground_no_blocks = false;
            self.main_supporting_block = None;
            return;
        }
        let bb = self.bounding_box();
        let below = Aabb::new(bb.min_x, bb.min_y - 1.0e-6, bb.min_z, bb.max_x, bb.min_y, bb.max_z);
        let ctx = self.collision_context();
        let position = Vec3::new(self.pos[0], self.pos[1], self.pos[2]);
        let mut found = find_supporting_block_in(blocks, &ctx, position, &below);
        if found.is_some() || self.on_ground_no_blocks {
            self.main_supporting_block = found;
        } else if let Some(m) = movement {
            let back = below.offset(-m[0], 0.0, -m[2]);
            found = find_supporting_block_in(blocks, &ctx, position, &back);
            self.main_supporting_block = found;
        }
        self.on_ground_no_blocks = found.is_none();
    }

    /// `getOnPos(offset)`.
    pub(crate) fn on_pos(&self, blocks: Blocks, offset: f32) -> BlockPos {
        if let Some(pos) = self.main_supporting_block {
            if offset > 1.0e-5 {
                let state = blocks(pos).unwrap_or(0);
                if (offset as f64 <= 0.5 && has_tag(state, Tag::Fences))
                    || has_tag(state, Tag::Walls)
                    || kind(state) == Kind::FenceGate
                {
                    return pos;
                }
                return BlockPos::new(pos.x, (self.pos[1] - offset as f64).floor() as i32, pos.z);
            }
            return pos;
        }
        BlockPos::new(self.pos[0].floor() as i32, (self.pos[1] - offset as f64).floor() as i32, self.pos[2].floor() as i32)
    }

    /// `Entity.touchingUnloadedChunk`: a chunk within a block of the box is not loaded.
    fn touching_unloaded_chunk(&self, blocks: Blocks) -> bool {
        let bb = self.bounding_box().inflate_all(1.0);
        let y = (self.pos[1].floor() as i32).clamp(-64, 319);
        let (x0, x1) = ((bb.min_x.floor() as i32) >> 4, (bb.max_x.floor() as i32) >> 4);
        let (z0, z1) = ((bb.min_z.floor() as i32) >> 4, (bb.max_z.floor() as i32) >> 4);
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if blocks(BlockPos::new(cx * 16, y, cz * 16)).is_none() {
                    return true;
                }
            }
        }
        false
    }

    /// The fall part of `ServerGamePacketListenerImpl.handlePlayerPositionChange` after the move
    /// `d` that ended `on_ground`: `ServerPlayer.doCheckFallDamage`, then the reset of a client
    /// that moved up (`client_up`: it went up since the last good position).
    pub(crate) fn after_move_fall(&mut self, d: [f64; 3], on_ground: bool, client_up: bool, blocks: Blocks, ctx: &mut DamageCtx) {
        // `Entity.move`: a move of a block or more through a fall-damage-resetting block or
        // water ends the fall (before the move's own distance is added).
        if self.fall_distance != 0.0 && d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= 1.0 {
            let from = Vec3::new(self.pos[0] - d[0], self.pos[1] - d[1], self.pos[2] - d[2]);
            let moved = Vec3::new(d[0], d[1], d[2]);
            let end = from + moved.normalize().scale(moved.length().min(8.0));
            if clip_fall_damage_resetting(blocks, from, end) {
                self.reset_fall_distance();
            }
        }
        self.check_supporting_block(blocks, on_ground, Some(d));
        self.do_check_fall_damage(d, on_ground, blocks, ctx);
        if client_up {
            self.reset_fall_distance();
        }
    }

    /// `Entity.doCheckFallDamage(dx, dy, dz, onGround)`.
    pub(crate) fn do_check_fall_damage(&mut self, d: [f64; 3], on_ground: bool, blocks: Blocks, ctx: &mut DamageCtx) {
        if self.touching_unloaded_chunk(blocks) {
            return;
        }
        self.check_supporting_block(blocks, on_ground, Some(d));
        let pos = self.on_pos(blocks, 0.2);
        let state = blocks(pos).unwrap_or(0);
        self.check_fall_damage(d[1], on_ground, state, pos, blocks, ctx);
    }

    /// `LivingEntity.checkFallDamage` and `Entity.checkFallDamage`.
    fn check_fall_damage(&mut self, dy: f64, on_ground: bool, state: u16, pos: BlockPos, blocks: Blocks, ctx: &mut DamageCtx) {
        if !self.was_touching_water {
            // `updateFluidInteraction`: water under the new position resets the fall.
            let at = |p: BlockPos| blocks(p).unwrap_or(0);
            let fluids = self.fluids(&at);
            if fluids.in_water {
                self.reset_fall_distance();
            }
            self.was_touching_water = fluids.in_water;
            self.was_eye_in_water = fluids.eye_in_water;
        }
        if !self.was_touching_water && dy < 0.0 {
            self.fall_distance -= dy as f32 as f64;
        }
        // `ServerPlayer.trackStartFallingPosition`.
        if self.fall_distance > 0.0 && self.starting_to_fall.is_none() {
            self.starting_to_fall = Some(self.pos);
        }
        if on_ground {
            if self.fall_distance > 0.0 {
                // `LivingEntity.checkFallDamage`: landing is a change of block for the enchantments.
                self.loc_landed = true;
                self.fall_on(state, pos, ctx);
            }
            self.reset_fall_distance();
        }
    }

    /// `Block.fallOn` for the block landed on.
    fn fall_on(&mut self, state: u16, pos: BlockPos, ctx: &mut DamageCtx) {
        let distance = self.fall_distance;
        match kind(state) {
            Kind::HoneyBlock => {
                self.queue_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
                self.entity_events.push(HONEY_LANDING_EVENT);
                self.send(kiln_proto::packets::entity::entity_event(self.entity_id, HONEY_LANDING_EVENT));
                self.cause_fall_damage(distance, 0.2, Cause::Fall(distance), ctx);
            }
            Kind::Slime => {
                // A sneaking player lands on slime without a thing happening.
                if !self.sneaking {
                    self.cause_fall_damage(distance, 0.0, Cause::Fall(distance), ctx);
                }
            }
            Kind::Farmland => {
                // Every landing draws from the level's random; a heavy enough body tramples.
                let trample = (self.level_rng.next_float() as f64) < distance - 0.5;
                let (w, h, _) = self.dimensions();
                if trample && w * w * h > 0.512 {
                    self.block_edits.push(BlockEdit::Dirt(pos));
                }
                self.default_fall_on(state, distance, ctx);
            }
            Kind::PowderSnow => {}
            _ => match block_name(state) {
                "minecraft:turtle_egg" => {
                    // `destroyEgg(.., 3)`: a player that may break there breaks it one time in three.
                    let _ = self.level_rng.next_int_bounded(3);
                    self.default_fall_on(state, distance, ctx);
                }
                "minecraft:hay_block" => {
                    self.cause_fall_damage(distance, 0.2, Cause::Fall(distance), ctx);
                }
                "minecraft:pointed_dripstone" => {
                    let info = kiln_data::blocks_types::block_of(state);
                    if info.property(state, "vertical_direction") == Some("up") && info.property(state, "thickness") == Some("tip") {
                        self.cause_fall_damage(distance + 2.5, 2.0, Cause::Other("minecraft:stalagmite"), ctx);
                    } else {
                        self.default_fall_on(state, distance, ctx);
                    }
                }
                _ => self.default_fall_on(state, distance, ctx),
            },
        }
    }

    /// `Block.fallOn`: the distance less the block's reduction.
    fn default_fall_on(&mut self, state: u16, distance: f64, ctx: &mut DamageCtx) {
        let reduction = kiln_entity::physics::block_factors(state).fall_reduction;
        self.cause_fall_damage(distance * (1.0 - reduction) as f64, 1.0, Cause::Fall(self.fall_distance), ctx);
    }

    /// `Player.causeFallDamage`, `LivingEntity.causeFallDamage`: true when it hurt.
    pub(crate) fn cause_fall_damage(&mut self, distance: f64, multiplier: f32, cause: Cause, ctx: &mut DamageCtx) -> bool {
        // `abilities.mayfly`.
        if matches!(self.game_mode, 1 | 3) {
            return false;
        }
        if distance >= 2.0 {
            self.award_stat(*crate::player_stats::stat::FALL_ONE_CM, (distance * 100.0).round() as i32);
        }
        let damage = self.calculate_fall_damage(distance, multiplier);
        if damage > 0 {
            let sound = if damage > 4 { "minecraft:entity.player.big_fall" } else { "minecraft:entity.player.small_fall" };
            self.queue_sound(sound, 1.0, 1.0);
            self.hurt(damage as f32, &cause.into(), ctx);
            return true;
        }
        false
    }

    /// `LivingEntity.calculateFallDamage`.
    fn calculate_fall_damage(&self, distance: f64, multiplier: f32) -> i32 {
        let power = distance + 1.0e-6 - self.attribute(crate::combat::SAFE_FALL_DISTANCE);
        (power * multiplier as f64 * self.attribute(crate::combat::FALL_DAMAGE_MULTIPLIER)).floor() as i32
    }
}
