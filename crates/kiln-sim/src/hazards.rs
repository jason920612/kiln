//! Fire, lava and air for players (vanilla `Entity.baseTick`'s burning, `LivingEntity.baseTick`'s
//! air supply, `Entity.applyEffectsFromBlocks` and the `entityInside` / `stepOn` of fire, soul
//! fire, campfires, lava, water and magma blocks).
//!
//! `remainingFireTicks` counts down while positive, burning for 1 every 20 ticks (not while in
//! lava; fire resistance and fire protection work through the damage pipeline). Standing in
//! fire first counts a negative value up (a player's is -20 when not burning), then sets 8
//! seconds of fire and adds one or two ticks every tick; lava sets 15 seconds and hurts 4.
//! Water and powder snow put the fire out. The shared flag 0 tells viewers (and the player's
//! own client) that it burns.
//!
//! The air supply (300) goes down by one a tick while the eyes are in water, unless water
//! breathing, conduit power or the breath of the nautilus lets the player breathe, or it is
//! invulnerable; respiration's oxygen bonus skips a tick with probability `bonus / (bonus + 1)`.
//! At -20 it resets to 0 with 2 drowning damage. Elsewhere it refills by 4 a tick.
//!
//! Randomness: the fire ignition bump draws from the player's stand-in for the level's random
//! (see [`crate::enchant`]); respiration and the sound pitches from the player's own random.

use crate::Player;
use crate::health::{Cause, DamageCtx};
use kiln_entity::blocks::{Kind, kind};
use kiln_entity::math::{Aabb, BlockPos, Vec3};
use kiln_entity::physics::{self, FluidKind, FluidState};
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::world_fx;

/// `Entity.getMaxAirSupply`.
pub(crate) const MAX_AIR: i32 = 300;
/// `Player.getFireImmuneTicks`: the fire counter rests at minus this.
pub(crate) const FIRE_IMMUNE_TICKS: i32 = 20;
/// `EntityEvent.DROWN_PARTICLES`.
const DROWN_PARTICLES: u8 = 67;

/// A block reader for the player's surroundings (unloaded blocks read as air).
pub(crate) type BlockAt<'a> = &'a dyn Fn(BlockPos) -> u16;

/// Where the player is in fluids (`EntityFluidInteraction.update`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Fluids {
    pub in_water: bool,
    pub in_lava: bool,
    pub eye_in_water: bool,
}

fn fluid_at(block: BlockAt, pos: BlockPos) -> FluidState {
    physics::fluid_state(block(pos))
}

/// `FluidState.getHeight`: 1 under the same fluid, else `amount / 9`.
pub(crate) fn fluid_height(block: BlockAt, pos: BlockPos, f: &FluidState) -> f32 {
    if f.kind.is_same(fluid_at(block, pos.above()).kind) { 1.0 } else { f.own_height() }
}

/// `FluidState.getHeightForCamera`: a source under a sturdy ceiling fills the block.
fn camera_height(block: BlockAt, pos: BlockPos, f: &FluidState) -> f32 {
    if f.source && physics::is_face_sturdy(block(pos.above()), kiln_entity::math::Direction::Down) {
        return 1.0;
    }
    fluid_height(block, pos, f)
}

impl Player {
    /// `EntityDimensions` of the standing or crouching player: (width, height, eye height).
    pub(crate) fn dimensions(&self) -> (f32, f32, f32) {
        crate::pose::dimensions_of(self.pose)
    }

    /// `getEyeY`.
    pub(crate) fn eye_y(&self) -> f64 {
        self.pos[1] + self.dimensions().2 as f64
    }

    /// `makeBoundingBox(pos)`.
    pub(crate) fn bounding_box_at(&self, pos: [f64; 3]) -> Aabb {
        let (w, h, _) = self.dimensions();
        let half = (w / 2.0) as f64;
        Aabb::new(pos[0] - half, pos[1], pos[2] - half, pos[0] + half, pos[1] + h as f64, pos[2] + half)
    }

