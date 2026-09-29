//! Minecarts (`AbstractMinecart` with vanilla's default `OldMinecartBehavior`): they follow
//! the rails (slopes pull, powered rails brake and accelerate, curves turn the motion, the
//! speed is capped at 0.4, 0.2 in water), leave them with the air's drag and a bounce of
//! nothing, take hits like boats (over 40 they drop their item), carry one rider on a
//! plain minecart and push or take in what they run into.
//!
//! The cargo carts: a chest minecart holds 27 slots and a hopper minecart 5 (opened as menus,
//! dropped when broken, the hopper pulling item entities and the container above and
//! switched off by a powered activator rail), a furnace minecart burns coal and charcoal
//! to push itself along, a TNT minecart is primed by an activator rail, fire, a hard landing
//! or an explosion and blows up with its speed. Spawners and command blocks are carts
//! without their extras here.

use crate::entity::{Entity, EntityKind, MoverType};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3, floor};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub mod cargo;
pub use cargo::Contents;

#[cfg(test)]
mod tests;

/// `MinecartFurnace`: what a piece of fuel is worth, and how much a cart can hold.
const FUEL_TICKS_PER_ITEM: i32 = 3600;
const MAX_FUEL_TICKS: i32 = 32000;
/// `MinecartTNT`: the fuse a primed cart burns, and the explosion's default power.
const FUSE_TICKS: i32 = 80;
const DEFAULT_EXPLOSION_POWER: f32 = 4.0;

/// `RailShape`, by its `shape` property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailShape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}

impl RailShape {
    fn of(name: &str) -> Option<RailShape> {
        Some(match name {
            "north_south" => RailShape::NorthSouth,
            "east_west" => RailShape::EastWest,
            "ascending_east" => RailShape::AscendingEast,
            "ascending_west" => RailShape::AscendingWest,
            "ascending_north" => RailShape::AscendingNorth,
            "ascending_south" => RailShape::AscendingSouth,
            "south_east" => RailShape::SouthEast,
            "south_west" => RailShape::SouthWest,
            "north_west" => RailShape::NorthWest,
            "north_east" => RailShape::NorthEast,
            _ => return None,
        })
    }

    pub fn is_slope(self) -> bool {
        matches!(self, RailShape::AscendingEast | RailShape::AscendingWest | RailShape::AscendingNorth | RailShape::AscendingSouth)
    }

    /// `AbstractMinecart.exits`: the two ends of the track piece as block offsets.
    fn exits(self) -> ([i32; 3], [i32; 3]) {
        const W: [i32; 3] = [-1, 0, 0];
        const E: [i32; 3] = [1, 0, 0];
        const N: [i32; 3] = [0, 0, -1];
        const S: [i32; 3] = [0, 0, 1];
        let below = |v: [i32; 3]| [v[0], v[1] - 1, v[2]];
        match self {
            RailShape::NorthSouth => (N, S),
            RailShape::EastWest => (W, E),
            RailShape::AscendingEast => (below(W), E),
            RailShape::AscendingWest => (W, below(E)),
            RailShape::AscendingNorth => (N, below(S)),
            RailShape::AscendingSouth => (below(N), S),
            RailShape::SouthEast => (S, E),
            RailShape::SouthWest => (S, W),
            RailShape::NorthWest => (N, W),
            RailShape::NorthEast => (N, E),
        }
    }
}

/// `BaseRailBlock.isRail` (also the `#rails` tag): the rail's shape.
pub fn rail_shape(state: u16) -> Option<RailShape> {
    if !matches!(crate::blocks::block_name(state), "minecraft:rail" | "minecraft:powered_rail" | "minecraft:detector_rail" | "minecraft:activator_rail") {
        return None;
    }
    kiln_data::blocks_types::block_of(state).property(state, "shape").and_then(RailShape::of)
}

fn is_rail(state: u16) -> bool {
    rail_shape(state).is_some()
}

fn powered(state: u16) -> bool {
    kiln_data::blocks_types::block_of(state).property(state, "powered") == Some("true")
}

