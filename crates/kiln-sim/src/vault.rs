//! Vaults (`VaultBlock`, `VaultBlockEntity`, `VaultState`): a trial chamber's reward block. A player near it
//! wakes it up (inactive to active); a trial key used on it, if the player has not been rewarded by it, opens
//! it (unlocking, then ejecting) and the loot table's items fly out one by one above it.

use crate::Player;
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use uuid::Uuid;

/// `VaultConfig.DEFAULT`.
const DEFAULT_LOOT_TABLE: &str = "minecraft:chests/trial_chambers/reward";
const DEFAULT_ACTIVATION: f64 = 4.0;
const DEFAULT_DEACTIVATION: f64 = 4.5;
/// `VaultServerData.MAX_REWARD_PLAYERS`.
const MAX_REWARDED: usize = 128;

/// A player as vaults see them (`PlayerDetector`): who, in which block, in which game mode.
#[derive(Clone, Debug)]
pub(crate) struct Near {
    pub uuid: Uuid,
    pub block: [i32; 3],
    pub game_mode: u8,
}

/// `VaultConfig`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Config {
    pub loot_table: String,
    pub activation: f64,
    pub deactivation: f64,
    pub key: ItemStack,
    pub display_table: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            loot_table: DEFAULT_LOOT_TABLE.to_owned(),
            activation: DEFAULT_ACTIVATION,
            deactivation: DEFAULT_DEACTIVATION,
            key: ItemStack::of("minecraft:trial_key", 1).unwrap_or_else(ItemStack::empty),
            display_table: None,
        }
    }
}

/// `VaultSharedData`: what the clients are shown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shared {
    pub display: ItemStack,
    pub players: Vec<Uuid>,
    pub particles_range: f64,
    pub dirty: bool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { display: ItemStack::empty(), players: Vec::new(), particles_range: DEFAULT_DEACTIVATION, dirty: false }
    }
}

/// `VaultServerData`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ServerData {
    pub rewarded: Vec<Uuid>,
    pub resumes_at: i64,
    pub eject: Vec<ItemStack>,
    pub last_fail: i64,
    pub total: i32,
    pub dirty: bool,
}

/// A vault's block entity data.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Vault {
    pub config: Config,
    pub shared: Shared,
    pub server: ServerData,
}

