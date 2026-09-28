//! The recipe book's "place recipe" (`RecipeBookMenu.handlePlacement`, `ServerPlaceRecipe`):
//! the grid is cleared back into the inventory, and if the inventory and grid hold enough of
//! the recipe's ingredients they are moved into the grid in the recipe's shape (one craft, or
//! as many as possible with `useMaxItems`); otherwise the client shows a ghost recipe.
//!
//! Approximation: vanilla's `StackedContents` picks which item of a multi-item ingredient to
//! use in hash order; Kiln prefers the item found first in the inventory.

use crate::menus::MenuKind;
use crate::stack::StackExt;
use crate::menu::{Env, Menu};
use crate::menus::FurnaceKind;
use crate::recipe::{Ingredient, Recipe, book};
use kiln_item::ItemStack;

/// `RecipeBookMenu.PostPlaceAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostPlace {
    Nothing,
    PlaceGhostRecipe,
}

/// `StackedItemContents.accountSimpleStack`: undamaged, unenchanted, unnamed stacks count.
fn simple(stack: &ItemStack) -> bool {
    !stack.is_empty()
        && !stack.is_damaged()
        && !stack.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|e| !e.is_empty())
        && !stack.has(kiln_item::component::ids::CUSTOM_NAME)
}

/// Available item counts, in the order they were first seen.
#[derive(Debug, Default, Clone)]
struct Stacked {
    items: Vec<(i32, i64)>,
}

impl Stacked {
    fn account(&mut self, stack: &ItemStack) {
        if !simple(stack) {
            return;
        }
        let item = stack.item();
        match self.items.iter_mut().find(|(i, _)| *i == item) {
            Some((_, n)) => *n += stack.count() as i64,
            None => self.items.push((item, stack.count() as i64)),
        }
    }

    /// `canCraft(recipe, amount, output)`: an item per ingredient such that every ingredient
    /// gets `amount` of it.
    fn pick(&self, ingredients: &[&Ingredient], amount: i64) -> Option<Vec<i32>> {
        fn go(s: &Stacked, ings: &[&Ingredient], amount: i64, used: &mut Vec<(i32, i64)>, out: &mut Vec<i32>) -> bool {
            let Some((first, rest)) = ings.split_first() else { return true };
            for &(item, have) in &s.items {
                if !first.items.contains(&item) {
                    continue;
                }
                let taken = used.iter().find(|(i, _)| *i == item).map_or(0, |(_, n)| *n);
                if have - taken < amount {
                    continue;
                }
                match used.iter_mut().find(|(i, _)| *i == item) {
                    Some((_, n)) => *n += amount,
                    None => used.push((item, amount)),
                }
                out.push(item);
                if go(s, rest, amount, used, out) {
                    return true;
                }
                out.pop();
                if let Some((_, n)) = used.iter_mut().find(|(i, _)| *i == item) {
                    *n -= amount;
                }
            }
            false
        }
        let mut out = Vec::new();
        go(self, ingredients, amount, &mut Vec::new(), &mut out).then_some(out)
    }

    /// `getBiggestCraftableStack`.
    fn biggest(&self, ingredients: &[&Ingredient]) -> i64 {
        let max = self.items.iter().map(|(_, n)| *n).max().unwrap_or(0);
        let (mut lo, mut hi) = (0i64, max);
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            if self.pick(ingredients, mid).is_some() { lo = mid } else { hi = mid - 1 }
        }
        lo
    }
}

