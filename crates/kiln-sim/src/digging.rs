//! Breaking blocks (`ServerPlayerGameMode.handleBlockBreakAction`, `destroyBlock`, `tick`):
//! creative players break at once; in survival the client reports when it starts and stops
//! digging, and the server accepts the break once the held tool had time to break the block
//! (or a little later, on its own clock). The speed follows `Player.getDestroySpeed`: the
//! tool's speed plus the `mining_efficiency` attribute (efficiency) for tools faster than the
//! hand, times `block_break_speed`, times `submerged_mining_speed` with the eyes in water (aqua
//! affinity raises it), a fifth in the air. Haste and Mining Fatigue do not exist yet.

use crate::{Player, combat};
use crate::blocks::RegionLevel;
use kiln_blocks::interact::{self, Actor};
use kiln_blocks::{BlockPos, Level};
use kiln_data::block_props;
use kiln_data::blocks_types::is_air;
use kiln_item::holder::HolderSet;
use kiln_item::{ItemStack, keys};
use kiln_proto::packets;

/// `ServerboundPlayerActionPacket.Action` ordinals (26.3).
pub(crate) const START_DESTROY_BLOCK: i32 = 0;
pub(crate) const ABORT_DESTROY_BLOCK: i32 = 2;
pub(crate) const STOP_DESTROY_BLOCK: i32 = 3;

/// A block being broken: where, and the game tick digging started.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Dig {
    pub pos: [i32; 3],
    pub start: i64,
    /// Crack stage last shown to other players.
    pub stage: i32,
}

fn block_pos(p: [i32; 3]) -> BlockPos {
    BlockPos::new(p[0], p[1], p[2])
}

/// Whether a tool rule's block set holds `state`.
fn rule_matches(blocks: &HolderSet, state: u16) -> bool {
    match blocks {
        HolderSet::Tag(tag) => kiln_blocks::tags::is(state, tag.as_str()),
        HolderSet::Direct(ids) => {
            let name = kiln_blocks::BlockId::of(state).name();
            kiln_data::builtin_id("minecraft:block", name).is_some_and(|id| ids.contains(&id))
        }
    }
}

/// `ItemStack.getDestroySpeed`: the tool's speed for the block, 1 without a tool.
fn tool_speed(stack: &ItemStack, state: u16) -> f32 {
    let Some(tool) = stack.get(keys::TOOL) else { return 1.0 };
    tool.rules.iter().find_map(|r| r.speed.filter(|_| rule_matches(&r.blocks, state))).unwrap_or(tool.default_mining_speed)
}

/// `Player.hasCorrectToolForDrops`.
pub(crate) fn has_correct_tool(stack: &ItemStack, state: u16) -> bool {
    if !block_props::requires_correct_tool(state) {
        return true;
    }
    let Some(tool) = stack.get(keys::TOOL) else { return false };
    tool.rules.iter().find_map(|r| r.correct_for_drops.filter(|_| rule_matches(&r.blocks, state))).unwrap_or(false)
}

impl Player {
    fn eye_height(&self) -> f64 {
        if self.sneaking { 1.27 } else { 1.62 }
    }

    /// `Player.isWithinBlockInteractionRange`: the block's box within the block interaction
    /// range (4.5, 5 in creative) plus `buffer` of the eyes.
    pub(crate) fn can_reach_block(&self, pos: [i32; 3], buffer: f64) -> bool {
        let range = if self.game_mode == 1 { 5.0 } else { 4.5 } + buffer;
        let eye = [self.pos[0], self.pos[1] + self.eye_height(), self.pos[2]];
        let d2: f64 = (0..3)
            .map(|i| {
                let (lo, hi) = (pos[i] as f64, pos[i] as f64 + 1.0);
                (lo - eye[i]).max(0.0).max(eye[i] - hi).powi(2)
            })
            .sum();
        d2 < range * range
    }

