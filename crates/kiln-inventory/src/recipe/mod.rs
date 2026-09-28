//! Recipes loaded from a datapack's `data/*/recipe/**.json` and vanilla's matching
//! (`RecipeManager`, first match in registry order).

pub mod book;
pub mod crafting;
pub mod ingredient;
pub mod input;
pub mod other;
pub mod special;
pub mod sync;

pub use crafting::{Shaped, Shapeless, Transmute, TransmuteResult};
pub use ingredient::Ingredient;
pub use input::CraftingInput;
pub use other::{Brewing, Cooking, CookingKind, PotionIngredient, SmithingTransform, SmithingTrim, Stonecutting};
pub use special::Special;

use crate::menu::World;
use crate::menus::FurnaceKind;
use crate::stack::{StackExt, create_checked};
use kiln_item::{ItemStack, ItemStackTemplate};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::path::Path;

/// A recipe of one of vanilla's serializers.
#[derive(Debug, Clone, PartialEq)]
pub enum Recipe {
    Shaped(Shaped),
    Shapeless(Shapeless),
    Transmute(Transmute),
    Special(Special),
    Cooking(Cooking),
    Stonecutting(Stonecutting),
    SmithingTransform(SmithingTransform),
    SmithingTrim(SmithingTrim),
    Brewing(Brewing),
}

impl Recipe {
    pub fn is_crafting(&self) -> bool {
        matches!(self, Recipe::Shaped(_) | Recipe::Shapeless(_) | Recipe::Transmute(_) | Recipe::Special(_))
    }

    /// `Recipe.isSpecial` (`CustomRecipe` and brewing recipes).
    pub fn is_special(&self) -> bool {
        match self {
            Recipe::Special(s) => s.is_special(),
            Recipe::Brewing(_) => true,
            _ => false,
        }
    }

    /// `CraftingRecipe.matches`.
    pub fn matches_crafting(&self, input: &CraftingInput, world: &dyn World) -> bool {
        match self {
            Recipe::Shaped(r) => r.matches(input),
            Recipe::Shapeless(r) => r.matches(input),
            Recipe::Transmute(r) => r.matches(input),
            Recipe::Special(r) => r.matches(input, world),
            _ => false,
        }
    }

    /// `CraftingRecipe.assemble`.
    pub fn assemble_crafting(&self, input: &CraftingInput) -> ItemStack {
        match self {
            Recipe::Shaped(r) => create_checked(&r.result),
            Recipe::Shapeless(r) => create_checked(&r.result),
            Recipe::Transmute(r) => r.assemble(input),
            Recipe::Special(r) => r.assemble(input),
            _ => ItemStack::empty(),
        }
    }

    /// `CraftingRecipe.getRemainingItems`.
    pub fn remaining_items(&self, input: &CraftingInput) -> Vec<ItemStack> {
        match self {
            Recipe::Special(r) => r.remaining_items(input),
            _ => default_remainders(input),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecipeHolder {
    /// `namespace:path`.
    pub id: String,
    pub recipe: Recipe,
    /// Category, group and notification for the recipe book.
    pub book: book::BookInfo,
}

/// A recipe file that did not load (vanilla logs these and skips the recipe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeError {
    pub id: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}: {source}")]
    Io { path: String, source: std::io::Error },
}

/// `RecipeManager`: every recipe in registry order.
#[derive(Debug, Clone, Default)]
pub struct RecipeManager {
    recipes: Vec<RecipeHolder>,
    crafting: Vec<usize>,
    by_id: HashMap<String, usize>,
    /// `RecipeManager.propertySets`, computed on first use.
    sets: std::sync::OnceLock<Vec<sync::PropertySet>>,
    /// `RecipeManager.allDisplays`, computed on first use.
    displays: std::sync::OnceLock<book::Displays>,
    /// Files that failed to parse.
    pub errors: Vec<RecipeError>,
}

impl RecipeManager {
    /// Loads `data/<namespace>/recipe/**/*.json` under `datapack`.
    pub fn load(datapack: &Path) -> Result<Self, LoadError> {
        Self::load_packs(&[datapack])
    }

