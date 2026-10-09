//! Menus opened on blocks: `useWithoutItem` of container blocks and workstations,
//! `ServerPlayer.openMenu` / `closeContainer`, `stillValid`, and the openers count
//! (`ContainerOpenersCounter`) with its sounds, block events, barrel state and trapped chest
//! signal.

use super::{BeKind, ContainerBe, Containers, translatable};
use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use kiln_blocks::behaviour::container::{self as cblock, chest_partner};
use kiln_blocks::{BlockId, BlockPos, Effect, Level, flags, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_inventory::persist::ItemList;
use kiln_inventory::{Container, Menu, SimpleContainer};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `Player.blockInteractionRange` default.
const BLOCK_INTERACTION_RANGE: f64 = 4.5;
/// `ContainerOpenersCounter.CHECK_TICK_DELAY`.
const RECHECK_DELAY: i32 = 5;

/// What a player's open menu shows besides the player's own inventory.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OpenBlock {
    /// Container block entities (a double chest's two halves: first, then second), each with
    /// the serial of the block entity it was opened on.
    Containers { first: (BlockPos, u64), second: Option<(BlockPos, u64)> },
    /// The player's ender chest items, opened at this ender chest.
    EnderChest { pos: BlockPos, serial: u64 },
    /// A workstation (crafting table, stonecutter, smithing table): valid while the block is
    /// there (`ContainerLevelAccess.stillValid`).
    Workstation { pos: BlockPos, block: BlockId },
    /// A chest or hopper minecart's slots (entity `entity`), shown through the player's own
    /// copy [`PlayerContainers::cart`], which the region keeps in step with the minecart
    /// around every menu operation (see [`crate::carts`]).
    Cart { entity: i32 },
}

/// The player's container state: the menu counter, what the open menu is on, and the ender
/// chest inventory (`PlayerEnderChestContainer`, saved as `EnderItems`).
#[derive(Debug, Clone)]
pub(crate) struct PlayerContainers {
    /// `ServerPlayer.containerCounter`.
    pub counter: i32,
    pub open: Option<OpenBlock>,
    pub ender: SimpleContainer,
    /// The open minecart menu's slots ([`OpenBlock::Cart`]).
    pub cart: SimpleContainer,
    /// Where the open entity menu's chest minecart or chest boat is (`stopOpen` posts
    /// `container_close` there when the menu closes); `None` for the others.
    pub cart_event_pos: Option<[f64; 3]>,
    /// For the screen of a mount: how often its inventory had been made anew when the screen
    /// opened (a new inventory closes the screen).
    pub cart_serial: Option<u32>,
    /// A menu closed while the entity was ticking: its `container_close` still to post.
    pub cart_closed: Option<[f64; 3]>,
    ender_undecoded: Vec<(i32, Tag)>,
    /// Workstation effects of the last menu operation (grindstone and anvil use), for the
    /// region to carry out at the workstation.
    pub pending: Vec<kiln_inventory::Effect>,
    /// `Player.enchantmentSeed` (saved as `XpSeed`).
    pub enchantment_seed: i32,
    /// Bookshelves around the open enchanting table, counted before each menu operation.
    pub bookshelves: i32,
    /// `LoomMenu.lastSoundTime`.
    pub last_loom_sound: i64,
    pub last_cartography_sound: i64,
}

impl PlayerContainers {
    /// From saved player data (`EnderItems`).
    pub fn load(player: &Tag) -> PlayerContainers {
        let list = ItemList::load(player.get("EnderItems"), 27);
        PlayerContainers {
            counter: 0,
            open: None,
            ender: SimpleContainer::from_items(list.stacks),
            cart: SimpleContainer::default(),
            cart_event_pos: None,
            cart_serial: None,
            cart_closed: None,
            ender_undecoded: list.undecoded,
            pending: Vec::new(),
            enchantment_seed: player.get("XpSeed").and_then(Tag::as_i64).unwrap_or(0) as i32,
            bookshelves: 0,
            last_loom_sound: i64::MIN,
            last_cartography_sound: i64::MIN,
        }
    }

    /// Writes `EnderItems` into saved player data.
    pub fn save_into(&self, player: &mut Tag) {
        let Tag::Compound(fields) = player else { return };
        let list = ItemList { stacks: self.ender.items.clone(), undecoded: self.ender_undecoded.clone() }.save();
        for (key, value) in [("EnderItems", list), ("XpSeed", Tag::Int(self.enchantment_seed))] {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => fields.push((key.into(), value)),
            }
        }
    }
}

/// `CompoundContainer`: a double chest's two halves as one container.
struct Compound<'a> {
    a: &'a mut ContainerBe,
    b: &'a mut ContainerBe,
}

impl Container for Compound<'_> {
    fn size(&self) -> usize {
        self.a.size() + self.b.size()
    }
    fn item(&self, slot: usize) -> &ItemStack {
        let n = self.a.size();
        if slot >= n { self.b.item(slot - n) } else { self.a.item(slot) }
    }
    fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        let n = self.a.size();
        if slot >= n { self.b.item_mut(slot - n) } else { self.a.item_mut(slot) }
    }
    fn set_item(&mut self, slot: usize, stack: ItemStack) {
        let n = self.a.size();
        if slot >= n { self.b.set_item(slot - n, stack) } else { self.a.set_item(slot, stack) }
    }
    fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        let n = self.a.size();
        if slot >= n { self.b.remove_item(slot - n, count) } else { self.a.remove_item(slot, count) }
    }
    fn remove_item_no_update(&mut self, slot: usize) -> ItemStack {
        let n = self.a.size();
        if slot >= n { self.b.remove_item_no_update(slot - n) } else { self.a.remove_item_no_update(slot) }
    }
    fn max_stack_size(&self) -> i32 {
        self.a.max_stack_size()
    }
    fn set_changed(&mut self) {
        self.a.set_changed();
        self.b.set_changed();
    }
}

