//! Boats and rafts (`AbstractBoat`): they float (water levels of their box, buoyancy, drag by
//! what they are in), sit on land with the ground's friction, take hits (+10 damage per point;
//! creative players break them at once, over 40 they drop their item), carry one or two
//! riders and push or take in what bumps into them. A boat a player steers is moved by the
//! player's client (`ServerboundMoveVehiclePacket`); the server only floats it while nobody
//! steers. Chest boats and chest rafts hold 27 slots like a chest minecart (loot table, saved
//! like a chest, dropped when the boat breaks, opened by a click while sneaking or when no
//! rider fits). Bubble columns are not simulated.

use crate::collision;
use crate::entity::{Entity, EntityKind, MoverType};
use crate::ext_entity::EntityExt;
use crate::fluid;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3, ceil, floor};
use crate::mob::interact::{Interactor, Outcome};
use crate::ext_entity::minecart::{Contents, drop_entity_contents};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

/// `AbstractBoat.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}

#[derive(Clone, Debug)]
pub struct Boat {
    pub raft: bool,
    pub chest: bool,
    pub status: Status,
    /// `oldStatus` (none before the first tick, as in vanilla's uninitialised field).
    old_status: Option<Status>,
    started: bool,
    /// `outOfControlTicks`: ticks spent under water (60 and the riders are thrown off).
    pub out_of_control: f32,
    delta_rotation: f32,
    water_level: f64,
    land_friction: f32,
    last_yd: f64,
    pub hurt_time: i32,
    hurt_dir: i32,
    pub damage: f32,
    pub paddles: [bool; 2],
    paddle_positions: [f32; 2],
    /// A chest boat's 27 slots (`AbstractChestBoat`).
    pub contents: Option<Contents>,
}

/// Whether `name` is a boat or raft type.
pub fn is_boat(name: &str) -> bool {
    name.ends_with("_boat") || name.ends_with("_raft") || name.ends_with("_chest_boat") || name.ends_with("_chest_raft")
}

impl Boat {
    fn of(name: &str) -> Boat {
        Boat {
            raft: name.ends_with("_raft") || name.ends_with("_chest_raft"),
            chest: name.contains("_chest_"),
            status: Status::InAir,
            old_status: None,
            started: false,
            out_of_control: 0.0,
            delta_rotation: 0.0,
            water_level: 0.0,
            land_friction: 0.0,
            last_yd: 0.0,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            paddles: [false; 2],
            paddle_positions: [0.0; 2],
            contents: name.contains("_chest_").then(|| Contents::new(27)),
        }
    }

    fn max_passengers(&self) -> usize {
        if self.chest { 1 } else { 2 }
    }

    /// `rideHeight` of the type (`Raft`: 0.888889 of the height, `Boat`: a third).
    fn ride_height(&self, e: &Entity) -> f64 {
        if self.raft { (e.height * 0.8888889) as f64 } else { (e.height / 3.0) as f64 }
    }

    /// `setPaddleState`.
    pub fn set_paddle_state(&mut self, left: bool, right: bool) {
        self.paddles = [left, right];
    }
}

/// A new boat of `type_name` at `pos` facing `y_rot` (`BoatItem.getBoat`).
pub fn new(type_name: &'static str, pos: Vec3, y_rot: f32, seed: i64) -> Entity {
    let mut e = Entity::new(type_name, 0, 0, EntityKind::Ext(Box::new(Boat::of(type_name))), seed);
    e.set_pos(pos);
    e.y_rot = y_rot;
    e.set_old_pos_and_rot();
    e
}

/// Reads a saved one.
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let mut b = Boat::of(type_name);
    // `readChestVehicleSaveData`.
    if let Some(c) = &b.contents {
        b.contents = Some(Contents::load(r, c.items.len()));
    }
    Some(Box::new(b))
}

/// The item a boat of `type_name` drops (the entity and item names agree).
pub fn drop_item(type_name: &str) -> &str {
    type_name
}

/// `Entity.getPassengerAttachmentPoint` of the boat for its passenger at `index` of `count`.
pub fn passenger_attachment(e: &Entity, b: &Boat, index: usize, count: usize, animal: bool) -> Vec3 {
    let mut x = if b.chest { 0.15f32 } else { 0.0 };
    if count > 1 {
        x = if index == 0 { 0.2 } else { -0.6 };
        if animal {
            x += 0.2;
        }
    }
    crate::ride::y_rot(Vec3::new(0.0, b.ride_height(e), x as f64), -e.y_rot * 0.017453292)
}

