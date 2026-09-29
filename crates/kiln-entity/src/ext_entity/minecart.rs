//! Minecarts (`AbstractMinecart` with vanilla's default `OldMinecartBehavior`): they follow
//! the rails (slopes pull, powered rails brake and accelerate, curves turn the motion, the
//! speed is capped at 0.4, 0.2 in water), leave them with the air's drag and a bounce of
//! nothing, take hits like boats (over 40 they drop their item), carry one rider on a
//! plain minecart and push or take in what they run into. Chests, hoppers, furnaces,
//! TNT, spawners and command blocks are carts without their cargo here.

use crate::entity::{Entity, EntityKind, MoverType};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3, floor};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

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
    /// A plain minecart (the others carry cargo Kiln does not model and take no rider).
    pub rideable: bool,
    pub furnace: bool,
    pub on_rails: bool,
    pub flipped: bool,
    pub hurt_time: i32,
    hurt_dir: i32,
    pub damage: f32,
}

/// Whether `name` is a minecart type.
pub fn is_minecart(name: &str) -> bool {
    name.ends_with("_minecart") || name == "minecraft:minecart"
}

impl Minecart {
    fn of(name: &str) -> Minecart {
        Minecart { rideable: name == "minecraft:minecart", furnace: name == "minecraft:furnace_minecart", on_rails: false, flipped: false, hurt_time: 0, hurt_dir: 1, damage: 0.0 }
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
    Some(Box::new(m))
}

/// `AbstractMinecart.getPassengerAttachmentPoint` (villagers sit lower).
fn seat() -> Vec3 {
    Vec3::new(0.0, 0.1875, 0.0)
}

fn max_speed(e: &Entity) -> f64 {
    if e.is_in_water() { 0.2 } else { 0.4 }
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
    fn slowdown(&self, e: &Entity, v: Vec3) -> Vec3 {
        let f = if e.passengers.is_empty() { 0.96 } else { 0.997 };
        let v = v.multiply(f, 0.0, f);
        if e.is_in_water() { v.scale(0.949999988079071) } else { v }
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
        let max = max_speed(e);
        let m = e.delta;
        let step = Vec3::new((xdd * m.x).clamp(-max, max), 0.0, (xdd * m.z).clamp(-max, max));
        e.do_move(level, MoverType::SelfMove, step);
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
        let max = max_speed(e);
        let m = e.delta;
        e.delta = Vec3::new(m.x.clamp(-max, max), m.y, m.z.clamp(-max, max));
        if e.on_ground {
            e.delta = e.delta.scale(0.5);
        }
        let movement = e.delta;
        e.do_move(level, MoverType::SelfMove, movement);
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
            for id in level.entities_in(&area, EntityFilter::Living, e.id) {
                let Some(other) = level.entity(id) else { continue };
                if other.is_removed() || !matches!(other.kind, EntityKind::Mob(_)) {
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
                let Some(mut other_state) = crate::ext_entity::get::<Minecart>(other).cloned() else { continue };
                let (om, e_furnace) = (other_state.furnace, self.furnace);
                push_other_minecart(other, &mut other_state, om, e, self, e_furnace);
                if let Some(s) = crate::ext_entity::get_mut::<Minecart>(other) {
                    *s = other_state;
                }
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
fn push_other_minecart(this: &mut Entity, this_state: &mut Minecart, this_furnace: bool, that: &mut Entity, _that_state: &Minecart, that_furnace: bool) {
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
    let _ = this_state;
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
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if self.damage > 0.0 {
            self.damage -= 1.0;
        }
        if e.y() < (level.min_y() - 64) as f64 {
            e.discard();
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
            // `destroy`: the minecart as an item.
            if let Some(stack) = kiln_item::ItemStack::of(e.type_name, 1) {
                let (id, seed) = (level.next_entity_id(), level.fresh_seed());
                let mut item = crate::item::new_at(id, 0, stack, e.position(), seed);
                if let EntityKind::Item(d) = &mut item.kind {
                    d.pickup_delay = 10;
                }
                level.add_entity(item);
            }
            crate::ride::eject(e, level);
            e.discard();
        } else if creative {
            crate::ride::eject(e, level);
            e.discard();
        }
        true
    }

    /// `Minecart.interact`: a click gets the player aboard unless sneaking or taken.
    fn interact(&mut self, e: &mut Entity, _level: &mut dyn EntityLevel, who: &Interactor) -> Option<Outcome> {
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
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("FlippedRotation", Tag::Byte(self.flipped as i8));
    }
}

impl Minecart {
    /// `Minecart.activateMinecart`: a powered activator rail throws the riders off and shakes
    /// the cart.
    fn activate(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, activated: bool) {
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
}