#[derive(Clone, Debug)]
pub struct Minecart {
    /// A plain minecart (the others take no rider).
    pub rideable: bool,
    pub furnace: bool,
    pub on_rails: bool,
    pub flipped: bool,
    pub hurt_time: i32,
    hurt_dir: i32,
    pub damage: f32,
    /// A chest or hopper minecart's slots.
    pub contents: Option<Contents>,
    /// `MinecartHopper.enabled` (an activator rail with power switches it off) and
    /// `consumedItemThisFrame`.
    pub enabled: bool,
    consumed_this_frame: bool,
    /// `MinecartFurnace.fuel` (ticks left) and `push` (the horizontal direction it drives).
    pub fuel: i32,
    pub push: Vec3,
    /// A TNT minecart: `fuse` (-1: not primed), who lit it (`ignitionSource`: the entity of the
    /// damage source, when there was one), `explosionPowerBase` and `explosionSpeedFactor`.
    pub tnt: bool,
    pub fuse: i32,
    pub ignition: Option<Option<i32>>,
    pub explosion_power: f32,
    pub explosion_speed_factor: f32,
}

/// Whether `name` is a minecart type.
pub fn is_minecart(name: &str) -> bool {
    name.ends_with("_minecart") || name == "minecraft:minecart"
}

impl Minecart {
    fn of(name: &str) -> Minecart {
        let contents = match name {
            "minecraft:chest_minecart" => Some(Contents::new(27)),
            "minecraft:hopper_minecart" => Some(Contents::new(5)),
            _ => None,
        };
        Minecart {
            rideable: name == "minecraft:minecart",
            furnace: name == "minecraft:furnace_minecart",
            on_rails: false,
            flipped: false,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            contents,
            enabled: true,
            consumed_this_frame: false,
            fuel: 0,
            push: Vec3::ZERO,
            tnt: name == "minecraft:tnt_minecart",
            fuse: -1,
            ignition: None,
            explosion_power: DEFAULT_EXPLOSION_POWER,
            explosion_speed_factor: 1.0,
        }
    }

    /// A chest or hopper minecart.
    pub fn is_container(&self) -> bool {
        self.contents.is_some()
    }

    pub fn hopper(&self, e: &Entity) -> bool {
        e.type_name == "minecraft:hopper_minecart"
    }

    /// `MinecartTNT.isPrimed`.
    pub fn is_primed(&self) -> bool {
        self.fuse > -1
    }
}

/// A new minecart of `type_name` at `pos` (`AbstractMinecart.createMinecart`).
pub fn new(type_name: &'static str, pos: Vec3, seed: i64) -> Entity {
    let mut e = Entity::new(type_name, 0, 0, EntityKind::Ext(Box::new(Minecart::of(type_name))), seed);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

/// Reads a saved one.
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let mut m = Minecart::of(type_name);
    m.flipped = r.bool_or("FlippedRotation", false);
    // `readChestVehicleSaveData` (chests and hoppers), `Enabled`, `PushX`, `PushZ`, `Fuel`,
    // and the TNT cart's `fuse`, `explosion_power` and `explosion_speed_factor`.
    if let Some(c) = &m.contents {
        m.contents = Some(Contents::load(r, c.items.len()));
    }
    if type_name == "minecraft:hopper_minecart" {
        m.enabled = r.bool_or("Enabled", true);
    }
    if m.furnace {
        m.push = Vec3::new(r.num("PushX").unwrap_or(0.0), 0.0, r.num("PushZ").unwrap_or(0.0));
        m.fuel = r.short_or("Fuel", 0);
    }
    if m.tnt {
        m.fuse = r.int_or("fuse", -1);
        m.explosion_power = r.float_or("explosion_power", DEFAULT_EXPLOSION_POWER).clamp(0.0, 128.0);
        m.explosion_speed_factor = r.float_or("explosion_speed_factor", 1.0).clamp(0.0, 128.0);
    }
    Some(Box::new(m))
}

/// `AbstractMinecart.getPassengerAttachmentPoint` (villagers sit lower).
fn seat() -> Vec3 {
    Vec3::new(0.0, 0.1875, 0.0)
}

impl Minecart {
    /// `getMaxSpeed`: 0.4 (0.2 in water); a furnace minecart is half as fast (three quarters
    /// of the water's speed).
    fn max_speed(&self, e: &Entity) -> f64 {
        let base = if e.is_in_water() { 0.2 } else { 0.4 };
        match (self.furnace, e.is_in_water()) {
            (true, true) => base * 0.75,
            (true, false) => base * 0.5,
            _ => base,
        }
    }
}