    /// Loads the recipes of several packs in order: a later pack's file replaces an earlier
    /// one with the same id. The first pack must have a `data` directory.
    pub fn load_packs(packs: &[&Path]) -> Result<Self, LoadError> {
        let mut by_id: HashMap<(String, String), std::path::PathBuf> = HashMap::new();
        for (i, datapack) in packs.iter().enumerate() {
            let data = datapack.join("data");
            let namespaces = match std::fs::read_dir(&data) {
                Ok(n) => n,
                Err(e) if i == 0 => return Err(LoadError::Io { path: data.display().to_string(), source: e }),
                Err(_) => continue,
            };
            let mut files = Vec::new();
            for ns in namespaces.flatten() {
                let Some(namespace) = ns.file_name().to_str().map(str::to_owned) else { continue };
                let dir = ns.path().join("recipe");
                if dir.is_dir() {
                    collect_json(&dir, "", &namespace, &mut files)?;
                }
            }
            for (ns, path, file) in files {
                by_id.insert((ns, path), file);
            }
        }
        let mut files: Vec<(String, String, std::path::PathBuf)> =
            by_id.into_iter().map(|((ns, path), file)| (ns, path, file)).collect();
        // Resource listings are sorted by `Identifier.compareTo` (path first, then namespace),
        // and the recipe registry keeps that order.
        files.sort_by(|a, b| (a.1.as_str(), a.0.as_str()).cmp(&(b.1.as_str(), b.0.as_str())));
        let mut entries = Vec::with_capacity(files.len());
        for (namespace, path, file) in files {
            let text = std::fs::read_to_string(&file).map_err(|e| LoadError::Io { path: file.display().to_string(), source: e })?;
            entries.push((format!("{namespace}:{path}"), text));
        }
        Ok(Self::from_json_entries(entries.iter().map(|(id, t)| (id.as_str(), t.as_str()))))
    }

    /// Builds a manager from `(id, json)` pairs, in the given (registry) order.
    pub fn from_json_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut m = RecipeManager::default();
        for (id, text) in entries {
            let parsed = serde_json::from_str::<Json>(text)
                .map_err(|e| e.to_string())
                .and_then(|v| parse_recipe(&v).map(|r| r.map(|r| (book::BookInfo::from_json(&v, &r), r))));
            match parsed {
                Ok(Some((info, recipe))) => m.push_with_book(id.to_owned(), recipe, info),
                Ok(None) => {}
                Err(message) => m.errors.push(RecipeError { id: id.to_owned(), message }),
            }
        }
        m
    }

    pub fn push(&mut self, id: String, recipe: Recipe) {
        self.push_with_book(id, recipe, book::BookInfo::default());
    }

    pub fn push_with_book(&mut self, id: String, recipe: Recipe, info: book::BookInfo) {
        let i = self.recipes.len();
        if recipe.is_crafting() {
            self.crafting.push(i);
        }
        self.by_id.insert(id.clone(), i);
        self.recipes.push(RecipeHolder { id, recipe, book: info });
        self.sets = std::sync::OnceLock::new();
        self.displays = std::sync::OnceLock::new();
    }

    /// The recipe book displays of every recipe (`RecipeManager.allDisplays`).
    pub fn displays(&self) -> &book::Displays {
        self.displays.get_or_init(|| book::build(self))
    }

    /// `getRecipeFromDisplay`: the recipe a display id belongs to.
    pub fn recipe_of_display(&self, id: i32) -> Option<usize> {
        self.displays().entries.get(usize::try_from(id).ok()?).map(|e| e.recipe)
    }

    /// The recipe property sets (`RecipePropertySet`s): the items furnaces, smithing tables and
    /// brewing stands accept.
    pub fn property_sets(&self) -> &[sync::PropertySet] {
        self.sets.get_or_init(|| sync::property_sets(self))
    }

    pub fn len(&self) -> usize {
        self.recipes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.recipes.is_empty()
    }

    pub fn recipes(&self) -> &[RecipeHolder] {
        &self.recipes
    }

    pub fn get(&self, index: usize) -> &RecipeHolder {
        &self.recipes[index]
    }

