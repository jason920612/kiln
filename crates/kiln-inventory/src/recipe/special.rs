//! The data-driven special crafting recipes (`CustomRecipe` subclasses, plus dyeing and
//! imbuing), each with the fields its JSON names.

use super::crafting::{Shaped, TransmuteResult, template_apply, with_original_components};
use super::ingredient::Ingredient;
use super::input::CraftingInput;
use super::{Bounds, crafting_remainder, template_from_json};
use crate::menu::World;
use kiln_item::component::{
    BannerPatternLayers, DyeColor, DyedColor, FireworkExplosion, FireworkShape, Fireworks, PotDecorations, ids,
};
use kiln_item::{DataComponentPatch, ItemStack, ItemStackTemplate, keys};
use serde_json::Value as Json;

#[derive(Debug, Clone, PartialEq)]
pub enum Special {
    /// `crafting_special_bannerduplicate`.
    BannerDuplicate { banner: Ingredient, result: ItemStackTemplate },
    /// `crafting_special_bookcloning`.
    BookCloning { source: Ingredient, material: Ingredient, allowed_generations: Bounds, result: ItemStackTemplate },
    /// `crafting_decorated_pot`.
    DecoratedPot { back: Ingredient, left: Ingredient, right: Ingredient, front: Ingredient, result: ItemStackTemplate },
    /// `crafting_dye` (dyeable armor): not special.
    Dye { target: Ingredient, dye: Ingredient, result: ItemStackTemplate },
    /// `crafting_special_firework_rocket`.
    FireworkRocket { shell: Ingredient, fuel: Ingredient, star: Ingredient, result: ItemStackTemplate },
    /// `crafting_special_firework_star`.
    FireworkStar { shapes: Vec<(FireworkShape, Ingredient)>, trail: Ingredient, twinkle: Ingredient, fuel: Ingredient, dye: Ingredient, result: ItemStackTemplate },
    /// `crafting_special_firework_star_fade`.
    FireworkStarFade { target: Ingredient, dye: Ingredient, result: ItemStackTemplate },
    /// `crafting_imbue` (tipped arrows): not special.
    Imbue { source: Ingredient, material: Ingredient, result: ItemStackTemplate },
    /// `crafting_special_mapextending`.
    MapExtending { map: Ingredient, pattern: Shaped, result: TransmuteResult },
    /// `crafting_special_repairitem`.
    RepairItem,
    /// `crafting_special_shielddecoration`.
    ShieldDecoration { banner: Ingredient, target: Ingredient, result: ItemStackTemplate },
}

fn ing(v: &Json, field: &str) -> Result<Ingredient, String> {
    Ingredient::from_json(v.get(field).ok_or_else(|| format!("missing {field}"))?)
}

fn result(v: &Json) -> Result<ItemStackTemplate, String> {
    template_from_json(v.get("result").ok_or("missing result")?)
}

