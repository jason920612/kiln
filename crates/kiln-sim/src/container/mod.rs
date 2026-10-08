//! Container block entities (chests, barrels, shulker boxes, hoppers, dispensers, droppers,
//! furnaces, brewing stands, beacons, ender chests): their contents as live state next to the region's block
//! machinery, the menus players open on them, and their ticks.
//!
//! A chunk keeps every block entity as NBT (kiln-world). While a chunk is in a region, the
//! containers in it are decoded into [`Containers`] (keyed by position, following the chunk
//! through region merges and splits like scheduled ticks) and written back into the chunk's
//! NBT when it is stored (autosave, unload). Block changes keep the two in step
//! ([`Containers::block_changed`]): a container whose block goes away drops its contents
//! (`BlockEntity.preRemoveSideEffects`, except shulker boxes).
//!
//! Block entities tick in position order within a region (vanilla ticks them in the order they
//! were added to the level), so the result never depends on how the world is split into
//! regions (an approximation, I class).

pub(crate) mod beacon;
pub(crate) mod brewing;
pub(crate) mod dispense;
pub(crate) mod furnace;
pub(crate) mod hopper;
pub(crate) mod open;
pub(crate) mod world;

use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Level};
use kiln_inventory::FurnaceKind;
use kiln_inventory::persist::ItemList;
use kiln_inventory::stack::StackExt;
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_world::{Blocks, ChunkPos};
use kiln_world::block_entity::{BlockEntity, type_name};
use kiln_world::chunk::Chunk;
use std::collections::BTreeMap;

/// Which container block entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BeKind {
    Chest,
    TrappedChest,
    Barrel,
    ShulkerBox,
    Hopper,
    Dispenser,
    Dropper,
    Furnace(FurnaceKind),
    /// Holds nothing itself (the items are the player's `EnderItems`); counts its openers.
    EnderChest,
    BrewingStand,
    /// Holds no items (the payment is the menu's); ticks its beam and powers.
    Beacon,
    /// One slot for a music disc (`JukeboxBlockEntity`); plays the disc's song.
    Jukebox,
    /// Four spots for food to cook on (`CampfireBlockEntity`; not a `Container`).
    Campfire,
    /// Six slots for one book each (`ChiseledBookShelfBlockEntity`).
    ChiseledBookshelf,
    /// Holds nothing; works its signal out every 20 ticks (`DaylightDetectorBlockEntity`).
    DaylightDetector,
    /// Holds nothing; shakes when hit (`BellBlockEntity`).
    Bell,
    /// Holds up to three bees (`BeehiveBlockEntity`).
    Beehive,
    /// One slot, and four sherds that decorate the sides (`DecoratedPotBlockEntity`).
    DecoratedPot,
    /// One book and the page it is open at (`LecternBlockEntity`; not a `Container` for hoppers).
    Lectern,
}

impl BeKind {
    /// By `minecraft:block_entity_type` name.
    pub fn by_type(name: &str) -> Option<BeKind> {
        Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
            "chest" => BeKind::Chest,
            "trapped_chest" => BeKind::TrappedChest,
            "barrel" => BeKind::Barrel,
            "shulker_box" => BeKind::ShulkerBox,
            "hopper" => BeKind::Hopper,
            "dispenser" => BeKind::Dispenser,
            "dropper" => BeKind::Dropper,
            "furnace" => BeKind::Furnace(FurnaceKind::Furnace),
            "blast_furnace" => BeKind::Furnace(FurnaceKind::BlastFurnace),
            "smoker" => BeKind::Furnace(FurnaceKind::Smoker),
            "ender_chest" => BeKind::EnderChest,
            "brewing_stand" => BeKind::BrewingStand,
            "beacon" => BeKind::Beacon,
            "jukebox" => BeKind::Jukebox,
            "campfire" => BeKind::Campfire,
            "chiseled_bookshelf" => BeKind::ChiseledBookshelf,
            "daylight_detector" => BeKind::DaylightDetector,
            "bell" => BeKind::Bell,
            "beehive" => BeKind::Beehive,
            "decorated_pot" => BeKind::DecoratedPot,
            "lectern" => BeKind::Lectern,
            _ => return None,
        })
    }

    /// `getContainerSize`.
    pub fn size(self) -> usize {
        match self {
            BeKind::Chest | BeKind::TrappedChest | BeKind::Barrel | BeKind::ShulkerBox => 27,
            BeKind::Hopper => 5,
            BeKind::Dispenser | BeKind::Dropper => 9,
            BeKind::Furnace(_) => 3,
            BeKind::BrewingStand => 5,
            BeKind::Jukebox => 1,
            BeKind::Campfire => 4,
            BeKind::ChiseledBookshelf => 6,
            BeKind::DecoratedPot | BeKind::Lectern => 1,
            BeKind::EnderChest | BeKind::Beacon | BeKind::DaylightDetector | BeKind::Bell | BeKind::Beehive => 0,
        }
    }

    /// `RandomizableContainerBlockEntity`: can hold an unopened loot table.
    pub fn randomizable(self) -> bool {
        !matches!(
            self,
            BeKind::Furnace(_) | BeKind::EnderChest | BeKind::BrewingStand | BeKind::Beacon | BeKind::Jukebox | BeKind::Campfire | BeKind::ChiseledBookshelf | BeKind::DaylightDetector | BeKind::Bell | BeKind::Beehive | BeKind::Lectern
        )
    }

    /// A `Container` (dropped when its block goes, read by comparators).
    pub fn is_container(self) -> bool {
        !matches!(self, BeKind::EnderChest | BeKind::Beacon | BeKind::Campfire | BeKind::DaylightDetector | BeKind::Bell | BeKind::Beehive | BeKind::Lectern)
    }

    /// `getDefaultName` translation key.
    pub fn default_name(self) -> &'static str {
        match self {
            BeKind::Chest | BeKind::TrappedChest => "container.chest",
            BeKind::Barrel => "container.barrel",
            BeKind::ShulkerBox => "container.shulkerBox",
            BeKind::Hopper => "container.hopper",
            BeKind::Dispenser => "container.dispenser",
            BeKind::Dropper => "container.dropper",
            BeKind::Furnace(FurnaceKind::Furnace) => "container.furnace",
            BeKind::Furnace(FurnaceKind::BlastFurnace) => "container.blast_furnace",
            BeKind::Furnace(FurnaceKind::Smoker) => "container.smoker",
            BeKind::EnderChest => "container.enderchest",
            BeKind::BrewingStand => "container.brewing",
            BeKind::Beacon => "container.beacon",
            BeKind::Jukebox => "container.jukebox",
            BeKind::Campfire => "container.campfire",
            BeKind::ChiseledBookshelf => "container.chiseled_bookshelf",
            BeKind::DaylightDetector => "container.daylight_detector",
            BeKind::Bell => "block.minecraft.bell",
            BeKind::Beehive => "block.minecraft.beehive",
            BeKind::DecoratedPot => "block.minecraft.decorated_pot",
            BeKind::Lectern => "container.lectern",
        }
    }
}

