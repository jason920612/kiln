//! The entity model and vanilla's shared `Entity` behaviour: `move`, collision and step-up,
//! supporting blocks, fall distance, bounce restitution, gravity and the base tick.

use crate::blocks::{Kind, Tag, has_tag, kind};
use crate::collision::{self, CollisionContext};
use crate::fluid::FluidInteraction;
use crate::inside::InsideCollector;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, Axis, BlockPos, Direction, Vec3, floor, jmax, jmin, lerp, mth_equal};
use crate::physics;
use crate::shape::Collider;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::collections::VecDeque;

/// `MoverType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoverType {
    SelfMove,
    Player,
    Piston,
    ShulkerBox,
    Shulker,
}

/// `Entity.RemovalReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalReason {
    Killed,
    Discarded,
    UnloadedToChunk,
    UnloadedWithPlayer,
    ChangedDimension,
}

/// `Entity.Movement`: one `move` call's path, replayed by `applyEffectsFromBlocks`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Movement {
    pub from: Vec3,
    pub to: Vec3,
    /// The requested movement (before collision), when the path follows it axis by axis.
    pub axis_dependent_original: Option<Vec3>,
}

/// Per-type state of the entities this crate simulates.
#[derive(Clone, Debug)]
pub enum EntityKind {
    Item(crate::item::ItemData),
    ExperienceOrb(crate::xp_orb::OrbData),
    FallingBlock(crate::falling_block::FallingBlockData),
    Tnt(crate::tnt::TntData),
    /// Snowballs, eggs, ender pearls, thrown potions and experience bottles.
    Throwable(crate::projectile::ThrowableData),
    /// Arrows and spectral arrows.
    Arrow(crate::arrow::ArrowData),
    /// A player, for the server's movement check (`player::server_move`); not ticked here.
    Player(crate::player::PlayerData),
    /// A mob (see [`crate::mob`]).
    Mob(Box<crate::mob::MobData>),
    /// A mob while its own tick holds its data (only the ticking mob itself sees this).
    MobTicking { gravity: f64 },
    /// An entity simulated elsewhere (players), present so behaviours can see it.
    Other { type_name: &'static str },
    /// An entity type with its behaviour in its own module (see [`crate::ext_entity`]).
    Ext(Box<dyn crate::ext_entity::EntityExt>),
}

#[derive(Clone, Debug)]
pub struct Entity {
    /// A mob's landing (`causeFallDamage(distance, multiplier)`) during its move, applied by
    /// the mob once its travel is done (its data is out of the entity meanwhile).
    pub pending_fall: Option<(f64, f32)>,
    /// Damage a minecart took from what it stood in (lava, fire) while its own tick held its
    /// state: (kind, amount, attacker), taken by the cart right after.
    pub pending_hurts: Vec<(DamageKind, f32, Option<i32>)>,
    /// `Projectile.lastDeflectedBy`: the entity that last deflected this projectile (it flies
    /// through that one's box without being turned again).
    pub last_deflected_by: Option<i32>,
    pub id: i32,
    pub uuid: u128,
    pub kind: EntityKind,
    pub type_name: &'static str,
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    position: Vec3,
    block_position: BlockPos,
    bb: Aabb,
    pub delta: Vec3,
    pub y_rot: f32,
    pub x_rot: f32,
    pub old_pos: Vec3,
    pub y_rot_o: f32,
    pub x_rot_o: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub vertical_collision_below: bool,
    pub minor_horizontal_collision: bool,
    pub fall_distance: f64,
    pub tick_count: i32,
    pub remaining_fire_ticks: i32,
    pub air_supply: i32,
    pub ticks_frozen: i32,
    pub is_in_powder_snow: bool,
    pub was_in_powder_snow: bool,
    pub no_physics: bool,
    pub no_gravity: bool,
    pub silent: bool,
    pub invulnerable: bool,
    pub shift_key_down: bool,
    pub removed: Option<RemovalReason>,
    pub first_tick: bool,
    pub invulnerable_time: i32,
    /// Set when the velocity changed enough that trackers must resend it (`hasImpulse`/`needsSync`).
    pub needs_sync: bool,
    pub max_up_step: f32,
    /// `moveDist`, `flyDist`, `nextStep`: step and swim sound pacing (`applyMovementEmissionAndPlaySound`).
    pub move_dist: f32,
    pub fly_dist: f32,
    pub next_step: f32,
    pub stuck_speed_multiplier: Vec3,
    pub main_supporting_block_pos: Option<BlockPos>,
    on_ground_no_blocks: bool,
    pub(crate) movement_this_tick: VecDeque<Movement>,
    pub(crate) final_movements_this_tick: Vec<Movement>,
    pub(crate) fluid: FluidInteraction,
    pub was_touching_water: bool,
    pub was_eye_in_water: bool,
    last_known_position: Option<Vec3>,
    pub last_known_speed: Vec3,
    pub(crate) inside: InsideCollector,
    pub random: LegacyRandom,
    /// The entity this one rides (`Entity.vehicle`) and the ones riding it, first the
    /// controlling one (`passengers`); players by their network id.
    pub vehicle: Option<i32>,
    pub passengers: Vec<i32>,
    /// `canStandOnFluid(lava)`: lava sources hold the entity up (striders).
    pub stands_on_lava: bool,
    /// Saved fields Kiln does not model (custom name, tags, passengers, ...), written back
    /// unchanged by [`crate::persist::save`].
    pub extra: Vec<(String, kiln_proto::nbt::Tag)>,
    /// `Leashable.getLeashData` (mobs and boats; see [`crate::leash`]).
    pub leash: Option<Box<crate::leash::LeashData>>,
}

impl Entity {
    /// A new entity of vanilla type `type_name` (`minecraft:item`, ...) at the origin, as the
    /// `EntityType` constructor leaves it (before per-type constructor randomness).
    pub fn new(type_name: &'static str, id: i32, uuid: u128, kind: EntityKind, random_seed: i64) -> Entity {
        let t = kiln_data::entities::by_name(type_name).unwrap_or_else(|| panic!("unknown entity type {type_name}"));
        let mut e = Entity {
            pending_fall: None,
            pending_hurts: Vec::new(),
            last_deflected_by: None,
            id,
            uuid,
            kind,
            type_name,
            width: t.width,
            height: t.height,
            eye_height: t.eye_height,
            position: Vec3::ZERO,
            block_position: BlockPos::default(),
            bb: Aabb::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            delta: Vec3::ZERO,
            y_rot: 0.0,
            x_rot: 0.0,
            old_pos: Vec3::ZERO,
            y_rot_o: 0.0,
            x_rot_o: 0.0,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            minor_horizontal_collision: false,
            fall_distance: 0.0,
            tick_count: 0,
            remaining_fire_ticks: 0,
            air_supply: 300,
            ticks_frozen: 0,
            is_in_powder_snow: false,
            was_in_powder_snow: false,
            no_physics: false,
            no_gravity: false,
            silent: false,
            invulnerable: false,
            shift_key_down: false,
            removed: None,
            first_tick: true,
            invulnerable_time: 0,
            needs_sync: false,
            max_up_step: 0.0,
            move_dist: 0.0,
            fly_dist: 0.0,
            next_step: 1.0,
            stuck_speed_multiplier: Vec3::ZERO,
            main_supporting_block_pos: None,
            on_ground_no_blocks: false,
            movement_this_tick: VecDeque::new(),
            final_movements_this_tick: Vec::new(),
            fluid: FluidInteraction::default(),
            was_touching_water: false,
            was_eye_in_water: false,
            last_known_position: None,
            last_known_speed: Vec3::ZERO,
            inside: InsideCollector::default(),
            random: LegacyRandom::new(random_seed),
            vehicle: None,
            passengers: Vec::new(),
            stands_on_lava: false,
            extra: Vec::new(),
            leash: None,
        };
        e.set_pos(Vec3::ZERO);
        e.bb = e.make_bounding_box(e.position);
        e
    }

