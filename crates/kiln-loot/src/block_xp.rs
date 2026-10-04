//! Experience a broken block yields (`Block.spawnAfterBreak` with `dropExperience`):
//! `DropExperienceBlock` / `RedStoneOreBlock` ores roll their `xpRange` and let the tool's
//! `block_experience` enchantment effects (silk touch sets it to 0) change it, sculk blocks do
//! the same with a constant, the spawner rolls `15 + nextInt(15) + nextInt(15)` and ignores the
//! tool; `Block.popExperience` then awards the amount as orbs (`ExperienceOrb.award`).
//!
//! Evidence: `tools/LootVectors.java` records vanilla's orbs for every block and several tools
//! (`block_xp.jsonl`), replayed by `tests/vanilla_parity.rs`.

use crate::LootData;
use crate::effects::{ItemContext, ValueComponent};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// How a block rolls its experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XpRule {
    /// `tryDropExperience` with `ConstantInt.of(n)`.
    Constant(i32),
    /// `tryDropExperience` with `UniformInt.of(min, max)`.
    Uniform(i32, i32),
    /// `SpawnerBlock.spawnAfterBreak`: `popExperience(15 + nextInt(15) + nextInt(15))`.
    Spawner,
}

/// The experience rule of the block named `block` (`None`: it yields none).
pub fn xp_rule(block: &str) -> Option<XpRule> {
    let name = block.strip_prefix("minecraft:").unwrap_or(block);
    let name = name.strip_prefix("deepslate_").unwrap_or(name);
    Some(match name {
        "coal_ore" => XpRule::Uniform(0, 2),
        "nether_gold_ore" => XpRule::Uniform(0, 1),
        "lapis_ore" | "nether_quartz_ore" => XpRule::Uniform(2, 5),
        "redstone_ore" => XpRule::Uniform(1, 5),
        "diamond_ore" | "emerald_ore" => XpRule::Uniform(3, 7),
        // `new DropExperienceBlock(ConstantInt.of(0), ...)`: no experience.
        "copper_ore" | "iron_ore" | "gold_ore" => XpRule::Constant(0),
        "sculk" => XpRule::Constant(1),
        "sculk_sensor" | "calibrated_sculk_sensor" | "sculk_shrieker" | "sculk_catalyst" => XpRule::Constant(5),
        "spawner" => XpRule::Spawner,
        _ => return None,
    })
}

impl LootData {
    /// `EnchantmentHelper.processBlockExperience`: the `block_experience` effects of the tool's
    /// enchantments on `amount`, truncated.
    pub fn modify_block_experience(&self, tool: &ItemStack, rng: &mut dyn RandomSource, amount: i32) -> i32 {
        let mut value = amount as f32;
        self.for_each_enchantment(tool, |e, level| {
            let ctx = ItemContext { tool, level };
            value = self.apply_value_effects(e, ValueComponent::BlockExperience, level, &ctx, rng, value);
        });
        value as i32
    }

    /// The experience `spawnAfterBreak(.., tool, true)` of block `block` pops: 0 when the block
    /// has none (or silk touch took it). Draws from `rng` (the level's random) like vanilla.
    pub fn block_experience(&self, block: &str, tool: &ItemStack, rng: &mut dyn RandomSource) -> i32 {
        let Some(rule) = xp_rule(block) else { return 0 };
        let amount = match rule {
            XpRule::Spawner => {
                let a = rng.next_int_bounded(15);
                15 + a + rng.next_int_bounded(15)
            }
            XpRule::Constant(n) => self.modify_block_experience(tool, rng, n),
            XpRule::Uniform(min, max) => {
                let n = min + rng.next_int_bounded(max - min + 1);
                self.modify_block_experience(tool, rng, n)
            }
        };
        amount.max(0)
    }
}

/// `ExperienceOrb.getExperienceValue`: the largest orb size not above `amount`.
pub fn orb_value(amount: i32) -> i32 {
    [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3].into_iter().find(|&v| amount >= v).unwrap_or(1)
}

/// `ExperienceOrb.award`: the orb values `amount` splits into, largest first. Each orb first
/// tries to merge into one nearby (`tryMergeToExisting` draws `nextInt(40)`); merging itself is
/// the caller's business, only the random draws are made here.
pub fn orb_values(mut amount: i32, rng: &mut dyn RandomSource) -> Vec<i32> {
    let mut out = Vec::new();
    while amount > 0 {
        let v = orb_value(amount);
        amount -= v;
        rng.next_int_bounded(40);
        out.push(v);
    }
    out
}