/// Saved fields a container block entity models; the rest of its NBT is kept as is.
const MODELED: [&str; 34] = [
    "Book",
    "Page",
    "item",
    "bees",
    "flower_pos",
    "last_interacted_slot",
    "CookingTimes",
    "CookingTotalTimes",
    "RecordItem",
    "ticks_since_song_started",
    "primary_effect",
    "secondary_effect",
    "Levels",
    "BrewTime",
    "total_brew_time",
    "Fuel",
    "total_fuel",
    "Items",
    "CustomName",
    "lock",
    "LootTable",
    "LootTableSeed",
    "TransferCooldown",
    "cooking_time_spent",
    "cooking_total_time",
    "lit_time_remaining",
    "lit_total_time",
    "speed_multiplier",
    "RecipesUsed",
    "id",
    "x",
    "y",
    "z",
    "keepPacked",
];

/// `LecternBlockEntity.getPageCount`: the pages of a written or writable book (0 for anything else).
pub(crate) fn page_count(book: &ItemStack) -> i32 {
    if let Some(w) = book.get(kiln_item::keys::WRITTEN_BOOK_CONTENT) {
        return w.pages.len() as i32;
    }
    book.get(kiln_item::keys::WRITABLE_BOOK_CONTENT).map_or(0, |w| w.pages.len() as i32)
}

/// A container block entity's live state.
#[derive(Debug, Clone)]
pub(crate) struct ContainerBe {
    pub kind: BeKind,
    /// `minecraft:block_entity_type` id (for keeping the chunk's entry in step).
    pub type_id: u16,
    /// Tells this block entity from one that replaced it at the same position (menus stay
    /// valid only on the block entity they were opened on).
    pub serial: u64,
    pub items: Vec<ItemStack>,
    /// `Items` entries that did not decode, written back unchanged.
    undecoded: Vec<(i32, Tag)>,
    /// `CustomName` (a text component) as saved.
    pub custom_name: Option<Tag>,
    /// `lock` (an item predicate) as saved.
    pub lock: Option<Tag>,
    /// `LootTable` not yet rolled, and its seed.
    pub loot_table: Option<String>,
    pub loot_seed: i64,
    /// `ContainerOpenersCounter.openCount` and `maxInteractionRange` (not saved).
    pub openers: i32,
    pub max_range: f64,
    /// Hopper: `cooldownTime` and `tickedGameTime`.
    pub cooldown: i32,
    pub ticked_game_time: i64,
    /// Furnace: `litTimeRemaining`, `litTotalTime`, `cookingTimer`, `cookingTotalTime`,
    /// `speedMultiplier`, `recipesUsed` (recipe id and count, in first-use order).
    pub lit_remaining: i32,
    pub lit_total: i32,
    pub cook_timer: i32,
    pub cook_total: i32,
    pub speed: f32,
    pub recipes_used: Vec<(String, i32)>,
    /// The furnace's `quickCheck`: the recipe it last found.
    pub last_recipe: Option<usize>,
    /// Brewing stand (which keeps `fuel`, `totalFuel`, `brewTime`, `totalBrewTime` and its
    /// speed in the furnace fields above): the `ingredient` item the brewing started with, and
    /// `lastPotionCount` (the bottles its block state last showed; not saved).
    pub ingredient: Option<i32>,
    pub last_bottles: Option<[bool; 3]>,
    /// A beacon's beam, levels and powers.
    pub beacon: Option<beacon::Beacon>,
    /// A furnace's input changed to another item (its `setItem` on slot 0): the cook timer
    /// resets once the recipes are at hand ([`furnace::apply_input_change`]).
    pub input_changed: bool,
    /// A jukebox's `JukeboxSongPlayer`: the song playing (`minecraft:jukebox_song` network id) and
    /// the ticks since it started.
    pub song: Option<(i32, i64)>,
    /// `song` was read from the saved data and has yet to be checked against the song's length
    /// (the data of the songs is the level's, not at hand when the chunk loads).
    pub song_unchecked: bool,
    /// A campfire's `cookingTimes` and `cookingTotalTimes` per spot.
    pub cooking: [i32; 4],
    /// A chiseled bookshelf's `lastInteractedSlot` (-1: none).
    pub last_slot: i32,
    pub cooking_total: [i32; 4],
    /// A jukebox's item changed (`setTheItem`): its block state, song and neighbours follow once
    /// the block entity is back in the level.
    pub item_changed: bool,
    /// `setChanged` calls: comparators and the chunk's saved data follow.
    pub changes: u64,
    /// A bell's shaking.
    pub bell: Option<Box<crate::bell::BellState>>,
    /// A lectern's page (`LecternBlockEntity.page`) and whether it was turned and its block is yet to pulse.
    pub page: i32,
    pub page_turned: bool,
    /// A beehive's bees.
    pub hive: Option<Box<crate::beehive::Hive>>,
    /// Changed since its NBT was last written into the chunk.
    pub dirty: bool,
    /// Saved fields not modeled here (`components`, ...).
    extra: Vec<(String, Tag)>,
}

