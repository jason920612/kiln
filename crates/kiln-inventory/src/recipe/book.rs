//! The recipe book side of recipes: what the client is told about a recipe
//! (`RecipeDisplayEntry`: a display id, the `RecipeDisplay`, group, category and crafting
//! requirements) and the recipe book packets (`ClientboundRecipeBookAddPacket`,
//! `...RemovePacket`, `...SettingsPacket`, `ClientboundPlaceGhostRecipePacket`).
//!
//! Display ids number the displays of every recipe in registry order
//! (`RecipeManager.unpackRecipeInfo`); special recipes have no display and are never in a
//! recipe book.

use super::{Ingredient, Recipe, RecipeManager};
use bytes::{Bytes, BytesMut};
use kiln_item::ItemStackTemplate;
use kiln_proto::WriteExt;
use serde_json::Value as Json;

/// What a recipe file says about its place in the recipe book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookInfo {
    /// `minecraft:recipe_book_category` entry (`Recipe.recipeBookCategory`).
    pub category: &'static str,
    /// `group`: recipes of a group share a book entry.
    pub group: String,
    /// `show_notification`: a toast when unlocked.
    pub show_notification: bool,
}

impl Default for BookInfo {
    fn default() -> Self {
        BookInfo { category: "minecraft:crafting_misc", group: String::new(), show_notification: true }
    }
}

impl BookInfo {
    /// From the recipe file: `category` (`CraftingBookCategory` or `CookingBookCategory`,
    /// `misc` by default), `group` and `show_notification`.
    pub fn from_json(v: &Json, recipe: &Recipe) -> BookInfo {
        let category_field = v.get("category").and_then(Json::as_str).unwrap_or("misc");
        let category = match recipe {
            Recipe::Cooking(c) => match (c.kind, category_field) {
                (super::CookingKind::Smelting, "food") => "minecraft:furnace_food",
                (super::CookingKind::Smelting, "blocks") => "minecraft:furnace_blocks",
                (super::CookingKind::Smelting, _) => "minecraft:furnace_misc",
                (super::CookingKind::Blasting, "blocks") => "minecraft:blast_furnace_blocks",
                (super::CookingKind::Blasting, _) => "minecraft:blast_furnace_misc",
                (super::CookingKind::Smoking, _) => "minecraft:smoker_food",
                (super::CookingKind::Campfire, _) => "minecraft:campfire",
            },
            Recipe::Stonecutting(_) => "minecraft:stonecutter",
            Recipe::SmithingTransform(_) | Recipe::SmithingTrim(_) => "minecraft:smithing",
            _ => match category_field {
                "building" => "minecraft:crafting_building_blocks",
                "redstone" => "minecraft:crafting_redstone",
                "equipment" => "minecraft:crafting_equipment",
                _ => "minecraft:crafting_misc",
            },
        };
        BookInfo {
            category,
            group: v.get("group").and_then(Json::as_str).unwrap_or("").to_owned(),
            show_notification: v.get("show_notification").and_then(Json::as_bool).unwrap_or(true),
        }
    }
}

/// `SlotDisplay`: how the book shows one slot.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotDisplay<'a> {
    Empty,
    AnyFuel,
    WithAnyPotion(Box<SlotDisplay<'a>>),
    OnlyWithComponent(Box<SlotDisplay<'a>>, &'static str),
    Item(i32),
    Stack(ItemStackTemplate),
    /// `TagSlotDisplay`: an ingredient's holder set (`Ingredient.display`).
    Tag(&'a Ingredient),
    Dyed(Box<SlotDisplay<'a>>, Box<SlotDisplay<'a>>),
    SmithingTrim(Box<SlotDisplay<'a>>, Box<SlotDisplay<'a>>, i32),
}

impl SlotDisplay<'_> {
    fn type_id(&self) -> i32 {
        let name = match self {
            SlotDisplay::Empty => "minecraft:empty",
            SlotDisplay::AnyFuel => "minecraft:any_fuel",
            SlotDisplay::WithAnyPotion(_) => "minecraft:with_any_potion",
            SlotDisplay::OnlyWithComponent(..) => "minecraft:only_with_component",
            SlotDisplay::Item(_) => "minecraft:item",
            SlotDisplay::Stack(_) => "minecraft:item_stack",
            SlotDisplay::Tag(_) => "minecraft:tag",
            SlotDisplay::Dyed(..) => "minecraft:dyed",
            SlotDisplay::SmithingTrim(..) => "minecraft:smithing_trim",
        };
        kiln_data::builtin_id("minecraft:slot_display", name).expect("slot display type")
    }

    /// `SlotDisplay.STREAM_CODEC`.
    pub fn write(&self, b: &mut BytesMut) {
        b.put_varint(self.type_id());
        match self {
            SlotDisplay::Empty | SlotDisplay::AnyFuel => {}
            SlotDisplay::WithAnyPotion(inner) => inner.write(b),
            SlotDisplay::OnlyWithComponent(inner, component) => {
                inner.write(b);
                b.put_varint(kiln_data::builtin_id("minecraft:data_component_type", component).unwrap_or(0));
            }
            SlotDisplay::Item(item) => b.put_varint(*item),
            SlotDisplay::Stack(t) => t.write(b),
            SlotDisplay::Tag(ing) => super::sync::write_ingredient(ing, b),
            SlotDisplay::Dyed(dye, target) => {
                dye.write(b);
                target.write(b);
            }
            SlotDisplay::SmithingTrim(base, material, pattern) => {
                base.write(b);
                material.write(b);
                // A registry reference holder: id + 1.
                b.put_varint(*pattern + 1);
            }
        }
    }
}