/// `OldMinecartBehavior.getPos`: the point of the track under `(x, y, z)`, with the
/// height of its slope, or `None` off the rails.
fn track_pos(level: &dyn EntityLevel, x: f64, y: f64, z: f64) -> Option<Vec3> {
    let (ix, mut iy, iz) = (floor(x), floor(y), floor(z));
    if is_rail(level.block(BlockPos::new(ix, iy - 1, iz))) {
        iy -= 1;
    }
    let shape = rail_shape(level.block(BlockPos::new(ix, iy, iz)))?;
    let (e0, e1) = shape.exits();
    let xa = ix as f64 + 0.5 + e0[0] as f64 * 0.5;
    let ya = iy as f64 + 0.0625 + e0[1] as f64 * 0.5;
    let za = iz as f64 + 0.5 + e0[2] as f64 * 0.5;
    let xb = ix as f64 + 0.5 + e1[0] as f64 * 0.5;
    let yb = iy as f64 + 0.0625 + e1[1] as f64 * 0.5;
    let zb = iz as f64 + 0.5 + e1[2] as f64 * 0.5;
    let xd = xb - xa;
    let yd = (yb - ya) * 2.0;
    let zd = zb - za;
    let progress = if xd == 0.0 {
        z - iz as f64
    } else if zd == 0.0 {
        x - ix as f64
    } else {
        ((x - xa) * xd + (z - za) * zd) * 2.0
    };
    let (nx, mut ny, nz) = (xa + xd * progress, ya + yd * progress, za + zd * progress);
    if yd < 0.0 {
        ny += 1.0;
    } else if yd > 0.0 {
        ny += 0.5;
    }
    Some(Vec3::new(nx, ny, nz))
}

fn conductor(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    kiln_data::block_logic::is_redstone_conductor(level.block(pos))
}

impl Minecart {
    /// `getCurrentBlockPosOrRailBelow`.
    fn block_or_rail_below(e: &Entity, level: &dyn EntityLevel) -> BlockPos {
        let (x, mut y, z) = (floor(e.x()), floor(e.y()), floor(e.z()));
        if is_rail(level.block(BlockPos::new(x, y - 1, z))) {
            y -= 1;
        }
        BlockPos::new(x, y, z)
    }

    /// `applyNaturalSlowdown`.
    fn slowdown(&mut self, e: &Entity, v: Vec3) -> Vec3 {
        // `AbstractMinecartContainer`: 0.98, less the fuller it is, unless its loot table has
        // not been rolled.
        if let Some(c) = &self.contents {
            let mut f = 0.98f32;
            if c.loot_table.is_none() {
                f += (15 - c.signal()) as f32 * 0.001;
            }
            if e.is_in_water() {
                f *= 0.95;
            }
            return v.multiply(f as f64, 0.0, f as f64);
        }
        // `MinecartFurnace`: the push drives it along, then the base slowdown.
        let v = if self.furnace {
            if self.push.length_sqr() > 1.0E-7 {
                self.push = self.new_push_along(v);
                let d = v.multiply(0.8, 0.0, 0.8) + self.push;
                if e.is_in_water() { d.scale(0.1) } else { d }
            } else {
                v.multiply(0.98, 0.0, 0.98)
            }
        } else {
            v
        };
        let f = if e.passengers.is_empty() { 0.96 } else { 0.997 };
        let v = v.multiply(f, 0.0, f);
        if e.is_in_water() { v.scale(0.949999988079071) } else { v }
    }

    /// `MinecartFurnace.calculateNewPushAlong`: the push turns to follow the motion.
    fn new_push_along(&self, motion: Vec3) -> Vec3 {
        if self.push.horizontal_distance_sqr() > 1.0E-4 && motion.horizontal_distance_sqr() > 0.001 {
            self.push.projected_on(motion).normalize().scale(self.push.length())
        } else {
            self.push
        }
    }

