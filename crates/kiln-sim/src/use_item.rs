//! Shared pieces of item use (`Item.use`, `Item.useOn`): the player's view ray
//! (`Item.getPlayerPOVHitResult`), reading and replacing the item in a hand
//! (`Player.setItemInHand`) and `ItemUtils.createFilledResult`.

use crate::Player;
use crate::entities::Spawn;
use kiln_entity::math::{BlockPos as EBlockPos, Vec3};
use kiln_inventory::Container;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

/// `ClipContext.Fluid`: which fluids stop the ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FluidMode {
    None,
    SourceOnly,
    /// `ClipContext.Fluid.ANY`.
    #[allow(dead_code)]
    Any,
}

/// A block the ray hit: the block, the face and the point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PovHit {
    pub pos: kiln_blocks::BlockPos,
    pub face: kiln_blocks::Direction,
    pub location: [f64; 3],
}

/// `Entity.calculateViewVector(xRot, yRot)` (`Mth` sine table).
pub(crate) fn view_vector(rot: [f32; 2]) -> Vec3 {
    kiln_entity::ext_entity::fireball::view_vector(rot[1], rot[0])
}

/// `Item.getPlayerPOVHitResult`: `level.clip` from the eyes along the view for the block
/// interaction range, with the blocks' outline shapes (`getShape`) and the fluids of `fluid`.
/// `None`: a miss.
pub(crate) fn pov_hit(p: &Player, block: &dyn Fn(kiln_blocks::BlockPos) -> u16, fluid: FluidMode) -> Option<PovHit> {
    pov_hit_rot(p, p.rot, block, fluid)
}

/// [`pov_hit`] looking at `rot` instead of the player's rotation.
pub(crate) fn pov_hit_rot(p: &Player, rot: [f32; 2], block: &dyn Fn(kiln_blocks::BlockPos) -> u16, fluid: FluidMode) -> Option<PovHit> {
    let eye = p.eye_position();
    let from = Vec3::new(eye[0], eye[1], eye[2]);
    let range = p.block_interaction_range();
    let to = from + view_vector(rot).scale(range);
    clip(from, to, block, fluid)
}

/// `Level.clip` with `ClipContext.Block.OUTLINE` (see [`pov_hit`] for the shapes).
pub(crate) fn clip(from: Vec3, to: Vec3, block: &dyn Fn(kiln_blocks::BlockPos) -> u16, fluid: FluidMode) -> Option<PovHit> {
    use kiln_entity::clip::{shape_clip, traverse_blocks};
    use kiln_entity::physics;
    let kb = |p: EBlockPos| kiln_blocks::BlockPos::new(p.x, p.y, p.z);
    traverse_blocks(from, to, |pos| {
        let state = block(kb(pos));
        let shape = physics::outline_shape(state);
        let block_hit = match physics::outline_offset(state, pos.x, pos.z) {
            // Offset blocks (flowers): the shape moved to where the block sits.
            Some((ox, oz)) => shape_clip(&shape.moved(ox, 0.0, oz), from, to, pos),
            None => shape_clip(shape, from, to, pos),
        };
        let f = physics::fluid_state(state);
        let pick = match fluid {
            FluidMode::None => false,
            FluidMode::SourceOnly => f.source,
            FluidMode::Any => !f.is_empty(),
        };
        let fluid_hit = if pick {
            // `FluidState.getShape`: a full block under the same fluid, else its height.
            let above = physics::fluid_state(block(kb(pos.above())));
            let h = if !above.is_empty() && above.kind.is_same(f.kind) { 1.0 } else { f.own_height() as f64 };
            kiln_entity::shape::Shape::from_box(&kiln_entity::math::Aabb::new(0.0, 0.0, 0.0, 1.0, h, 1.0))
                .and_then(|s| shape_clip(&s, from, to, pos))
        } else {
            None
        };
        let d = |h: Option<(Vec3, kiln_entity::math::Direction)>| h.map_or(f64::MAX, |(l, _)| from.distance_to_sqr(l));
        let best = if d(block_hit) <= d(fluid_hit) { block_hit } else { fluid_hit };
        best.map(|(location, face)| PovHit {
            pos: kb(pos),
            face: kiln_blocks::Direction::from_index(face.index()),
            location: [location.x, location.y, location.z],
        })
    })
}

impl Player {
    /// `Player.blockInteractionRange` (4.5, 5 in creative).
    pub(crate) fn block_interaction_range(&self) -> f64 {
        if self.game_mode == 1 { 5.0 } else { 4.5 }
    }

    /// The inventory slot of a hand.
    pub(crate) fn hand_index(&self, off_hand: bool) -> usize {
        let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
        kiln_inventory::inventory::equipment_index(slot, self.inv.selected)
    }

    /// The item in a hand.
    pub(crate) fn in_hand(&self, off_hand: bool) -> &ItemStack {
        self.inv.item(self.hand_index(off_hand))
    }

    /// `Player.setItemInHand`.
    pub(crate) fn set_in_hand(&mut self, off_hand: bool, stack: ItemStack) {
        let i = self.hand_index(off_hand);
        *self.inv.item_mut(i) = stack;
        self.inv.times_changed += 1;
    }

    /// `Abilities.instabuild` / `hasInfiniteMaterials`.
    pub(crate) fn infinite_materials(&self) -> bool {
        self.game_mode == 1
    }

    /// `ItemUtils.createFilledResult(held, player, filled, limitCreativeStackSize)` on the item
    /// in a hand, which becomes the result: in creative the held item stays (and the filled one
    /// is added unless the inventory has it, when `limit_creative`); otherwise one held item
    /// becomes `filled` (into the inventory, or dropped, when more are left).
    pub(crate) fn fill_in_hand(&mut self, off_hand: bool, filled: ItemStack, limit_creative: bool, spawns: &mut Vec<Spawn>) {
        let mut filled = filled;
        if limit_creative && self.infinite_materials() {
            // `Inventory.contains`.
            let has = (0..self.inv.size()).any(|j| {
                let s = self.inv.item(j);
                !s.is_empty() && s.is_same_item_same_components(&filled)
            });
            if !has {
                self.add_to_inventory(&mut filled);
            }
            return;
        }
        let i = self.hand_index(off_hand);
        // `ItemStack.consume`: creative players keep theirs.
        if !self.infinite_materials() {
            self.inv.item_mut(i).shrink(1);
        }
        self.inv.times_changed += 1;
        if self.inv.item(i).is_empty() {
            *self.inv.item_mut(i) = filled;
            return;
        }
        // `Inventory.add`, else `Player.drop`.
        self.add_to_inventory(&mut filled);
        if !filled.is_empty() {
            spawns.push(self.throw(filled));
        }
    }
}
