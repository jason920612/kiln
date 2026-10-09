//! Archaeology: the brush (`BrushItem`) wears suspicious sand and gravel away (`BrushableBlockEntity.brush`, ten
//! brushes break the block), and what is buried in them comes out (`dropContent`). `BrushableBlock.tick` lets the
//! brushing fade (`checkReset`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::BeKind;
use crate::entities::{Body, Spawn};
use crate::use_item::{FluidMode, pov_hit};
use kiln_blocks::{BlockId, BlockPos, Direction, Effect, Level, TickPriority, schedule_block_tick, state};
use kiln_inventory::stack::StackExt;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

pub(crate) const ITEM: &str = "minecraft:brush";

/// `BrushItem.getUseDuration`.
pub(crate) const USE_DURATION: i32 = 200;

/// `BrushableBlockEntity.BRUSH_COOLDOWN_TICKS`, `BRUSH_RESET_TICKS` and `REQUIRED_BRUSHES_TO_BREAK`.
const BRUSH_COOLDOWN: i64 = 10;
const BRUSH_RESET: i64 = 40;
const REQUIRED_BRUSHES: i32 = 10;

/// How far a suspicious block has been brushed.
#[derive(Debug, Clone, Default)]
pub(crate) struct Brushable {
    /// `brushCount`.
    pub count: i32,
    /// `brushCountResetsAtTick`.
    pub resets_at: i64,
    /// `coolDownEndsAtTick`.
    pub cooldown_ends: i64,
    /// `hitDirection`: the face first brushed (not saved).
    pub hit: Option<Direction>,
}

/// `BrushableBlockEntity.getCompletionState`: the `dusted` property.
fn completion(count: i32) -> i32 {
    match count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        _ => 3,
    }
}

/// `BrushItem.useOn`: with a block in view the brush starts being used; the click is consumed either way.
pub(crate) fn use_on(p: &mut Player, level: &RegionLevel, off_hand: bool) -> bool {
    let block = |pos: BlockPos| level.block(pos);
    if p.using.is_none() && pov_hit(p, &block, FluidMode::None).is_some() {
        let stack = p.in_hand(off_hand).clone();
        p.start_using(off_hand, &stack, USE_DURATION);
    }
    true
}

/// `BrushItem.onUseTick` (the region's share, with the level at hand): the view must still be on a block; every tenth
/// tick (the fifth of ten) the block is brushed.
pub(crate) fn use_tick(p: &mut Player, level: &mut RegionLevel, used: i32, spawns: &mut Vec<Spawn>) {
    let Some(off_hand) = p.using.map(|u| u.off_hand) else { return };
    let hit = {
        let block = |pos: BlockPos| level.block(pos);
        pov_hit(p, &block, FluidMode::None)
    };
    let Some(hit) = hit else {
        p.stop_using();
        return;
    };
    if used % 10 != 5 {
        return;
    }
    let pos = hit.pos;
    let s = level.block(pos);
    // `BrushableBlock.getBrushSound`, else the generic one (dust particles are the client's).
    let sound = match BlockId::of(s).name() {
        "minecraft:suspicious_sand" => "minecraft:item.brush.brushing.sand",
        "minecraft:suspicious_gravel" => "minecraft:item.brush.brushing.gravel",
        _ => "minecraft:item.brush.brushing.generic",
    };
    level.effect(Effect::ActorSound { pos, sound, volume: 1.0, pitch: 1.0 });
    if level.blocks.containers.get(pos).is_none_or(|c| c.kind != BeKind::Brushable) {
        return;
    }
    let tool = p.in_hand(off_hand).clone();
    if brush(level, pos, hit.face, p, &tool, spawns) {
        p.hurt_and_break(if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand }, 1, None);
    }
}

/// `BrushableBlockEntity.brush`: true when the block broke (the tool wears).
fn brush(level: &mut RegionLevel, pos: BlockPos, face: Direction, p: &mut Player, tool: &ItemStack, spawns: &mut Vec<Spawn>) -> bool {
    let now = level.env.game_time;
    let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.brushable.as_mut()) else { return false };
    if b.hit.is_none() {
        b.hit = Some(face);
    }
    b.resets_at = now + BRUSH_RESET;
    if now < b.cooldown_ends {
        return false;
    }
    b.cooldown_ends = now + BRUSH_COOLDOWN;
    unpack_loot(level, pos, p, tool);
    let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.brushable.as_mut()) else { return false };
    let before = completion(b.count);
    b.count += 1;
    let count = b.count;
    if count >= REQUIRED_BRUSHES {
        completed(level, pos, spawns);
        return true;
    }
    let s = level.block(pos);
    schedule_block_tick(level, pos, BlockId::of(s), 2, TickPriority::Normal);
    let after = completion(count);
    if before != after {
        kiln_blocks::set_block_and_update(level, pos, state::set_int(s, "dusted", after));
    }
    false
}