impl Special {
    pub fn from_json(kind: &str, v: &Json) -> Result<Option<Self>, String> {
        Ok(Some(match kind {
            "crafting_special_bannerduplicate" => Special::BannerDuplicate { banner: ing(v, "banner")?, result: result(v)? },
            "crafting_special_bookcloning" => Special::BookCloning {
                source: ing(v, "source")?,
                material: ing(v, "material")?,
                allowed_generations: match v.get("allowed_generations") {
                    Some(b) => Bounds::from_json(b)?,
                    None => Bounds { min: Some(0), max: Some(1) },
                },
                result: result(v)?,
            },
            "crafting_decorated_pot" => Special::DecoratedPot {
                back: ing(v, "back")?,
                left: ing(v, "left")?,
                right: ing(v, "right")?,
                front: ing(v, "front")?,
                result: result(v)?,
            },
            "crafting_dye" => Special::Dye { target: ing(v, "target")?, dye: ing(v, "dye")?, result: result(v)? },
            "crafting_special_firework_rocket" => {
                Special::FireworkRocket { shell: ing(v, "shell")?, fuel: ing(v, "fuel")?, star: ing(v, "star")?, result: result(v)? }
            }
            "crafting_special_firework_star" => {
                let shapes_json = v.get("shapes").and_then(Json::as_object).ok_or("missing shapes")?;
                let mut shapes = Vec::new();
                for (name, i) in shapes_json {
                    let shape = FireworkShape::from_name(name).ok_or_else(|| format!("unknown firework shape {name}"))?;
                    shapes.push((shape, Ingredient::from_json(i)?));
                }
                Special::FireworkStar {
                    shapes,
                    trail: ing(v, "trail")?,
                    twinkle: ing(v, "twinkle")?,
                    fuel: ing(v, "fuel")?,
                    dye: ing(v, "dye")?,
                    result: result(v)?,
                }
            }
            "crafting_special_firework_star_fade" => Special::FireworkStarFade { target: ing(v, "target")?, dye: ing(v, "dye")?, result: result(v)? },
            "crafting_imbue" => Special::Imbue { source: ing(v, "source")?, material: ing(v, "material")?, result: result(v)? },
            "crafting_special_mapextending" => {
                let map = ing(v, "map")?;
                let material = ing(v, "material")?;
                let mut cells = vec![Some(material); 9];
                cells[4] = Some(map.clone());
                let pattern = Shaped::new(3, 3, cells, ItemStackTemplate::new(kiln_item::stack::air(), 1));
                let result = TransmuteResult::from_json(v.get("result").ok_or("missing result")?)?;
                Special::MapExtending { map, pattern, result }
            }
            "crafting_special_repairitem" => Special::RepairItem,
            "crafting_special_shielddecoration" => Special::ShieldDecoration { banner: ing(v, "banner")?, target: ing(v, "target")?, result: result(v)? },
            _ => return Ok(None),
        }))
    }

    /// `Recipe.isSpecial`: every `CustomRecipe` (not dyeing or imbuing).
    pub fn is_special(&self) -> bool {
        !matches!(self, Special::Dye { .. } | Special::Imbue { .. })
    }