impl Player {
    /// Runs `f` on the open menu (or the inventory menu) with the block container it shows,
    /// and carries out its effects: packets to the client, dropped items to `spawns`. Without
    /// `containers` (outside region work) a menu on block containers is left alone and `f`
    /// gets the inventory menu.
    pub(crate) fn with_menu_at<R>(
        &mut self,
        rules: &kiln_inventory::Rules,
        spawns: &mut Vec<Spawn>,
        mut containers: Option<&mut Containers>,
        f: impl FnOnce(&mut Menu, Option<&mut Menu>, &mut kiln_inventory::Env) -> R,
    ) -> R {
        let mut out = Vec::new();
        let player = self.player_flags();
        let result = {
            let Player { inv, menu, open_menu, maps, containers: pc, loot, level_rng, entity_rng, limited_crafting, recipe_book, .. } = self;
            let PlayerContainers { open, ender, cart, bookshelves, .. } = pc;
            let mut world = super::world::SimWorld {
                loot: loot.as_deref(),
                rng: level_rng,
                player_rng: entity_rng,
                bookshelves: *bookshelves,
                limited_crafting: *limited_crafting,
                recipes: &*recipe_book,
                maps,
            };
            // A double chest's second half is taken out while the menu works on both.
            let mut second_taken: Option<(BlockPos, ContainerBe)> = None;
            if let (Some(OpenBlock::Containers { second: Some((p, _)), .. }), Some(cs)) = (&*open, containers.as_deref_mut()) {
                second_taken = cs.map.remove(p).map(|c| (*p, c));
            }
            let result;
            {
                let mut both: Option<Compound>;
                let (block, use_open): (Option<&mut dyn Container>, bool) = match (&*open, containers.as_deref_mut()) {
                    (Some(OpenBlock::Containers { first, second }), Some(cs)) => match (cs.map.get_mut(&first.0), second_taken.as_mut()) {
                        (Some(a), Some((_, b))) if second.is_some() => {
                            both = Some(Compound { a, b });
                            (both.as_mut().map(|c| c as &mut dyn Container), true)
                        }
                        (Some(a), None) if second.is_none() => (Some(a as &mut dyn Container), true),
                        // A half is gone: the menu closes at the next validity check.
                        _ => (None, false),
                    },
                    (Some(OpenBlock::Containers { .. }), None) => (None, false),
                    (Some(OpenBlock::EnderChest { .. }), _) => (Some(ender as &mut dyn Container), true),
                    (Some(OpenBlock::Cart { .. }), _) => (Some(cart as &mut dyn Container), true),
                    _ => (None, true),
                };
                let mut env = kiln_inventory::Env { inventory: inv, block, player, rules, world: &mut world, out: &mut out };
                result = match open_menu {
                    Some(o) if use_open => f(o, Some(menu), &mut env),
                    _ => f(menu, None, &mut env),
                };
            }
            if let (Some((p, c)), Some(cs)) = (second_taken, containers.as_deref_mut()) {
                cs.map.insert(p, c);
            }
            result
        };
        if let Some(st) = self.open_menu.as_mut().and_then(|m| m.merchant_state_mut()) {
            let v = st.merchant;
            self.merchant_events.extend(st.drain().into_iter().map(|e| (v, e)));
        }
        let mut took_result = false;
        let mut crafted_recipes = Vec::new();
        for effect in out {
            if let Some(pkt) = effect.encode() {
                self.send(pkt);
            }
            match effect {
                kiln_inventory::Effect::Drop { stack, retain_ownership } => {
                    // `ServerPlayer.drop` with ownership kept: the dropped statistics.
                    if retain_ownership && !stack.is_empty() {
                        self.award_stat(crate::player_stats::Stat::item(crate::player_stats::DROPPED, stack.item()), stack.count());
                        self.award_stat(*crate::player_stats::stat::DROP, 1);
                    }
                    spawns.push(self.throw(stack))
                }
                kiln_inventory::Effect::Crafted { item, amount, recipe } => {
                    took_result = true;
                    self.award_stat(crate::player_stats::Stat::item(crate::player_stats::CRAFTED, item), amount);
                    if let Some(r) = recipe {
                        crafted_recipes.push(r);
                        let id = rules.recipes.id(r).to_owned();
                        self.recipe_crafted(&id, &[]);
                    }
                }
                kiln_inventory::Effect::InventoryChanged { stack, .. } => self.inventory_changed(&stack),
                // `ArmorSlot.setByPlayer` → `LivingEntity.onEquipItem`: the equip sound.
                kiln_inventory::Effect::Equip { slot, old, new } => self.on_equip_item(slot, &old, &new),
                // `BrewedPotionTrigger`.
                kiln_inventory::Effect::BrewedPotion { potion } => {
                    self.fire_conds("minecraft:brewed_potion", None, |c, _, _| {
                        c.get("potion").and_then(|v| v.as_str()).is_none_or(|want| {
                            potion.and_then(|p| kiln_item::registry::POTION.name(p)).is_some_and(|have| {
                                kiln_item::ident::Identifier::parse(want).is_some_and(|w| w.to_string() == have)
                            })
                        })
                    });
                }
                e @ (kiln_inventory::Effect::GrindstoneUsed { .. }
                | kiln_inventory::Effect::AnvilUsed { .. }
                | kiln_inventory::Effect::LoomUsed
                | kiln_inventory::Effect::CartographyUsed
                | kiln_inventory::Effect::Enchanted { .. }) => self.containers.pending.push(e),
                _ => {}
            }
        }
        // `RecipeCraftingHolder.awardUsedRecipes`.
        if !crafted_recipes.is_empty() {
            self.award_recipes(rules, &crafted_recipes);
        }
        // `FurnaceResultSlot.checkTakeAchievements`: the furnace's experience pops at the player
        // and its recipes unlock.
        if took_result
            && let Some(OpenBlock::Containers { first, second: None }) = &self.containers.open
            && let Some(c) = containers.as_deref_mut().and_then(|cs| cs.map.get_mut(&first.0))
            && matches!(c.kind, BeKind::Furnace(_))
        {
            let at = self.pos;
            let used = super::furnace::pop_experience(c, rules, at, &mut self.entity_rng, spawns);
            self.award_recipes(rules, &used);
        }
        // Menu changes to a furnace's input restart its cooking right away.
        if let Some(OpenBlock::Containers { first, second: None }) = &self.containers.open
            && let Some(c) = containers.and_then(|cs| cs.map.get_mut(&first.0))
        {
            super::furnace::apply_input_change(c, rules);
        }
        result
    }

