//! Powder snow, freezing and suffocation for players (vanilla `LivingEntity.aiStep`'s freezing
//! part, `LivingEntity.canFreeze`, `Entity.isInWall` and `LivingEntity.baseTick`'s wall damage).
//!
//! Standing in powder snow (`InsideBlockEffectType.FREEZE`) counts `ticksFrozen` up by one a tick
//! to 140; out of it, or wearing leather armor (the `freeze_immune_wearables` tag), the count
//! falls by two a tick. A frozen player is slowed (a movement speed modifier of -0.05 times the
//! share frozen, while standing on a block) and, fully frozen, takes 1 freezing damage every 40
//! ticks. A player whose eyes are inside a suffocating block takes 1 `in_wall` damage a tick.

use crate::Player;
use crate::health::{Cause, DamageCtx};
use kiln_entity::math::{Aabb, BlockPos};

/// `Entity.getTicksRequiredToFreeze`.
pub(crate) const TICKS_REQUIRED_TO_FREEZE: i32 = 140;

/// Whether an item id is in an item tag.
fn item_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:item")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

impl Player {
    /// `LivingEntity.canFreeze`: not a spectator, and no worn piece in `freeze_immune_wearables`.
    pub(crate) fn can_freeze(&self) -> bool {
        use kiln_item::component::EquipmentSlot as S;
        if self.game_mode == 3 {
            return false;
        }
        ![S::Feet, S::Legs, S::Chest, S::Head].into_iter().any(|slot| {
            let stack = self.worn(slot);
            !stack.is_empty() && item_tag(stack.item(), "minecraft:freeze_immune_wearables")
        })
    }

    /// `InsideBlockEffectType.FREEZE`.
    pub(crate) fn freeze_effect(&mut self) {
        self.is_in_powder_snow = true;
        if self.can_freeze() {
            self.ticks_frozen = TICKS_REQUIRED_TO_FREEZE.min(self.ticks_frozen + 1);
        }
    }

    /// The freezing part of `LivingEntity.aiStep`: thaw, the frost speed modifier, the damage.
    pub(crate) fn tick_freezing(&mut self, block: crate::hazards::BlockAt, ctx: &mut DamageCtx) {
        let can_freeze = self.can_freeze();
        if !self.is_in_powder_snow || !can_freeze {
            self.ticks_frozen = (self.ticks_frozen - 2).max(0);
        }
        // `removeFrost` and `tryAddFrost` (the block under the feet is not air).
        let blocks = |p: BlockPos| Some(block(p));
        let under = blocks(self.on_pos(&blocks, 0.2)).unwrap_or(0);
        let frost = if !kiln_data::blocks_types::is_air(under) && self.ticks_frozen > 0 {
            let percent = self.ticks_frozen.min(TICKS_REQUIRED_TO_FREEZE) as f32 / TICKS_REQUIRED_TO_FREEZE as f32;
            Some((-0.05f32 * percent) as f64)
        } else {
            None
        };
        if frost != self.frost_speed {
            self.frost_speed = frost;
            self.attributes_dirty = true;
        }
        if self.tick_count % 40 == 0 && self.ticks_frozen >= TICKS_REQUIRED_TO_FREEZE && self.can_freeze() {
            self.hurt(1.0, &Cause::Other("minecraft:freeze").into(), ctx);
        }
    }

    /// `Entity.isInWall` (`LivingEntity.isInWall`: not while sleeping): a block that suffocates
    /// within a 0.48 wide, paper thin box around the eyes.
    pub(crate) fn is_in_wall(&self, block: crate::hazards::BlockAt) -> bool {
        if self.game_mode == 3 || self.sleep.pos.is_some() {
            return false;
        }
        let (w, _, _) = self.dimensions();
        let f = (w * 0.8) as f64;
        if std::env::var_os("KILN_DBG_MOVE").is_some() && self.tick_count < 3 {
            let at = |x: i32, y: i32, z: i32| {
                let s = block(BlockPos::new(x, y, z));
                (s, kiln_entity::physics::is_suffocating(s))
            };
            eprintln!("DBG inwall eye {} b100 {:?} b101 {:?}", self.eye_y(), at(0, 100, 0), at(0, 101, 0));
        }
        let eye = self.eye_y();
        let b = Aabb::new(self.pos[0] - f / 2.0, eye - 5.0e-7, self.pos[2] - f / 2.0, self.pos[0] + f / 2.0, eye + 5.0e-7, self.pos[2] + f / 2.0);
        let floor = |v: f64| v.floor() as i32;
        for x in floor(b.min_x)..=floor(b.max_x) {
            for y in floor(b.min_y)..=floor(b.max_y) {
                for z in floor(b.min_z)..=floor(b.max_z) {
                    let pos = BlockPos::new(x, y, z);
                    let state = block(pos);
                    if kiln_data::blocks_types::is_air(state) || !kiln_entity::physics::is_suffocating(state) {
                        continue;
                    }
                    let (shape, _) = kiln_entity::collision::collision_shape(state, pos, &kiln_entity::collision::CollisionContext::EMPTY);
                    if shape.boxes().iter().any(|sb| sb.offset(x as f64, y as f64, z as f64).intersects(&b)) {
                        return true;
                    }
                }
            }
        }
        false
    }
}
