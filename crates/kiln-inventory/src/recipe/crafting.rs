//! Normal crafting recipes: shaped, shapeless and transmute.

use super::ingredient::{Ingredient, item_id};
use super::input::CraftingInput;
use super::{Bounds, json_value, template_from_json};
use crate::stack::same_item_same_components;
use kiln_item::{DataComponentPatch, ItemStack, ItemStackTemplate};
use serde_json::Value as Json;

/// `ShapedRecipePattern` + result.
#[derive(Debug, Clone, PartialEq)]
pub struct Shaped {
    pub width: usize,
    pub height: usize,
    /// Row-major; `None` is an empty cell.
    pub ingredients: Vec<Option<Ingredient>>,
    pub result: ItemStackTemplate,
    ingredient_count: usize,
    symmetrical: bool,
}

impl Shaped {
    pub fn new(width: usize, height: usize, ingredients: Vec<Option<Ingredient>>, result: ItemStackTemplate) -> Self {
        let ingredient_count = ingredients.iter().flatten().count();
        let symmetrical = (0..height).all(|y| (0..width / 2).all(|x| ingredients[x + y * width] == ingredients[width - 1 - x + y * width]));
        Shaped { width, height, ingredients, result, ingredient_count, symmetrical }
    }

    /// `ShapedRecipePattern.Data` unpacking: `key` + `pattern`, trimmed of blank rows and columns.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let (width, height, ingredients) = pattern_from_json(v)?;
        let result = template_from_json(v.get("result").ok_or("missing result")?)?;
        Ok(Shaped::new(width, height, ingredients, result))
    }

    /// `ShapedRecipePattern.matches`: exact size and count, mirrored or not.
    pub fn matches(&self, input: &CraftingInput) -> bool {
        if input.ingredient_count() != self.ingredient_count || input.width() != self.width || input.height() != self.height {
            return false;
        }
        (!self.symmetrical && self.matches_mirrored(input, true)) || self.matches_mirrored(input, false)
    }

    fn matches_mirrored(&self, input: &CraftingInput, mirror: bool) -> bool {
        for y in 0..self.height {
            for x in 0..self.width {
                let ing = if mirror { &self.ingredients[self.width - x - 1 + y * self.width] } else { &self.ingredients[x + y * self.width] };
                let stack = input.get_xy(x, y);
                let ok = match ing {
                    Some(i) => i.test(stack),
                    None => stack.is_empty(),
                };
                if !ok {
                    return false;
                }
            }
        }
        true
    }
}

/// Parses `key` and `pattern` (`ShapedRecipePattern.Data.MAP_CODEC` + `unpack`).
pub(crate) fn pattern_from_json(v: &Json) -> Result<(usize, usize, Vec<Option<Ingredient>>), String> {
    let rows: Vec<&str> = v.get("pattern").and_then(Json::as_array).ok_or("missing pattern")?.iter().map(|r| r.as_str().ok_or("pattern row is not a string")).collect::<Result<_, _>>()?;
    if rows.is_empty() || rows.len() > 3 {
        return Err("invalid pattern size".into());
    }
    let len = rows[0].chars().count();
    if len == 0 || len > 3 || rows.iter().any(|r| r.chars().count() != len) {
        return Err("invalid pattern rows".into());
    }
    let key_json = v.get("key").and_then(Json::as_object).ok_or("missing key")?;
    let mut key = Vec::new();
    for (k, ing) in key_json {
        let mut chars = k.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else { return Err(format!("invalid key entry {k:?}")) };
        if c == ' ' {
            return Err("key ' ' is reserved".into());
        }
        key.push((c, Ingredient::from_json(ing)?));
    }
    let rows = shrink(&rows);
    let height = rows.len();
    let width = rows.first().map_or(0, |r| r.len());
    let mut unused: Vec<char> = key.iter().map(|(c, _)| *c).collect();
    let mut ingredients = Vec::with_capacity(width * height);
    for row in &rows {
        for &c in row {
            if c == ' ' {
                ingredients.push(None);
            } else {
                let ing = key.iter().find(|(k, _)| *k == c).map(|(_, i)| i.clone()).ok_or_else(|| format!("pattern references undefined symbol {c:?}"))?;
                unused.retain(|u| *u != c);
                ingredients.push(Some(ing));
            }
        }
    }
    if !unused.is_empty() {
        return Err(format!("key defines symbols that aren't used in pattern: {unused:?}"));
    }
    if width == 0 || height == 0 {
        return Err("empty pattern".into());
    }
    Ok((width, height, ingredients))
}