    /// `ServerPlayer.closeContainer` (with the Container Close packet when `notify`):
    /// `removed` on the open menu, the inventory menu takes over its view, and the block
    /// container loses an opener.
    pub(crate) fn close_block_menu(&mut self, rules: &kiln_inventory::Rules, spawns: &mut Vec<Spawn>, level: &mut RegionLevel, notify: bool) {
        let Some(open) = self.open_menu.as_ref() else { return };
        if notify {
            self.send(kiln_inventory::effect::container_close(open.container_id));
        }
        self.with_menu_at(rules, spawns, Some(&mut level.blocks.containers), |open, inv_menu, env| {
            kiln_inventory::click::close_container(open, inv_menu, env)
        });
        self.open_menu = None;
        if let Some(block) = self.containers.open.take() {
            stop_open(level, &block, self.game_mode == 3);
        }
        // `ChestMenu.removed` → `MinecartChest.stopOpen` / `AbstractChestBoat.stopOpen`.
        if let Some(at) = self.containers.cart_event_pos.take() {
            self.post_container_close(level, at);
        }
    }

    /// `level.gameEvent(CONTAINER_CLOSE, position, Context.of(player))`.
    pub(crate) fn post_container_close(&self, level: &mut RegionLevel, at: [f64; 3]) {
        if crate::sculk::listening(level) {
            let ctx = kiln_entity::vibration::Context { source: Some(crate::blocks::player_source(self)), affected_state: None };
            crate::sculk::post(level, "minecraft:container_close", kiln_entity::math::Vec3::new(at[0], at[1], at[2]), ctx);
        }
    }

    /// `AbstractContainerMenu.stillValid` for the open menu: its block (entity) is still there
    /// and within reach (`Player.isWithinBlockInteractionRange(pos, 4.0)`).
    pub(crate) fn menu_still_valid(&self, level: &RegionLevel) -> bool {
        let Some(open) = &self.containers.open else { return true };
        let near = |pos: BlockPos| self.within_block_reach(pos, 4.0);
        let be_ok = |(pos, serial): (BlockPos, u64)| level.blocks.containers.get(pos).is_some_and(|c| c.serial == serial) && near(pos);
        match *open {
            OpenBlock::Containers { first, second } => be_ok(first) && second.is_none_or(be_ok),
            OpenBlock::EnderChest { pos, serial } => be_ok((pos, serial)),
            // A minecart's menu is checked against the entity ([`crate::carts::check_menus`]).
            OpenBlock::Cart { .. } => true,
            OpenBlock::Workstation { pos, block } => {
                // `AnvilMenu.isValidBlock`: any anvil (it wears while open).
                let now = level.block(pos);
                let anvil = |s: u16| kiln_blocks::tags::is(s, "minecraft:anvil");
                (BlockId::of(now) == block || anvil(now) && anvil(block.default_state())) && near(pos)
            }
        }
    }

    /// `Player.isWithinBlockInteractionRange`: the eyes within the reach (plus `extra`) of the
    /// block's box.
    fn within_block_reach(&self, pos: BlockPos, extra: f64) -> bool {
        let eye = [self.pos[0], self.pos[1] + if self.sneaking { 1.27 } else { 1.62 }, self.pos[2]];
        let lo = [pos.x as f64, pos.y as f64, pos.z as f64];
        let d2: f64 = (0..3).map(|i| (eye[i] - eye[i].clamp(lo[i], lo[i] + 1.0)).powi(2)).sum();
        let r = BLOCK_INTERACTION_RANGE + extra;
        d2 < r * r
    }
}

/// A menu to open: its kind's constructor, title and what it shows.
struct Provider {
    make: fn(i32) -> Menu,
    title: Tag,
    block: OpenBlock,
    /// Positions whose containers gain an opener (`startOpen`).
    openers: Vec<BlockPos>,
}

fn generic_3(id: i32) -> Menu {
    Menu::generic(id, 3)
}
fn generic_6(id: i32) -> Menu {
    Menu::generic(id, 6)
}
fn furnace_menu(kind: kiln_inventory::FurnaceKind) -> fn(i32) -> Menu {
    match kind {
        kiln_inventory::FurnaceKind::Furnace => |id| Menu::furnace(id, kiln_inventory::FurnaceKind::Furnace),
        kiln_inventory::FurnaceKind::BlastFurnace => |id| Menu::furnace(id, kiln_inventory::FurnaceKind::BlastFurnace),
        kiln_inventory::FurnaceKind::Smoker => |id| Menu::furnace(id, kiln_inventory::FurnaceKind::Smoker),
    }
}

/// `ChestBlock.isChestBlockedAt`: a redstone conductor above (cats sitting on chests are not
/// simulated).
fn chest_blocked(level: &RegionLevel, pos: BlockPos) -> bool {
    logic::is_redstone_conductor(level.block(pos.above()))
}

/// `ShulkerBoxBlock.canOpen` for a closed box: nothing in the way of its lid (approximated by
/// the block it faces having no collision).
fn shulker_blocked(level: &RegionLevel, pos: BlockPos, s: u16, openers: i32) -> bool {
    if openers > 0 {
        return false;
    }
    let facing = state::get_dir(s, "facing").unwrap_or(kiln_blocks::Direction::Up);
    !kiln_data::block_props::collision(level.block(pos.relative(facing))).is_empty()
}