fn water(level: &dyn EntityLevel, pos: BlockPos) -> Option<crate::physics::FluidState> {
    let f = fluid::fluid_at(level, pos);
    f.kind.is_water().then_some(f)
}

impl Boat {
    /// `isUnderwater`: `None` when the top of the box is out of the water.
    fn under_water(&self, bb: &Aabb, level: &dyn EntityLevel) -> Option<Status> {
        let d = bb.max_y + 0.001;
        let mut flag = false;
        for x in floor(bb.min_x)..ceil(bb.max_x) {
            for y in floor(bb.max_y)..ceil(d) {
                for z in floor(bb.min_z)..ceil(bb.max_z) {
                    let pos = BlockPos::new(x, y, z);
                    let Some(f) = water(level, pos) else { continue };
                    if d < y as f64 + fluid::height(level, pos, &f) as f64 {
                        if f.source {
                            flag = true;
                        } else {
                            return Some(Status::UnderFlowingWater);
                        }
                    }
                }
            }
        }
        flag.then_some(Status::UnderWater)
    }

    /// `checkInWater` (sets the water level).
    fn check_in_water(&mut self, bb: &Aabb, level: &dyn EntityLevel) -> bool {
        self.water_level = f64::MIN;
        let mut in_water = false;
        for x in floor(bb.min_x)..ceil(bb.max_x) {
            for y in floor(bb.min_y)..ceil(bb.min_y + 0.001) {
                for z in floor(bb.min_z)..ceil(bb.max_z) {
                    let pos = BlockPos::new(x, y, z);
                    let Some(f) = water(level, pos) else { continue };
                    let top = (y as f32 + fluid::height(level, pos, &f)) as f64;
                    self.water_level = self.water_level.max(top);
                    in_water |= bb.min_y < top;
                }
            }
        }
        in_water
    }