    pub fn matches(&self, input: &CraftingInput, world: &dyn World) -> bool {
        match self {
            Special::BannerDuplicate { banner, .. } => {
                if input.ingredient_count() != 2 {
                    return false;
                }
                let mut color = None;
                let (mut blank, mut patterned) = (false, false);
                for s in input.stacks() {
                    let Some(c) = banner_color(s).filter(|_| banner.test(s)) else { return false };
                    if color.is_some_and(|k| k != c) {
                        return false;
                    }
                    color = Some(c);
                    let layers = banner_layers(s);
                    if layers > 6 {
                        return false;
                    }
                    if layers > 0 {
                        if patterned {
                            return false;
                        }
                        patterned = true;
                    } else {
                        if blank {
                            return false;
                        }
                        blank = true;
                    }
                }
                patterned && blank
            }
            Special::BookCloning { source, material, allowed_generations, .. } => {
                if input.ingredient_count() < 2 {
                    return false;
                }
                let (mut book, mut mat) = (false, false);
                for s in input.stacks() {
                    if source.test(s) {
                        match s.get(keys::WRITTEN_BOOK_CONTENT) {
                            Some(c) if allowed_generations.matches(c.generation) => {}
                            _ => return false,
                        }
                        if book {
                            return false;
                        }
                        book = true;
                    } else if material.test(s) {
                        mat = true;
                    } else {
                        return false;
                    }
                }
                book && mat
            }
            Special::DecoratedPot { back, left, right, front, .. } => {
                input.width() == 3
                    && input.height() == 3
                    && input.ingredient_count() == 4
                    && back.test(input.get_xy(1, 0))
                    && left.test(input.get_xy(0, 1))
                    && right.test(input.get_xy(2, 1))
                    && front.test(input.get_xy(1, 2))
            }
            Special::Dye { target, dye, .. } => {
                if input.ingredient_count() < 2 {
                    return false;
                }
                let (mut found, mut dyed) = (false, false);
                for s in input.stacks() {
                    if target.test(s) {
                        if found {
                            return false;
                        }
                        found = true;
                    } else if dye.test(s) && s.has(ids::DYE) {
                        dyed = true;
                    } else {
                        return false;
                    }
                }
                found && dyed
            }
            Special::FireworkRocket { shell, fuel, star, .. } => {
                if input.ingredient_count() < 2 {
                    return false;
                }
                let (mut has_shell, mut fuels) = (false, 0);
                for s in input.stacks() {
                    if shell.test(s) {
                        if has_shell {
                            return false;
                        }
                        has_shell = true;
                    } else if fuel.test(s) {
                        fuels += 1;
                        if fuels > 3 {
                            return false;
                        }
                    } else if !star.test(s) {
                        return false;
                    }
                }
                has_shell && fuels >= 1
            }
            Special::FireworkStar { shapes, trail, twinkle, fuel, dye, .. } => {
                if input.ingredient_count() < 2 {
                    return false;
                }
                let (mut has_fuel, mut has_dye, mut has_shape, mut has_trail, mut has_twinkle) = (false, false, false, false, false);
                for s in input.stacks() {
                    let flag = if twinkle.test(s) {
                        &mut has_twinkle
                    } else if trail.test(s) {
                        &mut has_trail
                    } else if fuel.test(s) {
                        &mut has_fuel
                    } else if dye.test(s) && s.has(ids::DYE) {
                        has_dye = true;
                        continue;
                    } else if find_shape(shapes, s).is_some() {
                        &mut has_shape
                    } else {
                        return false;
                    };
                    if *flag {
                        return false;
                    }
                    *flag = true;
                }
                has_fuel && has_dye
            }
            Special::FireworkStarFade { target, dye, .. } => {
                if input.ingredient_count() < 2 {
                    return false;
                }
                let (mut dyed, mut found) = (false, false);
                for s in input.stacks() {
                    if dye.test(s) && s.has(ids::DYE) {
                        dyed = true;
                    } else if target.test(s) {
                        if found {
                            return false;
                        }
                        found = true;
                    } else {
                        return false;
                    }
                }
                dyed && found
            }
            Special::Imbue { source, material, .. } => {
                if input.width() != 3 || input.height() != 3 || input.ingredient_count() != 9 {
                    return false;
                }
                (0..3).all(|y| {
                    (0..3).all(|x| {
                        let s = input.get_xy(x, y);
                        !s.is_empty() && if x == 1 && y == 1 { source.test(s) } else { material.test(s) }
                    })
                })
            }
            Special::MapExtending { pattern, .. } => {
                if !pattern.matches(input) {
                    return false;
                }
                let Some(map) = find_filled_map(input) else { return false };
                let Some(id) = map.get(keys::MAP_ID) else { return false };
                world.map_scale(id.0).is_some_and(|scale| scale < 4)
            }
            Special::RepairItem => items_to_combine(input).is_some(),
            Special::ShieldDecoration { banner, target, .. } => {
                if input.ingredient_count() != 2 {
                    return false;
                }
                let (mut has_target, mut has_banner) = (false, false);
                for s in input.stacks() {
                    if banner.test(s) && banner_color(s).is_some() {
                        if has_banner {
                            return false;
                        }
                        has_banner = true;
                    } else if target.test(s) {
                        if has_target || banner_layers(s) > 0 {
                            return false;
                        }
                        has_target = true;
                    } else {
                        return false;
                    }
                }
                has_target && has_banner
            }
        }
    }