/// The container menu of a container block at `pos` (`getMenuProvider`), if it has one.
fn container_provider(level: &RegionLevel, pos: BlockPos, s: u16) -> Option<Provider> {
    let c = level.blocks.containers.get(pos)?;
    let single = |make: fn(i32) -> Menu| Provider {
        make,
        title: c.title(),
        block: OpenBlock::Containers { first: (pos, c.serial), second: None },
        openers: vec![pos],
    };
    Some(match c.kind {
        BeKind::Chest | BeKind::TrappedChest => {
            if chest_blocked(level, pos) {
                return None;
            }
            match chest_partner(s, pos) {
                Some(other) => {
                    let o = level.blocks.containers.get(other).filter(|o| o.kind == c.kind && cblock::chest_can_connect_to(s, level.block(other)))?;
                    if chest_blocked(level, other) {
                        return None;
                    }
                    // `DoubleBlockCombiner`: the right half comes first.
                    let (first, second) = if state::get(s, "type") == Some("right") { ((pos, c), (other, o)) } else { ((other, o), (pos, c)) };
                    let title = first.1.custom_name.clone().or_else(|| second.1.custom_name.clone()).unwrap_or_else(|| translatable("container.chestDouble"));
                    Provider {
                        make: generic_6,
                        title,
                        block: OpenBlock::Containers { first: (first.0, first.1.serial), second: Some((second.0, second.1.serial)) },
                        openers: vec![first.0, second.0],
                    }
                }
                None => single(generic_3),
            }
        }
        BeKind::Barrel => single(generic_3),
        BeKind::ShulkerBox => {
            if shulker_blocked(level, pos, s, c.openers) {
                return None;
            }
            single(Menu::shulker_box)
        }
        BeKind::Hopper => single(Menu::hopper),
        BeKind::Dispenser | BeKind::Dropper => single(Menu::generic_3x3),
        BeKind::Crafter => single(Menu::crafter),
        BeKind::Furnace(kind) => single(furnace_menu(kind)),
        BeKind::BrewingStand => single(Menu::brewing_stand),
        BeKind::Beacon => single(Menu::beacon),
        // `LecternBlock.getMenuProvider`: only with a book.
        BeKind::Lectern if state::get_bool(s, "has_book") => single(Menu::lectern),
        BeKind::Lectern => return None,
        BeKind::EnderChest | BeKind::Jukebox | BeKind::Campfire | BeKind::ChiseledBookshelf | BeKind::DaylightDetector | BeKind::Bell | BeKind::Beehive | BeKind::Vault | BeKind::DecoratedPot | BeKind::Brushable | BeKind::CommandBlock => return None,
    })
}

/// Workstations (`SimpleMenuProvider`s over `ContainerLevelAccess`) Kiln has menus for.
fn workstation_provider(s: u16, pos: BlockPos) -> Option<Provider> {
    let (make, title): (fn(i32) -> Menu, &str) = match logic::block_class(s) {
        C::CraftingTableBlock => (Menu::crafting, "container.crafting"),
        C::GrindstoneBlock => (Menu::grindstone, "container.grindstone_title"),
        C::AnvilBlock => (Menu::anvil, "container.repair"),
        C::LoomBlock => (Menu::loom, "container.loom"),
        C::CartographyTableBlock => (Menu::cartography_table, "container.cartography_table"),
        // The seed is the player's (set when the menu opens).
        C::EnchantingTableBlock => (|id| Menu::enchantment(id, 0), "container.enchant"),
        C::StonecutterBlock => (Menu::stonecutter, "container.stonecutter"),
        C::SmithingTableBlock => (Menu::smithing, "container.upgrade"),
        _ => return None,
    };
    Some(Provider { make, title: translatable(title), block: OpenBlock::Workstation { pos, block: BlockId::of(s) }, openers: Vec::new() })
}

/// `useWithoutItem` of blocks that open menus: `Some(consumed)` for those blocks (chests
/// answer even when blocked), `None` for others.
pub(crate) fn use_block(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) -> Option<bool> {
    let s = level.block(pos);
    if logic::block_class(s) == C::EnderChestBlock {
        // `EnderChestBlock.useWithoutItem`: blocked by a conductor above (still consumed).
        if !chest_blocked(level, pos)
            && let Some(c) = level.blocks.containers.get(pos)
        {
            let provider = Provider {
                make: generic_3,
                title: translatable("container.enderchest"),
                block: OpenBlock::EnderChest { pos, serial: c.serial },
                openers: vec![pos],
            };
            open_menu(p, level, provider, spawns);
            p.award_stat(*crate::player_stats::stat::OPEN_ENDERCHEST, 1);
        }
        return Some(true);
    }
    if let Some(provider) = workstation_provider(s, pos) {
        open_menu(p, level, provider, spawns);
        if let Some(stat) = crate::player_stats::interact_stat(s) {
            p.award_stat(stat, 1);
        }
        return Some(true);
    }
    // `CommandBlock.useWithoutItem`: a game master's click opens the block's screen.
    if logic::block_class(s) == C::CommandBlock {
        return crate::command_block::use_without_item(p, level, pos);
    }
    // `LecternBlock.useWithoutItem`: a lectern with a book opens its menu; without one the click is consumed.
    if logic::block_class(s) == C::LecternBlock {
        if state::get_bool(s, "has_book")
            && let Some(provider) = container_provider(level, pos, s)
        {
            open_menu(p, level, provider, spawns);
            p.award_stat(*crate::player_stats::stat::INTERACT_WITH_LECTERN, 1);
        }
        return Some(true);
    }
    // (A jukebox has no menu: its own `useWithoutItem` takes the disc out.)
    if matches!(level.blocks.containers.get(pos)?.kind, BeKind::Jukebox | BeKind::Campfire | BeKind::ChiseledBookshelf | BeKind::DaylightDetector | BeKind::Bell | BeKind::Beehive | BeKind::Vault | BeKind::DecoratedPot | BeKind::Brushable | BeKind::CommandBlock) {
        return None;
    }
    if let Some(provider) = container_provider(level, pos, s) {
        open_menu(p, level, provider, spawns);
        if let Some(stat) = crate::player_stats::interact_stat(s) {
            p.award_stat(stat, 1);
        }
    }
    Some(true)
}