    /// `OldMinecartBehavior.moveAlongTrack`.
    fn move_along_track(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let pos = Self::block_or_rail_below(e, level);
        let state = level.block(pos);
        e.fall_distance = 0.0;
        let (x, y0, z) = (e.x(), e.y(), e.z());
        let pos_offs = track_pos(level, x, y0, z);
        let mut y = pos.y as f64;
        let mut is_powered = false;
        let mut brake = false;
        if crate::blocks::block_name(state) == "minecraft:powered_rail" {
            is_powered = powered(state);
            brake = !is_powered;
        }
        let mut slope = 0.0078125;
        if e.is_in_water() {
            slope *= 0.2;
        }
        let shape = rail_shape(state).unwrap_or(RailShape::NorthSouth);
        match shape {
            RailShape::AscendingEast => {
                e.delta = e.delta.add(-slope, 0.0, 0.0);
                y += 1.0;
            }
            RailShape::AscendingWest => {
                e.delta = e.delta.add(slope, 0.0, 0.0);
                y += 1.0;
            }
            RailShape::AscendingNorth => {
                e.delta = e.delta.add(0.0, 0.0, slope);
                y += 1.0;
            }
            RailShape::AscendingSouth => {
                e.delta = e.delta.add(0.0, 0.0, -slope);
                y += 1.0;
            }
            _ => {}
        }
        let mut movement = e.delta;
        let (e0, e1) = shape.exits();
        let mut dx = (e1[0] - e0[0]) as f64;
        let mut dz = (e1[2] - e0[2]) as f64;
        let dist = (dx * dx + dz * dz).sqrt();
        if movement.x * dx + movement.z * dz < 0.0 {
            dx = -dx;
            dz = -dz;
        }
        let pow = (2.0f64).min(movement.horizontal_distance());
        movement = Vec3::new(pow * dx / dist, movement.y, pow * dz / dist);
        e.delta = movement;
        // (The rider's steering nudge, `getLastClientMoveIntent`, is not modelled.)
        if brake {
            let speed = e.delta.horizontal_distance();
            if speed < 0.03 {
                e.delta = Vec3::ZERO;
            } else {
                e.delta = e.delta.multiply(0.5, 0.0, 0.5);
            }
        }
        let xx = pos.x as f64 + 0.5 + e0[0] as f64 * 0.5;
        let zz = pos.z as f64 + 0.5 + e0[2] as f64 * 0.5;
        let x2 = pos.x as f64 + 0.5 + e1[0] as f64 * 0.5;
        let z2 = pos.z as f64 + 0.5 + e1[2] as f64 * 0.5;
        let dx = x2 - xx;
        let dz = z2 - zz;
        let progress = if dx == 0.0 {
            e.z() - pos.z as f64
        } else if dz == 0.0 {
            e.x() - pos.x as f64
        } else {
            ((e.x() - xx) * dx + (e.z() - zz) * dz) * 2.0
        };
        e.set_pos(Vec3::new(xx + dx * progress, y, zz + dz * progress));
        let xdd = if e.passengers.is_empty() { 1.0 } else { 0.75 };
        let max = self.max_speed(e);
        let m = e.delta;
        let step = Vec3::new((xdd * m.x).clamp(-max, max), 0.0, (xdd * m.z).clamp(-max, max));
        e.do_move(level, MoverType::SelfMove, step);
        self.settle_fall(e, level);
        e.apply_effects_from_blocks(level);
        if e0[1] != 0 && floor(e.x()) - pos.x == e0[0] && floor(e.z()) - pos.z == e0[2] {
            e.set_pos(Vec3::new(e.x(), e.y() + e0[1] as f64, e.z()));
        } else if e1[1] != 0 && floor(e.x()) - pos.x == e1[0] && floor(e.z()) - pos.z == e1[2] {
            e.set_pos(Vec3::new(e.x(), e.y() + e1[1] as f64, e.z()));
        }
        e.delta = self.slowdown(e, e.delta);
        let new_pos = track_pos(level, e.x(), e.y(), e.z());
        if let (Some(new_pos), Some(old)) = (new_pos, pos_offs) {
            let adjust = (old.y - new_pos.y) * 0.05;
            let m = e.delta;
            let speed = m.horizontal_distance();
            if speed > 0.0 {
                e.delta = m.multiply((speed + adjust) / speed, 1.0, (speed + adjust) / speed);
            }
            e.set_pos(Vec3::new(e.x(), new_pos.y, e.z()));
        }
        let (fx, fz) = (floor(e.x()), floor(e.z()));
        if fx != pos.x || fz != pos.z {
            let m = e.delta;
            let speed = m.horizontal_distance();
            e.delta = Vec3::new(speed * (fx - pos.x) as f64, m.y, speed * (fz - pos.z) as f64);
        }
        if is_powered {
            let m = e.delta;
            let speed = m.horizontal_distance();
            if speed > 0.01 {
                let accel = 0.06;
                e.delta = m.add(m.x / speed * accel, 0.0, m.z / speed * accel);
            } else {
                let (mut ddx, mut ddz) = (m.x, m.z);
                match shape {
                    RailShape::EastWest => {
                        if conductor(level, pos.offset(-1, 0, 0)) {
                            ddx = 0.02;
                        } else if conductor(level, pos.offset(1, 0, 0)) {
                            ddx = -0.02;
                        }
                    }
                    RailShape::NorthSouth => {
                        if conductor(level, pos.offset(0, 0, -1)) {
                            ddz = 0.02;
                        } else if conductor(level, pos.offset(0, 0, 1)) {
                            ddz = -0.02;
                        }
                    }
                    _ => return,
                }
                e.delta = Vec3::new(ddx, m.y, ddz);
            }
        }
    }

