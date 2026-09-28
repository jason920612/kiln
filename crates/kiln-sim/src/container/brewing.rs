//! `BrewingStandBlockEntity.serverTick`: fuel (`brewing_fuel` uses and speed, resolved through
//! the datapack's context providers), the data-driven brewing recipes (`BrewingRecipe` over
//! each bottle with the ingredient), the ingredient's remainder, and the bottles shown in the
//! block state.
//!
//! The stand keeps its values in [`ContainerBe`]'s furnace fields: `lit_remaining` is `fuel`,
//! `lit_total` `totalFuel`, `cook_timer` `brewTime`, `cook_total` `totalBrewTime`, `speed`
//! `speedMultiplier`.

use super::ContainerBe;
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Level, state};
use kiln_inventory::Rules;
use kiln_inventory::stack::StackExt;
use kiln_item::ItemStack;
use kiln_item::component::{ResolvableFloat, ResolvableInt};

/// `BrewingStandBlockEntity.canPlaceItem`: fuel in slot 4, a reagent in slot 3, a potion input
/// in an empty bottle slot.
pub(crate) fn can_place_item(c: &ContainerBe, slot: usize, stack: &ItemStack, rules: &Rules) -> bool {
    match slot {
        4 => stack.has(kiln_item::component::ids::BREWING_FUEL),
        3 => rules.recipes.property_set_accepts("minecraft:brewing_reagent", stack),
        _ => kiln_inventory::slot::is_potion_input(stack, rules) && c.items[slot].is_empty(),
    }
}

/// The stand's loot context (`getLootContext`): its block state, position and block entity.
struct StandContext {
    state: u16,
    origin: [f64; 3],
}

impl kiln_loot::LootContext for StandContext {
    fn block_state(&self) -> Option<u16> {
        Some(self.state)
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn has_block_entity(&self) -> bool {
        true
    }
}

/// `isBrewable`: a reagent in the ingredient slot that some recipe takes with one of the
/// bottles.
fn is_brewable(c: &mut ContainerBe, rules: &Rules) -> bool {
    let reagent = &c.items[3];
    if reagent.is_empty() || !rules.recipes.property_set_accepts("minecraft:brewing_reagent", reagent) {
        return false;
    }
    for i in 0..3 {
        if c.items[i].is_empty() {
            continue;
        }
        if let Some(r) = rules.recipes.find_brewing(&c.items[i], &c.items[3], c.last_recipe) {
            c.last_recipe = Some(r);
            return true;
        }
    }
    false
}

/// `doBrew`: every bottle a recipe takes becomes its output; the ingredient is used up
/// (leaving its remainder, or dropping it when some ingredient is left). Returns the dropped
/// remainder.
fn do_brew(c: &mut ContainerBe, rules: &Rules) -> Option<ItemStack> {
    for i in 0..3 {
        if let Some(r) = rules.recipes.find_brewing(&c.items[i], &c.items[3], c.last_recipe) {
            c.last_recipe = Some(r);
            c.items[i] = rules.recipes.assemble_single(r);
        }
    }
    let remainder = kiln_inventory::recipe::crafting_remainder(c.items[3].effective_item());
    c.items[3].shrink_count(1);
    match remainder {
        Some(rem) if c.items[3].is_empty() => {
            c.items[3] = rem;
            None
        }
        other => other,
    }
}

/// What a tick leaves for the level.
#[derive(Default)]
pub(crate) struct TickOut {
    /// Items to drop at the stand (remainders that did not fit).
    pub drops: Vec<ItemStack>,
    /// Brewing finished (level event 1035).
    pub brewed: bool,
    /// The bottles the block state should show, when they changed.
    pub bottles: Option<[bool; 3]>,
}

/// `serverTick` on the block entity alone.
pub(crate) fn tick(c: &mut ContainerBe, rules: &Rules, loot: Option<&kiln_loot::LootData>, ctx_state: u16, pos: BlockPos) -> TickOut {
    let mut out = TickOut::default();
    let ctx = StandContext { state: ctx_state, origin: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5] };
    // Fuel: a fresh item when the last one is used up.
    if c.lit_remaining <= 0
        && let Some(fuel) = c.items[4].get(kiln_item::keys::BREWING_FUEL).cloned()
    {
        let mut rng = kiln_javamath::random::LegacyRandom::new(0);
        c.lit_remaining = match &fuel.uses {
            ResolvableInt::Constant(v) => *v,
            ResolvableInt::Reference(id) => loot.and_then(|l| l.context_int(id, &ctx, &mut rng)).unwrap_or(0),
        };
        c.lit_total = c.lit_remaining;
        c.speed = match &fuel.speed_multiplier {
            ResolvableFloat::Constant(v) => *v,
            ResolvableFloat::Reference(id) => loot.and_then(|l| l.context_float(id, &ctx, &mut rng)).unwrap_or(1.0),
        };
        let remainder = kiln_inventory::recipe::crafting_remainder(c.items[4].effective_item());
        c.items[4].shrink_count(1);
        if let Some(rem) = remainder {
            if c.items[4].is_empty() {
                c.items[4] = rem;
            } else {
                out.drops.push(rem);
            }
        }
        c.mark_changed();
    }
    let brewable = is_brewable(c, rules);
    let ingredient = c.items[3].item();
    if c.cook_timer > 0 {
        c.cook_timer -= 1;
        if c.cook_timer == 0 && brewable {
            out.drops.extend(do_brew(c, rules));
            out.brewed = true;
        } else if !brewable || c.items[3].is_empty() || Some(ingredient) != c.ingredient {
            c.cook_timer = 0;
        }
        c.mark_changed();
    } else if brewable && c.lit_remaining > 0 {
        let speed = if c.speed > 0.0 { c.speed } else { 1.0 };
        c.lit_remaining -= 1;
        c.cook_timer = (400.0f32 / speed).ceil() as i32;
        c.cook_total = c.cook_timer;
        c.ingredient = Some(ingredient);
        c.mark_changed();
    }
    let bottles = [0, 1, 2].map(|i| !c.items[i].is_empty());
    if c.last_bottles != Some(bottles) {
        c.last_bottles = Some(bottles);
        out.bottles = Some(bottles);
    }
    out
}