impl ContainerBe {
    /// A block entity of `kind` read from its saved NBT (`loadAdditional`).
    pub fn load(kind: BeKind, type_id: u16, nbt: &Tag) -> ContainerBe {
        let int = |k: &str, default: i32| nbt.get(k).and_then(Tag::as_i64).map_or(default, |v| v as i32);
        let loot_table = if kind.randomizable() { nbt.get("LootTable").and_then(Tag::as_str).map(str::to_owned) } else { None };
        let loot_seed = if loot_table.is_some() { nbt.get("LootTableSeed").and_then(Tag::as_i64).unwrap_or(0) } else { 0 };
        // `tryLoadLootTable`: a container with a loot table does not read its items.
        let list = if loot_table.is_some() { ItemList::load(None, kind.size()) } else { ItemList::load(nbt.get("Items"), kind.size()) };
        let recipes_used = match nbt.get("RecipesUsed") {
            Some(Tag::Compound(fields)) => fields.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()? as i32))).collect(),
            _ => Vec::new(),
        };
        let extra = match nbt {
            Tag::Compound(fields) => fields.iter().filter(|(k, _)| !MODELED.contains(&k.as_str())).cloned().collect(),
            _ => Vec::new(),
        };
        // `JukeboxBlockEntity.loadAdditional`: the disc, and how far its song had got.
        let mut list = list;
        let (mut song, mut song_unchecked) = (None, false);
        if kind == BeKind::Jukebox {
            let disc = nbt.get("RecordItem").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(ItemStack::empty);
            if let Some(ticks) = nbt.get("ticks_since_song_started").and_then(Tag::as_i64)
                && let Some(id) = crate::jukebox::song_id(&disc)
            {
                song = Some((id, ticks));
                song_unchecked = true;
            }
            list.stacks = vec![disc];
        }
        // `LecternBlockEntity.loadAdditional`: the book and the page it is open at.
        let mut page = 0;
        if kind == BeKind::Lectern {
            let book = nbt.get("Book").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(ItemStack::empty);
            page = int("Page", 0).clamp(0, (page_count(&book) - 1).max(0));
            list.stacks = vec![book];
        }
        // `DecoratedPotBlockEntity.loadAdditional`: its one item is saved as `item`.
        if kind == BeKind::DecoratedPot {
            let item = if loot_table.is_some() { None } else { nbt.get("item").and_then(|t| ItemStack::from_nbt(t).ok()) };
            list.stacks = vec![item.filter(|s| !s.is_empty()).unwrap_or_else(ItemStack::empty)];
        }
        // `BrewingStandBlockEntity.loadAdditional`: brewing under way remembers its ingredient.
        let ingredient = (kind == BeKind::BrewingStand && int("BrewTime", 0) > 0).then(|| list.stacks.get(3).map_or(0, ItemStack::item));
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut loaded = ContainerBe {
            kind,
            type_id,
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            items: list.stacks,
            undecoded: list.undecoded,
            custom_name: nbt.get("CustomName").cloned(),
            lock: nbt.get("lock").cloned(),
            loot_table,
            loot_seed,
            openers: 0,
            max_range: 0.0,
            cooldown: if kind == BeKind::Hopper { int("TransferCooldown", -1) } else { -1 },
            ticked_game_time: 0,
            lit_remaining: if kind == BeKind::BrewingStand { int("Fuel", 0) } else { int("lit_time_remaining", 0) },
            lit_total: if kind == BeKind::BrewingStand { int("total_fuel", 20) } else { int("lit_total_time", 0) },
            cook_timer: if kind == BeKind::BrewingStand { int("BrewTime", 0) } else { int("cooking_time_spent", 0) },
            cook_total: if kind == BeKind::BrewingStand { int("total_brew_time", 400) } else { int("cooking_total_time", 0) },
            speed: nbt.get("speed_multiplier").and_then(Tag::as_f64).map_or(1.0, |v| v as f32),
            recipes_used,
            last_recipe: None,
            ingredient,
            last_bottles: None,
            beacon: (kind == BeKind::Beacon).then(|| beacon::Beacon::load(nbt)),
            input_changed: false,
            song,
            song_unchecked,
            cooking: [0; 4],
            last_slot: if kind == BeKind::ChiseledBookshelf { int("last_interacted_slot", -1) } else { -1 },
            cooking_total: [0; 4],
            item_changed: false,
            changes: 0,
            bell: (kind == BeKind::Bell).then(Default::default),
            page,
            page_turned: false,
            hive: (kind == BeKind::Beehive).then(|| Box::new(crate::beehive::Hive::load(nbt))),
            dirty: false,
            extra,
        };
        if kind == BeKind::Campfire {
            crate::campfire::load_timers(&mut loaded, nbt);
        }
        loaded
    }

    /// `saveAdditional`: the saved NBT (without `id` and position, which kiln-world adds).
    pub fn save(&self) -> Tag {
        let mut out: Vec<(String, Tag)> = Vec::new();
        if let Some(lock) = &self.lock {
            out.push(("lock".into(), lock.clone()));
        }
        if let Some(name) = &self.custom_name {
            out.push(("CustomName".into(), name.clone()));
        }
        match self.kind {
            BeKind::EnderChest | BeKind::Bell | BeKind::DaylightDetector => {}
            BeKind::Beehive => {
                if let Some(h) = &self.hive {
                    h.save(&mut out);
                }
            }
            BeKind::Lectern => {
                if let Some(book) = self.items.first().filter(|s| !s.is_empty()) {
                    out.push(("Book".into(), book.to_nbt()));
                    out.push(("Page".into(), Tag::Int(self.page)));
                }
            }
            // `sherds` is kept in `extra`; then the loot table or the item.
            BeKind::DecoratedPot => match &self.loot_table {
                Some(table) => {
                    out.push(("LootTable".into(), Tag::String(table.clone())));
                    if self.loot_seed != 0 {
                        out.push(("LootTableSeed".into(), Tag::Long(self.loot_seed)));
                    }
                }
                None => {
                    if let Some(item) = self.items.first().filter(|s| !s.is_empty()) {
                        out.push(("item".into(), item.to_nbt()));
                    }
                }
            },
            BeKind::Jukebox => {
                if let Some(disc) = self.items.first().filter(|s| !s.is_empty()) {
                    out.push(("RecordItem".into(), disc.to_nbt()));
                }
                if let Some((_, ticks)) = self.song {
                    out.push(("ticks_since_song_started".into(), Tag::Long(ticks)));
                }
            }
            BeKind::Beacon => {
                if let Some(b) = &self.beacon {
                    b.save(&mut out);
                }
            }
            BeKind::ChiseledBookshelf => {
                out.push(("Items".into(), self.item_list().save()));
                out.push(("last_interacted_slot".into(), Tag::Int(self.last_slot)));
            }
            BeKind::Campfire => {
                out.push(("Items".into(), self.item_list().save()));
                out.push(("CookingTimes".into(), Tag::IntArray(self.cooking.to_vec())));
                out.push(("CookingTotalTimes".into(), Tag::IntArray(self.cooking_total.to_vec())));
            }
            BeKind::BrewingStand => {
                out.push(("BrewTime".into(), Tag::Int(self.cook_timer)));
                out.push(("total_brew_time".into(), Tag::Int(self.cook_total)));
                out.push(("Items".into(), self.item_list().save()));
                out.push(("Fuel".into(), Tag::Int(self.lit_remaining)));
                out.push(("total_fuel".into(), Tag::Int(self.lit_total)));
                out.push(("speed_multiplier".into(), Tag::Float(self.speed)));
            }
            BeKind::Furnace(_) => {
                out.push(("cooking_time_spent".into(), Tag::Int(self.cook_timer)));
                out.push(("cooking_total_time".into(), Tag::Int(self.cook_total)));
                out.push(("lit_time_remaining".into(), Tag::Int(self.lit_remaining)));
                out.push(("lit_total_time".into(), Tag::Int(self.lit_total)));
                out.push(("speed_multiplier".into(), Tag::Float(self.speed)));
                out.push(("Items".into(), self.item_list().save()));
                let used = self.recipes_used.iter().map(|(k, v)| (k.clone(), Tag::Int(*v))).collect();
                out.push(("RecipesUsed".into(), Tag::Compound(used)));
            }
            _ => {
                match &self.loot_table {
                    // `trySaveLootTable`.
                    Some(table) => {
                        out.push(("LootTable".into(), Tag::String(table.clone())));
                        if self.loot_seed != 0 {
                            out.push(("LootTableSeed".into(), Tag::Long(self.loot_seed)));
                        }
                    }
                    None => out.push(("Items".into(), self.item_list().save())),
                }
                if self.kind == BeKind::Hopper {
                    out.push(("TransferCooldown".into(), Tag::Int(self.cooldown)));
                }
            }
        }
        out.extend(self.extra.iter().cloned());
        Tag::Compound(out)
    }

    fn item_list(&self) -> ItemList {
        ItemList { stacks: self.items.clone(), undecoded: self.undecoded.clone() }
    }

    /// `BlockEntity.setChanged`.
    pub fn mark_changed(&mut self) {
        self.changes += 1;
        self.dirty = true;
    }

    /// `Container.isEmpty` (of the items as they are; loot tables are the caller's).
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(ItemStack::is_empty)
    }

    /// `BaseContainerBlockEntity.canOpen` (`LockCode.canUnlock`: spectators pass any lock,
    /// others need a main hand item matching the lock's item predicate) and
    /// `RandomizableContainerBlockEntity.canOpen` (spectators cannot open an unrolled loot
    /// container). Without loot data (tags) a lock stays shut.
    pub fn can_open(&self, spectator: bool, held: &ItemStack, loot: Option<&kiln_loot::LootData>) -> bool {
        if self.loot_table.is_some() && spectator {
            return false;
        }
        let Some(lock) = &self.lock else { return true };
        if spectator {
            return true;
        }
        let predicate = <kiln_item::component::LockCode as kiln_item::component::ComponentValue>::from_value(&kiln_item::Value::from_nbt(lock));
        match (predicate, loot) {
            (Ok(p), Some(loot)) => kiln_loot::predicate::item_matches(&loot.tags, &p.0, held),
            _ => false,
        }
    }

    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` over these items.
    pub fn analog(&self) -> i32 {
        signal_from_items(self.items.iter(), self.items.len())
    }

    /// `BlockEntity.collectComponents` (`collectImplicitComponents` of
    /// `BaseContainerBlockEntity` and `RandomizableContainerBlockEntity`): what a block item
    /// dropped from it copies (`copy_components` with the block entity as source).
    pub fn components(&self) -> Vec<kiln_item::component::Component> {
        use kiln_item::component::{Component, ItemContainerContents, LockCode, SeededContainerLoot};
        let mut out = Vec::new();
        if let Some(name) = self.custom_name.clone().and_then(kiln_item::Text::from_nbt) {
            out.push(Component::CustomName(name));
        }
        if let Some(lock) = self.lock.as_ref().and_then(|t| <LockCode as kiln_item::component::ComponentValue>::from_value(&kiln_item::Value::from_nbt(t)).ok()) {
            out.push(Component::Lock(lock));
        }
        if let Some(h) = &self.hive {
            out.extend(h.components());
        }
        // `DecoratedPotBlockEntity.collectImplicitComponents`: the sherds (the container follows below).
        if self.kind == BeKind::DecoratedPot {
            let sherds = self.extra.iter().find(|(k, _)| k == "sherds").map(|(_, v)| v.clone());
            let decorations = sherds
                .and_then(|t| <kiln_item::component::PotDecorations as kiln_item::component::ComponentValue>::from_value(&kiln_item::Value::from_nbt(&t)).ok())
                .unwrap_or_default();
            out.push(Component::PotDecorations(decorations));
        }
        if self.kind.is_container() {
            // `ItemContainerContents.fromItems`: up to the last occupied slot.
            let last = self.items.iter().rposition(|s| !s.is_empty());
            let slots = last.map_or(Vec::new(), |l| {
                self.items[..=l].iter().map(|s| (!s.is_empty()).then(|| kiln_item::ItemStackTemplate::from_stack(s))).collect()
            });
            out.push(Component::Container(ItemContainerContents(slots)));
        }
        if let Some(table) = self.loot_table.as_deref().and_then(kiln_item::ident::Identifier::parse) {
            out.push(Component::ContainerLoot(SeededContainerLoot { loot_table: table, seed: self.loot_seed }));
        }
        out
    }

    /// The display title for `open_screen`: the custom name, or the default translation.
    pub fn title(&self) -> Tag {
        self.custom_name.clone().unwrap_or_else(|| translatable(self.kind.default_name()))
    }
}

/// `{"translate": key}` as network NBT.
pub(crate) fn translatable(key: &str) -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String(key.into()))])
}

/// `AbstractContainerMenu.getRedstoneSignalFromContainer` for `size` slots holding `items`.
pub(crate) fn signal_from_items<'a>(items: impl Iterator<Item = &'a ItemStack>, size: usize) -> i32 {
    if size == 0 {
        return 0;
    }
    let mut f = 0.0f32;
    for s in items.filter(|s| !s.is_empty()) {
        // `getMaxStackSize(stack)`: min(99, the item's maximum).
        f += s.count() as f32 / 99.min(s.max_stack_size()) as f32;
    }
    f /= size as f32;
    // `Mth.lerpDiscrete(f, 0, 15)`.
    (f * 14.0).floor() as i32 + i32::from(f > 0.0)
}

impl kiln_inventory::Container for ContainerBe {
    fn size(&self) -> usize {
        self.items.len()
    }

    fn item(&self, slot: usize) -> &ItemStack {
        &self.items[slot]
    }

    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        &mut self.items[slot]
    }

    fn set_item(&mut self, slot: usize, mut stack: ItemStack) {
        if let BeKind::Furnace(_) = self.kind {
            // `AbstractFurnaceBlockEntity.setItem`: another input item restarts the cooking.
            let same = !stack.is_empty() && kiln_inventory::stack::same_item_same_components(&self.items[slot], &stack);
            let max = self.max_stack_size_for(&stack);
            stack.limit_size(max);
            self.items[slot] = stack;
            if slot == 0 && !same {
                self.input_changed = true;
                self.cook_timer = 0;
                self.mark_changed();
            }
            return;
        }
        let max = self.max_stack_size_for(&stack);
        stack.limit_size(max);
        self.items[slot] = stack;
        // `JukeboxBlockEntity.setTheItem`: block state, song and neighbours follow.
        if self.kind == BeKind::Jukebox {
            self.item_changed = true;
        }
        // `HopperBlockEntity.setItem` does not call `setChanged`.
        if self.kind != BeKind::Hopper {
            self.mark_changed();
        }
    }

    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        let removed = kiln_inventory::container::remove_item(&mut self.items, slot, count);
        // `JukeboxBlockEntity.splitTheItem`: the whole item goes.
        if self.kind == BeKind::Jukebox && !removed.is_empty() {
            self.item_changed = true;
        }
        // `HopperBlockEntity.removeItem` does not call `setChanged`.
        if !removed.is_empty() && self.kind != BeKind::Hopper {
            self.mark_changed();
        }
        removed
    }

    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        let removed = kiln_inventory::container::take_item(&mut self.items, slot);
        if self.kind == BeKind::Jukebox && !removed.is_empty() {
            self.item_changed = true;
        }
        removed
    }

    /// `JukeboxBlockEntity.getMaxStackSize`: one disc.
    fn max_stack_size(&self) -> i32 {
        if matches!(self.kind, BeKind::Jukebox | BeKind::ChiseledBookshelf) { 1 } else { 99 }
    }

    fn set_changed(&mut self) {
        self.mark_changed();
    }

    fn set_data(&mut self, index: usize, value: i32) {
        // `LecternBlockEntity.dataAccess` / `setPage`: clamped to the book, the pulse follows a change.
        if self.kind == BeKind::Lectern && index == 0 {
            let page = value.clamp(0, (page_count(&self.items[0]) - 1).max(0));
            if page != self.page {
                self.page = page;
                self.page_turned = true;
                self.mark_changed();
            }
        }
    }

    fn data(&self, index: usize) -> i32 {
        if self.kind == BeKind::Lectern {
            return if index == 0 { self.page } else { 0 };
        }
        if let Some(b) = &self.beacon {
            // `BeaconBlockEntity.dataAccess`.
            return b.data(index);
        }
        if self.kind == BeKind::BrewingStand {
            // `BrewingStandBlockEntity.dataAccess`.
            return match index {
                0 => self.cook_timer,
                1 => self.lit_remaining,
                2 => self.cook_total,
                3 => self.lit_total,
                _ => 0,
            };
        }
        // `AbstractFurnaceBlockEntity.dataAccess`.
        match index {
            0 => self.lit_remaining,
            1 => self.lit_total,
            2 => self.cook_timer,
            3 => self.cook_total,
            _ => 0,
        }
    }
}

/// The container block entities of a region's chunks, by position.
#[derive(Default)]
pub(crate) struct Containers {
    pub map: BTreeMap<BlockPos, ContainerBe>,
}

fn chunk_of(pos: BlockPos) -> ChunkPos {
    ChunkPos::of_block(pos.x, pos.z)
}

fn be_pos(chunk: ChunkPos, x: usize, y: i32, z: usize) -> BlockPos {
    BlockPos::new(chunk.x * 16 + x as i32, y, chunk.z * 16 + z as i32)
}

/// The container kind of a block entity, if it is one.
fn kind_of(be: &BlockEntity) -> Option<BeKind> {
    BeKind::by_type(type_name(be.kind))
}

impl Containers {
    /// A chunk entered the region: its containers are decoded.
    pub fn chunk_loaded(&mut self, pos: ChunkPos, chunk: &Chunk) {
        for ((x, y, z), be) in chunk.block_entities() {
            if let Some(kind) = kind_of(be) {
                self.map.insert(be_pos(pos, x, y, z), ContainerBe::load(kind, be.kind, &be.nbt));
            }
        }
    }

    /// Writes the chunk's changed containers into its NBT (which marks it for saving).
    pub fn store(&mut self, pos: ChunkPos, chunk: &mut Chunk) {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        for (p, c) in self.map.range_mut(lo..=hi) {
            // (A jukebox's song advances without `setChanged`: it is saved as it stands.)
            let playing = c.kind == BeKind::Jukebox && c.song.is_some();
            if !(c.dirty || playing) || chunk_of(*p) != pos {
                continue;
            }
            c.dirty = false;
            let (x, z) = ((p.x & 15) as usize, (p.z & 15) as usize);
            let Some(old) = chunk.block_entity(x, p.y, z) else { continue };
            if old.kind != c.type_id {
                continue;
            }
            let mut be = BlockEntity::new(c.type_id);
            if let (Tag::Compound(out), Tag::Compound(fields)) = (&mut be.nbt, c.save()) {
                out.extend(fields);
            }
            chunk.set_block_entity(x, p.y, z, be);
        }
    }

    /// A chunk left the region (after [`Containers::store`]).
    pub fn chunk_unloaded(&mut self, pos: ChunkPos) {
        let gone: Vec<BlockPos> = self.chunk_positions(pos);
        for p in gone {
            self.map.remove(&p);
        }
    }

    fn chunk_positions(&self, pos: ChunkPos) -> Vec<BlockPos> {
        let lo = BlockPos::new(pos.x * 16, i32::MIN, pos.z * 16);
        let hi = BlockPos::new(pos.x * 16 + 15, i32::MAX, pos.z * 16 + 15);
        self.map.range(lo..=hi).map(|(p, _)| *p).filter(|p| chunk_of(*p) == pos).collect()
    }

    /// The chunk's block entity at `pos` was replaced from outside (commands): reload it.
    pub fn reload(&mut self, pos: BlockPos, be: Option<&BlockEntity>) {
        match be.and_then(|be| Some((kind_of(be)?, be))) {
            Some((kind, be)) => {
                self.map.insert(pos, ContainerBe::load(kind, be.kind, &be.nbt));
            }
            None => {
                self.map.remove(&pos);
            }
        }
    }

    /// After the chunk set a block at `pos` (and created or dropped its block entity): the
    /// container that is no longer there is returned (for its removal side effects), and a new
    /// container block entity gets its live state.
    pub fn block_changed(&mut self, pos: BlockPos, now: Option<&BlockEntity>) -> Option<ContainerBe> {
        let kept = match (self.map.get(&pos), now) {
            (Some(c), Some(be)) => c.type_id == be.kind,
            (Some(_), None) => false,
            (None, _) => true,
        };
        let removed = if kept { None } else { self.map.remove(&pos) };
        if let Some(be) = now
            && !self.map.contains_key(&pos)
            && let Some(kind) = kind_of(be)
        {
            self.map.insert(pos, ContainerBe::load(kind, be.kind, &be.nbt));
        }
        removed
    }

    pub fn get(&self, pos: BlockPos) -> Option<&ContainerBe> {
        self.map.get(&pos)
    }

    pub fn get_mut(&mut self, pos: BlockPos) -> Option<&mut ContainerBe> {
        self.map.get_mut(&pos)
    }

    /// Moves the containers of chunks `owner` assigns elsewhere into `parts`.
    pub fn split_into(&mut self, parts: &mut [&mut Containers], owner: impl Fn(ChunkPos) -> usize) {
        for (p, c) in std::mem::take(&mut self.map) {
            parts[owner(chunk_of(p))].map.insert(p, c);
        }
    }

    pub fn merge(&mut self, from: Containers) {
        self.map.extend(from.map);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Keeps the region's containers in step with a block change at `pos` (`LevelChunk.