    /// `comeOffTrack`: the air's drag, half the speed on the ground.
    fn come_off_track(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let max = self.max_speed(e);
        let m = e.delta;
        e.delta = Vec3::new(m.x.clamp(-max, max), m.y, m.z.clamp(-max, max));
        if e.on_ground {
            e.delta = e.delta.scale(0.5);
        }
        let movement = e.delta;
        e.do_move(level, MoverType::SelfMove, movement);
        self.settle_fall(e, level);
        e.apply_effects_from_blocks(level);
        if !e.on_ground {
            e.delta = e.delta.scale(0.949999988079071);
        }
    }

    /// `pushAndPickupEntities`: riders board a moving minecart, things are pushed, carts
    /// shove one another.
    fn push_and_pickup(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let area = e.bounding_box().inflate(0.20000000298023224, 0.0, 0.20000000298023224);
        if self.rideable && e.delta.horizontal_distance_sqr() >= 0.01 {
            let living = level.entities_in(&area, EntityFilter::Living, e.id);
            for id in level.entities_in(&area, EntityFilter::Any, e.id) {
                let Some(other) = level.entity(id) else { continue };
                if other.is_removed() {
                    continue;
                }
                // A pushable minecart in the way is pushed like any other (`entity.push(minecart)`).
                if let Some(other_furnace) = crate::ext_entity::get::<Minecart>(other).map(|m| m.furnace) {
                    if let Some(other) = level.entity_mut(id) {
                        push_other_minecart(other, other_furnace, e, self.furnace);
                    }
                    continue;
                }
                if !matches!(other.kind, EntityKind::Mob(_)) || !living.contains(&id) {
                    continue;
                }
                let type_name = other.type_name;
                let is_golem = type_name == "minecraft:iron_golem";
                if !is_golem && e.passengers.is_empty() && other.vehicle.is_none() {
                    if let Some(rider) = level.entity_mut(id) {
                        crate::ride::start_riding(rider, e, false);
                    }
                } else {
                    push_apart(e, level, id);
                }
            }
        } else {
            for id in level.entities_in(&area, EntityFilter::Any, e.id) {
                if e.passengers.contains(&id) {
                    continue;
                }
                let Some(other) = level.entity_mut(id) else { continue };
                let Some(other_furnace) = crate::ext_entity::get::<Minecart>(other).map(|m| m.furnace) else { continue };
                push_other_minecart(other, other_furnace, e, self.furnace);
            }
        }
    }
}

/// `Entity.push(entity)` from a mobile thing bumping into the cart: both go apart a little.
fn push_apart(e: &mut Entity, level: &mut dyn EntityLevel, id: i32) {
    let Some(other) = level.entity(id) else { return };
    let (mut dx, mut dz) = (e.x() - other.x(), e.z() - other.z());
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
    // The other pushes `e`: `e` moves by `dx`, the other by `-dx`.
    e.delta = e.delta.add(dx, 0.0, dz);
    e.needs_sync = true;
    level.push(id, Vec3::new(-dx, 0.0, -dz));
}

/// `AbstractMinecart.push(entity)` with `entity` a minecart, run for `this` (the cart found
/// nearby) with `that` the cart being ticked.
fn push_other_minecart(this: &mut Entity, this_furnace: bool, that: &mut Entity, that_furnace: bool) {
    if this.no_physics || that.no_physics {
        return;
    }
    let (mut xd, mut zd) = (that.x() - this.x(), that.z() - this.z());
    let mut dd = xd * xd + zd * zd;
    if dd < 9.999999747378752E-5 {
        return;
    }
    dd = dd.sqrt();
    xd /= dd;
    zd /= dd;
    let pow = (1.0 / dd).min(1.0);
    xd *= pow * 0.10000000149011612 * 0.5;
    zd *= pow * 0.10000000149011612 * 0.5;
    // The carts must be lined up (`pushOtherMinecart`).
    let rad = this.y_rot * 0.017453292;
    let facing = Vec3::new(crate::mob::mth::cos(rad as f64) as f64, 0.0, crate::mob::mth::sin(rad as f64) as f64).normalize();
    let dir = Vec3::new(that.x() - this.x(), 0.0, that.z() - this.z()).normalize();
    let dot = (dir.x * facing.x + dir.y * facing.y + dir.z * facing.z).abs();
    if dot < 0.800000011920929 {
        return;
    }
    let movement = this.delta;
    let other_movement = that.delta;
    if that_furnace && !this_furnace {
        this.delta = movement.multiply(0.2, 1.0, 0.2);
        this.delta = this.delta.add(other_movement.x - xd, 0.0, other_movement.z - zd);
        that.delta = other_movement.multiply(0.95, 1.0, 0.95);
    } else if !that_furnace && this_furnace {
        that.delta = other_movement.multiply(0.2, 1.0, 0.2);
        that.delta = that.delta.add(movement.x + xd, 0.0, movement.z + zd);
        this.delta = movement.multiply(0.95, 1.0, 0.95);
    } else {
        let (ax, az) = ((other_movement.x + movement.x) / 2.0, (other_movement.z + movement.z) / 2.0);
        this.delta = movement.multiply(0.2, 1.0, 0.2).add(ax - xd, 0.0, az - zd);
        that.delta = other_movement.multiply(0.2, 1.0, 0.2).add(ax + xd, 0.0, az + zd);
    }
    this.needs_sync = true;
    that.needs_sync = true;
}

