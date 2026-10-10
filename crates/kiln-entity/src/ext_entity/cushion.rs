//! Cushions (`Cushion`): a block-attached entity 1 by 0.25 blocks that a player sits on. It is put
//! on the top of a block (`CushionItem.useOn`, in the simulation) at the height of the click, it
//! needs something under it within 1/64 block (`hasAnchorBelow`) and may not be inside solid
//! blocks, and every 100 ticks it checks that and whether it is in fire. A hit (or a push, or
//! lightning) breaks it and drops the cushion item with the cushion's name.

use crate::blocks::{Tag as BlockTag, has_tag};
use crate::collision::{CollisionContext, for_each_block_collision};
use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, Axis, BlockPos, Vec3};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use crate::physics;
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const TYPE: &str = "minecraft:cushion";

/// `DyeColor` names in id order.
pub const COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// The id of the dye color called `name`.
pub fn color_of(name: &str) -> Option<u8> {
    COLORS.iter().position(|c| *c == name).map(|i| i as u8)
}

#[derive(Clone, Debug)]
pub struct Cushion {
    /// `DATA_COLOR` (a `DyeColor` id).
    pub color: u8,
    /// `BlockAttachedEntity.ticksSinceLastCheck`.
    since_check: i32,
}

/// A cushion at `pos` turned to `yaw`.
pub fn new(id: i32, pos: Vec3, yaw: f32, color: u8, seed: i64) -> Entity {
    let mut e = Entity::new(TYPE, id, 0, EntityKind::Ext(Box::new(Cushion { color, since_check: 0 })), seed);
    e.set_pos(pos);
    e.y_rot = yaw;
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    // (`block_pos` is the block the position is in: `Pos` carries the same.)
    let _ = r.get("block_pos");
    let color = match r.get("color") {
        Some(Tag::String(s)) => color_of(s).unwrap_or(0),
        _ => 0,
    };
    Some(Box::new(Cushion { color, since_check: 0 }))
}

/// Whether `e` is a cushion.
pub fn is_cushion(e: &Entity) -> bool {
    e.type_name == TYPE
}

/// The cushion item of `color`.
pub fn item_name(color: u8) -> String {
    format!("minecraft:{}_cushion", COLORS[color as usize & 15])
}

/// `EntityType.getSpawnAABB(pos)` of the cushion type (1 by 0.25).
pub fn spawn_box(pos: Vec3) -> Aabb {
    Aabb::new(pos.x - 0.5, pos.y, pos.z - 0.5, pos.x + 0.5, pos.y + 0.25f32 as f64, pos.z + 0.5)
}

/// `BlockPos.betweenClosed(box)`: the blocks the box touches (`containing` of both corners).
fn blocks_in(b: &Aabb) -> impl Iterator<Item = BlockPos> {
    let lo = BlockPos::containing(b.min_x, b.min_y, b.min_z);
    let hi = BlockPos::containing(b.max_x, b.max_y, b.max_z);
    (lo.x..=hi.x).flat_map(move |x| (lo.y..=hi.y).flat_map(move |y| (lo.z..=hi.z).map(move |z| BlockPos::new(x, y, z))))
}

/// `Cushion.canBePlacedAt`.
pub fn can_be_placed_at(level: &dyn EntityLevel, bb: &Aabb) -> bool {
    would_survive_at(level, bb) && !is_anchor_buried(level, bb)
}

/// `Cushion.wouldSurviveAt`.
pub fn would_survive_at(level: &dyn EntityLevel, bb: &Aabb) -> bool {
    has_anchor_below(level, bb) && !is_covered_by_suffocating_blocks(level, bb)
}

/// `Cushion.hasAnchorBelow`: a block whose shape reaches into the slab just under the box.
fn has_anchor_below(level: &dyn EntityLevel, bb: &Aabb) -> bool {
    let slab = Aabb::new(bb.min_x, bb.min_y - 0.015625, bb.min_z, bb.max_x.next_down(), bb.min_y, bb.max_z.next_down());
    blocks_in(&slab.expand_towards(0.0, -0.125, 0.0)).any(|pos| {
        let state = level.block(pos);
        let shape = physics::outline_shape(state);
        if shape.is_empty() {
            return false;
        }
        let (ox, oz) = physics::outline_offset(state, pos.x, pos.z).unwrap_or((0.0, 0.0));
        let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
        let bounds = Aabb::new(
            shape.min(Axis::X, 0.0) + px + ox,
            shape.min(Axis::Y, 0.0) + py,
            shape.min(Axis::Z, 0.0) + pz + oz,
            shape.max(Axis::X, 0.0) + px + ox,
            shape.max(Axis::Y, 0.0) + py,
            shape.max(Axis::Z, 0.0) + pz + oz,
        );
        bounds.intersects(&slab)
    })
}