/// setBlockState`): a container that went away runs `preRemoveSideEffects` (drops its
/// contents, except shulker boxes; a furnace pops its experience) unless `flags` skip it.
pub(crate) fn block_set(level: &mut RegionLevel, pos: BlockPos, flags: u32, old: u16) {
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let now = level.cells.chunk(chunk_of(pos)).and_then(|c| c.block_entity(x, pos.y, z));
    let Some(mut removed) = level.blocks.containers.block_changed(pos, now) else { return };
    level.out.removed_components.push((pos, removed.components()));
    // `JukeboxBlockEntity.preRemoveSideEffects`: the disc pops out; a jukebox cleared by a command
    // (`Clearable.tryClear`, no side effects) loses it and the music stops.
    if removed.kind == BeKind::Jukebox {
        if flags & kiln_blocks::flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS != 0 {
            crate::jukebox::cleared(level, pos, &mut removed);
        } else {
            crate::jukebox::removed(level, pos, &mut removed);
        }
        return;
    }
    // `LecternBlockEntity.preRemoveSideEffects`: the book pops out.
    if removed.kind == BeKind::Lectern && flags & kiln_blocks::flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0 {
        crate::lectern::removed(level, pos, old, &removed);
        return;
    }
    // `CampfireBlockEntity.preRemoveSideEffects`: the food on the fire drops.
    if removed.kind == BeKind::Campfire && flags & kiln_blocks::flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0 {
        let mut rng = pos_random(level, pos, 1);
        drop_contents(pos, &removed.items, &mut rng, &mut level.out.spawns);
        return;
    }
    if flags & kiln_blocks::flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS != 0 || !removed.kind.is_container() || removed.kind == BeKind::ShulkerBox {
        return;
    }
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    unpack_loot(&mut removed, pos, loot.as_deref(), false, game_time, seed);
    let mut rng = pos_random(level, pos, 1);
    drop_contents(pos, &removed.items, &mut rng, &mut level.out.spawns);
    if let BeKind::Furnace(_) = removed.kind {
        let rules = level.env.menus.clone();
        let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
        furnace::pop_experience(&mut removed, &rules, at, &mut rng, &mut level.out.spawns);
    }
}