    /// `Player.getDestroySpeed`: the tool, efficiency, haste or conduit power (20% a level),
    /// mining fatigue (0.3 to the power of the level), the break speed attribute, water and air.
    pub(crate) fn destroy_speed(&self, state: u16, eye_in_water: bool) -> f32 {
        let mut speed = tool_speed(self.inv.selected_item(), state);
        if speed > 1.0 {
            speed += self.attribute(combat::MINING_EFFICIENCY) as f32;
        }
        // `MobEffectUtil.hasDigSpeed` / `getDigSpeedAmplification`.
        let haste = self.effect_amplifier("minecraft:haste");
        let conduit = self.effect_amplifier("minecraft:conduit_power");
        if haste.is_some() || conduit.is_some() {
            let amplifier = haste.unwrap_or(0).max(conduit.unwrap_or(0));
            speed *= 1.0 + (amplifier + 1) as f32 * 0.2;
        }
        if let Some(amplifier) = self.effect_amplifier("minecraft:mining_fatigue") {
            speed *= 0.3f64.powf((amplifier + 1) as f64) as f32;
        }
        speed *= self.attribute(combat::BLOCK_BREAK_SPEED) as f32;
        if eye_in_water {
            speed *= self.attribute(combat::SUBMERGED_MINING_SPEED) as f32;
        }
        if !self.on_ground {
            speed /= 5.0;
        }
        speed
    }

    /// `BlockBehaviour.getDestroyProgress`: the share of the block broken per tick.
    fn destroy_progress(&self, level: &RegionLevel, state: u16) -> f32 {
        let hardness = block_props::hardness(state);
        if hardness < 0.0 {
            return 0.0;
        }
        let speed = self.destroy_speed(state, self.eye_in_water(level));
        speed / hardness / if has_correct_tool(self.inv.selected_item(), state) { 30.0 } else { 100.0 }
    }

    /// `isEyeInFluid(WATER)` ([`Player::fluids`]).
    fn eye_in_water(&self, level: &RegionLevel) -> bool {
        let block = |p: kiln_entity::math::BlockPos| level.block(BlockPos::new(p.x, p.y, p.z));
        self.fluids(&block).eye_in_water
    }

    fn actor(&self) -> Actor {
        Actor { yaw: self.rot[0], may_build: self.game_mode <= 1, creative: self.game_mode == 1 }
    }

    /// Resends the block at `pos` to this player (the break or use did not happen).
    pub(crate) fn resend_block(&mut self, level: &RegionLevel, pos: [i32; 3]) {
        let state = level.block(block_pos(pos));
        self.send(packets::block_update(pos, state));
    }
}

/// `ServerPlayerGameMode.handleBlockBreakAction`.
pub(crate) fn player_action(p: &mut Player, level: &mut RegionLevel, action: i32, pos: [i32; 3]) {
    if !p.can_reach_block(pos, 1.0) {
        return;
    }
    if pos[1] >= level.env.min_y + level.env.height {
        p.resend_block(level, pos);
        return;
    }
    let now = level.env.game_time;
    let bp = block_pos(pos);
    match action {
        START_DESTROY_BLOCK => {
            if p.game_mode == 1 {
                destroy_or_resend(p, level, pos);
                return;
            }
            // Adventure (without `can_break`) and spectator players cannot break blocks.
            if p.game_mode != 0 {
                p.resend_block(level, pos);
                return;
            }
            let state = level.block(bp);
            let progress = if is_air(state) { 1.0 } else { p.destroy_progress(level, state) };
            if !is_air(state) && progress >= 1.0 {
                destroy_or_resend(p, level, pos);
                return;
            }
            if let Some(old) = p.digging.take() {
                p.resend_block(level, old.pos);
            }
            let stage = (progress * 10.0) as i32;
            level.out.destruction.push((p.entity_id, pos, stage));
            p.digging = Some(Dig { pos, start: now, stage });
        }
        STOP_DESTROY_BLOCK => {
            let Some(dig) = p.digging.filter(|d| d.pos == pos) else { return };
            let state = level.block(bp);
            if is_air(state) {
                return;
            }
            let progress = p.destroy_progress(level, state) * (now - dig.start + 1) as f32;
            if progress >= 0.7 {
                p.digging = None;
                level.out.destruction.push((p.entity_id, pos, -1));
                destroy_or_resend(p, level, pos);
            } else if p.delayed_destroy.is_none() {
                // The client finished early (lag): the break happens once the server's clock
                // agrees.
                p.digging = None;
                p.delayed_destroy = Some(dig);
            }
        }
        ABORT_DESTROY_BLOCK => {
            if let Some(dig) = p.digging.take()
                && dig.pos != pos
            {
                level.out.destruction.push((p.entity_id, dig.pos, -1));
            }
            level.out.destruction.push((p.entity_id, pos, -1));
        }
        _ => {}
    }
}