    pub fn assemble(&self, input: &CraftingInput) -> ItemStack {
        match self {
            Special::BannerDuplicate { result, .. } => input
                .stacks()
                .find(|s| (1..=6).contains(&banner_layers(s)))
                .map_or_else(ItemStack::empty, |s| with_original_components(result, s, 0)),
            Special::BookCloning { source, material, result, .. } => {
                let mut materials = 0;
                let mut book: Option<&ItemStack> = None;
                for s in input.stacks() {
                    if source.test(s) && s.has(ids::WRITTEN_BOOK_CONTENT) {
                        if book.is_some() {
                            return ItemStack::empty();
                        }
                        book = Some(s);
                    } else if material.test(s) {
                        materials += 1;
                    } else {
                        return ItemStack::empty();
                    }
                }
                let Some(book) = book else { return ItemStack::empty() };
                let Some(content) = book.get(keys::WRITTEN_BOOK_CONTENT) else { return ItemStack::empty() };
                let mut copy = content.clone();
                copy.generation += 1;
                let mut out = with_original_components(result, book, materials - 1);
                out.insert(keys::WRITTEN_BOOK_CONTENT, copy);
                out
            }
            Special::DecoratedPot { result, .. } => {
                let side = |s: &ItemStack| (!s.is_empty()).then(|| ItemStackTemplate { item: s.item(), count: 1, patch: s.patch().clone() });
                let decorations = PotDecorations {
                    back: side(input.get_xy(1, 0)),
                    left: side(input.get_xy(0, 1)),
                    right: side(input.get_xy(2, 1)),
                    front: side(input.get_xy(1, 2)),
                };
                let mut patch = DataComponentPatch::new();
                patch.set(keys::POT_DECORATIONS.wrap(decorations));
                template_apply(result, patch)
            }
            Special::Dye { target, dye, result } => {
                let mut colors = Vec::new();
                let mut found = ItemStack::empty();
                for s in input.stacks() {
                    if target.test(s) {
                        if !found.is_empty() {
                            return ItemStack::empty();
                        }
                        found = s.clone();
                    } else if dye.test(s) {
                        colors.push(s.get(keys::DYE).copied().unwrap_or(DyeColor::White));
                    } else {
                        return ItemStack::empty();
                    }
                }
                if found.is_empty() || colors.is_empty() {
                    return ItemStack::empty();
                }
                let dyed = apply_dyes(found.get(keys::DYED_COLOR).map(|c| c.0), &colors);
                let mut out = with_original_components(result, &found, 0);
                out.insert(keys::DYED_COLOR, DyedColor(dyed));
                out
            }
            Special::FireworkRocket { fuel, star, result, .. } => {
                let mut flight = 0;
                let mut explosions = Vec::new();
                for s in input.stacks() {
                    if fuel.test(s) {
                        flight += 1;
                    } else if star.test(s)
                        && let Some(e) = s.get(keys::FIREWORK_EXPLOSION)
                    {
                        explosions.push(e.clone());
                    }
                }
                let mut patch = DataComponentPatch::new();
                patch.set(keys::FIREWORKS.wrap(Fireworks { flight_duration: flight, explosions }));
                template_apply(result, patch)
            }
            Special::FireworkStar { shapes, trail, twinkle, dye, result, .. } => {
                let mut shape = FireworkShape::SmallBall;
                let (mut has_twinkle, mut has_trail) = (false, false);
                let mut colors = Vec::new();
                for s in input.stacks() {
                    if let Some(sh) = find_shape(shapes, s) {
                        shape = sh;
                    } else if twinkle.test(s) {
                        has_twinkle = true;
                    } else if trail.test(s) {
                        has_trail = true;
                    } else if dye.test(s) {
                        colors.push(firework_color(s.get(keys::DYE).copied().unwrap_or(DyeColor::White)));
                    }
                }
                let mut out = result.create();
                out.insert(keys::FIREWORK_EXPLOSION, FireworkExplosion { shape, colors, fade_colors: Vec::new(), has_trail, has_twinkle });
                out
            }
            Special::FireworkStarFade { target, dye, result } => {
                let mut colors = Vec::new();
                let mut star: Option<&ItemStack> = None;
                for s in input.items() {
                    if dye.test(s) {
                        colors.push(firework_color(s.get(keys::DYE).copied().unwrap_or(DyeColor::White)));
                    } else if target.test(s) {
                        star = Some(s);
                    }
                }
                let Some(star) = star.filter(|_| !colors.is_empty()) else { return ItemStack::empty() };
                let mut out = with_original_components(result, star, 0);
                let mut e = out.get(keys::FIREWORK_EXPLOSION).cloned().unwrap_or(FireworkExplosion {
                    shape: FireworkShape::SmallBall,
                    colors: Vec::new(),
                    fade_colors: Vec::new(),
                    has_trail: false,
                    has_twinkle: false,
                });
                e.fade_colors = colors;
                out.insert(keys::FIREWORK_EXPLOSION, e);
                out
            }
            Special::Imbue { result, .. } => {
                let center = input.get_xy(1, 1);
                let mut out = result.create();
                match center.get(keys::POTION_CONTENTS) {
                    Some(p) => out.insert(keys::POTION_CONTENTS, p.clone()),
                    None => out.remove(ids::POTION_CONTENTS),
                }
                out
            }
            Special::MapExtending { result, .. } => {
                let Some(map) = find_filled_map(input) else { return ItemStack::empty() };
                let mut out = with_original_components(&result.resolve(map.item()), map, 0);
                out.insert(keys::MAP_POST_PROCESSING, kiln_item::component::MapPostProcessing::Scale);
                out
            }
            Special::RepairItem => {
                let Some((a, b)) = items_to_combine(input) else { return ItemStack::empty() };
                let max = a.max_damage().max(b.max_damage());
                let left_a = a.max_damage() - a.damage();
                let left_b = b.max_damage() - b.damage();
                let total = left_a + left_b + max * 5 / 100;
                let mut out = ItemStack::new(a.item(), 1);
                out.insert(keys::MAX_DAMAGE, max);
                let damage = (max - total).max(0);
                out.insert(keys::DAMAGE, damage.clamp(0, out.max_damage()));
                let ea = crafting_enchantments(a);
                let eb = crafting_enchantments(b);
                let key = enchantments_key(&out);
                if let Some(current) = out.get(key).cloned() {
                    let mut merged = current;
                    for id in ea.0.iter().map(|e| e.0).chain(eb.0.iter().map(|e| e.0).filter(|id| ea.level(*id) == 0)) {
                        if crate::tags::contains("minecraft:enchantment", "minecraft:curse", id) {
                            merged.set(id, ea.level(id).max(eb.level(id)));
                        }
                    }
                    out.insert(key, merged);
                }
                out
            }
            Special::ShieldDecoration { banner, target, result } => {
                let mut patterns: Option<BannerPatternLayers> = None;
                let mut color = DyeColor::White;
                let mut shield = ItemStack::empty();
                for s in input.stacks() {
                    if banner.test(s)
                        && let Some(c) = banner_color(s)
                    {
                        patterns = s.get(keys::BANNER_PATTERNS).cloned();
                        color = c;
                    } else if target.test(s) {
                        shield = s.clone();
                    }
                }
                let mut out = with_original_components(result, &shield, 0);
                match patterns {
                    Some(p) => out.insert(keys::BANNER_PATTERNS, p),
                    None => out.remove(ids::BANNER_PATTERNS),
                }
                out.insert(keys::BASE_COLOR, color);
                out
            }
        }
    }