    /// `getGroundFriction`: the mean friction of the blocks the underside of the box touches.
    fn ground_friction(&self, bb: &Aabb, level: &dyn EntityLevel) -> f32 {
        let slab = Aabb::new(bb.min_x, bb.min_y - 0.001, bb.min_z, bb.max_x, bb.min_y, bb.max_z);
        let (x0, x1) = (floor(bb.min_x) - 1, ceil(bb.max_x) + 1);
        let (y0, y1) = (floor(bb.min_y) - 1, ceil(bb.max_y) + 1);
        let (z0, z1) = (floor(bb.min_z) - 1, ceil(bb.max_z) + 1);
        let mut total = 0.0f32;
        let mut count = 0;
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = (x == x0 || x == x1 - 1) as i32 + (z == z0 || z == z1 - 1) as i32;
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges > 0 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let pos = BlockPos::new(x, y, z);
                    let state = level.block(pos);
                    if crate::blocks::block_name(state) == "minecraft:lily_pad" {
                        continue;
                    }
                    let (shape, _) = collision::collision_shape(state, pos, &collision::CollisionContext::EMPTY);
                    let hit = shape.boxes().iter().any(|b| {
                        let b = b.offset(x as f64, y as f64, z as f64);
                        b.min_x < slab.max_x && b.max_x > slab.min_x && b.min_y < slab.max_y && b.max_y > slab.min_y && b.min_z < slab.max_z && b.max_z > slab.min_z
                    });
                    if hit {
                        total += crate::physics::block_factors(state).friction;
                        count += 1;
                    }
                }
            }
        }
        total / count as f32
    }

    /// `getWaterLevelAbove`.
    fn water_level_above(&self, e: &Entity, level: &dyn EntityLevel) -> f32 {
        let bb = e.bounding_box();
        let (y0, y1) = (floor(bb.max_y), ceil(bb.max_y - self.last_yd));
        for y in y0..y1 {
            let mut f = 0.0f32;
            'scan: for x in floor(bb.min_x)..ceil(bb.max_x) {
                for z in floor(bb.min_z)..ceil(bb.max_z) {
                    let pos = BlockPos::new(x, y, z);
                    if let Some(w) = water(level, pos) {
                        f = f.max(fluid::height(level, pos, &w));
                    }
                    if f >= 1.0 {
                        break 'scan;
                    }
                }
            }
            if f < 1.0 {
                return y as f32 + f;
            }
        }
        (y1 + 1) as f32
    }

    fn get_status(&mut self, e: &Entity, level: &dyn EntityLevel) -> Status {
        let bb = e.bounding_box();
        if let Some(under) = self.under_water(&bb, level) {
            self.water_level = bb.max_y;
            return under;
        }
        if self.check_in_water(&bb, level) {
            return Status::InWater;
        }
        let f = self.ground_friction(&bb, level);
        if f > 0.0 {
            self.land_friction = f;
            return Status::OnLand;
        }
        Status::InAir
    }

    /// `floatBoat`.
    fn float_boat(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, player_controlled: bool) {
        let mut d0 = -0.04;
        let mut d1 = 0.0;
        let f;
        if self.old_status == Some(Status::InAir) && self.status != Status::InAir && self.status != Status::OnLand {
            self.water_level = e.y() + e.height as f64;
            d1 = (self.water_level_above(e, level) - e.height) as f64 + 0.101;
            let moved = e.bounding_box().offset(0.0, d1 - e.y(), 0.0);
            if collision::no_collision(level, &e.collision_context(), e.id, &moved) {
                e.set_pos(Vec3::new(e.x(), d1, e.z()));
                e.delta = e.delta.multiply(1.0, 0.0, 1.0);
                self.last_yd = 0.0;
            }
            self.status = Status::InWater;
        } else {
            match self.status {
                Status::InWater => {
                    d1 = (self.water_level - e.y()) / e.height as f64;
                    f = 0.9;
                }
                Status::UnderFlowingWater => {
                    d0 = -7.0e-4;
                    f = 0.9;
                }
                Status::UnderWater => {
                    d1 = 0.009999999776482582;
                    f = 0.45;
                }
                Status::InAir => f = 0.9,
                Status::OnLand => {
                    f = self.land_friction;
                    if player_controlled {
                        self.land_friction /= 2.0;
                    }
                }
            }
            let v = e.delta;
            e.delta = Vec3::new(v.x * f as f64, v.y + d0, v.z * f as f64);
            self.delta_rotation *= f;
            if d1 > 0.0 {
                let v = e.delta;
                e.delta = Vec3::new(v.x, (v.y + d1 * (0.04 / 0.65)) * 0.75, v.z);
            }
        }
    }
}

/// `AbstractBoat.checkFallDamage`: no damage; the fall is only counted over dry ground.
pub(crate) fn check_fall_damage(e: &mut Entity, level: &mut dyn EntityLevel, y: f64, on_ground: bool) {
    if e.vehicle.is_some() {
        return;
    }
    if on_ground {
        e.fall_distance = 0.0;
    } else if !fluid::fluid_at(level, e.block_position().below()).kind.is_water() && y < 0.0 {
        e.fall_distance -= y as f32 as f64;
    }
}

impl EntityExt for Boat {
    crate::entity_ext_boilerplate!();