    // ------------------------------------------------------------------ position

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn x(&self) -> f64 {
        self.position.x
    }

    pub fn y(&self) -> f64 {
        self.position.y
    }

    pub fn z(&self) -> f64 {
        self.position.z
    }

    pub fn block_position(&self) -> BlockPos {
        self.block_position
    }

    pub fn bounding_box(&self) -> Aabb {
        self.bb
    }

    pub fn set_bounding_box(&mut self, bb: Aabb) {
        self.bb = bb;
    }

    pub fn eye_y(&self) -> f64 {
        self.position.y + self.eye_height as f64
    }

    /// `Entity.setPos`: position, block position and bounding box.
    pub fn set_pos(&mut self, p: Vec3) {
        self.set_pos_raw(p);
        self.bb = self.make_bounding_box(p);
    }

    pub fn set_pos_raw(&mut self, p: Vec3) {
        if self.position != p {
            self.position = p;
            self.block_position = BlockPos::new(floor(p.x), floor(p.y), floor(p.z));
        }
    }

    /// `EntityDimensions.makeBoundingBox`.
    pub fn make_bounding_box(&self, p: Vec3) -> Aabb {
        let w = self.width / 2.0;
        let h = self.height;
        Aabb::new(p.x - w as f64, p.y, p.z - w as f64, p.x + w as f64, p.y + h as f64, p.z + w as f64)
    }

    /// `Entity.setOldPosAndRot`.
    pub fn set_old_pos_and_rot(&mut self) {
        self.old_pos = self.position;
        self.y_rot_o = self.y_rot;
        self.x_rot_o = self.x_rot;
    }