    /// `getRemainingItems`.
    pub fn remaining_items(&self, input: &CraftingInput) -> Vec<ItemStack> {
        let mut out = vec![ItemStack::empty(); input.size()];
        match self {
            Special::BannerDuplicate { .. } => {
                for (i, s) in input.items().iter().enumerate() {
                    if s.is_empty() {
                        continue;
                    }
                    if let Some(r) = crafting_remainder(s.item()) {
                        out[i] = r;
                    } else if banner_layers(s) > 0 {
                        out[i] = crate::stack::StackExt::copy_with_count(s, 1);
                    }
                }
            }
            Special::BookCloning { .. } => {
                for (i, s) in input.items().iter().enumerate() {
                    if let Some(r) = crafting_remainder(s.item()).filter(|_| !s.is_empty()) {
                        out[i] = r;
                    } else if s.has(ids::WRITTEN_BOOK_CONTENT) {
                        out[i] = crate::stack::StackExt::copy_with_count(s, 1);
                        break;
                    }
                }
            }
            _ => return super::default_remainders(input),
        }
        out
    }
}

fn find_shape(shapes: &[(FireworkShape, Ingredient)], s: &ItemStack) -> Option<FireworkShape> {
    shapes.iter().find(|(_, i)| i.test(s)).map(|(shape, _)| *shape)
}