    /// `AbstractBoat.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.old_status = self.started.then_some(self.status);
        self.started = true;
        self.status = self.get_status(e, level);
        if matches!(self.status, Status::UnderWater | Status::UnderFlowingWater) {
            self.out_of_control += 1.0;
        } else {
            self.out_of_control = 0.0;
        }
        // Under water for three seconds: everybody aboard is thrown off.
        if self.out_of_control >= 60.0 {
            crate::ride::eject(e, level);
        }
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if self.damage > 0.0 {
            self.damage -= 1.0;
        }
        e.base_tick(level);
        let player_controlled = e.passengers.first().is_some_and(|&p| level.player(p).is_some());
        if !player_controlled {
            // Nobody steers: the server floats it.
            self.paddles = [false; 2];
            self.float_boat(e, level, false);
            let movement = e.delta;
            // (`checkFallDamage` remembers the vertical speed it was moved with.)
            self.last_yd = movement.y;
            e.do_move(level, MoverType::SelfMove, movement);
        } else {
            e.delta = Vec3::ZERO;
            self.delta_rotation = 0.0;
        }
        e.apply_effects_from_blocks(level);
        // The paddles: the blades turn and splash.
        for i in 0..2 {
            if self.paddles[i] {
                let p = self.paddle_positions[i];
                if !e.silent && (p % 6.2831855) as f64 <= 0.7853981852531433 && ((p + 0.3926991) % 6.2831855) as f64 >= 0.7853981852531433 {
                    let sound = match self.status {
                        Status::InWater | Status::UnderWater | Status::UnderFlowingWater => Some("minecraft:entity.boat.paddle_water"),
                        Status::OnLand => Some("minecraft:entity.boat.paddle_land"),
                        Status::InAir => None,
                    };
                    if let Some(sound) = sound {
                        let yaw = -e.y_rot * 0.017453292;
                        let (vx, vz) = (crate::mob::mth::sin(yaw as f64) as f64, crate::mob::mth::cos(yaw as f64) as f64);
                        let dx = if i == 1 { -vz } else { vz };
                        let dz = if i == 1 { vx } else { -vx };
                        let pitch = 0.8 + 0.4 * e.random.next_float();
                        level.emit(Event::Sound { pos: e.position().add(dx, 0.0, dz), sound, source: "neutral", volume: 1.0, pitch });
                    }
                }
                self.paddle_positions[i] += 0.3926991;
            } else {
                self.paddle_positions[i] = 0.0;
            }
        }
        // Things bumping into the boat: mobs climb aboard when nobody steers, the rest is
        // pushed away.
        let area = e.bounding_box().inflate(0.2, -0.009999999776482582, 0.2);
        let can_board = !player_controlled;
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            if e.passengers.contains(&id) {
                continue;
            }
            let Some(other) = level.entity(id) else { continue };
            if other.is_removed() || other.vehicle.is_some() || !matches!(other.kind, EntityKind::Mob(_)) {
                continue;
            }
            let boards = can_board && e.passengers.len() < self.max_passengers() && other.width < e.width && !cannot_board(other.type_name);
            if boards {
                if let Some(rider) = level.entity_mut(id) {
                    crate::ride::start_riding(rider, e, false);
                }
            } else {
                push_apart(e, self, level, id);
            }
        }
    }

    /// `VehicleEntity.hurtServer`.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        if e.is_removed() {
            return true;
        }
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        self.hurt_dir = -self.hurt_dir;
        self.hurt_time = 10;
        self.damage += amount * 10.0;
        e.needs_sync = true;
        level.emit(Event::GameEvent { event: "minecraft:entity_damage", pos: e.position(), entity: attacker });
        let creative = attacker.and_then(|a| level.player(a)).is_some_and(|p| p.creative);
        if !creative && self.damage > 40.0 {
            self.destroy(e, level, kind, attacker);
        } else if creative {
            // `discard`: a chest boat drops what it holds (`shouldDestroy`).
            self.remove(e, level, true);
        }
        true
    }

    /// `AbstractBoat.interact`: a click gets the player aboard unless sneaking, under water
    /// too long, or the boat is full. A chest boat then opens its menu for a player who
    /// sneaks or cannot board (`AbstractChestBoat.interact`).
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, _stack: &kiln_item::ItemStack) -> Option<Outcome> {
        let full = e.passengers.len() >= self.max_passengers();
        if !who.sneaking && self.out_of_control < 60.0 && !full {
            let mut out = Outcome::success(crate::mob::interact::HeldChange::None);
            out.ride = true;
            return Some(out);
        }
        if let Some(c) = &mut self.contents {
            // `interactWithContainerVehicle` (the loot table rolls with the player's luck),
            // then `container_open` and angry piglins.
            c.unpack(level, e.position(), Some(who.id));
            let mut out = Outcome::success(crate::mob::interact::HeldChange::None);
            out.open_container = true;
            level.emit(Event::GameEvent { event: "minecraft:container_open", pos: e.position(), entity: Some(who.id) });
            crate::mob::kinds::piglin::anger_nearby_piglins(level, who.id, true);
            return Some(out);
        }
        Some(Outcome::PASS)
    }

    fn container(&self) -> Option<&Contents> {
        self.contents.as_ref()
    }

    fn container_mut(&mut self) -> Option<&mut Contents> {
        self.contents.as_mut()
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        if let Some(c) = &self.contents {
            c.save(o);
        }
    }

    fn attackable(&self) -> bool {
        true
    }

    fn passenger_offset(&self, e: &Entity, index: usize, animal: bool) -> Option<Vec3> {
        Some(passenger_attachment(e, self, index, e.passengers.len(), animal))
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        if self.hurt_time != 0 {
            d.set(data::vehicle_entity::ID_HURT, &DataValue::Int(self.hurt_time));
        }
        if self.hurt_dir != 1 {
            d.set(data::vehicle_entity::ID_HURTDIR, &DataValue::Int(self.hurt_dir));
        }
        if self.damage != 0.0 {
            d.set(data::vehicle_entity::ID_DAMAGE, &DataValue::Float(self.damage));
        }
        if self.paddles[0] {
            d.set(data::abstract_boat::ID_PADDLE_LEFT, &DataValue::Boolean(true));
        }
        if self.paddles[1] {
            d.set(data::abstract_boat::ID_PADDLE_RIGHT, &DataValue::Boolean(true));
        }
    }

}