    pub fn is_alive(&self) -> bool {
        self.removed.is_none()
    }

    pub fn is_removed(&self) -> bool {
        self.removed.is_some()
    }

    pub fn discard(&mut self) {
        self.removed.get_or_insert(RemovalReason::Discarded);
    }

    // ------------------------------------------------------------------ type properties

    /// `getDefaultGravity` (0 with `NoGravity`).
    pub fn gravity(&self) -> f64 {
        if self.no_gravity {
            return 0.0;
        }
        match self.kind {
            EntityKind::Item(_) | EntityKind::FallingBlock(_) | EntityKind::Tnt(_) => 0.04,
            EntityKind::ExperienceOrb(_) => 0.03,
            EntityKind::Player(_) => 0.08,
            EntityKind::Throwable(ref d) => crate::projectile::gravity(d),
            EntityKind::Arrow(_) => 0.05,
            EntityKind::Mob(ref m) => m.attrs.value(crate::mob::attributes::Attr::Gravity),
            EntityKind::MobTicking { gravity } => gravity,
            EntityKind::Ext(ref x) => x.gravity(),
            EntityKind::Other { .. } => 0.0,
        }
    }

    pub fn air_drag(&self) -> f32 {
        0.98
    }

    fn is_living(&self) -> bool {
        matches!(self.kind, EntityKind::Player(_) | EntityKind::Mob(_) | EntityKind::MobTicking { .. })
    }

    /// `fireImmune`.
    pub fn fire_immune(&self) -> bool {
        match &self.kind {
            EntityKind::Item(item) => crate::item::fire_immune(item),
            EntityKind::Tnt(_) => true,
            EntityKind::Mob(m) => m.kind.fire_immune(),
            // A mob in its own tick (striders walking through lava).
            EntityKind::MobTicking { .. } => crate::mob::MobKind::by_name(self.type_name).is_some_and(|k| k.fire_immune()),
            _ => false,
        }
    }

    pub fn is_pushed_by_fluid(&self) -> bool {
        if matches!(self.kind, EntityKind::Mob(_) | EntityKind::MobTicking { .. }) {
            return crate::mob::pushed_by_fluid(self.type_name);
        }
        !matches!(&self.kind, EntityKind::Arrow(a) if a.in_ground)
    }

    pub fn is_suppressing_bounce(&self) -> bool {
        self.shift_key_down
    }

    pub fn is_descending(&self) -> bool {
        self.shift_key_down
    }

    fn entity_bounciness(&self) -> f64 {
        0.0
    }

    /// `canFreeze` (the `freeze_immune_entity_types` tag).
    pub fn can_freeze(&self) -> bool {
        true
    }

    pub fn is_affected_by_blocks(&self) -> bool {
        !self.is_removed() && !self.no_physics
    }

    /// `getBlockPosBelowThatAffectsMyMovement`.
    pub fn block_pos_below_that_affects_movement(&self, level: &dyn EntityLevel) -> BlockPos {
        let offset = match self.kind {
            EntityKind::Item(_) | EntityKind::ExperienceOrb(_) => 0.999999,
            _ => 0.500001,
        };
        self.on_pos(level, offset)
    }

    /// `EntityCollisionContext` for this entity.
    pub fn collision_context(&self) -> CollisionContext {
        CollisionContext {
            descending: self.is_descending(),
            entity_bottom: self.position.y,
            placement: false,
            always_collide_with_fluid: false,
            has_entity: true,
            fall_distance: self.fall_distance,
            falling_block: matches!(self.kind, EntityKind::FallingBlock(_)),
            walks_on_powder_snow: matches!(&self.kind, EntityKind::Player(p) if p.walks_on_powder_snow),
            stands_on_lava: self.stands_on_lava,
        }
    }

    // ------------------------------------------------------------------ tick

    /// `Entity.commonTick`, run by the level before `tick`.
    /// `getLookAngle` (`calculateViewVector(xRot, yRot)`).
    pub fn view_vector(&self) -> Vec3 {
        let f = self.x_rot * 0.017453292;
        let g = -self.y_rot * 0.017453292;
        let h = crate::mob::mth::cos(g as f64);
        let i = crate::mob::mth::sin(g as f64);
        let j = crate::mob::mth::cos(f as f64);
        let k = crate::mob::mth::sin(f as f64);
        Vec3::new((i * j) as f64, (-k) as f64, (h * j) as f64)
    }

    pub fn common_tick(&mut self) {
        if self.invulnerable_time > 0 {
            self.invulnerable_time -= 1;
        }
        self.set_old_pos_and_rot();
        self.tick_count += 1;
    }