    pub fn id(&self, index: usize) -> &str {
        &self.recipes[index].id
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    pub fn is_special(&self, index: usize) -> bool {
        self.recipes[index].recipe.is_special()
    }

    /// `getRecipeFor(CRAFTING, input, level, hint)`: the hint if it still matches, else the
    /// first crafting recipe that matches (none for an empty grid).
    pub fn find_crafting(&self, input: &CraftingInput, hint: Option<usize>, world: &dyn World) -> Option<usize> {
        if let Some(h) = hint
            && self.recipes[h].recipe.is_crafting()
            && self.recipes[h].recipe.matches_crafting(input, world)
        {
            return Some(h);
        }
        if input.is_empty() {
            return None;
        }
        self.crafting.iter().copied().find(|&i| self.recipes[i].recipe.matches_crafting(input, world))
    }

    /// `assemble` of a crafting recipe.
    pub fn assemble(&self, index: usize, input: &CraftingInput, _world: &dyn World) -> ItemStack {
        self.recipes[index].recipe.assemble_crafting(input)
    }

    /// The crafting result of a grid, as a crafting table computes it (no recipe book limit).
    pub fn craft(&self, input: &CraftingInput, world: &dyn World) -> ItemStack {
        self.find_crafting(input, None, world).map_or_else(ItemStack::empty, |i| self.assemble(i, input, world))
    }

    /// `ResultSlot.getRemainingItems`: the matching recipe's remainders, or (no recipe) a copy
    /// of every input.
    pub fn remaining_items(&self, input: &CraftingInput, world: &dyn World) -> Vec<ItemStack> {
        match self.find_crafting(input, None, world) {
            Some(i) => self.recipes[i].recipe.remaining_items(input),
            None => input.items().to_vec(),
        }
    }

    /// Stonecutter recipes in order (`RecipeManager.stonecutterRecipes`).
    pub fn stonecutter(&self) -> impl Iterator<Item = (usize, &Stonecutting)> {
        self.recipes.iter().enumerate().filter_map(|(i, h)| match &h.recipe {
            Recipe::Stonecutting(s) => Some((i, s)),
            _ => None,
        })
    }

    /// `SelectableRecipe.SingleInputSet.selectByInput`: the stonecutter recipes taking `input`.
    pub fn stonecutter_for(&self, input: &ItemStack) -> Vec<usize> {
        self.stonecutter().filter(|(_, s)| s.ingredient.test(input)).map(|(i, _)| i).collect()
    }

    /// `getRecipeFor(SMITHING, input)`: the first smithing recipe matching the three slots.
    pub fn find_smithing(&self, template: &ItemStack, base: &ItemStack, addition: &ItemStack) -> Option<usize> {
        if template.is_empty() && base.is_empty() && addition.is_empty() {
            return None;
        }
        self.recipes.iter().position(|h| match &h.recipe {
            Recipe::SmithingTransform(r) => r.matches(template, base, addition),
            Recipe::SmithingTrim(r) => r.matches(template, base, addition),
            _ => false,
        })
    }

    /// `SmithingRecipe.assemble`.
    pub fn assemble_smithing(&self, index: usize, base: &ItemStack, addition: &ItemStack) -> ItemStack {
        match &self.recipes[index].recipe {
            Recipe::SmithingTransform(r) => r.assemble(base),
            Recipe::SmithingTrim(r) => r.assemble(base, addition),
            _ => ItemStack::empty(),
        }
    }

    /// `CachedCheck.getRecipeFor` for furnaces and campfires: the first `kind` recipe taking
    /// the stack, trying `hint` (the check's last recipe) first.
    pub fn find_cooking(&self, kind: CookingKind, input: &ItemStack, hint: Option<usize>) -> Option<usize> {
        let fits = |h: &RecipeHolder| matches!(&h.recipe, Recipe::Cooking(c) if c.kind == kind && c.matches(input));
        self.find_single(!input.is_empty(), hint, fits)
    }

    /// `CachedCheck.getRecipeFor(BrewingInput)` for brewing stands.
    pub fn find_brewing(&self, input: &ItemStack, reagent: &ItemStack, hint: Option<usize>) -> Option<usize> {
        let fits = |h: &RecipeHolder| matches!(&h.recipe, Recipe::Brewing(b) if b.matches(input, reagent));
        self.find_single(!(input.is_empty() && reagent.is_empty()), hint, fits)
    }

    fn find_single(&self, has_input: bool, hint: Option<usize>, fits: impl Fn(&RecipeHolder) -> bool) -> Option<usize> {
        if !has_input {
            return None;
        }
        hint.filter(|&h| self.recipes.get(h).is_some_and(&fits)).or_else(|| self.recipes.iter().position(fits))
    }

    /// `assemble` of a cooking, stonecutting or brewing recipe (their results ignore the input).
    pub fn assemble_single(&self, index: usize) -> ItemStack {
        match &self.recipes[index].recipe {
            Recipe::Cooking(c) => create_checked(&c.result),
            Recipe::Stonecutting(s) => s.assemble(),
            Recipe::Brewing(b) => create_checked(&b.output),
            _ => ItemStack::empty(),
        }
    }

    /// Whether a recipe property set (`minecraft:smithing_base`, ...) holds the stack's item.
    pub fn property_set_accepts(&self, key: &str, stack: &ItemStack) -> bool {
        let item = stack.effective_item();
        self.property_sets().iter().any(|s| s.key == key && s.items.binary_search(&item).is_ok())
    }

    /// `AbstractFurnaceMenu.canSmelt`: the furnace's recipe property set (items some recipe of
    /// its type takes).
    pub fn furnace_accepts(&self, kind: FurnaceKind, stack: &ItemStack) -> bool {
        let key = match kind {
            FurnaceKind::Furnace => "minecraft:furnace_input",
            FurnaceKind::BlastFurnace => "minecraft:blast_furnace_input",
            FurnaceKind::Smoker => "minecraft:smoker_input",
        };
        self.property_set_accepts(key, stack)
    }
}

fn collect_json(dir: &Path, prefix: &str, namespace: &str, out: &mut Vec<(String, String, std::path::PathBuf)>) -> Result<(), LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|e| LoadError::Io { path: dir.display().to_string(), source: e })?;
    for e in entries.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if path.is_dir() {
            collect_json(&path, &format!("{prefix}{name}/"), namespace, out)?;
        } else if let Some(stem) = name.strip_suffix(".json") {
            out.push((namespace.to_owned(), format!("{prefix}{stem}"), path));
        }
    }
    Ok(())
}

