//! A player's recipe book (`ServerRecipeBook`): the recipes it knows, those still highlighted
//! as new, and the open/filtering settings of the four books. Saved in player data as
//! `recipeBook`; sent on join (Recipe Book Settings, then Recipe Book Add replacing the
//! client's book); recipes unlock through advancement rewards, crafting and `/recipe`.

use crate::Player;
use kiln_inventory::Rules;
use kiln_inventory::recipe::book;
use kiln_proto::nbt::Tag;
use std::collections::BTreeSet;

/// `RecipeBookSettings` field names per book (crafting, furnace, blast furnace, smoker).
const SETTINGS: [(&str, &str); 4] = [
    ("isGuiOpen", "isFilteringCraftable"),
    ("isFurnaceGuiOpen", "isFurnaceFilteringCraftable"),
    ("isBlastingFurnaceGuiOpen", "isBlastingFurnaceFilteringCraftable"),
    ("isSmokerGuiOpen", "isSmokerFilteringCraftable"),
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecipeBook {
    /// Recipe ids the player knows.
    pub known: BTreeSet<String>,
    /// Known recipes not yet seen in the book (`toBeDisplayed`).
    pub highlight: BTreeSet<String>,
    /// (open, filtering) per book.
    pub settings: [(bool, bool); 4],
}

impl RecipeBook {
    /// `ServerRecipeBook.Packed` from player data (`recipeBook`).
    pub fn load(tag: Option<&Tag>) -> RecipeBook {
        let mut book = RecipeBook::default();
        let Some(tag) = tag else { return book };
        let list = |key: &str| -> BTreeSet<String> {
            match tag.get(key) {
                Some(Tag::List(items)) => items.iter().filter_map(Tag::as_str).map(normalize).collect(),
                _ => BTreeSet::new(),
            }
        };
        book.known = list("recipes");
        book.highlight = list("toBeDisplayed");
        for (i, (open, filtering)) in SETTINGS.iter().enumerate() {
            let flag = |k: &str| tag.get(k).and_then(Tag::as_i64).is_some_and(|v| v != 0);
            book.settings[i] = (flag(open), flag(filtering));
        }
        book
    }

    /// `ServerRecipeBook.Packed.CODEC` as NBT.
    pub fn to_nbt(&self) -> Tag {
        let mut fields: Vec<(String, Tag)> = Vec::new();
        for (i, (open, filtering)) in SETTINGS.iter().enumerate() {
            fields.push(((*open).into(), Tag::Byte(self.settings[i].0 as i8)));
            fields.push(((*filtering).into(), Tag::Byte(self.settings[i].1 as i8)));
        }
        let list = |set: &BTreeSet<String>| Tag::List(set.iter().map(|s| Tag::String(s.clone())).collect());
        fields.push(("recipes".into(), list(&self.known)));
        fields.push(("toBeDisplayed".into(), list(&self.highlight)));
        Tag::Compound(fields)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.known.contains(id)
    }

    /// `loadUntrusted`: recipes that no longer exist are dropped.
    pub fn retain_existing(&mut self, rules: &Rules) {
        let exists = |id: &String| {
            let ok = rules.recipes.index_of(id).is_some();
            if !ok {
                tracing::error!("Tried to load unrecognized recipe: {id} removed now.");
            }
            ok
        };
        self.known.retain(exists);
        self.highlight.retain(|id| rules.recipes.index_of(id).is_some());
    }
}

fn normalize(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

impl Player {
    /// `ServerRecipeBook.addRecipes`: unlocks the recipes the book does not know yet (special
    /// recipes never), highlighted, with the Recipe Book Add packet; returns how many.
    pub(crate) fn award_recipes(&mut self, rules: &Rules, recipes: &[usize]) -> i32 {
        let displays = rules.recipes.displays();
        let mut entries = Vec::new();
        let mut count = 0;
        for &r in recipes {
            let Some(h) = rules.recipes.recipes().get(r) else { continue };
            if self.recipe_book.known.contains(&h.id) || h.recipe.is_special() {
                continue;
            }
            self.recipe_book.known.insert(h.id.clone());
            self.recipe_book.highlight.insert(h.id.clone());
            for &id in displays.by_recipe.get(r).map(Vec::as_slice).unwrap_or(&[]) {
                entries.push((id, h.book.show_notification, true));
            }
            self.recipe_unlocked(&h.id);
            count += 1;
        }
        if !entries.is_empty() {
            self.send(book::recipe_book_add(displays, &entries, false));
        }
        count
    }

    /// `ServerRecipeBook.removeRecipes`: forgets known recipes (Recipe Book Remove).
    pub(crate) fn reset_recipes(&mut self, rules: &Rules, recipes: &[usize]) -> i32 {
        let displays = rules.recipes.displays();
        let mut ids = Vec::new();
        let mut count = 0;
        for &r in recipes {
            let Some(h) = rules.recipes.recipes().get(r) else { continue };
            if !self.recipe_book.known.remove(&h.id) {
                continue;
            }
            self.recipe_book.highlight.remove(&h.id);
            ids.extend_from_slice(displays.by_recipe.get(r).map(Vec::as_slice).unwrap_or(&[]));
            count += 1;
        }
        if !ids.is_empty() {
            self.send(book::recipe_book_remove(&ids));
        }
        count
    }

    /// `awardRecipesByKey` (advancement rewards): unknown ids are skipped with a warning.
    pub(crate) fn award_recipes_by_key(&mut self, rules: &Rules, ids: &[String]) {
        let mut found = Vec::new();
        for id in ids {
            match rules.recipes.index_of(id) {
                Some(i) => found.push(i),
                None => tracing::warn!("Tried to award recipe {id} to {} but it doesn't exist", self.name),
            }
        }
        self.award_recipes(rules, &found);
    }

    /// `sendInitialRecipeBook`: the settings, then every known recipe replacing the client's
    /// book (highlighted ones flagged, no notification).
    pub(crate) fn send_initial_recipe_book(&mut self, rules: &Rules) {
        self.send(book::recipe_book_settings(&self.recipe_book.settings));
        let displays = rules.recipes.displays();
        let mut entries = Vec::new();
        for id in &self.recipe_book.known {
            let Some(r) = rules.recipes.index_of(id) else { continue };
            let highlight = self.recipe_book.highlight.contains(id);
            for &d in displays.by_recipe.get(r).map(Vec::as_slice).unwrap_or(&[]) {
                entries.push((d, false, highlight));
            }
        }
        self.send(book::recipe_book_add(displays, &entries, true));
    }

    /// `handleRecipeBookChangeSettingsPacket`.
    pub(crate) fn recipe_book_settings(&mut self, book: kiln_proto::packets::serverbound::RecipeBookType, open: bool, filtering: bool) {
        use kiln_proto::packets::serverbound::RecipeBookType as T;
        let i = match book {
            T::Crafting => 0,
            T::Furnace => 1,
            T::BlastFurnace => 2,
            T::Smoker => 3,
        };
        self.recipe_book.settings[i] = (open, filtering);
    }

    /// `handleRecipeBookSeenRecipePacket`: the recipe is no longer highlighted.
    pub(crate) fn recipe_seen(&mut self, rules: &Rules, display: i32) {
        if let Some(r) = rules.recipes.recipe_of_display(display) {
            let id = rules.recipes.id(r).to_owned();
            self.recipe_book.highlight.remove(&id);
        }
    }

    /// `handlePlaceRecipe`: a known recipe fills the open crafting grid or furnace, or the
    /// client shows its ghost.
    pub(crate) fn place_recipe(
        &mut self,
        level: &mut crate::blocks::RegionLevel,
        spawns: &mut Vec<crate::entities::Spawn>,
        container_id: i32,
        display: i32,
        use_max: bool,
    ) {
        let rules = level.env.menus.clone();
        let open_id = self.open_menu.as_ref().map_or(0, |m| m.container_id);
        if self.game_mode == 3 || open_id != container_id {
            return;
        }
        let Some(r) = rules.recipes.recipe_of_display(display) else { return };
        if !self.recipe_book.contains(rules.recipes.id(r)) {
            return;
        }
        let creative = self.game_mode == 1;
        let outcome = crate::container::open::menu_op(self, level, spawns, |menu, _, env| menu.place_recipe(env, r, use_max, creative));
        if outcome == kiln_inventory::place::PostPlace::PlaceGhostRecipe {
            let e = &rules.recipes.displays().entries[display as usize];
            self.send(book::place_ghost_recipe(container_id, &e.display));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_book_recipes() {
        let dir = crate::datapack_dir(None);
        let Ok(rules) = kiln_inventory::Rules::load(&dir) else { return };
        assert_eq!(rules.recipes.errors, Vec::new());
        assert_eq!(rules.recipes.len(), 2042, "every recipe file loads");
        let book = rules.recipes.recipes().iter().filter(|r| !r.recipe.is_special()).count();
        // Every non-special recipe: vanilla 26.3's book after `/recipe give @s *` on the
        // reference world (feature packs add more).
        assert_eq!(book, 1739);
    }

    #[test]
    fn packed_round_trip() {
        let mut b = RecipeBook::default();
        b.known.insert("minecraft:stick".into());
        b.highlight.insert("minecraft:stick".into());
        b.settings[1] = (true, false);
        let tag = b.to_nbt();
        assert_eq!(RecipeBook::load(Some(&tag)), b);
    }
}