impl EntityExt for Minecart {
    crate::entity_ext_boilerplate!();

    /// `AbstractMinecart.tick` with the old behaviour.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `MinecartHopper.tick`.
        self.consumed_this_frame = false;
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if self.damage > 0.0 {
            self.damage -= 1.0;
        }
        if e.y() < (level.min_y() - 64) as f64 {
            self.remove(e, level, true);
            return;
        }
        e.compute_speed();
        // `OldMinecartBehavior.tick`: gravity, then the rails or the air.
        if !e.no_gravity {
            let g = if e.is_in_water() { 0.005 } else { 0.04 };
            e.delta = e.delta.add(0.0, -g, 0.0);
        }
        let pos = Self::block_or_rail_below(e, level);
        let state = level.block(pos);
        let on_rails = is_rail(state);
        self.on_rails = on_rails;
        if on_rails {
            self.move_along_track(e, level);
            if crate::blocks::block_name(state) == "minecraft:activator_rail" {
                self.activate(e, level, powered(state));
            }
        } else {
            self.come_off_track(e, level);
        }
        e.apply_effects_from_blocks(level);
        e.x_rot = 0.0;
        let (dx, dz) = (e.old_pos.x - e.x(), e.old_pos.z - e.z());
        if dx * dx + dz * dz > 0.001 {
            e.y_rot = (crate::mob::mth::atan2(dz, dx) * 180.0 / std::f64::consts::PI) as f32;
            if self.flipped {
                e.y_rot += 180.0;
            }
        }
        let diff = crate::mob::mth::wrap_degrees(e.y_rot - e.y_rot_o) as f64;
        if !(-170.0..170.0).contains(&diff) {
            e.y_rot += 180.0;
            self.flipped = !self.flipped;
        }
        e.x_rot %= 360.0;
        e.y_rot %= 360.0;
        self.push_and_pickup(e, level);
        e.update_fluid_interaction(level);
        e.first_tick = false;
        // What the block effects did to the cart (lava, fire) while its state was out.
        self.take_pending_hurts(e, level);
        self.subclass_tick(e, level);
    }

    /// `VehicleEntity.hurtServer` (`MinecartTNT.hurtServer` on top).
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
        if (!creative && self.damage > 40.0) || self.should_source_destroy(kind) {
            self.destroy(e, level, kind, attacker);
        } else if creative {
            self.remove(e, level, true);
        }
        true
    }

    /// `Minecart.interact`: a click gets the player aboard unless sneaking or taken. The
    /// container minecarts open their menu, the furnace minecart takes fuel.
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Option<Outcome> {
        if self.contents.is_some() {
            // `interactWithContainerVehicle`: the menu opens (the loot table rolls with the
            // player's luck first).
            if let Some(c) = &mut self.contents {
                c.unpack(level, e.position(), Some(who.id));
            }
            let mut out = Outcome::success(HeldChange::None);
            out.open_container = true;
            if e.type_name == "minecraft:chest_minecart" {
                level.emit(Event::GameEvent { event: "minecraft:container_open", pos: e.position(), entity: Some(who.id) });
            }
            return Some(out);
        }
        if self.furnace {
            return Some(self.interact_furnace(e, level, who, stack));
        }
        if !self.rideable || who.sneaking || !e.passengers.is_empty() {
            return Some(Outcome::PASS);
        }
        let mut out = Outcome::success(HeldChange::None);
        out.ride = true;
        Some(out)
    }

    fn attackable(&self) -> bool {
        true
    }

    fn passenger_offset(&self, _e: &Entity, _index: usize, _animal: bool) -> Option<Vec3> {
        Some(seat())
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
        // `MinecartFurnace.DATA_ID_FUEL`: the client lights the furnace and smokes.
        if self.furnace {
            d.set(data::minecart_furnace::ID_FUEL, &DataValue::Boolean(self.fuel > 0));
        }
    }

    fn save(&self, e: &Entity, o: &mut Output) {
        o.put("FlippedRotation", Tag::Byte(self.flipped as i8));
        if let Some(c) = &self.contents {
            c.save(o);
        }
        if e.type_name == "minecraft:hopper_minecart" {
            o.put("Enabled", Tag::Byte(self.enabled as i8));
        }
        if self.furnace {
            o.put("PushX", Tag::Double(self.push.x));
            o.put("PushZ", Tag::Double(self.push.z));
            o.put("Fuel", Tag::Short(self.fuel as i16));
        }
        if self.tnt {
            o.put("fuse", Tag::Int(self.fuse));
            if self.explosion_power != DEFAULT_EXPLOSION_POWER {
                o.put("explosion_power", Tag::Float(self.explosion_power));
            }
            if self.explosion_speed_factor != 1.0 {
                o.put("explosion_speed_factor", Tag::Float(self.explosion_speed_factor));
            }
        }
    }
}

