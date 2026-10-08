//! Paintings (`Painting`): a picture of one of the variants (1x1 to 4x4 blocks) on a wall. A new
//! one takes the largest variant that fits the wall (`Painting.create`: of those, one at random);
//! a hit breaks it and it drops itself; the wall going takes it off.

use super::hanging::{self, center_relative, data_value, of_size};
use super::painting_variants::VARIANTS;
use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, Axis, BlockPos, Direction, Vec3};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const PAINTING: &str = "minecraft:painting";

#[derive(Clone, Debug)]
pub struct Painting {
    pub direction: Direction,
    pub pos: BlockPos,
    /// Index into [`VARIANTS`].
    pub variant: usize,
    since_check: i32,
}

/// The variant named `name`.
pub fn variant_index(name: &str) -> Option<usize> {
    VARIANTS.iter().position(|v| v.0 == name)
}

/// The painting variant a new painting starts with (`VariantUtils.getAny`: the first one).
const DEFAULT_VARIANT: usize = 0;

/// `new Painting(level, pos)` + `setDirection`: a painting with the default variant, not yet
/// placed on the wall.
pub fn bare(id: i32, pos: BlockPos, direction: Direction, seed: i64) -> Entity {
    let p = Painting { direction, pos, variant: DEFAULT_VARIANT, since_check: 0 };
    let mut e = Entity::new(PAINTING, id, 0, EntityKind::Other { type_name: PAINTING }, seed);
    p.place(&mut e);
    e.kind = EntityKind::Ext(Box::new(p));
    e.set_old_pos_and_rot();
    e
}

/// `Painting.create(level, pos, direction)`: of the placeable variants that survive on this wall,
/// the ones of the largest area; one of those at random (the painting's own random). `None`:
/// nothing fits.
pub fn create(level: &dyn EntityLevel, id: i32, pos: BlockPos, direction: Direction, seed: i64) -> Option<Entity> {
    let mut e = bare(id, pos, direction, seed);
    let mut fits: Vec<usize> = Vec::new();
    for (i, v) in VARIANTS.iter().enumerate() {
        if !v.3 {
            continue;
        }
        let Some(p) = crate::ext_entity::get_mut::<Painting>(&mut e) else { return None };
        p.variant = i;
        let p = p.clone();
        p.place(&mut e);
        if p.survives(&e, level) {
            fits.push(i);
        }
    }
    let max = fits.iter().map(|&i| VARIANTS[i].1 * VARIANTS[i].2).max()?;
    fits.retain(|&i| VARIANTS[i].1 * VARIANTS[i].2 >= max);
    let pick = fits[e.random.next_int_bounded(fits.len() as i32) as usize];
    let p = crate::ext_entity::get_mut::<Painting>(&mut e)?;
    p.variant = pick;
    let p = p.clone();
    p.place(&mut e);
    Some(e)
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let direction = match r.get("facing").and_then(Tag::as_i64) {
        Some(0) | None => Direction::South,
        Some(1) => Direction::West,
        Some(2) => Direction::North,
        Some(3) => Direction::East,
        Some(_) => Direction::South,
    };
    let variant = r.get("variant").and_then(Tag::as_str).and_then(|n| variant_index(&normalize(n))).unwrap_or(DEFAULT_VARIANT);
    Some(Box::new(Painting { direction, pos: BlockPos::new(0, 0, 0), variant, since_check: 0 }))
}

fn normalize(name: &str) -> String {
    if name.contains(':') { name.to_owned() } else { format!("minecraft:{name}") }
}

/// After the entity's position is read: `block_pos` (or the block it stands in) and the box.
pub fn after_load(e: &mut Entity, block_pos: Option<&Tag>) {
    let at = e.position();
    let pos = hanging::saved_pos(block_pos, at);
    let Some(p) = crate::ext_entity::get_mut::<Painting>(e) else { return };
    p.pos = pos;
    let p = p.clone();
    p.place(e);
}

impl Painting {
    pub fn size(&self) -> (i32, i32) {
        (VARIANTS[self.variant].1, VARIANTS[self.variant].2)
    }

    /// `calculateBoundingBox`.
    fn create_box(&self) -> Aabb {
        let (w, h) = self.size();
        let center = center_relative(self.pos, self.direction, -0.46875);
        let offset = |n: i32| if n % 2 == 0 { 0.5 } else { 0.0 };
        // `Direction.getCounterClockWise`.
        let left = match self.direction {
            Direction::North => Direction::West,
            Direction::West => Direction::South,
            Direction::South => Direction::East,
            Direction::East => Direction::North,
            other => other,
        };
        let (lx, _, lz) = left.step();
        let c = Vec3::new(center.x + lx as f64 * offset(w), center.y + offset(h), center.z + lz as f64 * offset(w));
        let axis = self.direction.axis();
        let sx = if axis == Axis::X { 0.0625 } else { w as f64 };
        let sz = if axis == Axis::Z { 0.0625 } else { w as f64 };
        of_size(c, sx, h as f64, sz)
    }

