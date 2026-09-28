//! `AbstractFurnaceBlockEntity.serverTick`: fuel burning (burn times and speed from the fuel's
//! `cooking_fuel` component, resolved through the datapack's context providers), cooking with
//! the furnace's recipe type, the `lit` state, and the experience of used recipes.

use super::{BeKind, ContainerBe};
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Level, state};
use kiln_inventory::recipe::{CookingKind, Recipe};
use kiln_inventory::stack::{StackExt, same_item_same_components};
use kiln_inventory::{FurnaceKind, Rules};
use kiln_item::ItemStack;
use kiln_item::component::{ResolvableFloat, ResolvableInt};

fn cooking_kind(kind: FurnaceKind) -> CookingKind {
    match kind {
        FurnaceKind::Furnace => CookingKind::Smelting,
        FurnaceKind::BlastFurnace => CookingKind::Blasting,
        FurnaceKind::Smoker => CookingKind::Smoking,
    }
}

/// The furnace's loot context (`getLootContext`, `CONTAINER_PROCESS`): its block state
/// decides the `fast_cooking` condition of fuel providers.
struct FurnaceContext {
    state: u16,
    origin: [f64; 3],
}

impl kiln_loot::LootContext for FurnaceContext {
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

/// Fuel values for a furnace: `getBurnDuration` and `getSpeedMultiplier`.
pub(crate) struct Fuel<'a> {
    pub loot: Option<&'a kiln_loot::LootData>,
    pub state: u16,
    pub pos: BlockPos,
}

impl Fuel<'_> {
    fn ctx(&self) -> FurnaceContext {
        FurnaceContext { state: self.state, origin: [self.pos.x as f64 + 0.5, self.pos.y as f64 + 0.5, self.pos.z as f64 + 0.5] }
    }

    /// `ResolvableInt.getFromItem(fuel, COOKING_FUEL, burnTime, ctx, 0)`.
    pub fn burn_duration(&self, fuel: &ItemStack) -> i32 {
        let Some(f) = fuel.get(kiln_item::keys::COOKING_FUEL) else { return 0 };
        match &f.burn_time {
            ResolvableInt::Constant(v) => *v,
            ResolvableInt::Reference(id) => {
                let mut rng = kiln_javamath::random::LegacyRandom::new(0);
                self.loot.and_then(|l| l.context_int(id, &self.ctx(), &mut rng)).unwrap_or(0)
            }
        }
    }

    /// `ResolvableFloat.getFromItem(fuel, COOKING_FUEL, speedMultiplier, ctx, 1)`.
    pub fn speed_multiplier(&self, fuel: &ItemStack) -> f32 {
        let Some(f) = fuel.get(kiln_item::keys::COOKING_FUEL) else { return 1.0 };
        match &f.speed_multiplier {
            ResolvableFloat::Constant(v) => *v,
            ResolvableFloat::Reference(id) => {
                let mut rng = kiln_javamath::random::LegacyRandom::new(0);
                self.loot.and_then(|l| l.context_float(id, &self.ctx(), &mut rng)).unwrap_or(1.0)
            }
        }
    }
}

/// The furnace's `quickCheck`: the recipe for its input (remembering it for next time).
fn find_recipe(c: &mut ContainerBe, rules: &Rules) -> Option<usize> {
    let BeKind::Furnace(kind) = c.kind else { return None };
    let found = rules.recipes.find_cooking(cooking_kind(kind), &c.items[0], c.last_recipe);
    if found.is_some() {
        c.last_recipe = found;
    }
    found
}

fn cooking_time(rules: &Rules, recipe: usize) -> i32 {
    match &rules.recipes.recipes()[recipe].recipe {
        Recipe::Cooking(c) => c.cooking_time,
        _ => 200,
    }
}

/// `getTotalCookTime(recipe, furnace)`: the recipe's time divided by the fuel's speed.
fn total_cook_time(c: &ContainerBe, rules: &Rules, recipe: usize) -> i32 {
    let t = cooking_time(rules, recipe);
    if c.speed > 0.0 { (t as f32 / c.speed).ceil() as i32 } else { t }
}