/// `serverTick` at `pos`: the block entity's tick, then the drops, the brewing sound and the
/// bottles in the block state.
pub(crate) fn server_tick(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<crate::entities::Spawn>) {
    let s = level.block(pos);
    let rules = level.env.menus.clone();
    let loot = level.env.loot.clone();
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let changes = c.changes;
    let out = tick(c, &rules, loot.as_deref(), s, pos);
    let changed = c.changes != changes;
    for (k, rem) in out.drops.into_iter().enumerate() {
        let at = [pos.x as f64, pos.y as f64, pos.z as f64];
        let mut rng = super::pos_random(level, pos, 3 + k as u64);
        super::drop_item_stack(at, rem, &mut rng, spawns);
    }
    if out.brewed {
        level.effect(kiln_blocks::Effect::LevelEvent { id: 1035, pos, data: 0 });
    }
    if let Some(bottles) = out.bottles
        && kiln_data::block_logic::block_class(s) == kiln_data::block_logic::BlockClass::BrewingStandBlock
    {
        let mut next = s;
        for (i, b) in bottles.into_iter().enumerate() {
            next = state::set_bool(next, &format!("has_bottle_{i}"), b);
        }
        kiln_blocks::set_block(level, pos, next, kiln_blocks::flags::CLIENTS);
    }
    if changed {
        let now = level.block(pos);
        if !kiln_data::blocks_types::is_air(now) {
            kiln_blocks::update::update_neighbour_for_output_signal(level, pos, kiln_blocks::BlockId::of(now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Option<std::sync::Arc<Rules>> {
        let dir = crate::datapack_dir(None);
        dir.join("data/minecraft/recipe").is_dir().then(|| std::sync::Arc::new(kiln_inventory::Rules::load(&dir).expect("recipes")))
    }

    fn potion(name: &str) -> ItemStack {
        let mut s = ItemStack::of("minecraft:potion", 1).unwrap();
        let id = kiln_item::registry::POTION.id(name).unwrap();
        s.insert(kiln_item::keys::POTION_CONTENTS, kiln_item::component::PotionContents { potion: Some(id), ..Default::default() });
        s
    }

    /// Water bottles and nether wart make awkward potions after 400 ticks, with blaze powder
    /// giving 20 brews.
    #[test]
    fn nether_wart_brews_awkward_potions() {
        let Some(rules) = rules() else { return };
        let mut c = ContainerBe::load(super::super::BeKind::BrewingStand, kiln_world::block_entity::type_id("minecraft:brewing_stand").unwrap(), &kiln_proto::nbt::Tag::Compound(Vec::new()));
        c.items[0] = potion("minecraft:water");
        c.items[2] = potion("minecraft:water");
        c.items[3] = ItemStack::of("minecraft:nether_wart", 2).unwrap();
        c.items[4] = ItemStack::of("minecraft:blaze_powder", 1).unwrap();
        let pos = BlockPos::new(0, 64, 0);
        let loot = kiln_loot::LootData::load_lenient(&crate::datapack_dir(None)).ok();
        let first = tick(&mut c, &rules, loot.as_ref(), 0, pos);
        assert_eq!(first.bottles, Some([true, false, true]));
        assert_eq!((c.lit_remaining, c.lit_total, c.cook_timer), (19, 20, 400), "fuel and brew time");
        assert!(c.items[4].is_empty(), "the blaze powder is used");
        let mut brewed = false;
        for _ in 0..400 {
            brewed |= tick(&mut c, &rules, loot.as_ref(), 0, pos).brewed;
        }
        assert!(brewed);
        let awkward = kiln_item::registry::POTION.id("minecraft:awkward").unwrap();
        for i in [0, 2] {
            assert_eq!(c.items[i].get(kiln_item::keys::POTION_CONTENTS).and_then(|p| p.potion), Some(awkward));
        }
        assert!(c.items[1].is_empty());
        assert_eq!(c.items[3].count(), 1, "one nether wart used");
    }
}