/// `Cushion.isAnchorBuried`: the sliver at the bottom of the box is covered by block shapes all over.
fn is_anchor_buried(level: &dyn EntityLevel, bb: &Aabb) -> bool {
    let sliver = Aabb::new(bb.min_x, bb.min_y, bb.min_z, bb.max_x, bb.min_y + 0.015625, bb.max_z).next_deflated();
    let mut boxes: Vec<Aabb> = Vec::new();
    for_each_block_collision(level, &CollisionContext::EMPTY, &sliver, |pos, shape, _| {
        let (px, py, pz) = (pos.x as f64, pos.y as f64, pos.z as f64);
        boxes.extend(shape.boxes().iter().map(|b| b.offset(px, py, pz)));
        true
    });
    covers(&sliver, &boxes)
}

/// Whether `boxes` cover all of `region`: `Shapes.join(region, shapes, ONLY_FIRST)` becoming empty. Cells
/// thinner than the shape merging tolerance (1.0E-7) do not count.
fn covers(region: &Aabb, boxes: &[Aabb]) -> bool {
    let cuts = |lo: f64, hi: f64, pick: &dyn Fn(&Aabb) -> (f64, f64)| -> Vec<f64> {
        let mut v = vec![lo, hi];
        for b in boxes {
            let (a, c) = pick(b);
            v.extend([a, c].into_iter().filter(|x| *x > lo && *x < hi));
        }
        v.sort_by(f64::total_cmp);
        v
    };
    let xs = cuts(region.min_x, region.max_x, &|b| (b.min_x, b.max_x));
    let ys = cuts(region.min_y, region.max_y, &|b| (b.min_y, b.max_y));
    let zs = cuts(region.min_z, region.max_z, &|b| (b.min_z, b.max_z));
    for xw in xs.windows(2) {
        for yw in ys.windows(2) {
            for zw in zs.windows(2) {
                if xw[1] - xw[0] < 1.0e-7 || yw[1] - yw[0] < 1.0e-7 || zw[1] - zw[0] < 1.0e-7 {
                    continue;
                }
                let (cx, cy, cz) = ((xw[0] + xw[1]) / 2.0, (yw[0] + yw[1]) / 2.0, (zw[0] + zw[1]) / 2.0);
                let hit = boxes.iter().any(|b| b.min_x <= cx && cx <= b.max_x && b.min_y <= cy && cy <= b.max_y && b.min_z <= cz && cz <= b.max_z);
                if !hit {
                    return false;
                }
            }
        }
    }
    true
}

/// `Cushion.isCoveredBySuffocatingBlocks`: every block the box touches suffocates.
fn is_covered_by_suffocating_blocks(level: &dyn EntityLevel, bb: &Aabb) -> bool {
    blocks_in(&bb.next_deflated()).all(|pos| physics::is_suffocating(level.block(pos)))
}

/// `Cushion.survives`.
pub fn survives(level: &dyn EntityLevel, e: &Entity) -> bool {
    would_survive_at(level, &e.bounding_box())
}

impl Cushion {
    /// The cushion as an item, with its name.
    fn item(&self, e: &Entity) -> ItemStack {
        let mut stack = ItemStack::of(&item_name(self.color), 1).unwrap_or_default();
        if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
            stack.set(kiln_item::component::Component::CustomName(name));
        }
        stack
    }

    /// `dropItem(level, by)`: the break sound, the cushion item unless a player with infinite materials broke it.
    fn drop_item(&self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>, lightning: bool) {
        e.play_sound(level, "minecraft:entity.cushion.break", 1.0, 1.0);
        if !level.entity_drops() {
            return;
        }
        if by.and_then(|a| level.player(a)).is_some_and(|p| p.creative) {
            return;
        }
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut item = crate::item::new_at(id, 0, self.item(e), e.position(), seed);
        if let EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
        }
        if lightning {
            item.invulnerable_time = 20;
        }
        level.add_entity(item);
    }

    /// `kill(level, by)`: removed as killed, the `entity_die` game event.
    fn kill(&self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>) {
        e.removed = Some(RemovalReason::Killed);
        level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: by });
    }

    /// `destroyIfInFire`: a fire in the box hurts it by 1 (which breaks it).
    fn destroy_if_in_fire(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if e.is_removed() {
            return;
        }
        let in_fire = blocks_in(&e.bounding_box().next_deflated()).any(|pos| has_tag(level.block(pos), BlockTag::Fire));
        if in_fire {
            self.hurt(e, level, DamageKind::InFire, 1.0, None);
        }
    }
}