/// `BrushableBlockEntity.unpackLootTable`: the table (`archaeology/...`) rolls once, a single item.
fn unpack_loot(level: &mut RegionLevel, pos: BlockPos, p: &mut Player, tool: &ItemStack) {
    let (loot, game_time, world_seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let Some(table) = c.loot_table.take() else { return };
    let seed = c.loot_seed;
    c.mark_changed();
    let Some(loot) = loot else { return };
    let Some(id) = kiln_item::ident::Identifier::parse(&table) else { return };
    // `CriteriaTriggers.GENERATE_LOOT`.
    let name = id.to_string();
    p.fire_conds("minecraft:player_generates_container_loot", None, |c, _, _| {
        c.get("loot_tables").and_then(|v| v.as_str()).and_then(kiln_item::ident::Identifier::parse).is_some_and(|i| i.to_string() == name)
    });
    // (A zero seed is the level's own random in vanilla; here it is made from the time and the place.)
    let seed = if seed != 0 {
        seed
    } else {
        let mut h = (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ world_seed as u64;
        for v in [pos.x, pos.y, pos.z] {
            h = (h ^ v as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        (h | 1) as i64
    };
    let ctx = ArchaeologyLoot { origin: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5], tool: tool.clone() };
    let mut rng = kiln_loot::random::seeded(seed);
    let items = loot.random_items(&id, &ctx, &mut rng);
    let first = items.into_iter().next().unwrap_or_else(ItemStack::empty);
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.items[0] = first;
    }
}

/// `LootContextParamSets.ARCHAEOLOGY`: where, who brushes and with what.
struct ArchaeologyLoot {
    origin: [f64; 3],
    tool: ItemStack,
}

impl kiln_loot::LootContext for ArchaeologyLoot {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn tool(&self) -> Option<&ItemStack> {
        Some(&self.tool)
    }
}

/// `BrushableBlockEntity.brushingCompleted`: the item pops out, the block turns into sand or gravel.
fn completed(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
    drop_content(level, pos, spawns);
    let s = level.block(pos);
    level.effect(Effect::LevelEvent { id: 3008, pos, data: s as i32 });
    let turns_into = match BlockId::of(s).name() {
        "minecraft:suspicious_gravel" => kiln_data::blocks::default_state::GRAVEL,
        "minecraft:suspicious_sand" => kiln_data::blocks::default_state::SAND,
        _ => kiln_data::blocks::default_state::AIR,
    };
    kiln_blocks::set_block_and_update(level, pos, turns_into);
}

/// `BrushableBlockEntity.dropContent`: up to 10-30 of the item, standing in the air on the side that was brushed.
fn drop_content(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
    let mut rng = crate::container::pos_random(level, pos, 8);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    if c.items[0].is_empty() {
        return;
    }
    let side = c.brushable.as_ref().and_then(|b| b.hit).unwrap_or(Direction::Up);
    let n = kiln_javamath::random::RandomSource::next_int_bounded(&mut rng, 21) + 10;
    let dropped = c.items[0].split_count(n);
    // (`this.item = ItemStack.EMPTY`: what the split left is gone.)
    c.items[0] = ItemStack::empty();
    if dropped.is_empty() {
        return;
    }
    // The item's width is 0.25 and its height 0.25.
    let at = pos.relative(side);
    let (width, height) = (0.25f64, 0.25f64);
    let x = at.x as f64 + 0.5 * (1.0 - width) + width / 2.0;
    let y = at.y as f64 + 0.5 + height / 2.0;
    let z = at.z as f64 + 0.5 * (1.0 - width) + width / 2.0;
    spawns.push(Spawn { kind: &kiln_data::entities::types::ITEM, pos: [x, y, z], vel: [0.0; 3], body: Body::Item { stack: dropped, pickup_delay: 0, thrower: None } });
}

/// `BrushableBlock.tick` → `BrushableBlockEntity.checkReset`: the brushing fades two at a time after a while.
pub(crate) fn check_reset(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    let now = level.env.game_time;
    let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.brushable.as_mut()) else { return };
    let mut set_dusted = None;
    if b.count != 0 && now >= b.resets_at {
        let before = completion(b.count);
        b.count = (b.count - 2).max(0);
        let after = completion(b.count);
        if before != after {
            set_dusted = Some(after);
        }
        b.resets_at = now + 4;
    }
    let again = b.count != 0;
    if b.count == 0 {
        b.hit = None;
        b.resets_at = 0;
        b.cooldown_ends = 0;
    }
    let mut s = s;
    if let Some(d) = set_dusted {
        s = state::set_int(s, "dusted", d);
        kiln_blocks::set_block_and_update(level, pos, s);
    }
    if again {
        schedule_block_tick(level, pos, BlockId::of(s), 2, TickPriority::Normal);
    }
}