impl Minecart {
    /// `activateMinecart`: a powered activator rail throws a minecart's riders off and shakes
    /// it; it switches a hopper minecart off and primes a TNT minecart.
    fn activate(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, activated: bool) {
        if e.type_name == "minecraft:hopper_minecart" {
            // `MinecartHopper.activateMinecart`: enabled unless the rail is powered.
            self.enabled = !activated;
            return;
        }
        if self.tnt {
            if activated && self.fuse < 0 {
                self.prime_fuse(e, level, None);
            }
            return;
        }
        if !self.rideable || !activated {
            return;
        }
        if !e.passengers.is_empty() {
            crate::ride::eject(e, level);
        }
        if self.hurt_time == 0 {
            self.hurt_dir = -self.hurt_dir;
            self.hurt_time = 10;
            self.damage = 50.0;
            e.needs_sync = true;
        }
    }

    /// The damage the cart took from block effects while its own tick held its state.
    fn take_pending_hurts(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        for (kind, amount, attacker) in std::mem::take(&mut e.pending_hurts) {
            self.hurt(e, level, kind, amount, attacker);
        }
    }

    /// A hard landing (`MinecartTNT.causeFallDamage`), noticed right after the move.
    fn settle_fall(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if let Some((distance, _)) = e.pending_fall.take()
            && self.tnt
            && distance >= 3.0
        {
            let d = distance / 10.0;
            self.explode(e, level, d * d);
        }
    }