    /// `EntityFluidInteraction.update`: the fluids the (slightly shrunk) box touches, and
    /// whether the eyes are under the surface in the player's own column.
    pub(crate) fn fluids(&self, block: BlockAt) -> Fluids {
        let full = self.bounding_box_at(self.pos);
        let bb = full.deflate_all(0.001);
        let floor = |v: f64| v.floor() as i32;
        let ceil = |v: f64| v.ceil() as i32;
        let column = [floor(self.pos[0]), floor(self.pos[2])];
        let eye = self.eye_y();
        let mut out = Fluids::default();
        for x in floor(bb.min_x)..ceil(bb.max_x) {
            for y in floor(bb.min_y)..ceil(bb.max_y) {
                for z in floor(bb.min_z)..ceil(bb.max_z) {
                    let pos = BlockPos::new(x, y, z);
                    let f = fluid_at(block, pos);
                    if f.is_empty() {
                        continue;
                    }
                    let top = y as f64 + fluid_height(block, pos, &f) as f64;
                    if top < bb.min_y {
                        continue;
                    }
                    let height = top - full.min_y;
                    if f.kind.is_water() {
                        out.in_water |= height > 0.0;
                    } else if f.kind.is_lava() {
                        out.in_lava |= height > 0.0;
                    }
                    if f.kind.is_water()
                        && [x, z] == column
                        && eye >= y as f64
                        && eye <= y as f64 + camera_height(block, pos, &f) as f64
                    {
                        out.eye_in_water = true;
                    }
                }
            }
        }
        out
    }

    /// `Player.setRemainingFireTicks`: invulnerable players burn for at most a tick.
    pub(crate) fn set_fire_ticks(&mut self, ticks: i32) {
        self.fire_ticks = if self.invulnerable() { ticks.min(1) } else { ticks };
    }

    /// `Entity.clearFire`.
    pub(crate) fn clear_fire(&mut self) {
        self.set_fire_ticks(self.fire_ticks.min(0));
    }

    fn alive(&self) -> bool {
        !self.dead && self.health > 0.0
    }

    /// `Entity.baseTick` and `LivingEntity.baseTick` for the parts Kiln models: burning, the
    /// void, the on-fire flag, the air supply and the effects. `commonTick` ages the player.
    pub(crate) fn base_tick(&mut self, block: BlockAt, min_y: i32, border: &crate::world_state::BorderBox, ctx: &mut DamageCtx) {
        self.tick_count += 1;
        // `LivingEntity.aiStep`: the grace time after an impulse runs down.
        if self.impulse_grace > 0 {
            self.impulse_grace -= 1;
        }
        self.crouch_attr = self.is_crouching();
        // `Entity.baseTick`: powder snow sets the flag again while the player is in it.
        self.is_in_powder_snow = false;
        let fluids = self.fluids(block);
        // `Entity.baseTick`: `updateFluidInteraction` (water ends a fall).
        if fluids.in_water {
            self.reset_fall_distance();
        }
        self.was_touching_water = fluids.in_water;
        self.was_eye_in_water = fluids.eye_in_water;
        {
            // `Entity.baseTick`'s `updateSwimming`: water at the feet is a water block or flowing water.
            let feet = BlockPos::containing(self.pos[0], self.pos[1], self.pos[2]);
            let water_at_feet = fluid_at(block, feet).kind.is_water();
            self.update_swimming(fluids.in_water, fluids.eye_in_water && fluids.in_water, water_at_feet);
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0 && !fluids.in_lava {
                self.hurt(1.0, &Cause::Other("minecraft:on_fire").into(), ctx);
            }
            self.set_fire_ticks(self.fire_ticks - 1);
        }
        if fluids.in_lava {
            self.fall_distance *= 0.5;
        }
        self.check_void(min_y, ctx);
        self.sync_on_fire_flag();
        // `LivingEntity.baseTick`: suffocation in a wall, else a player outside the world
        // border past its buffer.
        if self.alive() {
            if self.is_in_wall(block) {
                self.hurt(1.0, &Cause::Other("minecraft:in_wall").into(), ctx);
            } else {
                let (w, _, _) = self.dimensions();
                if let Some(amount) = border.damage(self.pos, w as f64 / 2.0) {
                    self.hurt(amount, &Cause::Other("minecraft:outside_border").into(), ctx);
                }
            }
        }
        if self.alive() {
            self.tick_air(&fluids, block, ctx);
        }
        self.tick_effects(ctx);
    }

    /// `setSharedFlagOnFire(remainingFireTicks > 0)`.
    pub(crate) fn sync_on_fire_flag(&mut self) {
        let on_fire = self.fire_ticks > 0;
        if self.on_fire_flag != on_fire {
            self.on_fire_flag = on_fire;
            self.meta_dirty = true;
            self.self_meta_dirty = true;
        }
    }