/// `ServerPlayerGameMode.useItemOn` for spectators: blocks with a menu provider open it.
pub(crate) fn spectator_use(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) -> bool {
    let s = level.block(pos);
    let provider = workstation_provider(s, pos).or_else(|| container_provider(level, pos, s));
    match provider {
        Some(provider) => {
            open_menu(p, level, provider, spawns);
            true
        }
        None => false,
    }
}

/// `ServerPlayer.openMenu`.
fn open_menu(p: &mut Player, level: &mut RegionLevel, provider: Provider, spawns: &mut Vec<Spawn>) {
    let rules = level.env.menus.clone();
    if p.open_menu.is_some() {
        p.close_block_menu(&rules, spawns, level, true);
    }
    let spectator = p.game_mode == 3;
    // `createMenu`: containers check their lock and roll their loot table first.
    if let OpenBlock::Containers { first, second } = &provider.block {
        let positions = [Some(first.0), second.map(|s| s.0)];
        let held = p.inv.selected_item().clone();
        let loot = level.env.loot.clone();
        let can_open = positions.iter().flatten().all(|&pos| level.blocks.containers.get(pos).is_none_or(|c| c.can_open(spectator, &held, loot.as_deref())));
        if !can_open {
            if spectator {
                p.send(kiln_proto::packets::system_chat(spectator_cant_open(), true));
            } else {
                let name = provider.title.clone();
                p.send(kiln_proto::packets::system_chat(locked_message(name), true));
                level.effect(Effect::Sound { pos: first.0, sound: "minecraft:block.chest.locked", volume: 1.0, pitch: 1.0 });
            }
            return;
        }
        let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
        for pos in positions.into_iter().flatten() {
            if let Some(c) = level.blocks.containers.get_mut(pos) {
                let table = c.loot_table.clone();
                super::unpack_loot(c, pos, loot.as_deref(), true, game_time, seed);
                // `unpackLootTable(player)`: `player_generates_container_loot`.
                if let Some(table) = table {
                    let table = kiln_item::ident::Identifier::parse(&table).map_or(table, |i| i.to_string());
                    p.fire_conds("minecraft:player_generates_container_loot", None, |c, _, _| {
                        c.get("loot_tables").and_then(|v| v.as_str()).and_then(kiln_item::ident::Identifier::parse).is_some_and(|i| i.to_string() == table)
                    });
                }
            }
        }
    }
    p.containers.counter = p.containers.counter % 100 + 1;
    let id = p.containers.counter;
    let mut menu = (provider.make)(id);
    if menu.kind == kiln_inventory::MenuKind::Enchantment {
        menu = Menu::enchantment(id, p.containers.enchantment_seed);
    }
    if let OpenBlock::Workstation { pos, .. } = provider.block {
        p.containers.bookshelves = super::world::count_bookshelves(level, pos);
    }
    // The menu constructor's `container.startOpen(player)`.
    for &pos in &provider.openers {
        start_open(level, pos, spectator);
    }
    if let Some(ty) = menu.kind.menu_type_id() {
        p.send(kiln_inventory::effect::open_screen(id, ty, &provider.title));
    }
    p.containers.open = Some(provider.block);
    p.open_menu = Some(menu);
    p.with_menu_at(&rules, spawns, Some(&mut level.blocks.containers), |menu, _, env| menu.open(env));
}

fn spectator_cant_open() -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String("container.spectatorCantOpen".into())), ("color".into(), Tag::String("red".into()))])
}

fn locked_message(name: Tag) -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String("container.isLocked".into())), ("with".into(), Tag::List(vec![name]))])
}

/// The open and close sounds of a chest-like block, and whether it signals its openers with
/// a block event.
fn opener_sounds(s: u16) -> Option<(&'static str, &'static str)> {
    let name = BlockId::of(s).name();
    Some(match logic::block_class(s) {
        C::BarrelBlock => ("minecraft:block.barrel.open", "minecraft:block.barrel.close"),
        C::EnderChestBlock => ("minecraft:block.ender_chest.open", "minecraft:block.ender_chest.close"),
        _ if logic::is_instance(s, C::CopperChestBlock) => {
            if name.contains("oxidized") {
                ("minecraft:block.copper_chest_oxidized.open", "minecraft:block.copper_chest_oxidized.close")
            } else if name.contains("weathered") {
                ("minecraft:block.copper_chest_weathered.open", "minecraft:block.copper_chest_weathered.close")
            } else {
                ("minecraft:block.copper_chest.open", "minecraft:block.copper_chest.close")
            }
        }
        _ if cblock::is_chest(s) => ("minecraft:block.chest.open", "minecraft:block.chest.close"),
        _ => return None,
    })
}

/// `ChestBlockEntity.playSound` / `BarrelBlockEntity.playSound` / the ender chest's: volume 0.5,
/// pitch 0.9 to 1.0. The left half of a double chest stays silent.
fn opener_sound(level: &mut RegionLevel, pos: BlockPos, s: u16, open: bool) {
    let Some((o, c)) = opener_sounds(s) else { return };
    if cblock::is_chest(s) && state::get(s, "type") == Some("left") {
        return;
    }
    let pitch = super::pos_random(level, pos, 5).next_float() * 0.1 + 0.9;
    level.effect(Effect::Sound { pos, sound: if open { o } else { c }, volume: 0.5, pitch });
}