fn uuid_tag(u: &Uuid) -> Tag {
    let (hi, lo) = u.as_u64_pair();
    Tag::IntArray(vec![(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32])
}

fn uuid_of(t: &Tag) -> Option<Uuid> {
    match t {
        Tag::IntArray(v) if v.len() == 4 => {
            let hi = ((v[0] as u32 as u64) << 32) | v[1] as u32 as u64;
            let lo = ((v[2] as u32 as u64) << 32) | v[3] as u32 as u64;
            Some(Uuid::from_u64_pair(hi, lo))
        }
        _ => None,
    }
}

fn uuids(t: Option<&Tag>) -> Vec<Uuid> {
    t.and_then(Tag::as_list).unwrap_or(&[]).iter().filter_map(uuid_of).collect()
}

fn stack_of(t: Option<&Tag>) -> ItemStack {
    t.and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(ItemStack::empty)
}

impl Vault {
    /// `VaultBlockEntity.loadAdditional`.
    pub(crate) fn load(nbt: &Tag) -> Vault {
        let mut v = Vault::default();
        if let Some(c) = nbt.get("config") {
            if let Some(t) = c.get("loot_table").and_then(Tag::as_str) {
                v.config.loot_table = t.to_owned();
            }
            if let Some(a) = c.get("activation_range").and_then(Tag::as_f64) {
                v.config.activation = a;
            }
            if let Some(a) = c.get("deactivation_range").and_then(Tag::as_f64) {
                v.config.deactivation = a;
            }
            // (`ItemStack.lenientOptionalFieldOf`: no key, or one that cannot be read, is empty; only a vault with no `config` at all has the trial key.)
            {
                let k = c.get("key_item");
                v.config.key = stack_of(k);
            }
            v.config.display_table = c.get("override_loot_table_to_display").and_then(Tag::as_str).map(str::to_owned);
        }
        if let Some(s) = nbt.get("shared_data") {
            v.shared.display = stack_of(s.get("display_item"));
            v.shared.players = uuids(s.get("connected_players"));
            if let Some(r) = s.get("connected_particles_range").and_then(Tag::as_f64) {
                v.shared.particles_range = r;
            }
        }
        if let Some(s) = nbt.get("server_data") {
            v.server.rewarded = uuids(s.get("rewarded_players"));
            v.server.resumes_at = s.get("state_updating_resumes_at").and_then(Tag::as_i64).unwrap_or(0);
            v.server.eject = s.get("items_to_eject").and_then(Tag::as_list).unwrap_or(&[]).iter().filter_map(|t| ItemStack::from_nbt(t).ok()).collect();
            v.server.total = s.get("total_ejections_needed").and_then(Tag::as_i64).unwrap_or(0) as i32;
        }
        v
    }

    /// `VaultBlockEntity.saveAdditional`: the three data compounds (a field with its default value is left out).
    pub(crate) fn save(&self, out: &mut Vec<(String, Tag)>) {
        let d = Config::default();
        let mut c = Vec::new();
        if self.config.loot_table != d.loot_table {
            c.push(("loot_table".to_owned(), Tag::String(self.config.loot_table.clone())));
        }
        if self.config.activation != d.activation {
            c.push(("activation_range".to_owned(), Tag::Double(self.config.activation)));
        }
        if self.config.deactivation != d.deactivation {
            c.push(("deactivation_range".to_owned(), Tag::Double(self.config.deactivation)));
        }
        if !self.config.key.is_empty() {
            c.push(("key_item".to_owned(), self.config.key.to_nbt()));
        }
        if let Some(t) = &self.config.display_table {
            c.push(("override_loot_table_to_display".to_owned(), Tag::String(t.clone())));
        }
        out.push(("config".to_owned(), Tag::Compound(c)));
        out.push(("shared_data".to_owned(), self.shared_tag()));
        let mut s = Vec::new();
        if !self.server.rewarded.is_empty() {
            s.push(("rewarded_players".to_owned(), Tag::List(self.server.rewarded.iter().map(uuid_tag).collect())));
        }
        if self.server.resumes_at != 0 {
            s.push(("state_updating_resumes_at".to_owned(), Tag::Long(self.server.resumes_at)));
        }
        if !self.server.eject.is_empty() {
            s.push(("items_to_eject".to_owned(), Tag::List(self.server.eject.iter().map(ItemStack::to_nbt).collect())));
        }
        if self.server.total != 0 {
            s.push(("total_ejections_needed".to_owned(), Tag::Int(self.server.total)));
        }
        out.push(("server_data".to_owned(), Tag::Compound(s)));
    }

    /// `getUpdateTag`: what the clients are sent (the shared data).
    fn shared_tag(&self) -> Tag {
        let mut s = Vec::new();
        if !self.shared.display.is_empty() {
            s.push(("display_item".to_owned(), self.shared.display.to_nbt()));
        }
        if !self.shared.players.is_empty() {
            s.push(("connected_players".to_owned(), Tag::List(self.shared.players.iter().map(uuid_tag).collect())));
        }
        if self.shared.particles_range != DEFAULT_DEACTIVATION {
            s.push(("connected_particles_range".to_owned(), Tag::Double(self.shared.particles_range)));
        }
        Tag::Compound(s)
    }
}

/// `ItemStack.matches`: the same item, count and components (empty stacks match).
fn same_stack(a: &ItemStack, b: &ItemStack) -> bool {
    if a.is_empty() || b.is_empty() {
        return a.is_empty() && b.is_empty();
    }
    a.count() == b.count() && a.is_same_item_same_components(b)
}

impl Shared {
    fn set_display(&mut self, stack: &ItemStack) {
        if same_stack(&self.display, stack) {
            return;
        }
        self.display = stack.clone();
        self.dirty = true;
    }
}

impl ServerData {
    fn next_to_eject(&self) -> ItemStack {
        self.eject.last().cloned().unwrap_or_else(ItemStack::empty)
    }

    fn pop_next(&mut self) -> ItemStack {
        let Some(s) = self.eject.pop() else { return ItemStack::empty() };
        self.dirty = true;
        s
    }

    fn set_eject(&mut self, items: Vec<ItemStack>) {
        self.total = items.len() as i32;
        self.eject = items;
        self.dirty = true;
    }

    fn pause_until(&mut self, tick: i64) {
        self.resumes_at = tick;
        self.dirty = true;
    }

    /// `VaultServerData.ejectionProgress`.
    fn progress(&self) -> f32 {
        if self.total == 1 {
            return 1.0;
        }
        // `1 - Mth.inverseLerp(items left, 1, total)`.
        let left = self.eject.len() as f32;
        1.0 - (left - 1.0) / (self.total as f32 - 1.0)
    }

    fn add_rewarded(&mut self, u: Uuid) {
        if !self.rewarded.contains(&u) {
            self.rewarded.push(u);
        }
        while self.rewarded.len() > MAX_REWARDED {
            self.rewarded.remove(0);
        }
        self.dirty = true;
    }
}

/// The `vault_state` values, in the block's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Inactive,
    Active,
    Unlocking,
    Ejecting,
}