/// `getAnalogOutputSignal` of container blocks (`AbstractContainerMenu.
/// getRedstoneSignalFromBlockEntity`; chests through `ChestBlock.getContainer`, so a blocked
/// chest reads nothing and a double chest reads both halves).
pub(crate) fn analog(level: &RegionLevel, pos: BlockPos, s: u16) -> i32 {
    let Some(c) = level.blocks.containers.get(pos) else { return 0 };
    // `LecternBlock.getAnalogOutputSignal`: how far into the book the page is.
    if c.kind == BeKind::Lectern {
        return if kiln_blocks::state::get_bool(s, "has_book") { crate::lectern::analog(c) } else { 0 };
    }
    if !c.kind.is_container() {
        return 0;
    }
    if c.kind == BeKind::Jukebox {
        return crate::jukebox::comparator_output(level, c);
    }
    if c.kind == BeKind::ChiseledBookshelf {
        return crate::bookshelf::analog(c);
    }
    if kiln_blocks::behaviour::container::is_chest(s) {
        let blocked = |p: BlockPos| kiln_data::block_logic::is_redstone_conductor(level.block(p.above()));
        if blocked(pos) {
            return 0;
        }
        if let Some(other) = kiln_blocks::behaviour::container::chest_partner(s, pos)
            && let Some(o) = level.blocks.containers.get(other).filter(|o| o.kind == c.kind)
            && kiln_blocks::behaviour::container::chest_can_connect_to(s, level.block(other))
        {
            if blocked(other) {
                return 0;
            }
            return signal_from_items(c.items.iter().chain(o.items.iter()), c.items.len() + o.items.len());
        }
    }
    c.analog()
}