    /// One server tick of the entity (the level calls `common_tick` first).
    pub fn tick(&mut self, level: &mut dyn EntityLevel) {
        match self.kind {
            EntityKind::Item(_) => crate::item::tick(self, level),
            EntityKind::ExperienceOrb(_) => crate::xp_orb::tick(self, level),
            EntityKind::FallingBlock(_) => crate::falling_block::tick(self, level),
            EntityKind::Tnt(_) => crate::tnt::tick(self, level),
            EntityKind::Throwable(_) => crate::projectile::tick(self, level),
            EntityKind::Arrow(_) => crate::arrow::tick(self, level),
            EntityKind::Player(_) | EntityKind::MobTicking { .. } => {}
            EntityKind::Mob(_) => crate::mob::tick(self, level),
            EntityKind::Other { .. } => self.base_tick(level),
            EntityKind::Ext(_) => crate::ext_entity::tick(self, level),
        }
    }

    /// `Entity.baseTick`.
    pub fn base_tick(&mut self, level: &mut dyn EntityLevel) {
        self.compute_speed();
        self.was_in_powder_snow = self.is_in_powder_snow;
        self.is_in_powder_snow = false;
        self.was_eye_in_water = self.fluid.is_eye_in_water();
        self.update_fluid_interaction(level);
        if self.remaining_fire_ticks > 0 {
            if self.fire_immune() {
                self.clear_fire();
            } else {
                if self.remaining_fire_ticks % 20 == 0 && !self.is_in_lava() {
                    self.hurt(level, DamageKind::OnFire, 1.0, None);
                }
                self.remaining_fire_ticks -= 1;
            }
        }
        if self.is_in_lava() {
            self.fall_distance *= 0.5;
        }
        if self.position.y < (level.min_y() - 64) as f64 {
            self.discard();
        }
        self.first_tick = false;
        // `Leashable.tickLeash` (boats; mobs do it in their own base tick).
        if self.leash.is_some() {
            crate::leash::tick_leash(self, None, level);
        }
    }

    pub(crate) fn compute_speed(&mut self) {
        let last = *self.last_known_position.get_or_insert(self.position);
        self.last_known_speed = self.position - last;
        self.last_known_position = Some(self.position);
    }

    pub fn clear_fire(&mut self) {
        self.remaining_fire_ticks = self.remaining_fire_ticks.min(0);
    }

    /// `igniteForTicks`.
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        if self.remaining_fire_ticks < ticks {
            self.remaining_fire_ticks = ticks;
        }
        self.ticks_frozen = 0;
    }

    /// `igniteForSeconds`.
    pub fn ignite_for_seconds(&mut self, seconds: f32) {
        self.ignite_for_ticks(kiln_javamath::math::floor_f32(seconds * 20.0));
    }

    pub fn is_on_fire(&self) -> bool {
        self.remaining_fire_ticks > 0
    }

    pub fn is_in_water(&self) -> bool {
        self.was_touching_water
    }

    /// `isInLava`: never on the first tick.
    pub fn is_in_lava(&self) -> bool {
        !self.first_tick && self.fluid.is_in_lava()
    }

    pub fn fluid_height_water(&self) -> f64 {
        self.fluid.height(true)
    }

    pub fn fluid_height_lava(&self) -> f64 {
        self.fluid.height(false)
    }

    /// `hurtServer` for the kinds simulated here; other kinds become an event.
    pub fn hurt(&mut self, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        match self.kind {
            EntityKind::Item(_) => crate::item::hurt(self, level, kind, amount, attacker),
            EntityKind::ExperienceOrb(_) => crate::xp_orb::hurt(self, level, kind, amount),
            EntityKind::FallingBlock(_) | EntityKind::Tnt(_) | EntityKind::Throwable(_) | EntityKind::Arrow(_) => false,
            EntityKind::Mob(_) => {
                let pos = attacker.and_then(|a| level.entity(a)).map(|a| a.position());
                let source = crate::mob::DamageSource { kind, attacker, direct: attacker, pos, attacker_is_player: false };
                crate::mob::hurt_entity(self, level, source, amount)
            }
            EntityKind::MobTicking { .. } => false,
            EntityKind::Ext(_) => {
                let placeholder = EntityKind::Other { type_name: self.type_name };
                let EntityKind::Ext(mut x) = std::mem::replace(&mut self.kind, placeholder) else { unreachable!() };
                let r = x.hurt(self, level, kind, amount, attacker);
                if matches!(self.kind, EntityKind::Other { .. }) {
                    self.kind = EntityKind::Ext(x);
                }
                r
            }
            // A minecart mid-tick: it takes the damage itself right after.
            EntityKind::Other { .. } if crate::ext_entity::minecart::is_minecart(self.type_name) => {
                self.pending_hurts.push((kind, amount, attacker));
                true
            }
            EntityKind::Player(_) | EntityKind::Other { .. } => {
                level.emit(Event::Hurt { target: self.id, amount, kind, attacker });
                true
            }
        }
    }