fn phase(s: u16) -> Phase {
    match state::get(s, "vault_state") {
        Some("active") => Phase::Active,
        Some("unlocking") => Phase::Unlocking,
        Some("ejecting") => Phase::Ejecting,
        _ => Phase::Inactive,
    }
}

fn with_phase(s: u16, p: Phase) -> u16 {
    let name = match p {
        Phase::Inactive => "inactive",
        Phase::Active => "active",
        Phase::Unlocking => "unlocking",
        Phase::Ejecting => "ejecting",
    };
    state::set(s, "vault_state", name)
}

/// The loot context of a vault (`LootContextParamSets.VAULT`): the vault's centre, and for a key the player
/// with their luck and the key as the tool.
struct VaultLoot {
    origin: [f64; 3],
    luck: f32,
    player: bool,
    tool: Option<ItemStack>,
}

impl kiln_loot::LootContext for VaultLoot {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.player && target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn tool(&self) -> Option<&ItemStack> {
        self.tool.as_ref()
    }
    fn luck(&self) -> f32 {
        self.luck
    }
}

fn center(pos: BlockPos) -> [f64; 3] {
    [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5]
}

/// `VaultBlockEntity.Server.getRandomDisplayItemFromLootTable`.
fn random_display_item(level: &RegionLevel, pos: BlockPos, table: &str) -> ItemStack {
    let Some(loot) = level.env.loot.clone() else { return ItemStack::empty() };
    let Some(id) = kiln_item::ident::Identifier::parse(table) else { return ItemStack::empty() };
    let ctx = VaultLoot { origin: center(pos), luck: 0.0, player: false, tool: None };
    let mut rng = crate::container::pos_random(level, pos, 11);
    let items = loot.random_items(&id, &ctx, &mut rng);
    if items.is_empty() {
        return ItemStack::empty();
    }
    let i = rng.next_int_bounded(items.len() as i32) as usize;
    items[i].clone()
}

fn can_eject_reward(config: &Config, st: Phase) -> bool {
    !config.key.is_empty() && st != Phase::Inactive
}

/// `cycleDisplayItemFromLootTable`.
fn cycle_display(level: &RegionLevel, st: Phase, v: &mut Vault, pos: BlockPos) {
    if !can_eject_reward(&v.config, st) {
        v.shared.set_display(&ItemStack::empty());
        return;
    }
    let table = v.config.display_table.clone().unwrap_or_else(|| v.config.loot_table.clone());
    let item = random_display_item(level, pos, &table);
    v.shared.set_display(&item);
}