/// `ShapedRecipePattern.shrink`: drops blank leading/trailing rows and the columns blank in
/// every row.
fn shrink(rows: &[&str]) -> Vec<Vec<char>> {
    let rows: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
    let (mut first, mut last) = (usize::MAX, 0usize);
    let (mut lead, mut trail) = (0, 0);
    for (i, r) in rows.iter().enumerate() {
        let f = r.iter().position(|&c| c != ' ').unwrap_or(r.len());
        first = first.min(f);
        let l = r.iter().rposition(|&c| c != ' ');
        if let Some(l) = l {
            last = last.max(l);
            trail = 0;
        } else {
            if lead == i {
                lead += 1;
            }
            trail += 1;
        }
    }
    if rows.len() == trail {
        return Vec::new();
    }
    rows[lead..rows.len() - trail].iter().map(|r| r[first..=last].to_vec()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shapeless {
    pub ingredients: Vec<Ingredient>,
    pub result: ItemStackTemplate,
}

impl Shapeless {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let list = v.get("ingredients").and_then(Json::as_array).ok_or("missing ingredients")?;
        if list.is_empty() || list.len() > 9 {
            return Err("shapeless recipes take 1 to 9 ingredients".into());
        }
        let ingredients = list.iter().map(Ingredient::from_json).collect::<Result<_, _>>()?;
        Ok(Shapeless { ingredients, result: template_from_json(v.get("result").ok_or("missing result")?)? })
    }

    /// `ShapelessRecipe.matches`: one ingredient per non-empty slot, in any arrangement.
    pub fn matches(&self, input: &CraftingInput) -> bool {
        if input.ingredient_count() != self.ingredients.len() {
            return false;
        }
        if input.size() == 1 && self.ingredients.len() == 1 {
            return self.ingredients[0].test(input.get(0));
        }
        let items: Vec<i32> = input.stacks().map(|s| s.item()).collect();
        perfect_matching(&self.ingredients, &items)
    }
}

/// Whether every ingredient can take a distinct item (`StackedContents.canCraft`).
fn perfect_matching(ingredients: &[Ingredient], items: &[i32]) -> bool {
    fn augment(i: usize, ingredients: &[Ingredient], items: &[i32], seen: &mut [bool], owner: &mut [Option<usize>]) -> bool {
        for (j, &item) in items.iter().enumerate() {
            if seen[j] || !ingredients[i].accepts(item) {
                continue;
            }
            seen[j] = true;
            if owner[j].is_none_or(|k| augment(k, ingredients, items, seen, owner)) {
                owner[j] = Some(i);
                return true;
            }
        }
        false
    }
    let mut owner = vec![None; items.len()];
    (0..ingredients.len()).all(|i| augment(i, ingredients, items, &mut vec![false; items.len()], &mut owner))
}

/// `TransmuteResult`: an item (or the input's), a count and components applied on top of the
/// input's.
#[derive(Debug, Clone, PartialEq)]
pub struct TransmuteResult {
    pub item: Option<i32>,
    pub count: i32,
    pub components: DataComponentPatch,
}

impl TransmuteResult {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let item = v.get("id").and_then(Json::as_str).map(item_id).transpose()?;
        let count = v.get("count").map(|c| c.as_i64().ok_or("count is not an integer")).transpose()?.unwrap_or(1) as i32;
        if !(1..=99).contains(&count) {
            return Err("count out of range".into());
        }
        let components = match v.get("components") {
            Some(c) => DataComponentPatch::from_value_strict(&json_value(c)).map_err(|e| e.0)?,
            None => DataComponentPatch::new(),
        };
        Ok(TransmuteResult { item, count, components })
    }

    /// `resolve(item)`.
    pub fn resolve(&self, input_item: i32) -> ItemStackTemplate {
        ItemStackTemplate { item: self.item.unwrap_or(input_item), count: self.count, patch: self.components.clone() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transmute {
    pub input: Ingredient,
    pub material: Ingredient,
    pub material_count: Bounds,
    pub result: TransmuteResult,
    pub add_material_count_to_result: bool,
}

impl Transmute {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let material_count = match v.get("material_count") {
            Some(b) => Bounds::from_json(b)?,
            None => Bounds { min: Some(1), max: Some(1) },
        };
        if material_count.min.is_some_and(|m| m < 1) || material_count.max.is_some_and(|m| m > 8) {
            return Err("material_count outside [1, 8]".into());
        }
        Ok(Transmute {
            input: Ingredient::from_json(v.get("input").ok_or("missing input")?)?,
            material: Ingredient::from_json(v.get("material").ok_or("missing material")?)?,
            material_count,
            result: TransmuteResult::from_json(v.get("result").ok_or("missing result")?)?,
            add_material_count_to_result: v.get("add_material_count_to_result").and_then(Json::as_bool).unwrap_or(false),
        })
    }

    fn min(&self) -> usize {
        self.material_count.min.unwrap_or(1) as usize
    }

    fn max(&self) -> usize {
        self.material_count.max.unwrap_or(8) as usize
    }

    fn result_size(&self, materials: i32) -> i32 {
        if self.add_material_count_to_result { materials + self.result.count } else { self.result.count }
    }

    fn compute(&self, input: &ItemStack, extra: i32) -> ItemStack {
        with_original_components(&self.result.resolve(input.item()), input, extra)
    }

    pub fn matches(&self, input: &CraftingInput) -> bool {
        let n = input.ingredient_count();
        if n < self.min() + 1 || n > self.max() + 1 {
            return false;
        }
        let mut found: Option<&ItemStack> = None;
        let mut materials = 0;
        for s in input.stacks() {
            if self.input.test(s) {
                if found.is_some() {
                    return false;
                }
                found = Some(s);
            } else if self.material.test(s) {
                materials += 1;
                if materials > self.max() {
                    return false;
                }
            } else {
                return false;
            }
        }
        let Some(found) = found else { return false };
        if !self.material_count.matches(materials as i32) {
            return false;
        }
        if self.result_size(materials as i32) != 1 {
            return true;
        }
        let result = self.compute(found, 0);
        !result.is_empty() && !same_item_same_components(found, &result)
    }

    pub fn assemble(&self, input: &CraftingInput) -> ItemStack {
        if self.add_material_count_to_result {
            let mut materials = 0;
            let mut found = ItemStack::empty();
            for s in input.stacks() {
                if self.input.test(s) {
                    found = s.clone();
                } else if self.material.test(s) {
                    materials += 1;
                }
            }
            return self.compute(&found, materials);
        }
        match input.stacks().find(|s| self.input.test(s)) {
            Some(s) => self.compute(s, 0),
            None => ItemStack::empty(),
        }
    }
}

/// `TransmuteRecipe.createWithOriginalComponents(template, stack, extra)`: the template's item
/// with the stack's components, then the template's components applied on top.
pub fn with_original_components(template: &ItemStackTemplate, source: &ItemStack, extra: i32) -> ItemStack {
    let patch = if source.is_empty() { DataComponentPatch::new() } else { source.patch().clone() };
    apply_with_count(template, template.count + extra, patch)
}

/// `ItemStackTemplate.apply(patch)`: the template's item and count with `patch`, then the
/// template's components applied on top.
pub fn template_apply(template: &ItemStackTemplate, patch: DataComponentPatch) -> ItemStack {
    apply_with_count(template, template.count, patch)
}

/// `ItemStackTemplate.apply(count, patch)`: nothing when the stack is invalid (a stackable
/// damageable item, a count over the maximum...).
fn apply_with_count(template: &ItemStackTemplate, count: i32, patch: DataComponentPatch) -> ItemStack {
    let mut stack = ItemStack::from_parts(template.item, count, patch);
    apply_components(&mut stack, &template.patch);
    if crate::stack::is_valid_strict(&stack) { stack } else { ItemStack::empty() }
}

/// `ItemStack.applyComponents` (`PatchedDataComponentMap.applyPatch`).
pub fn apply_components(stack: &mut ItemStack, patch: &DataComponentPatch) {
    for (id, value) in patch.iter() {
        match value {
            Some(v) => stack.set(v.clone()),
            None => stack.remove(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_trims_blank_rows_and_columns() {
        assert_eq!(shrink(&[" # ", " # ", "   "]), vec![vec!['#'], vec!['#']]);
        assert_eq!(shrink(&["   ", "# #", "   "]), vec![vec!['#', ' ', '#']]);
        assert!(shrink(&["  ", "  "]).is_empty());
    }

    #[test]
    fn matching_needs_distinct_items() {
        let a = Ingredient::of_items(vec![1, 2]);
        let b = Ingredient::of_items(vec![1]);
        assert!(perfect_matching(&[a.clone(), b.clone()], &[2, 1]));
        assert!(perfect_matching(&[a.clone(), b.clone()], &[1, 2]));
        assert!(!perfect_matching(&[b.clone(), b], &[1, 2]));
    }
}