    /// `hurtServer` for a hit by a projectile (`on_fire`, `speed_sqr`: the projectile's state at
    /// the hit) on an extension entity that cares about it; the others take it as [`Entity::hurt`].
    pub fn hurt_by_projectile(&mut self, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>, on_fire: bool, speed_sqr: f64) -> bool {
        match self.kind {
            EntityKind::Ext(_) => {
                let placeholder = EntityKind::Other { type_name: self.type_name };
                let EntityKind::Ext(mut x) = std::mem::replace(&mut self.kind, placeholder) else { unreachable!() };
                let r = x.hurt_by_projectile(self, level, kind, amount, attacker, on_fire, speed_sqr);
                if matches!(self.kind, EntityKind::Other { .. }) {
                    self.kind = EntityKind::Ext(x);
                }
                r
            }
            _ => self.hurt(level, kind, amount, attacker),
        }
    }

    /// `isInvulnerableToBase` for fire and explosion sources.
    pub fn is_invulnerable_to_base(&self, kind: DamageKind) -> bool {
        self.is_removed()
            || self.invulnerable
            || self.invulnerable_time > 0
            || (matches!(kind, DamageKind::OnFire | DamageKind::InFire | DamageKind::Lava) && self.fire_immune())
    }

    pub fn play_sound(&mut self, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
        if !self.silent {
            let source = match self.kind {
                EntityKind::Item(_) | EntityKind::ExperienceOrb(_) => "ambient",
                _ => "neutral",
            };
            level.emit(Event::Sound { pos: self.position, sound, source, volume, pitch });
        }
    }

    // ------------------------------------------------------------------ gravity and ground

    pub fn apply_gravity(&mut self) {
        let g = self.gravity();
        if g != 0.0 {
            self.delta = self.delta.add(0.0, -g, 0.0);
        }
    }

    pub fn set_on_ground(&mut self, level: &dyn EntityLevel, on_ground: bool) {
        self.on_ground = on_ground;
        self.check_supporting_block(level, on_ground, None);
    }

    fn set_on_ground_with_movement(&mut self, level: &dyn EntityLevel, on_ground: bool, horizontal: bool, movement: Vec3) {
        self.on_ground = on_ground;
        self.horizontal_collision = horizontal;
        self.check_supporting_block(level, on_ground, Some(movement));
    }

    /// `checkSupportingBlock`.
    fn check_supporting_block(&mut self, level: &dyn EntityLevel, on_ground: bool, movement: Option<Vec3>) {
        if on_ground {
            let bb = self.bb;
            let below = Aabb::new(bb.min_x, bb.min_y - 1.0e-6, bb.min_z, bb.max_x, bb.min_y, bb.max_z);
            let ctx = self.collision_context();
            let mut found = collision::find_supporting_block(level, &ctx, self.position, &below);
            if found.is_some() || self.on_ground_no_blocks {
                self.main_supporting_block_pos = found;
            } else if let Some(m) = movement {
                let back = below.offset(-m.x, 0.0, -m.z);
                found = collision::find_supporting_block(level, &ctx, self.position, &back);
                self.main_supporting_block_pos = found;
            }
            self.on_ground_no_blocks = found.is_none();
        } else {
            self.on_ground_no_blocks = false;
            self.main_supporting_block_pos = None;
        }
    }

    /// `getOnPos(offset)`.
    pub fn on_pos(&self, level: &dyn EntityLevel, offset: f32) -> BlockPos {
        if let Some(pos) = self.main_supporting_block_pos {
            if offset > 1.0e-5 {
                let state = level.block(pos);
                if (offset as f64 <= 0.5 && has_tag(state, Tag::Fences))
                    || has_tag(state, Tag::Walls)
                    || kind(state) == Kind::FenceGate
                {
                    return pos;
                }
                return pos.at_y(floor(self.position.y - offset as f64));
            }
            return pos;
        }
        BlockPos::new(floor(self.position.x), floor(self.position.y - offset as f64), floor(self.position.z))
    }

    pub fn on_pos_legacy(&self, level: &dyn EntityLevel) -> BlockPos {
        self.on_pos(level, 0.2)
    }

    /// `getBlockSpeedFactor`.
    pub fn block_speed_factor(&self, level: &dyn EntityLevel) -> f32 {
        let state = level.block(self.block_position);
        let f = physics::block_factors(state).speed;
        if matches!(kind(state), Kind::Water | Kind::BubbleColumn) {
            return f;
        }
        if f as f64 == 1.0 {
            physics::block_factors(level.block(self.block_pos_below_that_affects_movement(level))).speed
        } else {
            f
        }
    }

    /// `getBlockJumpFactor`.
    pub fn block_jump_factor(&self, level: &dyn EntityLevel) -> f32 {
        let f = physics::block_factors(level.block(self.block_position)).jump;
        let below = physics::block_factors(level.block(self.block_pos_below_that_affects_movement(level))).jump;
        if f as f64 == 1.0 { below } else { f }
    }