/// `PlayerDetector.INCLUDING_CREATIVE_PLAYERS` + `VaultSharedData.updateConnectedPlayersWithinRange`.
fn update_connected(level: &RegionLevel, pos: BlockPos, v: &mut Vault, range: f64) {
    let near: Vec<Uuid> = level
        .env
        .players
        .iter()
        .filter(|p| p.game_mode != 3)
        .filter(|p| {
            let d = [(p.block[0] - pos.x) as f64, (p.block[1] - pos.y) as f64, (p.block[2] - pos.z) as f64];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range
        })
        .map(|p| p.uuid)
        .filter(|u| !v.server.rewarded.contains(u))
        .collect();
    let same = near.len() == v.shared.players.len() && near.iter().all(|u| v.shared.players.contains(u));
    if !same {
        v.shared.players = near;
        v.shared.dirty = true;
    }
}

/// `VaultState.updateStateForConnectedPlayers`.
fn state_for_players(level: &RegionLevel, pos: BlockPos, v: &mut Vault, range: f64) -> Phase {
    update_connected(level, pos, v, range);
    v.server.pause_until(level.env.game_time + 20);
    if v.shared.players.is_empty() { Phase::Inactive } else { Phase::Active }
}

/// `VaultState.tickAndGetNext`.
fn tick_and_get_next(level: &mut RegionLevel, pos: BlockPos, st: Phase, v: &mut Vault) -> Phase {
    let now = level.env.game_time;
    match st {
        Phase::Inactive => state_for_players(level, pos, v, v.config.activation),
        Phase::Active => state_for_players(level, pos, v, v.config.deactivation),
        Phase::Unlocking => {
            v.server.pause_until(now + 20);
            Phase::Ejecting
        }
        Phase::Ejecting => {
            if v.server.eject.is_empty() {
                v.server.total = 0;
                v.server.dirty = true;
                return state_for_players(level, pos, v, v.config.deactivation);
            }
            let progress = v.server.progress();
            let item = v.server.pop_next();
            eject(level, pos, item, progress);
            let next = v.server.next_to_eject();
            v.shared.set_display(&next);
            v.server.pause_until(now + 20);
            Phase::Ejecting
        }
    }
}

/// `VaultState.ejectResultItem`.
fn eject(level: &mut RegionLevel, pos: BlockPos, item: ItemStack, progress: f32) {
    let at = [pos.x as f64 + 0.5, pos.y as f64 + 1.2, pos.z as f64 + 0.5];
    let mut rng = crate::container::pos_random(level, pos, 7);
    crate::container::dispense::spawn_item(level, &mut rng, item, 2, Direction::Up, at);
    level.effect(Effect::LevelEvent { id: 3017, pos, data: 0 });
    level.effect(Effect::Sound { pos, sound: "minecraft:block.vault.eject_item", volume: 1.0, pitch: 0.8 + 0.4 * progress });
}

/// `VaultBlockEntity.Server.setVaultState` + `VaultState.onTransition` (`onExit` of the old state, `onEnter` of
/// the new one).
fn set_phase(level: &mut RegionLevel, pos: BlockPos, old_state: u16, new_state: u16, v: &mut Vault) {
    let (old, new) = (phase(old_state), phase(new_state));
    kiln_blocks::set_block_and_update(level, pos, new_state);
    let ominous = i32::from(state::get_bool(new_state, "ominous"));
    if old == Phase::Ejecting {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.vault.close_shutter", volume: 1.0, pitch: 1.0 });
    }
    match new {
        Phase::Inactive => {
            v.shared.set_display(&ItemStack::empty());
            level.effect(Effect::LevelEvent { id: 3016, pos, data: ominous });
        }
        Phase::Active => {
            if v.shared.display.is_empty() {
                cycle_display(level, Phase::Active, v, pos);
            }
            level.effect(Effect::LevelEvent { id: 3015, pos, data: ominous });
        }
        Phase::Unlocking => level.effect(Effect::Sound { pos, sound: "minecraft:block.vault.insert_item", volume: 1.0, pitch: 1.0 }),
        Phase::Ejecting => level.effect(Effect::Sound { pos, sound: "minecraft:block.vault.open_shutter", volume: 1.0, pitch: 1.0 }),
    }
}