/// The scheduled tick of a block whose behaviour is its block entity's: dispensers and
/// droppers dispense now; chests, barrels and ender chests recount their openers once the
/// block phase is over (the region knows the players).
pub(crate) fn scheduled_tick(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    use kiln_data::block_logic::{self as logic, BlockClass as C};
    match logic::block_class(s) {
        C::DispenserBlock | C::DropperBlock => dispense::dispense_from(level, pos, s),
        _ => level.out.rechecks.push(pos),
    }
}

/// `Level.tickBlockEntities` for hoppers and furnaces in ticking chunks, in position order.
pub(crate) fn tick_block_entities(level: &mut RegionLevel, items: &mut dyn hopper::ItemEntities, ticking: &crate::blocks::Ticking) {
    let due: Vec<(BlockPos, BeKind)> = level
        .blocks
        .containers
        .map
        .iter()
        .filter(|(_, c)| matches!(c.kind, BeKind::Hopper | BeKind::Furnace(_) | BeKind::BrewingStand | BeKind::Beacon | BeKind::Jukebox | BeKind::Campfire | BeKind::DaylightDetector | BeKind::Beehive))
        .filter(|(p, _)| ticking.contains(chunk_of(**p)))
        .map(|(p, c)| (*p, c.kind))
        .collect();
    for (pos, kind) in due {
        // Removed by an earlier tick this phase.
        if level.blocks.containers.get(pos).is_none_or(|c| c.kind != kind) {
            continue;
        }
        match kind {
            BeKind::Hopper => hopper::push_items_tick(level, items, pos),
            BeKind::BrewingStand => {
                let mut spawns = std::mem::take(&mut level.out.spawns);
                brewing::server_tick(level, pos, &mut spawns);
                level.out.spawns.append(&mut spawns);
            }
            BeKind::Beacon => beacon::tick(level, pos),
            BeKind::Jukebox => crate::jukebox::tick(level, pos),
            BeKind::Campfire => crate::campfire::tick(level, pos),
            BeKind::Beehive => crate::beehive::tick(level, pos),
            BeKind::DaylightDetector => {
                // `DaylightDetectorBlock.tickEntity` (only where the level has sky light).
                let s = level.block(pos);
                if level.env.dim == crate::OVERWORLD_ID && level.env.game_time % 20 == 0 && kiln_data::block_logic::block_class(s) == kiln_data::block_logic::BlockClass::DaylightDetectorBlock {
                    kiln_blocks::behaviour::daylight::update_signal(level, s, pos);
                }
            }
            _ => {
                let mut spawns = std::mem::take(&mut level.out.spawns);
                furnace::server_tick(level, pos, &mut spawns);
                level.out.spawns.append(&mut spawns);
            }
        }
    }
}