/// Parses one recipe file; `Ok(None)` for serializers this crate does not model.
pub fn parse_recipe(v: &Json) -> Result<Option<Recipe>, String> {
    let kind = v.get("type").and_then(Json::as_str).ok_or("missing type")?;
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    Ok(Some(match kind {
        "crafting_shaped" => Recipe::Shaped(Shaped::from_json(v)?),
        "crafting_shapeless" => Recipe::Shapeless(Shapeless::from_json(v)?),
        "crafting_transmute" => Recipe::Transmute(Transmute::from_json(v)?),
        "smelting" | "blasting" | "smoking" | "campfire_cooking" => Recipe::Cooking(Cooking::from_json(kind, v)?),
        "stonecutting" => Recipe::Stonecutting(Stonecutting::from_json(v)?),
        "smithing_transform" => Recipe::SmithingTransform(SmithingTransform::from_json(v)?),
        "smithing_trim" => Recipe::SmithingTrim(SmithingTrim::from_json(v)?),
        "brewing" => Recipe::Brewing(Brewing::from_json(v)?),
        other => match Special::from_json(other, v)? {
            Some(s) => Recipe::Special(s),
            None => return Err(format!("unknown recipe type {other}")),
        },
    }))
}

/// `MinMaxBounds.Ints`: a number, or `{min, max}` with either bound optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub min: Option<i32>,
    pub max: Option<i32>,
}

impl Bounds {
    pub fn from_json(v: &Json) -> Result<Self, String> {
        if let Some(n) = v.as_i64() {
            return Ok(Bounds { min: Some(n as i32), max: Some(n as i32) });
        }
        let get = |k: &str| v.get(k).map(|x| x.as_i64().map(|n| n as i32).ok_or(format!("bound {k} is not an integer"))).transpose();
        let b = Bounds { min: get("min")?, max: get("max")? };
        if let (Some(a), Some(z)) = (b.min, b.max)
            && a > z
        {
            return Err("min greater than max".into());
        }
        Ok(b)
    }

    pub fn matches(&self, v: i32) -> bool {
        self.min.is_none_or(|m| v >= m) && self.max.is_none_or(|m| v <= m)
    }
}

/// `ItemStackTemplate.CODEC` from JSON.
pub(crate) fn template_from_json(v: &Json) -> Result<ItemStackTemplate, String> {
    ItemStackTemplate::from_value(&json_value(v)).map_err(|e| e.0)
}

/// JSON as a codec value (`JsonOps`): integers become ints (longs when too large), other
/// numbers doubles.
pub fn json_value(v: &Json) -> kiln_item::Value {
    use kiln_item::Value;
    match v {
        Json::Null => Value::Empty,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Value::Int(i as i32),
            Some(i) => Value::Long(i),
            None => Value::Double(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(s.clone()),
        Json::Array(a) => Value::List(a.iter().map(json_value).collect()),
        Json::Object(o) => Value::Map(o.iter().map(|(k, v)| (Value::String(k.clone()), json_value(v))).collect()),
    }
}

/// `Item.getCraftingRemainder` (`Item.Properties.craftRemainder` in `Items`).
pub fn crafting_remainder(item: i32) -> Option<ItemStack> {
    let name = kiln_item::registry::ITEM.name(item)?;
    let remainder = match name {
        "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:milk_bucket" => "minecraft:bucket",
        "minecraft:dragon_breath" | "minecraft:honey_bottle" => "minecraft:glass_bottle",
        _ => return None,
    };
    ItemStack::of(remainder, 1)
}

/// `CraftingRecipe.defaultCraftingReminder`.
pub fn default_remainders(input: &CraftingInput) -> Vec<ItemStack> {
    input.items().iter().map(|s| crafting_remainder(s.effective_item()).unwrap_or_default()).collect()
}