    fn place(&self, e: &mut Entity) {
        let bb = self.create_box();
        let (yaw, pitch) = hanging::rotations(self.direction);
        e.y_rot = yaw;
        e.x_rot = pitch;
        e.y_rot_o = yaw;
        e.x_rot_o = pitch;
        e.set_pos_raw(bb.center());
        e.set_bounding_box(bb);
    }

    /// `HangingEntity.survives`.
    fn survives(&self, e: &Entity, level: &dyn EntityLevel) -> bool {
        let pop = self.create_box();
        if hanging::has_block_collision(level, e, &pop) {
            return false;
        }
        // `calculateSupportBox`: the box moved half a block into the wall, deflated.
        let (dx, dy, dz) = self.direction.step();
        let s = pop.offset(dx as f64 * -0.5, dy as f64 * -0.5, dz as f64 * -0.5).deflate_all(1.0e-7);
        let (lo, hi) = (BlockPos::containing(s.min_x, s.min_y, s.min_z), BlockPos::containing(s.max_x, s.max_y, s.max_z));
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    if !hanging::is_supporting_block(level.block(BlockPos::new(x, y, z))) {
                        return false;
                    }
                }
            }
        }
        hanging::can_coexist(level, e, self.direction, &pop, false)
    }

    /// `dropItem(level, entity)`.
    fn drop_item(&self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>) {
        if !level.entity_drops() {
            return;
        }
        e.play_sound(level, "minecraft:entity.painting.break", 1.0, 1.0);
        if by.and_then(|a| level.player(a)).is_some_and(|p| p.creative) {
            return;
        }
        let mut stack = ItemStack::of("minecraft:painting", 1).unwrap_or_default();
        if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
            stack.set(kiln_item::component::Component::CustomName(name));
        }
        let (dx, _, dz) = self.direction.step();
        let p = e.position();
        let at = Vec3::new(p.x + (dx as f32 * 0.15f32) as f64, p.y, p.z + (dz as f32 * 0.15f32) as f64);
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut item = crate::item::new_at(id, 0, stack, at, seed);
        if let EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
        }
        level.add_entity(item);
    }

    /// `playPlacementSound`.
    pub fn placement_sound(&self) -> &'static str {
        "minecraft:entity.painting.place"
    }
}

impl EntityExt for Painting {
    crate::entity_ext_boilerplate!();

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.since_check += 1;
        if self.since_check >= 100 {
            self.since_check = 0;
            if !e.is_removed() && !self.survives(e, &*level) {
                e.discard();
                self.drop_item(e, level, None);
            }
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("facing", Tag::Byte(hanging::data_value_2d(self.direction) as i8));
        let b = self.pos;
        o.put("block_pos", Tag::IntArray(vec![b.x, b.y, b.z]));
        o.put("variant", Tag::String(VARIANTS[self.variant].0.to_owned()));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        d.set(kiln_data::entities::data::hanging_entity::DIRECTION, &DataValue::Direction(proto_direction(self.direction)));
        if let Some(id) = kiln_data::synced_id("minecraft:painting_variant", VARIANTS[self.variant].0) {
            d.set(kiln_data::entities::data::painting::PAINTING_VARIANT, &DataValue::PaintingVariant(id as i32));
        }
    }

    fn spawn_data(&self) -> i32 {
        data_value(self.direction)
    }

    fn attackable(&self) -> bool {
        true
    }

    /// `BlockAttachedEntity.hurtServer`.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, _amount: f32, attacker: Option<i32>) -> bool {
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        if !level.mob_griefing() && attacker.and_then(|a| level.entity(a)).is_some_and(|a| crate::mob::data(a).is_some()) {
            return false;
        }
        if !e.is_removed() {
            e.removed = Some(RemovalReason::Killed);
            level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: attacker });
            self.drop_item(e, level, attacker);
        }
        true
    }
}

fn proto_direction(d: Direction) -> kiln_proto::packets::entity::metadata::Direction {
    use kiln_proto::packets::entity::metadata::Direction as P;
    match d {
        Direction::Down => P::Down,
        Direction::Up => P::Up,
        Direction::North => P::North,
        Direction::South => P::South,
        Direction::West => P::West,
        Direction::East => P::East,
    }
}