    /// What the subclasses add to `tick` after the base tick.
    fn subclass_tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if self.furnace {
            // `MinecartFurnace.tick`: the fuel burns down and the push stops with it.
            if self.fuel > 0 {
                self.fuel -= 1;
            }
            if self.fuel <= 0 {
                self.push = Vec3::ZERO;
            }
            // The smoke is the client's; the server still draws its random.
            if self.fuel > 0 {
                e.random.next_int_bounded(4);
            }
        }
        if self.tnt {
            // `MinecartTNT.tick`: the fuse burns, then it goes off; a crash at speed sets it off.
            if self.fuse > 0 {
                self.fuse -= 1;
            } else if self.fuse == 0 {
                let speed = e.delta.horizontal_distance_sqr();
                self.explode(e, level, speed);
            }
            if e.horizontal_collision {
                let speed = e.delta.horizontal_distance_sqr();
                if speed >= 0.009999999776482582 {
                    self.explode(e, level, speed);
                }
            }
        }
        if e.type_name == "minecraft:hopper_minecart" {
            self.try_consume_items(e, level);
        }
    }

    /// `MinecartFurnace.interact`: fuel from the hand keeps it going and points its push away
    /// from the player.
    fn interact_furnace(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Outcome {
        let mut out = Outcome::success(HeldChange::None);
        let is_fuel = !stack.is_empty() && {
            use kiln_inventory::stack::StackExt;
            kiln_inventory::tags::contains("minecraft:item", "minecraft:furnace_minecart_fuel", stack.effective_item())
        };
        if is_fuel && self.fuel + FUEL_TICKS_PER_ITEM <= MAX_FUEL_TICKS {
            self.fuel += FUEL_TICKS_PER_ITEM;
            if self.fuel > 0 {
                // `position().subtract(player.position()).horizontal()`.
                let from = level.player(who.id).map_or(e.position(), |p| p.pos);
                self.push = (e.position() - from).horizontal();
            }
            out.held = HeldChange::Consume(1);
            e.needs_sync = true;
        }
        out
    }

    /// `VehicleEntity.shouldSourceDestroy` (`MinecartTNT`: fire and explosions).
    fn should_source_destroy(&self, kind: DamageKind) -> bool {
        self.tnt && damage_ignites_tnt(kind)
    }

    /// `remove`: the cart goes; a container minecart drops what it holds first
    /// (`shouldDestroy`: killed or discarded). Riders get off.
    fn remove(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, discarded: bool) {
        crate::ride::eject(e, level);
        self.drop_contents(e, level);
        if discarded {
            e.discard();
        } else {
            e.removed.get_or_insert(crate::entity::RemovalReason::Killed);
        }
    }

    /// `VehicleEntity.destroy(level, item)`: the cart is killed and, with entity drops on,
    /// leaves its item (named as the cart was).
    fn destroy_item(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.remove(e, level, false);
        if !level.entity_drops() {
            return;
        }
        let Some(mut stack) = kiln_item::ItemStack::of(e.type_name, 1) else { return };
        if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
            stack.set(kiln_item::component::Component::CustomName(name));
        }
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut item = crate::item::new_at(id, 0, stack, e.position(), seed);
        if let EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
        }
        level.add_entity(item);
    }

    /// `destroy(level, source)`: a plain cart leaves its item; a TNT minecart goes off when
    /// fire, an explosion or speed says so; a container minecart also drops its contents.
    fn destroy(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, attacker: Option<i32>) {
        if self.tnt {
            let speed = e.delta.horizontal_distance_sqr();
            if damage_ignites_tnt(kind) || speed >= 0.009999999776482582 {
                if self.fuse < 0 {
                    self.prime_fuse(e, level, Some(attacker));
                    self.fuse = e.random.next_int_bounded(20) + e.random.next_int_bounded(20);
                }
            } else {
                self.destroy_item(e, level);
            }
            return;
        }
        self.destroy_item(e, level);
        if self.contents.is_some() && level.entity_drops() {
            // `chestVehicleDestroyed`: the contents drop again (they already have).
            self.drop_contents(e, level);
        }
    }

    /// `MinecartTNT.primeFuse`: 80 ticks to go; the source's entity is remembered as who lit it.
    /// `source`: `Some(attacker)` for a damage source (with or without an entity).
    fn prime_fuse(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, source: Option<Option<i32>>) {
        if !level.tnt_explodes() {
            return;
        }
        self.fuse = FUSE_TICKS;
        if let Some(attacker) = source
            && self.ignition.is_none()
        {
            self.ignition = Some(attacker);
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 70 });
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.tnt.primed", source: "blocks", volume: 1.0, pitch: 1.0 });
        }
    }

    /// `MinecartTNT.explode`: the blast grows with the speed, up to 5 blocks per tick's worth.
    fn explode(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, speed_sqr: f64) {
        if !level.tnt_explodes() {
            if self.is_primed() {
                e.discard();
            }
            return;
        }
        let speed = speed_sqr.sqrt().min(5.0);
        let radius = (self.explosion_power as f64 + self.explosion_speed_factor as f64 * e.random.next_double() * 1.5 * speed) as f32;
        let centre = e.position();
        // A primed cart's blast spares rails and what they lie on (`getBlockExplosionResistance`
        // and `shouldBlockExplode`).
        let primed = self.is_primed();
        let is_rail_state = |s: u16| crate::ext_entity::minecart::rail_shape(s).is_some();
        let resistance = |state: u16, above: u16, res: f32| if primed && (is_rail_state(state) || is_rail_state(above)) { 0.0 } else { res };
        let should = |state: u16, above: u16| !(primed && (is_rail_state(state) || is_rail_state(above)));
        let rules = crate::explosion::BlockRules { resistance: Some(&resistance), should_explode: Some(&should) };
        crate::explosion::explode_ruled(level, Some(e.id), centre, radius, false, crate::explosion::Interaction::Tnt, rules, true);
        e.discard();
    }
}

/// `MinecartTNT.damageSourceIgnitesTnt` for a damage type: fire and explosions (a burning
/// projectile's own hit is the arrow's).
fn damage_ignites_tnt(kind: DamageKind) -> bool {
    matches!(
        kind,
        DamageKind::OnFire
            | DamageKind::InFire
            | DamageKind::Lava
            | DamageKind::HotFloor
            | DamageKind::Fireball
            | DamageKind::Explosion
            | DamageKind::PlayerExplosion
            | DamageKind::Fireworks
    )
}