fn find_filled_map(input: &CraftingInput) -> Option<&ItemStack> {
    input.items().iter().find(|s| s.has(ids::MAP_ID))
}

/// `BannerItem.getColor` of a banner item (`None` for anything else).
fn banner_color(s: &ItemStack) -> Option<DyeColor> {
    if s.is_empty() {
        return None;
    }
    let name = s.item_name().strip_prefix("minecraft:")?.strip_suffix("_banner")?;
    DyeColor::from_name(name)
}

fn banner_layers(s: &ItemStack) -> usize {
    s.get(keys::BANNER_PATTERNS).map_or(0, |p| p.0.len())
}

/// `RepairItemRecipe.getItemsToCombine`.
fn items_to_combine(input: &CraftingInput) -> Option<(&ItemStack, &ItemStack)> {
    if input.ingredient_count() != 2 {
        return None;
    }
    let mut stacks = input.stacks();
    let (a, b) = (stacks.next()?, stacks.next()?);
    let ok = b.item() == a.item()
        && a.count() == 1
        && b.count() == 1
        && a.has(ids::MAX_DAMAGE)
        && b.has(ids::MAX_DAMAGE)
        && a.has(ids::DAMAGE)
        && b.has(ids::DAMAGE);
    ok.then_some((a, b))
}

/// `EnchantmentHelper.getComponentType`: stored enchantments for enchanted books.
fn enchantments_key(s: &ItemStack) -> kiln_item::Key<kiln_item::component::Enchantments> {
    if s.item_name() == "minecraft:enchanted_book" { keys::STORED_ENCHANTMENTS } else { keys::ENCHANTMENTS }
}

fn crafting_enchantments(s: &ItemStack) -> kiln_item::component::Enchantments {
    s.get(enchantments_key(s)).cloned().unwrap_or_default()
}

/// `DyeColor.getTextureDiffuseColor` / `getFireworkColor`, by `DyeColor` id.
const DYE_COLORS: [(i32, i32); 16] = [
    (16383998, 15790320),
    (16351261, 15435844),
    (13061821, 12801229),
    (3847130, 6719955),
    (16701501, 14602026),
    (8439583, 4312372),
    (15961002, 14188952),
    (4673362, 4408131),
    (10329495, 11250603),
    (1481884, 2651799),
    (8991416, 8073150),
    (3949738, 2437522),
    (8606770, 5320730),
    (6192150, 3887386),
    (11546150, 11743532),
    (1908001, 1973019),
];

fn firework_color(c: DyeColor) -> i32 {
    DYE_COLORS[c.id() as usize].1
}

/// `DyedItemColor.applyDyes`.
fn apply_dyes(current: Option<i32>, dyes: &[DyeColor]) -> i32 {
    let (mut r, mut g, mut b, mut total, mut n) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let mut add = |rgb: i32| {
        let (cr, cg, cb) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
        total += cr.max(cg).max(cb);
        r += cr;
        g += cg;
        b += cb;
        n += 1;
    };
    if let Some(c) = current {
        add(c);
    }
    for d in dyes {
        add(DYE_COLORS[d.id() as usize].0);
    }
    let (r, g, b) = (r / n, g / n, b / n);
    let avg_max = total as f32 / n as f32;
    let max = r.max(g).max(b) as f32;
    let scale = |v: i32| (v as f32 * avg_max / max) as i32;
    (scale(r) & 0xFF) << 16 | (scale(g) & 0xFF) << 8 | (scale(b) & 0xFF)
}