/// `PlaceRecipeHelper.placeRecipe`: calls `out(ingredient index or -1, grid slot)` for each
/// pattern cell, centering a small recipe in the grid.
fn place_helper(grid_w: usize, grid_h: usize, recipe_w: usize, recipe_h: usize, cells: &[i32], mut out: impl FnMut(i32, usize)) {
    let mut it = cells.iter();
    let mut slot = 0usize;
    let mut y = 0usize;
    while y < grid_h {
        let shift_y = (recipe_h as f32) < grid_h as f32 / 2.0;
        let off_y = (grid_h as f32 / 2.0 - recipe_h as f32 / 2.0).floor() as i32;
        if shift_y && off_y > y as i32 {
            slot += grid_w;
            y += 1;
        }
        let mut x = 0usize;
        while x < grid_w {
            let Some(&cell) = it.as_slice().first() else { return };
            let shift_x = (recipe_w as f32) < grid_w as f32 / 2.0;
            let off_x = (grid_w as f32 / 2.0 - recipe_w as f32 / 2.0).floor() as i32;
            let mut end_x = recipe_w as i32;
            let mut place = x < recipe_w;
            if shift_x {
                end_x = off_x + recipe_w as i32;
                place = off_x <= x as i32 && (x as i32) < off_x + recipe_w as i32;
            }
            if place {
                it.next();
                out(cell, slot);
            } else if end_x == x as i32 {
                slot += grid_w - x;
                break;
            }
            slot += 1;
            x += 1;
        }
        y += 1;
    }
}

/// `PlacementInfo.slotsToIngredientIndex` and the recipe's size in the grid.
fn layout(recipe: &Recipe, grid_w: usize, grid_h: usize) -> Option<(usize, usize, Vec<i32>)> {
    match recipe {
        Recipe::Shaped(s) => {
            let mut next = 0;
            let cells = s
                .ingredients
                .iter()
                .map(|i| match i {
                    Some(_) => {
                        next += 1;
                        next - 1
                    }
                    None => -1,
                })
                .collect();
            Some((s.width, s.height, cells))
        }
        _ => {
            let n = book::placement_ingredients(recipe).len();
            (n <= grid_w * grid_h).then(|| (grid_w, grid_h, (0..n as i32).collect()))
        }
    }
}

/// Where a menu's grid lives: the crafting grid, or the furnace's ingredient slot.
#[derive(Clone, Copy)]
enum Grid {
    Craft,
    Furnace,
}

impl Grid {
    fn get<'a>(self, menu: &'a Menu, env: &'a Env, i: usize) -> &'a ItemStack {
        match self {
            Grid::Craft => &menu.craft.items[i],
            Grid::Furnace => env.block.as_deref().map_or(&menu.result.item, |b| b.item(i)),
        }
    }
    fn set(self, menu: &mut Menu, env: &mut Env, i: usize, stack: ItemStack) {
        match self {
            Grid::Craft => menu.craft.items[i] = stack,
            Grid::Furnace => {
                if let Some(b) = env.block.as_deref_mut() {
                    b.set_item(i, stack);
                }
            }
        }
    }
}