/// Called with the count before and after a change (`openerCountChanged`).
fn openers_changed(level: &mut RegionLevel, pos: BlockPos, s: u16, before: i32, after: i32) {
    if cblock::is_chest(s) || logic::block_class(s) == C::EnderChestBlock {
        // `ChestBlockEntity.signalOpenCount` (the lid).
        kiln_blocks::block_events::block_event(level, pos, BlockId::of(s), 1, after);
    }
    if logic::block_class(s) == C::TrappedChestBlock {
        cblock::trapped_chest_count_changed(level, s, pos, before, after);
    }
}

/// `onOpen` / `onClose` of the openers counter.
fn open_changed(level: &mut RegionLevel, pos: BlockPos, s: u16, open: bool) {
    opener_sound(level, pos, s, open);
    level.effect(Effect::GameEvent { pos, event: if open { "minecraft:container_open" } else { "minecraft:container_close" } });
    if logic::block_class(s) == C::BarrelBlock {
        // `BarrelBlockEntity.updateBlockState`.
        kiln_blocks::set_block(level, pos, state::set_bool(s, "open", open), flags::ALL);
    }
}

/// `startOpen` of the container at `pos` (spectators do not count).
pub(crate) fn start_open(level: &mut RegionLevel, pos: BlockPos, spectator: bool) {
    if spectator {
        return;
    }
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    match c.kind {
        BeKind::Chest | BeKind::TrappedChest | BeKind::Barrel | BeKind::EnderChest => {
            // `ContainerOpenersCounter.incrementOpeners`.
            let before = c.openers;
            c.openers += 1;
            c.max_range = c.max_range.max(BLOCK_INTERACTION_RANGE);
            let after = c.openers;
            if before == 0 {
                open_changed(level, pos, s, true);
                kiln_blocks::schedule_block_tick(level, pos, BlockId::of(s), RECHECK_DELAY, kiln_blocks::TickPriority::Normal);
            }
            openers_changed(level, pos, s, before, after);
        }
        BeKind::ShulkerBox => {
            // `ShulkerBoxBlockEntity.startOpen`.
            c.openers = c.openers.max(0) + 1;
            let n = c.openers;
            kiln_blocks::block_events::block_event(level, pos, BlockId::of(s), 1, n);
            if n == 1 {
                level.effect(Effect::GameEvent { pos, event: "minecraft:container_open" });
                let pitch = super::pos_random(level, pos, 5).next_float() * 0.1 + 0.9;
                level.effect(Effect::Sound { pos, sound: "minecraft:block.shulker_box.open", volume: 0.5, pitch });
            }
        }
        _ => {}
    }
}

/// `stopOpen` for every container the closed menu showed.
fn stop_open(level: &mut RegionLevel, block: &OpenBlock, spectator: bool) {
    if spectator {
        return;
    }
    let positions: Vec<BlockPos> = match *block {
        OpenBlock::Containers { first, second } => std::iter::once(first.0).chain(second.map(|s| s.0)).collect(),
        OpenBlock::EnderChest { pos, .. } => vec![pos],
        OpenBlock::Workstation { .. } | OpenBlock::Cart { .. } => Vec::new(),
    };
    for pos in positions {
        let serial_ok = match *block {
            OpenBlock::Containers { first, second } => [Some(first), second].into_iter().flatten().any(|(p, sr)| p == pos && level.blocks.containers.get(p).is_some_and(|c| c.serial == sr)),
            OpenBlock::EnderChest { serial, .. } => level.blocks.containers.get(pos).is_some_and(|c| c.serial == serial),
            OpenBlock::Workstation { .. } | OpenBlock::Cart { .. } => false,
        };
        if serial_ok {
            stop_open_at(level, pos);
        }
    }
}

fn stop_open_at(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    match c.kind {
        BeKind::Chest | BeKind::TrappedChest | BeKind::Barrel | BeKind::EnderChest => {
            // `ContainerOpenersCounter.decrementOpeners`.
            let before = c.openers;
            c.openers -= 1;
            let after = c.openers;
            if after == 0 {
                c.max_range = 0.0;
                open_changed(level, pos, s, false);
            }
            openers_changed(level, pos, s, before, after);
        }
        BeKind::ShulkerBox => {
            // `ShulkerBoxBlockEntity.stopOpen`.
            c.openers -= 1;
            let n = c.openers;
            kiln_blocks::block_events::block_event(level, pos, BlockId::of(s), 1, n);
            if n <= 0 {
                level.effect(Effect::GameEvent { pos, event: "minecraft:container_close" });
                let pitch = super::pos_random(level, pos, 5).next_float() * 0.1 + 0.9;
                level.effect(Effect::Sound { pos, sound: "minecraft:block.shulker_box.close", volume: 0.5, pitch });
            }
        }
        _ => {}
    }
}

/// Whether the player's open menu shows the container at `pos` (`isOwnContainer`).
fn has_open(p: &Player, pos: BlockPos) -> bool {
    match &p.containers.open {
        Some(OpenBlock::Containers { first, second }) => first.0 == pos || second.is_some_and(|s| s.0 == pos),
        Some(OpenBlock::EnderChest { pos: at, .. }) => *at == pos,
        _ => false,
    }
}