    // ------------------------------------------------------------------ move

    /// `Entity.move(MoverType, Vec3)`.
    pub fn do_move(&mut self, level: &mut dyn EntityLevel, mover: MoverType, mut movement: Vec3) {
        if self.no_physics {
            self.set_pos(self.position.add(movement.x, movement.y, movement.z));
            self.horizontal_collision = false;
            self.vertical_collision = false;
            self.vertical_collision_below = false;
            self.minor_horizontal_collision = false;
            return;
        }
        if self.stuck_speed_multiplier.length_sqr() > 1.0e-7 {
            if mover != MoverType::Piston {
                movement = movement.multiply_vec(self.stuck_speed_multiplier);
            }
            self.stuck_speed_multiplier = Vec3::ZERO;
            self.delta = Vec3::ZERO;
        }
        movement = self.maybe_back_off_from_edge(level, movement, mover);
        let collided = {
            crate::prof!("mv", "collide");
            self.collide(level, movement)
        };
        let d = collided.length_sqr();
        if d > 1.0e-7 || movement.length_sqr() - d < 1.0e-7 {
            if self.fall_distance != 0.0 && d >= 1.0 {
                let len = jmin(collided.length(), 8.0);
                let end = self.position + collided.normalize().scale(len);
                if crate::clip::clip_fall_damage_resetting(level, self.position, end, self) {
                    self.fall_distance = 0.0;
                }
            }
            let from = self.position;
            let to = from + collided;
            self.add_movement_this_tick(Movement { from, to, axis_dependent_original: Some(movement) });
            self.set_pos(to);
        }
        crate::prof!("mv", "after collide");
        let x_collision = !mth_equal(movement.x, collided.x);
        let z_collision = !mth_equal(movement.z, collided.z);
        self.horizontal_collision = x_collision || z_collision;
        let vertical_move = movement.y.abs() > 0.0;
        if vertical_move || self.is_local_instance_authoritative() {
            self.vertical_collision = movement.y != collided.y;
            self.vertical_collision_below = self.vertical_collision && movement.y < 0.0;
            let (below, horizontal) = (self.vertical_collision_below, self.horizontal_collision);
            self.set_on_ground_with_movement(level, below, horizontal, collided);
        }
        self.minor_horizontal_collision = false;
        let on_pos = self.on_pos_legacy(level);
        let on_state = level.block(on_pos);
        if self.is_local_instance_authoritative() {
            self.check_fall_damage(level, collided.y, self.on_ground, on_state, on_pos);
        }
        if self.is_removed() {
            return;
        }
        if self.can_simulate_movement() && ((vertical_move && self.vertical_collision) || self.horizontal_collision) {
            self.restitute_movement_after_collisions(level, on_state, x_collision, z_collision, collided);
        }
        if matches!(self.kind, EntityKind::Mob(_) | EntityKind::MobTicking { .. }) {
            crate::prof!("mv", "emission");
            self.apply_movement_emission(level, collided, on_pos, on_state);
        }
        crate::prof!("mv", "speed factor");
        let f = self.block_speed_factor(level) as f64;
        self.delta = self.delta.multiply(f, 1.0, f);
    }

    /// `applyMovementEmissionAndPlaySound` (`MovementEmission.ALL`, not riding): walking step
    /// sounds and, in water, swim sounds (their pitch draws from the random).
    fn apply_movement_emission(&mut self, level: &mut dyn EntityLevel, movement: Vec3, pos: BlockPos, state: u16) {
        let len = (movement.length() * 0.6000000238418579) as f32;
        let horizontal = (movement.horizontal_distance() * 0.6000000238418579) as f32;
        let on_pos = self.on_pos(level, 1.0e-5);
        let on_state = level.block(on_pos);
        let climbable = |s: u16| has_tag(s, Tag::Climbable);
        self.move_dist += if climbable(on_state) { len } else { horizontal };
        self.fly_dist += len;
        if !(self.move_dist > self.next_step) || kiln_data::blocks_types::is_air(on_state) {
            return;
        }
        // `vibrationAndSoundEffectsFromBlock`: a step on the ground or a climbable block (the step
        // sound draws nothing).
        let stepped = |e: &Entity, s: u16| !kiln_data::blocks_types::is_air(s) && (e.on_ground || climbable(s));
        let mut ok = stepped(self, state);
        if on_pos != pos {
            ok |= stepped(self, on_state);
        }
        // The step game event comes from the supporting block (the effect block when they are
        // the same), with that block as the context.
        let supporting = if on_pos == pos { state } else { on_state };
        if stepped(self, supporting) {
            level.block_game_event("minecraft:step", self.position, Some(self.id), supporting);
        }
        if ok {
            self.next_step = (self.move_dist as i32 + 1) as f32;
        } else if self.is_in_water() {
            self.next_step = (self.move_dist as i32 + 1) as f32;
            if let Some(sound) = crate::mob::swim_sound_of(self) {
                let d = self.delta;
                let volume = (1.0f32).min(((d.x * d.x * 0.20000000298023224 + d.y * d.y + d.z * d.z * 0.20000000298023224).sqrt() as f32) * 0.35);
                let pitch = 1.0 + (self.random_next_float_pub() - self.random_next_float_pub()) * 0.4;
                self.play_sound(level, sound, volume, pitch);
            }
            level.emit(Event::GameEvent { event: "minecraft:swim", pos: self.position, entity: Some(self.id) });
        }
    }