/// A random for a block entity's draws at `pos` this tick (dropped items, dispensing,
/// experience). Vanilla draws them from the level's random; a region's random would make them
/// depend on how the world is split, so each gets its own seed from the world seed, the time,
/// the position and `salt` (an approximation, I class).
pub(crate) fn pos_random(level: &RegionLevel, pos: BlockPos, salt: u64) -> kiln_javamath::random::LegacyRandom {
    pos_random_in(level.env, pos, salt)
}

/// [`pos_random`] from the environment.
pub(crate) fn pos_random_in(env: &crate::blocks::BlockEnv, pos: BlockPos, salt: u64) -> kiln_javamath::random::LegacyRandom {
    let mut h = (env.game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ env.seed as u64 ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    for v in [pos.x as i64, pos.y as i64, pos.z as i64] {
        h = (h ^ v as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
    }
    kiln_javamath::random::LegacyRandom::new(h as i64)
}

/// `Containers.dropItemStack`: an item entity at a random spot in the block, split into random
/// stacks of 10 to 30, flung up a little.
pub(crate) fn drop_item_stack(
    pos: [f64; 3],
    mut stack: ItemStack,
    rng: &mut dyn kiln_javamath::random::RandomSource,
    out: &mut Vec<crate::entities::Spawn>,
) {
    const WIDTH: f64 = 0.25;
    let (d, half) = (1.0 - WIDTH, WIDTH / 2.0);
    let x = pos[0].floor() + rng.next_double() * d + half;
    let y = pos[1].floor() + rng.next_double() * d;
    let z = pos[2].floor() + rng.next_double() * d + half;
    while !stack.is_empty() {
        let n = rng.next_int_bounded(21) + 10;
        let part = stack.split_count(n);
        // The `ItemEntity` constructor's random throw, replaced right after.
        rng.next_double();
        rng.next_double();
        let vel = [triangle(rng, 0.0, 0.11485000171139836), triangle(rng, 0.2, 0.11485000171139836), triangle(rng, 0.0, 0.11485000171139836)];
        out.push(crate::entities::Spawn {
            kind: &kiln_data::entities::types::ITEM,
            pos: [x, y, z],
            vel,
            body: crate::entities::Body::Item { stack: part, pickup_delay: 0, thrower: None },
        });
    }
}

/// `RandomSource.triangle`.
pub(crate) fn triangle(rng: &mut dyn kiln_javamath::random::RandomSource, mode: f64, deviation: f64) -> f64 {
    mode + deviation * (rng.next_double() - rng.next_double())
}

/// `Containers.dropContents`.
pub(crate) fn drop_contents(pos: BlockPos, items: &[ItemStack], rng: &mut dyn kiln_javamath::random::RandomSource, out: &mut Vec<crate::entities::Spawn>) {
    for s in items {
        drop_item_stack([pos.x as f64, pos.y as f64, pos.z as f64], s.clone(), rng, out);
    }
}

/// The loot context of a container rolling its loot table (`LootContextParamSets.CHEST`).
struct ChestLoot {
    origin: [f64; 3],
    player: bool,
}

impl kiln_loot::LootContext for ChestLoot {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.player && target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
}

/// `RandomizableContainer.unpackLootTable`: rolls the loot table into the container (once).
/// A zero seed means the table's random sequence in vanilla; Kiln seeds it from the position
/// and time so regions stay independent (an approximation, I class).
pub(crate) fn unpack_loot(c: &mut ContainerBe, pos: BlockPos, loot: Option<&kiln_loot::LootData>, player: bool, game_time: i64, world_seed: i64) {
    let Some(table) = c.loot_table.take() else { return };
    c.mark_changed();
    let Some(loot) = loot else { return };
    let origin = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    fill_from_table(&mut c.items, loot, &table, c.loot_seed, origin, [pos.x, pos.y, pos.z], player, game_time, world_seed);
}

/// `LootTable.fill` into `items` for a container (a block entity's or a minecart's) at `at`
/// (whose block position seeds a zero `seed`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_from_table(
    items: &mut [ItemStack],
    loot: &kiln_loot::LootData,
    table: &str,
    seed: i64,
    origin: [f64; 3],
    at: [i32; 3],
    player: bool,
    game_time: i64,
    world_seed: i64,
) {
    let Some(id) = kiln_item::ident::Identifier::parse(table) else { return };
    let seed = if seed != 0 {
        seed
    } else {
        let mut h = (game_time as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ world_seed as u64;
        for v in at {
            h = (h ^ v as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            h ^= h >> 31;
        }
        (h | 1) as i64
    };
    let ctx = ChestLoot { origin, player };
    let mut rng = kiln_loot::random::seeded(seed);
    loot.fill(&id, &ctx, &mut rng, items);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_form_round_trips() {
        let mut c = ContainerBe::load(BeKind::Chest, kiln_world::block_entity::type_id("minecraft:chest").unwrap(), &Tag::Compound(Vec::new()));
        c.items[3] = ItemStack::of("minecraft:stone", 12).unwrap();
        c.custom_name = Some(Tag::String("Loot".into()));
        let saved = c.save();
        let back = ContainerBe::load(BeKind::Chest, c.type_id, &saved);
        assert_eq!(back.items[3].count(), 12);
        assert_eq!(back.custom_name, c.custom_name);
        // A loot table replaces the items in the saved form.
        c.loot_table = Some("minecraft:chests/simple_dungeon".into());
        c.loot_seed = 5;
        let saved = c.save();
        assert!(saved.get("Items").is_none());
        assert_eq!(saved.get("LootTableSeed").and_then(Tag::as_i64), Some(5));
    }

    #[test]
    fn comparator_levels_follow_fullness() {
        let stone = |n| ItemStack::of("minecraft:stone", n).unwrap();
        let pearl = |n| ItemStack::of("minecraft:ender_pearl", n).unwrap();
        let mut items = vec![ItemStack::empty(); 27];
        assert_eq!(signal_from_items(items.iter(), 27), 0);
        items[0] = stone(1);
        assert_eq!(signal_from_items(items.iter(), 27), 1);
        items[0] = pearl(16);
        assert_eq!(signal_from_items(items.iter(), 27), 1);
        let full: Vec<ItemStack> = (0..27).map(|_| stone(64)).collect();
        assert_eq!(signal_from_items(full.iter(), 27), 15);
        let hopper = [stone(64), stone(64), ItemStack::empty(), ItemStack::empty(), ItemStack::empty()];
        // 2/5 full: floor(0.4 * 14) + 1 = 6.
        assert_eq!(signal_from_items(hopper.iter(), 5), 6);
    }
}