    /// The air supply part of `LivingEntity.baseTick`.
    fn tick_air(&mut self, fluids: &Fluids, block: BlockAt, ctx: &mut DamageCtx) {
        let eye = BlockPos::containing(self.pos[0], self.eye_y(), self.pos[2]);
        if fluids.eye_in_water && kind(block(eye)) != Kind::BubbleColumn {
            // `MobEffectUtil.hasWaterBreathing` and `shouldEffectsRefillAirsupply`.
            let (wb, conduit, nautilus) = (
                self.has_effect("minecraft:water_breathing"),
                self.has_effect("minecraft:conduit_power"),
                self.has_effect("minecraft:breath_of_the_nautilus"),
            );
            if !(wb || conduit || nautilus) && !self.invulnerable() {
                self.air = self.decrease_air(self.air);
                if self.air <= -20 {
                    self.air = 0;
                    self.entity_events.push(DROWN_PARTICLES);
                    self.send(kiln_proto::packets::entity::entity_event(self.entity_id, DROWN_PARTICLES));
                    self.hurt(2.0, &Cause::Other("minecraft:drown").into(), ctx);
                }
            } else if self.air < MAX_AIR && (!nautilus || wb || conduit) {
                self.air = (self.air + 4).min(MAX_AIR);
            }
        } else if self.air < MAX_AIR {
            self.air = (self.air + 4).min(MAX_AIR);
        }
    }

    /// `LivingEntity.decreaseAirSupply`: the oxygen bonus (respiration) may spare the tick.
    fn decrease_air(&mut self, air: i32) -> i32 {
        let bonus = self.attribute(crate::combat::OXYGEN_BONUS);
        if bonus > 0.0 && self.entity_rng.next_double() >= 1.0 / (bonus + 1.0) { air } else { air - 1 }
    }

    /// `BaseFireBlock.fireIgnite` for a player: a negative counter goes up by one, a burning one
    /// by one or two (the level's random), then at least 8 seconds of fire.
    fn fire_ignite(&mut self) {
        if self.fire_ticks < 0 {
            self.set_fire_ticks(self.fire_ticks + 1);
        } else {
            let bump = 1 + self.level_rng.next_int_bounded(2);
            self.set_fire_ticks(self.fire_ticks + bump);
        }
        if self.fire_ticks >= 0 {
            self.ignite_for_seconds(8.0);
        }
    }

    /// `Entity.lavaHurt`: 4 lava damage; the burn sound's pitch draws from the player's random.
    fn lava_hurt(&mut self, ctx: &mut DamageCtx) {
        if self.hurt(4.0, &Cause::Other("minecraft:lava").into(), ctx) {
            let pitch = 2.0 + self.entity_rng.next_float() * 0.4;
            self.queue_sound("minecraft:entity.generic.burn", 0.4, pitch);
        }
    }

    /// `Player.playSound`: heard by the player's viewers (the player's own client plays it).
    pub(crate) fn queue_sound(&mut self, sound: &str, volume: f32, pitch: f32) {
        self.queue_sound_at(self.pos, sound, volume, pitch);
    }