    fn random_next_float_pub(&mut self) -> f32 {
        kiln_javamath::random::RandomSource::next_float(&mut self.random)
    }

    /// Server side: false for players, whose client is authoritative.
    fn is_local_instance_authoritative(&self) -> bool {
        !matches!(self.kind, EntityKind::Player(_))
    }

    fn can_simulate_movement(&self) -> bool {
        true
    }

    /// `maybeBackOffFromEdge`: only players override it.
    fn maybe_back_off_from_edge(&self, level: &dyn EntityLevel, movement: Vec3, mover: MoverType) -> Vec3 {
        crate::player::back_off_from_edge(self, level, movement, mover)
    }

    fn add_movement_this_tick(&mut self, m: Movement) {
        if self.movement_this_tick.len() >= 100 {
            let a = self.movement_this_tick.pop_front().unwrap();
            let b = self.movement_this_tick.pop_front().unwrap();
            self.movement_this_tick.push_front(Movement { from: a.from, to: b.to, axis_dependent_original: None });
        }
        self.movement_this_tick.push_back(m);
    }

    /// `Entity.collide`: block and entity collision, then step-up.
    pub fn collide(&self, level: &dyn EntityLevel, movement: Vec3) -> Vec3 {
        let bb = self.bb;
        let ctx = self.collision_context();
        let entity_shapes = {
            crate::prof!("mv", "entity_colliders");
            collision::entity_colliders(level, self.id, &bb.expand_towards_vec(movement).expand_towards(0.0, self.max_up_step as f64, 0.0))
        };
        let collided = if movement.length_sqr() == 0.0 {
            movement
        } else {
            crate::prof!("mv", "collide_bounding_box");
            collision::collide_bounding_box(level, &ctx, movement, &bb, &entity_shapes)
        };
        let x_changed = movement.x != collided.x;
        let y_changed = movement.y != collided.y;
        let z_changed = movement.z != collided.z;
        let on_ground_after = y_changed && movement.y < 0.0;
        if self.max_up_step > 0.0 && (on_ground_after || self.on_ground) && (x_changed || z_changed) {
            let base = if on_ground_after { bb.offset(0.0, collided.y, 0.0) } else { bb };
            let mut step_box = base.expand_towards(movement.x, self.max_up_step as f64, movement.z);
            if !on_ground_after {
                step_box = step_box.expand_towards(0.0, -9.999999747378752e-6, 0.0);
            }
            let colliders = collision::collect_colliders(level, &ctx, &entity_shapes, &step_box);
            let fy = collided.y as f32;
            for h in candidate_step_up_heights(&base, &colliders, self.max_up_step, fy) {
                let stepped = collision::collide_with_shapes(Vec3::new(movement.x, h as f64, movement.z), &base, &colliders);
                if stepped.horizontal_distance_sqr() > collided.horizontal_distance_sqr() {
                    let d = bb.min_y - base.min_y;
                    return stepped.subtract(0.0, d, 0.0);
                }
            }
        }
        collided
    }

    /// `checkFallDamage`.
    fn check_fall_damage(&mut self, level: &mut dyn EntityLevel, y: f64, on_ground: bool, state: u16, pos: BlockPos) {
        // `AbstractBoat.checkFallDamage` (a boat is placeholder-typed while it ticks).
        if crate::ext_entity::boat::is_boat(self.type_name) {
            crate::ext_entity::boat::check_fall_damage(self, level, y, on_ground);
            return;
        }
        // `LivingEntity.checkFallDamage`: out of water, the fluid state is refreshed after the move
        // (a mob falling into water splashes in the same tick).
        if matches!(self.kind, EntityKind::MobTicking { .. } | EntityKind::Mob(_)) && !crate::mob::checks_fall_damage(self.type_name) {
            return;
        }
        // `Strider.checkFallDamage`: no falling in lava.
        if self.type_name == "minecraft:strider" && self.is_in_lava() {
            self.fall_distance = 0.0;
            return;
        }
        if self.is_living() && !self.is_in_water() {
            self.update_fluid_interaction(level);
        }
        if !self.is_in_water() && y < 0.0 {
            self.fall_distance -= y as f32 as f64;
        }
        if on_ground {
            if self.fall_distance > 0.0 {
                crate::fall::fall_on(self, level, state, pos);
                level.block_game_event("minecraft:hit_ground", self.position, Some(self.id), state);
            }
            self.fall_distance = 0.0;
        }
    }