/// `ContainerOpenersCounter.recheckOpeners` (the scheduled tick of chests, barrels and ender
/// chests): counts the players in range with the container open.
pub(crate) fn recheck_openers(level: &mut RegionLevel, players: &[&mut Player], pos: BlockPos) {
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get(pos) else { return };
    if !matches!(c.kind, BeKind::Chest | BeKind::TrappedChest | BeKind::Barrel | BeKind::EnderChest) {
        return;
    }
    let r = c.max_range + 4.0;
    let (lo, hi) = ([pos.x as f64 - r, pos.y as f64 - r, pos.z as f64 - r], [pos.x as f64 + 1.0 + r, pos.y as f64 + 1.0 + r, pos.z as f64 + 1.0 + r]);
    let viewers: Vec<&&mut Player> = players
        .iter()
        .filter(|p| p.game_mode != 3 && !p.disconnected && has_open(p, pos))
        .filter(|p| {
            let bb = [[p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3], [p.pos[0] + 0.3, p.pos[1] + 1.8, p.pos[2] + 0.3]];
            (0..3).all(|i| bb[0][i] < hi[i] && bb[1][i] > lo[i])
        })
        .collect();
    let n = viewers.len() as i32;
    let range = if n > 0 { BLOCK_INTERACTION_RANGE } else { 0.0 };
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    c.max_range = range;
    let before = c.openers;
    if before != n {
        c.openers = n;
        let (was, now) = (before > 0, n > 0);
        if now && !was {
            open_changed(level, pos, s, true);
        } else if !now {
            open_changed(level, pos, s, false);
        }
        openers_changed(level, pos, s, before, n);
    }
    if n > 0 {
        kiln_blocks::schedule_block_tick(level, pos, BlockId::of(s), RECHECK_DELAY, kiln_blocks::TickPriority::Normal);
    }
}

/// The container block entities the open menu shows, with their change counts.
fn open_changes(p: &Player, level: &RegionLevel) -> Vec<(BlockPos, u64)> {
    let positions: Vec<BlockPos> = match &p.containers.open {
        Some(OpenBlock::Containers { first, second }) => std::iter::once(first.0).chain(second.map(|s| s.0)).collect(),
        _ => Vec::new(),
    };
    positions.into_iter().filter_map(|pos| level.blocks.containers.get(pos).map(|c| (pos, c.changes))).collect()
}

/// [`menu_op`]'s broadcast for a player with no block menu open (`containers.open` is `None`):
/// no block entity, comparator or workstation takes part, so it needs only the player.
/// Returns what the player dropped.
pub(crate) fn own_menu_broadcast(p: &mut Player, rules: &kiln_inventory::Rules) -> Vec<Spawn> {
    debug_assert!(p.containers.open.is_none());
    let mut spawns = Vec::new();
    p.with_menu_at(rules, &mut spawns, None, |menu, _, env| menu.broadcast_changes(env));
    // `workstation_effects` without a workstation: pending effects are dropped.
    p.containers.pending.clear();
    spawns
}

/// A menu operation on the player's open menu within its region (clicks, broadcasts):
/// containers it changed update their comparators (`BlockEntity.setChanged`).
pub(crate) fn menu_op<R>(
    p: &mut Player,
    level: &mut RegionLevel,
    spawns: &mut Vec<Spawn>,
    f: impl FnOnce(&mut Menu, Option<&mut Menu>, &mut kiln_inventory::Env) -> R,
) -> R {
    let before = open_changes(p, level);
    if let Some(OpenBlock::Workstation { pos, .. }) = p.containers.open {
        p.containers.bookshelves = super::world::count_bookshelves(level, pos);
    }
    let rules = level.env.menus.clone();
    let r = p.with_menu_at(&rules, spawns, Some(&mut level.blocks.containers), f);
    workstation_effects(p, level);
    for (pos, n) in before {
        if level.blocks.containers.get(pos).is_some_and(|c| c.changes != n) {
            // A lectern's page turned or its book taken: its block follows (`LecternBlock.signalPageChange`,
            // `resetBookState`) before the comparators read it.
            crate::lectern::after_menu(level, pos);
            let s = level.block(pos);
            kiln_blocks::update::update_neighbour_for_output_signal(level, pos, BlockId::of(s));
        }
    }
    r
}

/// `handleSetBeaconPacket` → `BeaconMenu.updateEffects`: with a payment in the open beacon's
/// menu and powers the pyramid allows, the beacon takes them and the payment is used up.
pub(crate) fn set_beacon(p: &mut Player, level: &mut RegionLevel, spawns: &mut Vec<Spawn>, primary: Option<i32>, secondary: Option<i32>) {
    let Some(OpenBlock::Containers { first: (pos, _), second: None }) = p.containers.open else { return };
    let Some(levels) = level.blocks.containers.get(pos).and_then(|c| c.beacon.as_ref()).map(|b| b.levels) else { return };
    if !p.open_menu.as_ref().is_some_and(|m| m.has_beacon_payment()) || !super::beacon::valid_powers(primary, secondary, levels) {
        return;
    }
    super::beacon::set_powers(level, pos, primary, secondary);
    menu_op(p, level, spawns, |menu, _, env| {
        menu.take_beacon_payment();
        menu.broadcast_changes(env);
    });
}

/// Applies a placed block item's components to the block entity it made
/// (`BlockEntity.applyComponentsFromItemStack`): custom name, lock, contents, loot table.
pub(crate) fn apply_item_components(level: &mut RegionLevel, pos: BlockPos, stack: &ItemStack) {
    use kiln_item::keys;
    use kiln_world::Blocks as _;
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let mut touched = false;
    if let Some(name) = stack.get(keys::CUSTOM_NAME) {
        c.custom_name = Some(name.nbt().clone());
        touched = true;
    }
    if let Some(contents) = stack.get(keys::CONTAINER) {
        // `ItemContainerContents.copyInto`.
        for (i, slot) in c.items.iter_mut().enumerate() {
            *slot = contents.0.get(i).and_then(|s| s.as_ref()).map(|t| t.create()).unwrap_or_default();
        }
        touched = true;
    }
    // `DecoratedPotBlockEntity.applyImplicitComponents`: the sherds.
    if c.kind == BeKind::DecoratedPot {
        let decorations = stack.get(keys::POT_DECORATIONS).cloned().unwrap_or_default();
        c.extra.retain(|(k, _)| k != "sherds");
        if decorations != kiln_item::component::PotDecorations::default() {
            c.extra.push(("sherds".into(), <kiln_item::component::PotDecorations as kiln_item::component::ComponentValue>::to_value(&decorations).to_nbt()));
        }
        touched = true;
    }
    if let Some(h) = c.hive.as_deref_mut() {
        // `BeehiveBlockEntity.applyImplicitComponents`.
        h.occupants.clear();
        if let Some(bees) = stack.get(keys::BEES) {
            h.apply(bees);
        }
        touched = true;
    }
    if let Some(loot) = stack.get(keys::CONTAINER_LOOT)
        && c.kind.randomizable()
    {
        c.loot_table = Some(loot.loot_table.as_str().to_owned());
        c.loot_seed = loot.seed;
        touched = true;
    }
    if let Some(lock) = stack.component(kiln_item::component::ids::LOCK).and_then(|l| l.to_value()) {
        c.lock = Some(lock.to_nbt());
        touched = true;
    }
    if touched {
        c.mark_changed();
        // The chunk's copy is what the update packet clients get is made of (a decorated pot shows its sherds).
        sync_chunk_copy(level, pos);
    }
}