    /// [`Player::queue_sound`] where the player was when it made the sound.
    pub(crate) fn queue_sound_at(&mut self, at: [f64; 3], sound: &str, volume: f32, pitch: f32) {
        let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
        let seed = self.sound_seed.next_long();
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Players, at, volume, pitch, seed);
        self.pending_sounds.push(pkt);
    }

    /// `Entity.applyEffectsFromBlocks` for the move since the last player tick: magma under the
    /// feet, then the fire, lava, water and campfire blocks the box passed through, in vanilla's
    /// step order, then rain (`isInRain`) puts the fire out; a player that is not burning
    /// afterwards rests at -20 fire ticks.
    pub(crate) fn block_effects(&mut self, block: BlockAt, dim: crate::DimId, in_rain: bool, ctx: &mut DamageCtx) {
        // The movements of the tick (`applyEffectsFromBlocks`): the packet's and the server
        // body's; none recorded: the box where it stands.
        let mut movements = std::mem::take(&mut self.movements);
        let here = Vec3::new(self.pos[0], self.pos[1], self.pos[2]);
        match movements.last().copied() {
            None => movements.push(crate::phantom::Mv { from: here, to: here, original: None }),
            Some(last) if last.to.distance_to_sqr(here) > 9.999999439624929e-11 => {
                movements.push(crate::phantom::Mv { from: last.to, to: here, original: None });
            }
            _ => {}
        }
        if self.game_mode == 3 {
            return;
        }
        if self.on_ground {
            // `getOnPosLegacy`: 0.2 below the feet, through the supporting block.
            let at = |p: BlockPos| Some(block(p));
            let below = self.on_pos(&at, 0.2);
            if kind(block(below)) == Kind::MagmaBlock && !self.sneaking {
                self.hurt(1.0, &Cause::Other("minecraft:hot_floor").into(), ctx);
            }
        }
        let was_on_fire = self.fire_ticks > 0;
        let was_freezing = self.ticks_frozen > 0;
        let fire_before = self.fire_ticks;
        let effects = self.inside_blocks(block, dim, &movements, ctx);
        for e in effects {
            if !self.alive() {
                break;
            }
            match e {
                Inside::FireIgnite => self.fire_ignite(),
                Inside::LavaIgnite => self.ignite_for_seconds(15.0),
                Inside::Extinguish => self.clear_fire(),
                Inside::FireHurt(damage) => {
                    self.hurt(damage, &Cause::Other("minecraft:in_fire").into(), ctx);
                }
                Inside::LavaHurt => self.lava_hurt(ctx),
                Inside::Freeze => self.freeze_effect(),
                Inside::ClearFreeze => self.ticks_frozen = 0,
                Inside::MeltPowderSnow(pos) => {
                    // A burning player melts the powder snow it stands in.
                    if self.fire_ticks > 0 {
                        self.block_edits.push(crate::fall::BlockEdit::Destroy(pos));
                    }
                }
            }
        }
        if in_rain {
            self.clear_fire();
        }
        if (was_on_fire && self.fire_ticks <= 0) || (was_freezing && self.ticks_frozen <= 0) {
            // `playEntityOnFireExtinguishedSound`.
            let pitch = 1.6 + (self.entity_rng.next_float() - self.entity_rng.next_float()) * 0.4;
            self.queue_sound("minecraft:entity.generic.extinguish_fire", 0.7, pitch);
        }
        if self.fire_ticks <= 0 && self.fire_ticks <= fire_before {
            self.set_fire_ticks(-FIRE_IMMUNE_TICKS);
        }
    }

    /// `checkInsideBlocks` for one movement: the collector's effects in apply order. Campfires
    /// hurt at once, as their `entityInside` does.
    fn inside_blocks(&mut self, block: BlockAt, dim: crate::DimId, movements: &[crate::phantom::Mv], ctx: &mut DamageCtx) -> Vec<Inside> {
        use kiln_entity::math::Axis;
        let mut collector = Collector::default();
        let mut visited = Vec::new();
        for m in movements {
            let mut from = m.from;
            let d = m.to - m.from;
            let mut max_steps = 16;
            match m.original {
                // A move that follows its request axis by axis (`Entity.move`'s).
                Some(original) if d.length_sqr() > 0.0 => {
                    for axis in Axis::step_order(original) {
                        let v = d.get(axis);
                        if v != 0.0 {
                            let to = from.relative(axis.positive(), v);
                            max_steps -= self.inside_segment(block, dim, from, to, &mut visited, max_steps, &mut collector, ctx);
                            from = to;
                        }
                    }
                }
                _ => max_steps -= self.inside_segment(block, dim, m.from, m.to, &mut visited, 16, &mut collector, ctx),
            }
            if max_steps <= 0 {
                self.inside_segment(block, dim, m.to, m.to, &mut visited, 1, &mut collector, ctx);
            }
        }
        collector.finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn inside_segment(
        &mut self,
        block: BlockAt,
        dim: crate::DimId,
        from: Vec3,
        to: Vec3,
        visited: &mut Vec<i64>,
        max_steps: i32,
        c: &mut Collector,
        ctx: &mut DamageCtx,
    ) -> i32 {
        let bb = self.bounding_box_at([to.x, to.y, to.z]).deflate_all(9.999999747378752e-6);
        let from_box = self.bounding_box_at([from.x, from.y, from.z]);
        let travel = to - from;
        let too_far = from.distance_to_sqr(to) > 0.9999900000002526 * 0.9999900000002526;
        let mut blocks = Vec::new();
        kiln_entity::inside::for_each_block_intersected_between(from, to, &bb, |pos, step| {
            if step >= max_steps {
                return false;
            }
            blocks.push((pos, step));
            true
        });
        let mut counter = 0;
        for (pos, step) in blocks {
            if !self.alive() {
                break;
            }
            counter = step;
            let state = block(pos);
            if physics::is_air(state) {
                continue;
            }
            let collided = match self.inside_shape(state) {
                None => true,
                Some(shape) => {
                    let boxes: Vec<Aabb> =
                        shape.boxes().iter().map(|b| b.offset(pos.x as f64, pos.y as f64, pos.z as f64)).collect();
                    from_box.collided_along_vector(travel, &boxes)
                }
            };
            let f = physics::fluid_state(state);
            let fluid_collided = !f.is_empty() && {
                let h = fluid_height(block, pos, &f);
                let fluid_box = Aabb::new(
                    pos.x as f64,
                    pos.y as f64,
                    pos.z as f64,
                    pos.x as f64 + 1.0,
                    (pos.y as f32 + h) as f64,
                    pos.z as f64 + 1.0,
                );
                from_box.collided_along_vector(travel, &[fluid_box])
            };
            if (!collided && !fluid_collided) || visited.contains(&pos.as_long()) {
                continue;
            }
            visited.push(pos.as_long());
            if collided {
                c.advance(step);
                let intersects = too_far || bb.intersects_block(pos);
                self.entity_inside(state, pos, intersects, block, c, ctx);
                self.portal_inside(state, [pos.x, pos.y, pos.z], dim);
            }
            if fluid_collided {
                c.advance(step);
                match f.kind {
                    FluidKind::Water | FluidKind::FlowingWater => c.apply(Type::Extinguish),
                    FluidKind::Lava | FluidKind::FlowingLava => {
                        c.apply(Type::ClearFreeze);
                        c.apply(Type::LavaIgnite);
                        c.run_after(Type::LavaIgnite, Inside::LavaHurt);
                    }
                    FluidKind::Empty => {}
                }
            }
        }
        counter + 1
    }

    /// `getEntityInsideCollisionShape`; `None` for the full block. Powder snow's is what the
    /// entity would collide with (the full block for one that walks on it).
    fn inside_shape(&self, state: u16) -> Option<&'static kiln_entity::shape::Shape> {
        if kind(state) == Kind::PowderSnow {
            let feet = BlockPos::containing(self.pos[0], self.pos[1], self.pos[2]);
            let (shape, _) = kiln_entity::collision::collision_shape(state, feet, &self.collision_context());
            return match shape {
                std::borrow::Cow::Borrowed(s) if !s.is_empty() => Some(s),
                _ => None,
            };
        }
        physics::inside_shape(state)
    }

    /// `BlockState.entityInside` for the blocks that affect players.
    fn entity_inside(&mut self, state: u16, pos: BlockPos, intersects: bool, block: BlockAt, c: &mut Collector, ctx: &mut DamageCtx) {
        match kind(state) {
            Kind::Fire | Kind::SoulFire => {
                let damage = if kind(state) == Kind::SoulFire { 2.0 } else { 1.0 };
                c.apply(Type::ClearFreeze);
                c.apply(Type::FireIgnite);
                c.run_after(Type::FireIgnite, Inside::FireHurt(damage));
            }
            Kind::Campfire => {
                let info = kiln_data::blocks_types::block_of(state);
                if info.property(state, "lit") == Some("true") {
                    let soul = kiln_entity::blocks::block_name(state) == "minecraft:soul_campfire";
                    let damage = if soul { 2.0 } else { 1.0 };
                    self.hurt(damage, &Cause::Other("minecraft:campfire").into(), ctx);
                }
            }
            Kind::LavaCauldron => {
                c.apply(Type::LavaIgnite);
                c.run_after(Type::LavaIgnite, Inside::LavaHurt);
            }
            Kind::PowderSnow => {
                // `makeStuckInBlock(0.9, 1.5, 0.9)` (the slowdown is the client's; the fall ends).
                self.reset_fall_distance();
                let known = self.known_movement;
                if (known[0] != 0.0 || known[2] != 0.0) && self.level_rng.next_bool() {
                    // (The snowflake particles.)
                }
                c.run_before(Type::Extinguish, Inside::MeltPowderSnow(pos));
                c.apply(Type::Freeze);
                c.apply(Type::Extinguish);
            }
            Kind::Cobweb => self.reset_fall_distance(),
            Kind::SweetBerryBush => {
                // `makeStuckInBlock(0.8, 0.75, 0.8)`, then a grown bush hurts a player that moves.
                self.reset_fall_distance();
                let info = kiln_data::blocks_types::block_of(state);
                if info.property(state, "age") != Some("0") {
                    let m = self.known_movement;
                    if m[0] * m[0] + m[2] * m[2] > 0.0 && (m[0].abs() >= 0.003000000026077032 || m[2].abs() >= 0.003000000026077032) {
                        self.hurt(1.0, &Cause::Other("minecraft:sweet_berry_bush").into(), ctx);
                    }
                }
            }
            Kind::Cactus => {
                self.hurt(1.0, &Cause::Other("minecraft:cactus").into(), ctx);
            }
            Kind::WitherRose => {
                // `WitherRoseBlock.entityInside`: not on peaceful, not for who the wither spares.
                if ctx.rules.difficulty != 0 {
                    self.add_effect(crate::effects::Effect::new(
                        crate::effects::effect_id("minecraft:wither").expect("wither"),
                        40,
                        0,
                        false,
                        true,
                        true,
                    ));
                }
            }
            Kind::BubbleColumn => {
                // `BubbleColumnBlock.entityInside` (a precise hit; a flying player is left alone):
                // above the column the water throws it up (or pulls it down), inside it the fall
                // ends.
                if intersects && !self.flying {
                    let above = block(pos.above());
                    let drag = kiln_data::blocks_types::block_of(state).property(state, "drag") == Some("true");
                    let open = physics::collision_shape(above).is_empty() && physics::fluid_state(above).is_empty();
                    let v = self.server_delta;
                    if open {
                        self.server_delta[1] = if drag { kiln_entity::math::jmax(-0.9, v[1] - 0.03) } else { kiln_entity::math::jmin(1.8, v[1] + 0.1) };
                        // `sendBubbleColumnParticles`: the level's random places the particles.
                        for _ in 0..2 {
                            for _ in 0..4 {
                                self.level_rng.next_double();
                            }
                        }
                    } else {
                        self.server_delta[1] = if drag { kiln_entity::math::jmax(-0.3, v[1] - 0.03) } else { kiln_entity::math::jmin(0.7, v[1] + 0.06) };
                        self.reset_fall_distance();
                    }
                }
            }
            _ => {}
        }
    }
}