    /// `restituteMovementAfterCollisions`: bounces (slime, beds) and the velocity loss on impact.
    fn restitute_movement_after_collisions(
        &mut self,
        level: &mut dyn EntityLevel,
        state: u16,
        x_collision: bool,
        z_collision: bool,
        collided: Vec3,
    ) {
        let mut bounciness = if self.is_suppressing_bounce() { 0.0 } else { self.entity_bounciness() };
        let v = self.delta;
        let mut out = v;
        if x_collision {
            out = out.with(Axis::X, -v.x * bounciness);
        }
        if z_collision {
            out = out.with(Axis::Z, -v.z * bounciness);
        }
        let mut bounced = bounciness > 0.0 && (x_collision || z_collision);
        if self.vertical_collision {
            if self.vertical_collision_below {
                bounciness = if -v.y <= self.gravity() || self.is_suppressing_bounce() || has_tag(state, Tag::SuppressesBounce) {
                    0.0
                } else {
                    jmax(bounciness, self.block_bounciness(state))
                };
            }
            let (a, c);
            if bounciness > 0.0 {
                let ratio = collided.y / v.y;
                a = ratio * self.gravity();
                c = lerp(ratio, 1.0, self.air_drag() as f64);
                bounced = true;
            } else {
                a = 0.0;
                c = 1.0;
            }
            out = out.with(Axis::Y, (a - v.y) * c * bounciness);
        }
        if bounced {
            level.emit(Event::GameEvent { event: "minecraft:bounce", pos: self.position, entity: Some(self.id) });
            self.needs_sync = true;
        }
        self.delta = out;
    }

    fn block_bounciness(&self, state: u16) -> f64 {
        let mut f = physics::block_factors(state).bounce;
        if !self.is_living() {
            f *= 0.8;
        }
        f as f64
    }

    /// `moveTowardsClosestSpace`: nudges an entity stuck in a block toward the nearest open side.
    pub fn move_towards_closest_space(&mut self, level: &dyn EntityLevel, x: f64, y: f64, z: f64) {
        let pos = BlockPos::containing(x, y, z);
        let local = Vec3::new(x - pos.x as f64, y - pos.y as f64, z - pos.z as f64);
        let mut best_dir = Direction::Up;
        let mut best = f64::MAX;
        for dir in [Direction::North, Direction::South, Direction::West, Direction::East, Direction::Up] {
            let p = pos.relative(dir);
            if !kiln_data::block_props::full_collision(level.block(p)) {
                let c = local.get(dir.axis());
                let dist = if dir.is_positive() { 1.0 - c } else { c };
                if dist < best {
                    best = dist;
                    best_dir = dir;
                }
            }
        }
        let speed = self.random.next_float() * 0.2 + 0.1;
        let step = if best_dir.is_positive() { 1.0f32 } else { -1.0f32 };
        let v = self.delta.scale(0.75);
        self.delta = match best_dir.axis() {
            Axis::X => Vec3::new((step * speed) as f64, v.y, v.z),
            Axis::Y => Vec3::new(v.x, (step * speed) as f64, v.z),
            Axis::Z => Vec3::new(v.x, v.y, (step * speed) as f64),
        };
    }

    /// `makeStuckInBlock` (cobwebs, sweet berry bushes, powder snow).
    pub fn make_stuck_in_block(&mut self, multiplier: Vec3) {
        self.fall_distance = 0.0;
        self.stuck_speed_multiplier = multiplier;
    }

    /// `addDeltaMovement`: ignored unless finite.
    pub fn add_delta_movement(&mut self, v: Vec3) {
        if v.x.is_finite() && v.y.is_finite() && v.z.is_finite() {
            self.delta = self.delta + v;
        }
    }
}

/// `collectCandidateStepUpHeights`: collider tops within `max_up_step` above `base`, ascending.
fn candidate_step_up_heights(base: &Aabb, colliders: &[Collider], max_up_step: f32, collided_y: f32) -> Vec<f32> {
    let mut set: Vec<f32> = Vec::with_capacity(4);
    for c in colliders {
        for y in c.coords(Axis::Y) {
            let h = (y - base.min_y) as f32;
            if h < 0.0 || h == collided_y {
                continue;
            }
            if h > max_up_step {
                break;
            }
            if !set.iter().any(|v| v.to_bits() == h.to_bits()) {
                set.push(h);
            }
        }
    }
    set.sort_by(|a, b| a.total_cmp(b));
    set
}