/// `ServerPlayerGameMode.tick`: a delayed break completes when enough time passed; the crack
/// stage of the block being dug follows the progress.
pub(crate) fn tick(p: &mut Player, level: &mut RegionLevel) {
    let now = level.env.game_time;
    if let Some(dig) = p.delayed_destroy {
        let state = level.block(block_pos(dig.pos));
        if is_air(state) {
            p.delayed_destroy = None;
        } else if p.destroy_progress(level, state) * (now - dig.start + 1) as f32 >= 1.0 {
            p.delayed_destroy = None;
            destroy_block(p, level, dig.pos);
        }
    } else if let Some(dig) = p.digging.as_mut() {
        let state = level.block(block_pos(dig.pos));
        if is_air(state) {
            level.out.destruction.push((p.entity_id, dig.pos, -1));
            p.digging = None;
            return;
        }
        let progress = {
            let dig = *dig;
            p.destroy_progress(level, state) * (now - dig.start + 1) as f32
        };
        let dig = p.digging.as_mut().unwrap();
        let stage = (progress * 10.0) as i32;
        if stage != dig.stage {
            dig.stage = stage;
            level.out.destruction.push((p.entity_id, dig.pos, stage));
        }
    }
}

fn destroy_or_resend(p: &mut Player, level: &mut RegionLevel, pos: [i32; 3]) {
    if !destroy_block(p, level, pos) {
        p.resend_block(level, pos);
    }
}

/// `ServerPlayerGameMode.destroyBlock`: returns whether the break happened.
pub(crate) fn destroy_block(p: &mut Player, level: &mut RegionLevel, pos: [i32; 3]) -> bool {
    let bp = block_pos(pos);
    let state = level.block(bp);
    let stack = p.inv.selected_item();
    // `Item.canDestroyBlock`: swords and the like break nothing in creative.
    if p.game_mode == 1 && stack.get(keys::TOOL).is_some_and(|t| !t.can_destroy_blocks_in_creative) {
        return false;
    }
    if p.game_mode >= 2 {
        return false;
    }
    let actor = p.actor();
    let drops = !actor.creative && has_correct_tool(stack, state);
    // `ItemStack.mineBlock`: a tool counts as used.
    if !actor.creative && stack.get(keys::TOOL).is_some() {
        let item = stack.item();
        p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
    }
    let previous = level.actor.replace(p.conn);
    crate::container::open::player_will_destroy(level, bp, state, actor.creative);
    crate::heart::player_will_destroy(level, bp, state, p.entity_id, p.game_mode == 0 || p.game_mode == 2);
    let removed = interact::player_destroy(level, bp, &actor, drops);
    level.actor = previous;
    // `ItemStack.mineBlock` (survival only): a tool loses `damage_per_block` durability for
    // blocks that are not instantly broken.
    if removed && !actor.creative {
        let per_block = p.inv.selected_item().get(keys::TOOL).map_or(0, |t| t.damage_per_block);
        if per_block > 0 && block_props::hardness(state) != 0.0 {
            p.hurt_and_break(kiln_item::component::EquipmentSlot::MainHand, per_block, None);
        }
    }
    // `Block.playerDestroy`, which runs when the player can harvest the block.
    if removed && drops {
        p.award_stat(crate::player_stats::Stat::mined(state), 1);
        p.exhaust(0.005);
    }
    actor.creative || removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn tools_speed_up_and_gate_drops() {
        let pickaxe = ItemStack::of("minecraft:diamond_pickaxe", 1).unwrap();
        let wooden = ItemStack::of("minecraft:wooden_pickaxe", 1).unwrap();
        let hand = ItemStack::empty();
        assert!(tool_speed(&pickaxe, d::STONE) > 1.0);
        assert_eq!(tool_speed(&hand, d::STONE), 1.0);
        assert!(has_correct_tool(&pickaxe, d::STONE) && has_correct_tool(&wooden, d::STONE));
        assert!(!has_correct_tool(&hand, d::STONE));
        assert!(has_correct_tool(&pickaxe, d::OBSIDIAN) && !has_correct_tool(&wooden, d::OBSIDIAN));
        // Dirt needs no tool.
        assert!(has_correct_tool(&hand, d::DIRT));
    }
}