/// `#cannot_be_pushed_onto_boats`.
fn cannot_board(type_name: &str) -> bool {
    matches!(
        type_name,
        "minecraft:player"
            | "minecraft:elder_guardian"
            | "minecraft:cod"
            | "minecraft:pufferfish"
            | "minecraft:salmon"
            | "minecraft:tropical_fish"
            | "minecraft:dolphin"
            | "minecraft:squid"
            | "minecraft:glow_squid"
            | "minecraft:tadpole"
            | "minecraft:creaking"
            | "minecraft:nautilus"
            | "minecraft:zombie_nautilus"
            | "minecraft:sulfur_cube"
    )
}

impl Boat {
    /// `remove`: a chest boat drops what it holds first (`shouldDestroy`: killed or discarded).
    fn remove(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, discarded: bool) {
        crate::ride::eject(e, level);
        if let Some(c) = &mut self.contents {
            drop_entity_contents(c, e, level);
        }
        if discarded {
            e.discard();
        } else {
            e.removed.get_or_insert(crate::entity::RemovalReason::Killed);
        }
    }

    /// `VehicleEntity.destroy(level, source)`: the boat is killed and, with entity drops on,
    /// leaves its item named as it was; a chest boat drops its contents (`chestVehicleDestroyed`,
    /// which also angers piglins when a player hit it).
    fn destroy(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, attacker: Option<i32>) {
        self.remove(e, level, false);
        if !level.entity_drops() {
            return;
        }
        if let Some(mut stack) = kiln_item::ItemStack::of(drop_item(e.type_name), 1) {
            if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
                stack.set(kiln_item::component::Component::CustomName(name));
            }
            let (id, seed) = (level.next_entity_id(), level.fresh_seed());
            let mut item = crate::item::new_at(id, 0, stack, e.position(), seed);
            // `setDefaultPickUpDelay`.
            if let EntityKind::Item(d) = &mut item.kind {
                d.pickup_delay = 10;
            }
            level.add_entity(item);
        }
        if let Some(c) = &mut self.contents {
            drop_entity_contents(c, e, level);
            // `getDirectEntity() instanceof Player`: a melee hit.
            if let Some(a) = attacker.filter(|&a| kind == DamageKind::PlayerAttack && level.player(a).is_some()) {
                crate::mob::kinds::piglin::anger_nearby_piglins(level, a, true);
            }
        }
    }
}

/// `Entity.push(entity)`: the two are pushed apart a little.
fn push_apart(e: &mut Entity, _b: &Boat, level: &mut dyn EntityLevel, id: i32) {
    let Some(other) = level.entity(id) else { return };
    let (mut dx, mut dz) = (other.x() - e.x(), other.z() - e.z());
    let mut d2 = dx.abs().max(dz.abs());
    if d2 < 0.01 {
        return;
    }
    d2 = d2.sqrt();
    dx /= d2;
    dz /= d2;
    let d3 = (1.0 / d2).min(1.0);
    dx *= d3 * 0.05;
    dz *= d3 * 0.05;
    // Neither is a vehicle with riders: both are pushed.
    if e.passengers.is_empty() {
        e.delta = e.delta.add(-dx, 0.0, -dz);
    }
    level.push(id, Vec3::new(dx, 0.0, dz));
}