/// `AbstractFurnaceBlockEntity.setItem` on the input slot with another item: the cooking
/// restarts with the new recipe's time (200 without one).
pub(crate) fn apply_input_change(c: &mut ContainerBe, rules: &Rules) {
    if !std::mem::take(&mut c.input_changed) {
        return;
    }
    c.cook_total = match find_recipe(c, rules) {
        Some(r) => total_cook_time(c, rules, r),
        None => 200,
    };
    c.cook_timer = 0;
}

/// `AbstractFurnaceBlockEntity.canBurn`.
fn can_burn(items: &[ItemStack], max_stack: i32, result: &ItemStack) -> bool {
    let out = &items[2];
    if out.is_empty() {
        return true;
    }
    if !same_item_same_components(out, result) {
        return false;
    }
    out.count() + result.count() <= max_stack.min(out.max_stack_size())
}

/// `AbstractFurnaceBlockEntity.burn`: the result goes out, one input is used (a wet sponge
/// fills a bucket in the fuel slot).
fn burn(items: &mut [ItemStack], result: &ItemStack) {
    if items[2].is_empty() {
        items[2] = result.copy();
    } else {
        items[2].grow_count(result.count());
    }
    let is = |s: &ItemStack, name: &str| !s.is_empty() && s.item_name() == name;
    if is(&items[0], "minecraft:wet_sponge") && is(&items[1], "minecraft:bucket") {
        items[1] = ItemStack::of("minecraft:water_bucket", 1).unwrap_or_default();
    }
    items[0].shrink_count(1);
}

/// What a furnace tick leaves for the level.
#[derive(Default)]
pub(crate) struct TickOut {
    /// The `lit` state to set, when it changed.
    pub lit: Option<bool>,
    /// `setChanged`.
    pub changed: bool,
    /// A fuel's remainder that did not fit (dropped at the furnace).
    pub drop: Option<ItemStack>,
}

/// `AbstractFurnaceBlockEntity.serverTick` on the block entity alone.
pub(crate) fn tick(c: &mut ContainerBe, rules: &Rules, fuel: &Fuel) -> TickOut {
    let mut out = TickOut::default();
    apply_input_change(c, rules);
    let was_lit = c.lit_remaining > 0;
    let mut lit = false;
    if was_lit {
        c.lit_remaining -= 1;
        lit = c.lit_remaining > 0;
    }
    let has_input = !c.items[0].is_empty();
    let has_fuel = !c.items[1].is_empty();
    if lit || has_fuel && has_input {
        if has_input {
            if let Some(recipe) = find_recipe(c, rules) {
                let result = rules.recipes.assemble_single(recipe);
                if !result.is_empty() && can_burn(&c.items, 99, &result) {
                    if !lit {
                        let fuel_stack = c.items[1].clone();
                        let burn_time = fuel.burn_duration(&fuel_stack);
                        let speed = fuel.speed_multiplier(&fuel_stack);
                        c.lit_remaining = burn_time;
                        c.lit_total = burn_time;
                        c.speed = speed;
                        if c.cook_total > 0 && c.cook_timer < c.cook_total {
                            let progress = c.cook_timer as f32 / c.cook_total as f32;
                            c.cook_total = total_cook_time(c, rules, recipe);
                            c.cook_timer = (progress * c.cook_total as f32).ceil() as i32;
                        }
                        if burn_time > 0 {
                            // `consumeFuel`.
                            let remainder = kiln_inventory::recipe::crafting_remainder(fuel_stack.effective_item());
                            c.items[1].shrink_count(1);
                            if let Some(rem) = remainder {
                                if c.items[1].is_empty() {
                                    c.items[1] = rem;
                                } else {
                                    out.drop = Some(rem);
                                }
                            }
                            lit = true;
                            out.changed = true;
                        }
                    }
                    if lit {
                        c.cook_timer += 1;
                        if c.cook_timer >= c.cook_total {
                            c.cook_timer = 0;
                            c.cook_total = total_cook_time(c, rules, recipe);
                            burn(&mut c.items, &result);
                            // `setRecipeUsed`.
                            let id = rules.recipes.id(recipe).to_owned();
                            match c.recipes_used.iter_mut().find(|(k, _)| *k == id) {
                                Some((_, n)) => *n += 1,
                                None => c.recipes_used.push((id, 1)),
                            }
                            out.changed = true;
                        }
                    } else {
                        c.cook_timer = 0;
                    }
                } else {
                    c.cook_timer = 0;
                }
            }
        } else {
            c.cook_timer = 0;
        }
    } else if c.cook_timer > 0 {
        c.cook_timer = (c.cook_timer - 2).clamp(0, c.cook_total.max(0));
    }
    if was_lit != lit {
        out.changed = true;
        out.lit = Some(lit);
    }
    out
}