/// `InsideBlockEffectType`, in apply order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    Freeze = 0,
    ClearFreeze = 1,
    FireIgnite = 2,
    LavaIgnite = 3,
    Extinguish = 4,
}

const APPLY_ORDER: [Type; 5] = [Type::Freeze, Type::ClearFreeze, Type::FireIgnite, Type::LavaIgnite, Type::Extinguish];

/// An effect to carry out, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Inside {
    FireIgnite,
    LavaIgnite,
    Extinguish,
    Freeze,
    ClearFreeze,
    FireHurt(f32),
    LavaHurt,
    MeltPowderSnow(BlockPos),
}

/// `InsideBlockEffectApplier.StepBasedCollector`: per step, each type once with the actions
/// attached before and after it.
struct Collector {
    in_step: u8,
    before: [Vec<Inside>; 5],
    after: [Vec<Inside>; 5],
    out: Vec<Inside>,
    last_step: i32,
}

impl Default for Collector {
    fn default() -> Self {
        Collector { in_step: 0, before: Default::default(), after: Default::default(), out: Vec::new(), last_step: -1 }
    }
}

impl Collector {
    fn apply(&mut self, t: Type) {
        self.in_step |= 1 << t as u8;
    }

    fn run_before(&mut self, t: Type, a: Inside) {
        self.before[t as usize].push(a);
    }

    fn run_after(&mut self, t: Type, a: Inside) {
        self.after[t as usize].push(a);
    }

    fn advance(&mut self, step: i32) {
        if self.last_step != step {
            self.last_step = step;
            self.flush();
        }
    }

    fn flush(&mut self) {
        for t in APPLY_ORDER {
            let i = t as usize;
            self.out.append(&mut self.before[i]);
            if self.in_step & (1 << i) != 0 {
                self.in_step &= !(1 << i);
                match t {
                    Type::FireIgnite => self.out.push(Inside::FireIgnite),
                    Type::LavaIgnite => self.out.push(Inside::LavaIgnite),
                    Type::Extinguish => self.out.push(Inside::Extinguish),
                    Type::Freeze => self.out.push(Inside::Freeze),
                    Type::ClearFreeze => self.out.push(Inside::ClearFreeze),
                }
            }
            self.out.append(&mut self.after[i]);
        }
    }

    fn finish(mut self) -> Vec<Inside> {
        self.flush();
        self.out
    }
}