/// Writes the live block entity at `pos` into its chunk (the copy that is saved and sent to clients).
pub(crate) fn sync_chunk_copy(level: &mut RegionLevel, pos: BlockPos) {
    use kiln_world::Blocks as _;
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let (type_id, saved) = (c.type_id, c.chunk_tag());
    c.dirty = false;
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    if let Some(chunk) = level.cells.chunk_mut(kiln_world::ChunkPos::of_block(pos.x, pos.z))
        && chunk.block_entity(x, pos.y, z).is_some_and(|be| be.kind == type_id)
    {
        let mut be = kiln_world::block_entity::BlockEntity::new(type_id);
        if let (Tag::Compound(out), Tag::Compound(fields)) = (&mut be.nbt, saved) {
            out.extend(fields);
        }
        chunk.set_block_entity(x, pos.y, z, be);
    }
}

/// `ShulkerBoxBlock.playerWillDestroy`: a creative player breaking a filled shulker box still
/// gets it as an item, with its contents; otherwise its loot table rolls first.
pub(crate) fn player_will_destroy(level: &mut RegionLevel, pos: BlockPos, s: u16, creative: bool) {
    if logic::block_class(s) != C::ShulkerBoxBlock {
        return;
    }
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    if creative && (!c.is_empty() || c.loot_table.is_some()) {
        let Some(mut item) = ItemStack::of(BlockId::of(s).name(), 1) else { return };
        for component in c.components() {
            item.set(component);
        }
        level.out.spawns.push(Spawn {
            kind: &kiln_data::entities::types::ITEM,
            pos: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5],
            vel: [0.0; 3],
            body: crate::entities::Body::Item { stack: item, pickup_delay: 10, thrower: None },
        });
    } else if !creative {
        super::unpack_loot(c, pos, loot.as_deref(), true, game_time, seed);
    }
}

/// Carries out what grindstones and anvils did at the open workstation.
fn workstation_effects(p: &mut Player, level: &mut RegionLevel) {
    let pending = std::mem::take(&mut p.containers.pending);
    let Some(OpenBlock::Workstation { pos, .. }) = p.containers.open else { return };
    for effect in pending {
        match effect {
            kiln_inventory::Effect::GrindstoneUsed { experience } => {
                let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
                let mut rng = super::pos_random(level, pos, 3);
                super::furnace::award_experience(at, experience, &mut rng, &mut level.out.spawns);
                level.effect(Effect::LevelEvent { id: 1042, pos, data: 0 });
            }
            kiln_inventory::Effect::AnvilUsed { levels } => {
                p.pay_levels(levels);
                anvil_wear(p, level, pos);
            }
            kiln_inventory::Effect::LoomUsed => {
                let now = level.env.game_time;
                if p.containers.last_loom_sound != now {
                    p.containers.last_loom_sound = now;
                    level.effect(Effect::Sound { pos, sound: "minecraft:ui.loom.take_result", volume: 1.0, pitch: 1.0 });
                }
            }
            kiln_inventory::Effect::CartographyUsed => {
                let now = level.env.game_time;
                if p.containers.last_cartography_sound != now {
                    p.containers.last_cartography_sound = now;
                    level.effect(Effect::Sound { pos, sound: "minecraft:ui.cartography_table.take_result", volume: 1.0, pitch: 1.0 });
                }
            }
            kiln_inventory::Effect::Enchanted { levels, seed } => {
                p.award_stat(*crate::player_stats::stat::ENCHANT_ITEM, 1);
                p.enchanted_item(&kiln_item::ItemStack::empty(), levels);
                p.pay_levels(levels);
                p.containers.enchantment_seed = seed;
                let pitch = super::pos_random(level, pos, 5).next_float() * 0.1 + 0.9;
                level.effect(Effect::Sound { pos, sound: "minecraft:block.enchantment_table.use", volume: 1.0, pitch });
            }
            _ => {}
        }
    }
}

/// `AnvilMenu.onTake`'s block part: a survival player's anvil may get a step more damaged (or
/// break), with the anvil use or destroy sound.
fn anvil_wear(p: &mut Player, level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    let creative = p.game_mode == 1;
    if !creative && kiln_blocks::tags::is(s, "minecraft:anvil") && p.entity_rng.next_float() < 0.12 {
        // `AnvilBlock.damage`.
        let name = BlockId::of(s).name();
        let next = match name {
            "minecraft:anvil" => Some("minecraft:chipped_anvil"),
            "minecraft:chipped_anvil" => Some("minecraft:damaged_anvil"),
            _ => None,
        };
        match next.and_then(BlockId::by_name) {
            Some(b) => {
                let damaged = state::with_properties_of(b.default_state(), s);
                kiln_blocks::set_block(level, pos, damaged, flags::CLIENTS);
                level.effect(Effect::LevelEvent { id: 1030, pos, data: 0 });
            }
            None => {
                kiln_blocks::remove_block(level, pos, false);
                level.effect(Effect::LevelEvent { id: 1029, pos, data: 0 });
            }
        }
    } else {
        level.effect(Effect::LevelEvent { id: 1030, pos, data: 0 });
    }
}