/// `Cushion.destroyIfInFire` for a cushion that is not in the level yet or is borrowed apart from it.
pub fn destroy_if_in_fire(e: &mut Entity, level: &mut dyn EntityLevel) {
    let placeholder = EntityKind::Other { type_name: e.type_name };
    let EntityKind::Ext(mut x) = std::mem::replace(&mut e.kind, placeholder) else { return };
    if let Some(c) = x.as_any_mut().downcast_mut::<Cushion>() {
        c.destroy_if_in_fire(e, level);
    }
    e.kind = EntityKind::Ext(x);
}

impl EntityExt for Cushion {
    entity_ext_boilerplate!();

    /// `BlockAttachedEntity.tick`: every 100 ticks the fire check, then the support check.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // (`ticksSinceLastCheck++ >= 100`: the check is the 101st tick.)
        let before = self.since_check;
        self.since_check += 1;
        if before >= 100 {
            self.since_check = 0;
            // `tickAtCheckInterval`: lava in its block burns it (the fluid's `entityInside`, then `lavaHurt`).
            let at = e.block_position();
            let lava = crate::fluid::fluid_at(&*level, at);
            if matches!(lava.kind, crate::physics::FluidKind::Lava | crate::physics::FluidKind::FlowingLava) && at.y as f64 + f64::from(crate::fluid::height(&*level, at, &lava)) > e.y() {
                // (`lavaHurt`: the burn sound after a hurt that went through.)
                if self.hurt(e, level, DamageKind::Lava, 4.0, None) && !e.silent {
                    let pitch = 2.0 + kiln_javamath::random::RandomSource::next_float(&mut e.random) * 0.4;
                    level.emit(crate::level::Event::Sound { pos: e.position(), sound: "minecraft:entity.generic.burn", source: "neutral", volume: 0.4, pitch });
                }
            }
            self.destroy_if_in_fire(e, level);
            if !e.is_removed() && !survives(&*level, e) {
                e.discard();
                self.drop_item(e, level, None, false);
            }
        }
    }

    fn save(&self, e: &Entity, o: &mut Output) {
        let b = e.block_position();
        o.put("block_pos", Tag::IntArray(vec![b.x, b.y, b.z]));
        o.put("color", Tag::String(COLORS[self.color as usize & 15].into()));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        if self.color != 0 {
            d.set(data::cushion::COLOR, &DataValue::Enum(self.color as i32));
        }
    }

    fn attackable(&self) -> bool {
        true
    }

    /// `Cushion.hurtServer` / `BlockAttachedEntity.hurtServer`: a player who may not build cannot break it, nor
    /// can a mob when mobs do not grief; else it is killed and drops the item.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, _amount: f32, attacker: Option<i32>) -> bool {
        if attacker.and_then(|a| level.player(a)).is_some_and(|p| !p.may_build) {
            return false;
        }
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        if !level.mob_griefing() && attacker.and_then(|a| level.entity(a)).is_some_and(|a| crate::mob::data(a).is_some()) {
            return false;
        }
        if !e.is_removed() {
            self.kill(e, level, attacker);
            self.drop_item(e, level, attacker, false);
        }
        true
    }

    /// `Entity.thunderHit` of a cushion: it breaks (and its item cannot burn for a second).
    fn thunder_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, bolt: i32) -> bool {
        if !e.is_removed() {
            self.kill(e, level, Some(bolt));
            self.drop_item(e, level, Some(bolt), true);
        }
        true
    }

    /// `Cushion.interact`: a click sits the player on it, unless sneaking or something sits on it already.
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, _stack: &ItemStack) -> Option<Outcome> {
        if who.sneaking || !e.passengers.is_empty() {
            return Some(Outcome::PASS);
        }
        // (The player sits down after getting off what it rode, and the sit sound follows: the simulation does that.)
        let _ = level;
        let mut out = Outcome::success(HeldChange::None);
        out.ride = true;
        Some(out)
    }
}

impl Cushion {
    /// `removePassenger`: a sound when the rider leaves a cushion that stays.
    pub fn passenger_left(e: &mut Entity, level: &mut dyn EntityLevel) {
        if !e.is_removed() {
            e.play_sound(level, "minecraft:entity.cushion.get_up", 1.0, 1.0);
        }
    }
}