impl Menu {
    /// `RecipeBookMenu.handlePlacement` for `recipe` (the player knows it).
    pub fn place_recipe(&mut self, env: &mut Env, recipe: usize, use_max: bool, creative: bool) -> PostPlace {
        let rules = env.rules;
        let Some(holder) = rules.recipes.recipes().get(recipe) else { return PostPlace::Nothing };
        let ingredients = book::placement_ingredients(&holder.recipe);
        if ingredients.is_empty() {
            return PostPlace::Nothing;
        }
        let (grid, w, h, input, clear): (Grid, usize, usize, Vec<usize>, Vec<usize>) = match self.kind {
            MenuKind::Inventory | MenuKind::Crafting if holder.recipe.is_crafting() => {
                let (w, h) = (self.craft.width, self.craft.height);
                (Grid::Craft, w, h, (0..w * h).collect(), (0..w * h).collect())
            }
            MenuKind::Furnace(kind) => {
                let wanted = match kind {
                    FurnaceKind::Furnace => crate::recipe::CookingKind::Smelting,
                    FurnaceKind::BlastFurnace => crate::recipe::CookingKind::Blasting,
                    FurnaceKind::Smoker => crate::recipe::CookingKind::Smoking,
                };
                if !matches!(&holder.recipe, Recipe::Cooking(c) if c.kind == wanted) {
                    return PostPlace::Nothing;
                }
                (Grid::Furnace, 1, 1, vec![0], vec![0, 2])
            }
            _ => return PostPlace::Nothing,
        };
        let Some((rw, rh, cells)) = layout(&holder.recipe, w, h) else { return PostPlace::Nothing };
        if rw > w || rh > h {
            return PostPlace::Nothing;
        }
        if let Grid::Craft = grid {
            self.placing_recipe = true;
        }
        let result = self.place_in(env, grid, &input, &clear, &ingredients, (w, h, rw, rh, &cells), recipe, use_max, creative);
        if let Grid::Craft = grid {
            // `finishPlacingRecipe`: the result for the new grid, with the recipe as hint.
            self.placing_recipe = false;
            self.slot_changed_crafting_grid(env, Some(recipe));
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn place_in(
        &mut self,
        env: &mut Env,
        grid: Grid,
        input: &[usize],
        clear: &[usize],
        ingredients: &[&Ingredient],
        shape: (usize, usize, usize, usize, &[i32]),
        recipe: usize,
        use_max: bool,
        creative: bool,
    ) -> PostPlace {
        if !creative && !self.test_clear_grid(env, grid, input) {
            return PostPlace::Nothing;
        }
        let mut stacked = Stacked::default();
        for s in &env.inventory.items {
            stacked.account(s);
        }
        for &i in input {
            let s = grid.get(self, env, i).clone();
            stacked.account(&s);
        }
        if stacked.pick(ingredients, 1).is_none() {
            self.clear_grid(env, grid, clear);
            env.inventory.times_changed += 1;
            return PostPlace::PlaceGhostRecipe;
        }
        // `placeRecipe`.
        let matches = self.grid_matches(env, grid, recipe);
        let biggest = stacked.biggest(ingredients);
        if matches {
            for &i in input {
                let s = grid.get(self, env, i);
                if !s.is_empty() && (biggest.min(s.max_stack_size() as i64)) < s.count() as i64 + 1 {
                    return PostPlace::Nothing;
                }
            }
        }
        let mut amount = if use_max {
            biggest
        } else if matches {
            let least = input.iter().map(|&i| grid.get(self, env, i)).filter(|s| !s.is_empty()).map(|s| s.count()).min();
            least.map_or(1, |n| n as i64 + 1)
        } else {
            1
        };
        let Some(mut items) = stacked.pick(ingredients, amount) else { return PostPlace::Nothing };
        // `clampToMaxStackSize`.
        let max = items.iter().map(|&i| kiln_item::ItemStack::new(i, 1).max_stack_size() as i64).fold(amount, i64::min);
        if max != amount {
            amount = max;
            match stacked.pick(ingredients, amount) {
                Some(v) => items = v,
                None => return PostPlace::Nothing,
            }
        }
        self.clear_grid(env, grid, clear);
        let (gw, gh, rw, rh, cells) = shape;
        let mut moves = Vec::new();
        place_helper(gw, gh, rw, rh, cells, |cell, slot| {
            if cell >= 0 {
                moves.push((input[slot], items[cell as usize]));
            }
        });
        for (slot, item) in moves {
            let mut left = amount as i32;
            while left > 0 {
                left = self.move_item_to_grid(env, grid, slot, item, left);
                if left == -1 {
                    break;
                }
            }
        }
        env.inventory.times_changed += 1;
        PostPlace::Nothing
    }

    /// `CraftingMenuAccess.recipeMatches`.
    fn grid_matches(&self, env: &Env, grid: Grid, recipe: usize) -> bool {
        match grid {
            Grid::Craft => {
                let input = crate::recipe::CraftingInput::new(self.craft.width, self.craft.height, &self.craft.items);
                env.rules.recipes.recipes()[recipe].recipe.matches_crafting(&input, &*env.world)
            }
            Grid::Furnace => match &env.rules.recipes.recipes()[recipe].recipe {
                Recipe::Cooking(c) => c.matches(grid.get(self, env, 0)),
                _ => false,
            },
        }
    }

    /// `testClearGrid`: everything in the grid can go back into the inventory.
    fn test_clear_grid(&self, env: &Env, grid: Grid, input: &[usize]) -> bool {
        let free = env.inventory.items.iter().filter(|s| s.is_empty()).count();
        let mut pending: Vec<ItemStack> = Vec::new();
        for &i in input {
            let mut stack = grid.get(self, env, i).clone();
            if stack.is_empty() {
                continue;
            }
            if env.inventory.slot_with_remaining_space(&stack).is_some() {
                continue;
            }
            if free <= pending.len() {
                return false;
            }
            for p in pending.iter_mut() {
                if p.is_same_item(&stack) && p.count() != p.max_stack_size() && p.count() + stack.count() <= p.max_stack_size() {
                    p.grow_count(stack.count());
                    stack.set_count(0);
                    break;
                }
            }
            if !stack.is_empty() {
                if pending.len() >= free {
                    return false;
                }
                pending.push(stack);
            }
        }
        true
    }

    /// `clearGrid`: the grid's stacks go back into the inventory (dropped when they do not
    /// fit), then the grid and result are emptied.
    fn clear_grid(&mut self, env: &mut Env, grid: Grid, clear: &[usize]) {
        for &i in clear {
            let stack = match grid {
                Grid::Craft => self.craft.items[i].clone(),
                Grid::Furnace => env.block.as_deref().map_or_else(ItemStack::empty, |b| b.item(i).clone()),
            };
            if stack.is_empty() {
                continue;
            }
            let (_, left) = env.inventory.place_item_back(stack, false);
            if let Some(left) = left {
                env.out.push(crate::effect::Effect::Drop { stack: left, retain_ownership: false });
            }
            grid.set(self, env, i, ItemStack::empty());
        }
        if let Grid::Craft = grid {
            self.result.item = ItemStack::empty();
            for s in self.craft.items.iter_mut() {
                *s = ItemStack::empty();
            }
        }
    }

    /// `moveItemToGrid`: takes up to `count` of `item` from an inventory slot matching the grid
    /// slot's stack; returns what is still missing, or -1 when no slot has it.
    fn move_item_to_grid(&mut self, env: &mut Env, grid: Grid, slot: usize, item: i32, count: i32) -> i32 {
        let current = grid.get(self, env, slot).clone();
        let Some(from) = env.inventory.items.iter().position(|s| {
            simple(s) && s.item() == item && (current.is_empty() || crate::stack::same_item_same_components(s, &current))
        }) else {
            return -1;
        };
        let taken = {
            let s = &mut env.inventory.items[from];
            if count < s.count() { s.split_count(count) } else { std::mem::replace(s, ItemStack::empty()) }
        };
        let n = taken.count();
        if current.is_empty() {
            grid.set(self, env, slot, taken);
        } else {
            let mut grown = current;
            grown.grow_count(n);
            grid.set(self, env, slot, grown);
        }
        count - n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_recipes_are_centered() {
        let mut got = Vec::new();
        // A 1x2 stick recipe in a 3x3 grid.
        place_helper(3, 3, 1, 2, &[0, 1], |c, s| got.push((c, s)));
        assert_eq!(got, vec![(0, 1), (1, 4)]);
        got.clear();
        // A 2x2 recipe in a 2x2 grid fills it row by row.
        place_helper(2, 2, 2, 2, &[0, 1, 2, 3], |c, s| got.push((c, s)));
        assert_eq!(got, vec![(0, 0), (1, 1), (2, 2), (3, 3)]);
    }

    #[test]
    fn picking_shares_items_between_ingredients() {
        let plank = Ingredient { tag: None, items: vec![1, 2] };
        let mut s = Stacked::default();
        s.items = vec![(1, 3), (2, 1)];
        // Four planks of either kind: 3 + 1 fit one craft.
        let four = [&plank, &plank, &plank, &plank];
        assert_eq!(s.pick(&four, 1).unwrap(), vec![1, 1, 1, 2]);
        assert!(s.pick(&four, 2).is_none());
        assert_eq!(s.biggest(&four), 1);
    }
}