fn item(name: &str) -> SlotDisplay<'static> {
    SlotDisplay::Item(kiln_item::registry::ITEM.id(name).unwrap_or(0))
}

fn tag(i: &Ingredient) -> SlotDisplay<'_> {
    SlotDisplay::Tag(i)
}

fn opt_tag(i: &Option<Ingredient>) -> SlotDisplay<'_> {
    i.as_ref().map_or(SlotDisplay::Empty, SlotDisplay::Tag)
}

/// `RecipeDisplay`.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeDisplay<'a> {
    Shapeless { ingredients: Vec<SlotDisplay<'a>>, result: SlotDisplay<'a>, station: SlotDisplay<'a> },
    Shaped { width: i32, height: i32, ingredients: Vec<SlotDisplay<'a>>, result: SlotDisplay<'a>, station: SlotDisplay<'a> },
    Furnace { ingredient: SlotDisplay<'a>, fuel: SlotDisplay<'a>, result: SlotDisplay<'a>, station: SlotDisplay<'a>, duration: i32, experience: f32 },
    Stonecutter { input: SlotDisplay<'a>, result: SlotDisplay<'a>, station: SlotDisplay<'a> },
    Smithing { template: SlotDisplay<'a>, base: SlotDisplay<'a>, addition: SlotDisplay<'a>, result: SlotDisplay<'a>, station: SlotDisplay<'a> },
}

impl RecipeDisplay<'_> {
    /// `RecipeDisplay.STREAM_CODEC`.
    pub fn write(&self, b: &mut BytesMut) {
        let name = match self {
            RecipeDisplay::Shapeless { .. } => "minecraft:crafting_shapeless",
            RecipeDisplay::Shaped { .. } => "minecraft:crafting_shaped",
            RecipeDisplay::Furnace { .. } => "minecraft:furnace",
            RecipeDisplay::Stonecutter { .. } => "minecraft:stonecutter",
            RecipeDisplay::Smithing { .. } => "minecraft:smithing",
        };
        b.put_varint(kiln_data::builtin_id("minecraft:recipe_display", name).expect("recipe display type"));
        let list = |b: &mut BytesMut, l: &[SlotDisplay]| {
            b.put_varint(l.len() as i32);
            for s in l {
                s.write(b);
            }
        };
        match self {
            RecipeDisplay::Shapeless { ingredients, result, station } => {
                list(b, ingredients);
                result.write(b);
                station.write(b);
            }
            RecipeDisplay::Shaped { width, height, ingredients, result, station } => {
                b.put_varint(*width);
                b.put_varint(*height);
                list(b, ingredients);
                result.write(b);
                station.write(b);
            }
            RecipeDisplay::Furnace { ingredient, fuel, result, station, duration, experience } => {
                ingredient.write(b);
                fuel.write(b);
                result.write(b);
                station.write(b);
                b.put_varint(*duration);
                bytes::BufMut::put_f32(b, *experience);
            }
            RecipeDisplay::Stonecutter { input, result, station } => {
                input.write(b);
                result.write(b);
                station.write(b);
            }
            RecipeDisplay::Smithing { template, base, addition, result, station } => {
                template.write(b);
                base.write(b);
                addition.write(b);
                result.write(b);
                station.write(b);
            }
        }
    }

    /// The encoded display.
    pub fn bytes(&self) -> Bytes {
        let mut b = BytesMut::with_capacity(64);
        self.write(&mut b);
        b.freeze()
    }
}

/// `Recipe.display()`: the displays of a recipe (none for special recipes and brewing).
pub fn displays(recipe: &Recipe) -> Vec<RecipeDisplay<'_>> {
    let ct = || item("minecraft:crafting_table");
    match recipe {
        Recipe::Shaped(s) => vec![RecipeDisplay::Shaped {
            width: s.width as i32,
            height: s.height as i32,
            ingredients: s.ingredients.iter().map(opt_tag).collect(),
            result: SlotDisplay::Stack(s.result.clone()),
            station: ct(),
        }],
        Recipe::Shapeless(s) => vec![RecipeDisplay::Shapeless {
            ingredients: s.ingredients.iter().map(tag).collect(),
            result: SlotDisplay::Stack(s.result.clone()),
            station: ct(),
        }],
        Recipe::Transmute(t) => {
            let (min, max) = (t.material_count.min.unwrap_or(1).max(1), t.material_count.max.unwrap_or(1).max(1));
            (min..=max)
                .map(|n| {
                    let mut ingredients = vec![tag(&t.input)];
                    ingredients.extend((0..n).map(|_| tag(&t.material)));
                    let count = t.result.count + if t.add_material_count_to_result { n } else { 0 };
                    let result = match t.result.item {
                        Some(item) => SlotDisplay::Stack(ItemStackTemplate { item, count, patch: t.result.components.clone() }),
                        None => tag(&t.input),
                    };
                    RecipeDisplay::Shapeless { ingredients, result, station: ct() }
                })
                .collect()
        }
        Recipe::Special(super::Special::Dye { target, dye, .. }) => {
            let dye = SlotDisplay::OnlyWithComponent(Box::new(tag(dye)), "minecraft:dye");
            let target = tag(target);
            vec![RecipeDisplay::Shapeless {
                ingredients: vec![target.clone(), dye.clone()],
                result: SlotDisplay::Dyed(Box::new(dye), Box::new(target)),
                station: ct(),
            }]
        }
        Recipe::Special(super::Special::Imbue { source, material, result }) => {
            let m = tag(material);
            let s = SlotDisplay::WithAnyPotion(Box::new(tag(source)));
            vec![RecipeDisplay::Shaped {
                width: 3,
                height: 3,
                ingredients: vec![m.clone(), m.clone(), m.clone(), m.clone(), s, m.clone(), m.clone(), m.clone(), m],
                result: SlotDisplay::WithAnyPotion(Box::new(SlotDisplay::Stack(result.clone()))),
                station: ct(),
            }]
        }
        Recipe::Special(_) | Recipe::Brewing(_) => Vec::new(),
        Recipe::Cooking(c) => {
            let station = match c.kind {
                super::CookingKind::Smelting => "minecraft:furnace",
                super::CookingKind::Blasting => "minecraft:blast_furnace",
                super::CookingKind::Smoking => "minecraft:smoker",
                super::CookingKind::Campfire => "minecraft:campfire",
            };
            vec![RecipeDisplay::Furnace {
                ingredient: tag(&c.ingredient),
                fuel: SlotDisplay::AnyFuel,
                result: SlotDisplay::Stack(c.result.clone()),
                station: item(station),
                duration: c.cooking_time,
                experience: c.experience,
            }]
        }
        Recipe::Stonecutting(s) => vec![RecipeDisplay::Stonecutter {
            input: tag(&s.ingredient),
            result: SlotDisplay::Stack(s.result.clone()),
            station: item("minecraft:stonecutter"),
        }],
        Recipe::SmithingTransform(s) => vec![RecipeDisplay::Smithing {
            template: opt_tag(&s.template),
            base: tag(&s.base),
            addition: opt_tag(&s.addition),
            result: SlotDisplay::Stack(s.result.clone()),
            station: item("minecraft:smithing_table"),
        }],
        Recipe::SmithingTrim(s) => vec![RecipeDisplay::Smithing {
            template: tag(&s.template),
            base: tag(&s.base),
            addition: tag(&s.addition),
            result: SlotDisplay::SmithingTrim(Box::new(tag(&s.base)), Box::new(tag(&s.addition)), s.pattern),
            station: item("minecraft:smithing_table"),
        }],
    }
}

/// `Recipe.placementInfo().ingredients()`: the ingredients the book checks the inventory for
/// (none when an ingredient can never match: `PlacementInfo.NOT_PLACEABLE`).
pub fn placement_ingredients(recipe: &Recipe) -> Vec<&Ingredient> {
    let list: Vec<&Ingredient> = match recipe {
        Recipe::Shaped(s) => s.ingredients.iter().flatten().collect(),
        Recipe::Shapeless(s) => s.ingredients.iter().collect(),
        Recipe::Transmute(t) => vec![&t.input, &t.material],
        Recipe::Special(super::Special::Dye { target, dye, .. }) => vec![target, dye],
        Recipe::Special(super::Special::Imbue { source, material, .. }) => {
            vec![material, material, material, material, source, material, material, material, material]
        }
        Recipe::Special(_) | Recipe::Brewing(_) => Vec::new(),
        Recipe::Cooking(c) => vec![&c.ingredient],
        Recipe::Stonecutting(s) => vec![&s.ingredient],
        Recipe::SmithingTransform(s) => [s.template.as_ref(), Some(&s.base), s.addition.as_ref()].into_iter().flatten().collect(),
        Recipe::SmithingTrim(s) => vec![&s.template, &s.base, &s.addition],
    };
    if list.iter().any(|i| i.items.is_empty()) { Vec::new() } else { list }
}

/// One display of a recipe, with everything the book packets need.
#[derive(Debug, Clone)]
pub struct DisplayEntry {
    /// `RecipeDisplayId`.
    pub id: i32,
    /// Index of the recipe in the manager.
    pub recipe: usize,
    /// The encoded `RecipeDisplay` (for the ghost recipe packet).
    pub display: Bytes,
    /// The encoded `RecipeDisplayEntry` after the id: display, group, category, requirements.
    pub body: Bytes,
}

/// Every display of every recipe (`RecipeManager.allDisplays`), and per recipe its display ids.
#[derive(Debug, Clone, Default)]
pub struct Displays {
    pub entries: Vec<DisplayEntry>,
    pub by_recipe: Vec<Vec<i32>>,
}

/// `RecipeManager.unpackRecipeInfo`: groups get ids in first-seen order.
pub fn build(m: &RecipeManager) -> Displays {
    let mut out = Displays { entries: Vec::new(), by_recipe: vec![Vec::new(); m.len()] };
    let mut groups: Vec<&str> = Vec::new();
    for (i, h) in m.recipes().iter().enumerate() {
        if h.recipe.is_special() {
            continue;
        }
        let group = (!h.book.group.is_empty()).then(|| match groups.iter().position(|g| *g == h.book.group) {
            Some(g) => g,
            None => {
                groups.push(&h.book.group);
                groups.len() - 1
            }
        });
        for d in displays(&h.recipe) {
            let id = out.entries.len() as i32;
            let display = d.bytes();
            let mut body = BytesMut::with_capacity(display.len() + 16);
            body.extend_from_slice(&display);
            // `ByteBufCodecs.OPTIONAL_VAR_INT`: 0 for none, else the value + 1.
            body.put_varint(group.map_or(0, |g| g as i32 + 1));
            body.put_varint(kiln_data::builtin_id("minecraft:recipe_book_category", h.book.category).unwrap_or(0));
            let requirements = placement_ingredients(&h.recipe);
            body.put_bool(true);
            body.put_varint(requirements.len() as i32);
            for ing in requirements {
                super::sync::write_ingredient(ing, &mut body);
            }
            out.by_recipe[i].push(id);
            out.entries.push(DisplayEntry { id, recipe: i, display, body: body.freeze() });
        }
    }
    out
}

/// `ClientboundRecipeBookAddPacket`: entries (display id, notification flag, highlight flag)
/// and whether the client's book is replaced.
pub fn recipe_book_add(displays: &Displays, entries: &[(i32, bool, bool)], replace: bool) -> Bytes {
    let mut b = BytesMut::with_capacity(64 + entries.len() * 32);
    b.put_varint(kiln_data::packets::play::clientbound::RECIPE_BOOK_ADD);
    b.put_varint(entries.len() as i32);
    for &(id, notification, highlight) in entries {
        let e = &displays.entries[id as usize];
        b.put_varint(e.id);
        b.extend_from_slice(&e.body);
        bytes::BufMut::put_u8(&mut b, notification as u8 | (highlight as u8) << 1);
    }
    b.put_bool(replace);
    b.freeze()
}

/// `ClientboundRecipeBookRemovePacket`.
pub fn recipe_book_remove(ids: &[i32]) -> Bytes {
    let mut b = BytesMut::with_capacity(8 + ids.len() * 3);
    b.put_varint(kiln_data::packets::play::clientbound::RECIPE_BOOK_REMOVE);
    b.put_varint(ids.len() as i32);
    for &id in ids {
        b.put_varint(id);
    }
    b.freeze()
}

/// `ClientboundRecipeBookSettingsPacket`: (open, filtering) for the crafting, furnace, blast
/// furnace and smoker books.
pub fn recipe_book_settings(settings: &[(bool, bool); 4]) -> Bytes {
    let mut b = BytesMut::with_capacity(10);
    b.put_varint(kiln_data::packets::play::clientbound::RECIPE_BOOK_SETTINGS);
    for &(open, filtering) in settings {
        b.put_bool(open);
        b.put_bool(filtering);
    }
    b.freeze()
}

/// `ClientboundPlaceGhostRecipePacket`.
pub fn place_ghost_recipe(container_id: i32, display: &Bytes) -> Bytes {
    let mut b = BytesMut::with_capacity(8 + display.len());
    b.put_varint(kiln_data::packets::play::clientbound::PLACE_GHOST_RECIPE);
    b.put_varint(container_id);
    b.extend_from_slice(display);
    b.freeze()
}