/// Puts the vault's data back and sends what changed (`setChanged`, `sendBlockUpdated`).
fn store(level: &mut RegionLevel, pos: BlockPos, mut v: Vault) {
    let shared_dirty = v.shared.dirty;
    let dirty = shared_dirty || v.server.dirty;
    v.shared.dirty = false;
    v.server.dirty = false;
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.vault = Some(Box::new(v));
        if dirty {
            c.mark_changed();
        }
    }
    if dirty {
        crate::container::open::sync_chunk_copy(level, pos);
    }
    if shared_dirty {
        level.out.changed.push([pos.x, pos.y, pos.z]);
    }
}

/// `VaultBlockEntity` ticker (server): the display item cycles, the state follows the players, changes are sent.
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if logic::block_class(s) != C::VaultBlock {
        return;
    }
    let Some(mut v) = level.blocks.containers.get_mut(pos).and_then(|c| c.vault.take()).map(|b| *b) else { return };
    let now = level.env.game_time;
    let st = phase(s);
    if now % 20 == 0 && st == Phase::Active {
        cycle_display(level, st, &mut v, pos);
    }
    if now >= v.server.resumes_at {
        let next = tick_and_get_next(level, pos, st, &mut v);
        let next_state = with_phase(s, next);
        if next_state != s {
            set_phase(level, pos, s, next_state, &mut v);
        }
    }
    store(level, pos, v);
}

/// `VaultBlock.useItemOn`: a trial key on an active vault. `None`: not for the vault.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    let held = p.in_hand(off_hand).clone();
    if held.is_empty() || phase(s) != Phase::Active {
        return None;
    }
    let mut v = level.blocks.containers.get_mut(pos).and_then(|c| c.vault.take()).map(|b| *b)?;
    try_insert_key(p, level, pos, s, &mut v, &held, off_hand);
    store(level, pos, v);
    Some(true)
}

fn insert_fail_sound(level: &mut RegionLevel, v: &mut Vault, pos: BlockPos, sound: &'static str) {
    let now = level.env.game_time;
    if now >= v.server.last_fail + 15 {
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
        v.server.last_fail = now;
    }
}

/// `VaultBlockEntity.Server.tryInsertKey`.
fn try_insert_key(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, v: &mut Vault, held: &ItemStack, off_hand: bool) {
    if !can_eject_reward(&v.config, phase(s)) {
        return;
    }
    let valid = v.config.key.is_same_item_same_components(held) && held.count() >= v.config.key.count();
    if !valid {
        insert_fail_sound(level, v, pos, "minecraft:block.vault.insert_item_fail");
        return;
    }
    if v.server.rewarded.contains(&p.uuid) {
        insert_fail_sound(level, v, pos, "minecraft:block.vault.reject_rewarded_player");
        return;
    }
    // `resolveItemsToEject`.
    let Some(loot) = level.env.loot.clone() else { return };
    let Some(id) = kiln_item::ident::Identifier::parse(&v.config.loot_table) else { return };
    let ctx = VaultLoot { origin: center(pos), luck: p.attribute(crate::combat::LUCK) as f32, player: true, tool: Some(held.clone()) };
    let mut rng = crate::container::pos_random(level, pos, 8);
    let items = loot.random_items(&id, &ctx, &mut rng);
    if items.is_empty() {
        return;
    }
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, held.item()), 1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(v.config.key.count());
        p.inv.times_changed += 1;
    }
    // `unlock`.
    v.server.set_eject(items);
    let next = v.server.next_to_eject();
    v.shared.set_display(&next);
    v.server.pause_until(level.env.game_time + 14);
    set_phase(level, pos, s, with_phase(s, Phase::Unlocking), v);
    v.server.add_rewarded(p.uuid);
    update_connected(level, pos, v, v.config.deactivation);
}
