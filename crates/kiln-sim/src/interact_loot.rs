//! Loot tables the game rolls for what a player does to a block (`Block.dropFromBlockInteractLootTable`, `LootContextParamSets.BLOCK_INTERACT`:
//! harvesting berries, shearing a pumpkin or a beehive, hoeing rooted dirt) and the ways such drops come out (`Block.popResource`,
//! `popResourceFromFace`). The tables are the data packs' own, so a pack that changes one changes the game.

use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use kiln_blocks::{BlockPos, Direction, Level};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// The loot context of `dropFromBlockInteractLootTable`: the block's state before the interaction, the middle of the block, the block entity (only
/// whether there is one), the tool in use (`None` for the bare hand of berry picking) and the player.
pub(crate) struct InteractContext<'a> {
    pub state: u16,
    pub origin: [f64; 3],
    pub block_entity: bool,
    pub tool: Option<&'a ItemStack>,
    pub interacting: bool,
}

impl kiln_loot::LootContext for InteractContext<'_> {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.interacting && target == kiln_loot::EntityTarget::InteractingEntity
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn block_state(&self) -> Option<u16> {
        Some(self.state)
    }
    fn has_block_entity(&self) -> bool {
        self.block_entity
    }
    fn tool(&self) -> Option<&ItemStack> {
        self.tool
    }
}

/// `Block.dropFromBlockInteractLootTable` without the popping: what the table `table` rolls for `pos` (state `state`, before the interaction),
/// `None` when the level has no loot data at all (the callers then give what the game gives without data packs). Vanilla draws from the table's
/// server-wide random sequence; regions cannot share one, so the draw is seeded from the place and the tick, like block drops.
pub(crate) fn block_interact(
    level: &RegionLevel,
    table: &str,
    pos: BlockPos,
    state: u16,
    tool: Option<&ItemStack>,
    interacting: bool,
    salt: usize,
) -> Option<Vec<ItemStack>> {
    let loot = level.env.loot.as_ref()?;
    let id = kiln_item::ident::Identifier::parse(table)?;
    let Some(t) = loot.table(&id) else { return Some(Vec::new()) };
    let ctx = InteractContext {
        state,
        origin: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5],
        block_entity: level.blocks.containers.get(pos).is_some(),
        tool,
        interacting,
    };
    let seed = (crate::blocks::effect_hash(level.env, pos, salt) | 1) as i64;
    let (mut sequences, mut level_random) = (kiln_loot::RandomSequences::new(0), kiln_javamath::random::LegacyRandom::new(seed));
    let mut rng = t.random(seed, &mut sequences, &mut level_random);
    Some(loot.random_items(&id, &ctx, rng.source()).into_iter().filter(|s| !s.is_empty()).collect())
}

/// `Block.popResource(level, pos, stack)`: an item entity around the middle of the block, half an item's height lower (three draws of the
/// level random), with the default pickup delay; nothing when the `block_drops` rule is off.
pub(crate) fn pop_resource(level: &mut RegionLevel, pos: BlockPos, stack: ItemStack, spawns: &mut Vec<Spawn>) {
    if !level.env.drops || stack.is_empty() {
        return;
    }
    let r = level.random();
    let at = [
        pos.x as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
        pos.y as f64 + 0.5 + (r.next_double() * 0.5 - 0.25) - 0.125,
        pos.z as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
    ];
    let salt = (pos.x as u64) << 24 ^ pos.z as u64 ^ pos.y as u64 ^ (spawns.len() as u64) << 48;
    let mut spawn = crate::mobs::drop_item(stack, at, salt);
    spawn.pos = at;
    spawns.push(spawn);
}

/// `Block.popResourceFromFace(level, pos, face, stack)`: the item comes out of the clicked face with a push away from it (the speeds are
/// the entity's own: vanilla gives them to the constructor).
pub(crate) fn pop_resource_from_face(level: &mut RegionLevel, pos: BlockPos, face: Direction, stack: ItemStack, spawns: &mut Vec<Spawn>) {
    if !level.env.drops || stack.is_empty() {
        return;
    }
    let step = face.step();
    // (`EntityTypes.ITEM`'s width and height are 0.25.)
    let (half_width, half_height) = (0.125, 0.125);
    let r = level.random();
    // The draws come in the order x, y, z for the position and then x, y, z for the speed, each only on the axes the face does not point along.
    let x = pos.x as f64 + 0.5 + if step[0] == 0 { r.next_double() * 0.5 - 0.25 } else { step[0] as f64 * (0.5 + half_width) };
    let y = pos.y as f64 + 0.5 + if step[1] == 0 { r.next_double() * 0.5 - 0.25 } else { step[1] as f64 * (0.5 + half_height) } - half_height;
    let z = pos.z as f64 + 0.5 + if step[2] == 0 { r.next_double() * 0.5 - 0.25 } else { step[2] as f64 * (0.5 + half_width) };
    let vx = if step[0] == 0 { r.next_double() * 0.2 - 0.1 } else { step[0] as f64 * 0.1 };
    let vy = if step[1] == 0 { r.next_double() * 0.1 } else { step[1] as f64 * 0.1 + 0.1 };
    let vz = if step[2] == 0 { r.next_double() * 0.2 - 0.1 } else { step[2] as f64 * 0.1 };
    let salt = (pos.x as u64) << 20 ^ pos.z as u64 ^ (spawns.len() as u64) << 48;
    let mut spawn = crate::mobs::drop_item(stack, [x, y, z], salt);
    spawn.vel = [vx, vy, vz];
    spawns.push(spawn);
}