/// `AbstractFurnaceBlockEntity.serverTick` at `pos`: the block entity's tick, then the `lit`
/// state, the dropped remainder and `setChanged`.
pub(crate) fn server_tick(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<crate::entities::Spawn>) {
    let s = level.block(pos);
    let rules = level.env.menus.clone();
    let loot = level.env.loot.clone();
    let fuel = Fuel { loot: loot.as_deref(), state: s, pos };
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let out = tick(c, &rules, &fuel);
    if out.changed {
        c.mark_changed();
    }
    if let Some(rem) = out.drop {
        let at = [pos.x as f64, pos.y as f64, pos.z as f64];
        super::drop_item_stack(at, rem, &mut level.blocks.random, spawns);
    }
    if let Some(lit) = out.lit {
        kiln_blocks::set_block(level, pos, state::set_bool(s, "lit", lit), kiln_blocks::flags::ALL);
    }
    if out.changed {
        let now = level.block(pos);
        if !kiln_data::blocks_types::is_air(now) {
            kiln_blocks::update::update_neighbour_for_output_signal(level, pos, kiln_blocks::BlockId::of(now));
        }
    }
}

/// `ExperienceOrb.getExperienceValue`: the largest orb size not above `amount`.
fn orb_value(amount: i32) -> i32 {
    [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3].into_iter().find(|&v| amount >= v).unwrap_or(1)
}

/// `AbstractFurnaceBlockEntity.getRecipesToAwardAndPopExperience`: the experience of the
/// recipes used since the last take, as orbs at `at`; the tally clears.
pub(crate) fn pop_experience(c: &mut ContainerBe, rules: &Rules, at: [f64; 3], rng: &mut dyn kiln_javamath::random::RandomSource, spawns: &mut Vec<crate::entities::Spawn>) {
    for (id, count) in std::mem::take(&mut c.recipes_used) {
        let Some(i) = rules.recipes.index_of(&id) else { continue };
        let Recipe::Cooking(cook) = &rules.recipes.recipes()[i].recipe else { continue };
        // `createExperience`: the fraction rounds up by chance.
        let f = count as f32 * cook.experience;
        let mut amount = f.floor() as i32;
        let frac = f - amount as f32;
        if frac != 0.0 && rng.next_float() < frac {
            amount += 1;
        }
        award_experience(at, amount, rng, spawns);
    }
    c.mark_changed();
}

/// `ExperienceOrb.award`: `amount` experience as orbs at `at` (merging into orbs nearby is not
/// simulated).
pub(crate) fn award_experience(at: [f64; 3], mut amount: i32, rng: &mut dyn kiln_javamath::random::RandomSource, spawns: &mut Vec<crate::entities::Spawn>) {
    while amount > 0 {
        let v = orb_value(amount);
        amount -= v;
        let seed = rng.next_long();
        let orb = kiln_entity::xp_orb::new_at(0, 0, kiln_entity::math::Vec3::new(at[0], at[1], at[2]), v, seed);
        spawns.push(crate::entities::Spawn {
            kind: &kiln_data::entities::types::EXPERIENCE_ORB,
            pos: at,
            vel: [orb.delta.x, orb.delta.y, orb.delta.z],
            body: crate::entities::Body::Ready(Box::new(orb)),
        });
    }
}
