//! Villager: type, profession, level, trading, and the `Brain` that runs it (`Villager.BRAIN_PROVIDER`,
//! `VillagerGoalPackages`): the schedule (`minecraft:villager_schedule`) of work, meeting at the
//! bell and sleeping in a bed, points of interest claimed in the level's POI manager (bed, job
//! site, meeting point), panic, raids, breeding, gossip.
//!
//! The behaviours are in [`crate::mob::brain::village`]. Approximations: bells are not rung by
//! players (a villager's ring reaches the villagers around at once), the golem a village calls
//! for is spawned by a plain rule (no `SpawnUtil` attempts in vanilla's order), fireworks of a
//! raid victory are not launched, hero gifts drop at the villager, composting is not simulated.

use crate::entity::Entity;
use crate::level::{EntityLevel, Event, PoiOccupancy, TradeMerchant};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::{
    DoNothing, LookAtTargetSink, MoveToTargetSink, Swim, set_entity_look_target, set_walk_target_from_look_target,
};
use crate::mob::brain::sensors;
use crate::mob::brain::village::{self, poi, raid, social, stroll, work};
use crate::mob::brain::{self, Activity, ActivityData, Brain, Cx, Gate, Mem, Status, TriggerGate, shot};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, path};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_item::trading::{self, MerchantOffer};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Villager;

pub static KIND: Villager = Villager;

static INFO: Info = Info {
    ageable: true,
    sounds: Some("villager"),
    ..Info::misc("minecraft:villager", &[(MovementSpeed, 0.5)])
};

/// `VillagerData.NEXT_LEVEL_XP_THRESHOLDS`.
const XP_THRESHOLDS: [i32; 5] = [0, 10, 70, 150, 250];

/// `VillagerType.BY_BIOME`: biomes whose villagers are not plains villagers.
const BY_BIOME: &[(&str, &str)] = &[
    ("badlands", "desert"),
    ("desert", "desert"),
    ("eroded_badlands", "desert"),
    ("wooded_badlands", "desert"),
    ("bamboo_jungle", "jungle"),
    ("jungle", "jungle"),
    ("sparse_jungle", "jungle"),
    ("savanna_plateau", "savanna"),
    ("savanna", "savanna"),
    ("windswept_savanna", "savanna"),
    ("deep_frozen_ocean", "snow"),
    ("frozen_ocean", "snow"),
    ("frozen_river", "snow"),
    ("ice_spikes", "snow"),
    ("snowy_beach", "snow"),
    ("snowy_taiga", "snow"),
    ("snowy_plains", "snow"),
    ("grove", "snow"),
    ("snowy_slopes", "snow"),
    ("frozen_peaks", "snow"),
    ("jagged_peaks", "snow"),
    ("swamp", "swamp"),
    ("mangrove_swamp", "swamp"),
    ("old_growth_spruce_taiga", "taiga"),
    ("old_growth_pine_taiga", "taiga"),
    ("windswept_gravelly_hills", "taiga"),
    ("windswept_hills", "taiga"),
    ("taiga", "taiga"),
    ("windswept_forest", "taiga"),
];

/// `VillagerType.byBiome` for a `minecraft:worldgen/biome` entry name.
pub fn type_for_biome(biome: &str) -> &'static str {
    let path = biome.strip_prefix("minecraft:").unwrap_or(biome);
    let t = BY_BIOME.iter().find(|(b, _)| *b == path).map_or("plains", |(_, t)| *t);
    registry_name("minecraft:villager_type", &format!("minecraft:{t}")).unwrap_or("minecraft:plains")
}

/// The `minecraft:worldgen/biome` entry of a network id.
pub fn biome_name(id: i32) -> Option<&'static str> {
    let (_, entries) = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome")?;
    entries.get(usize::try_from(id).ok()?).copied()
}

fn registry_name(registry: &str, name: &str) -> Option<&'static str> {
    kiln_data::builtin_entries(registry).and_then(|e| e.iter().find(|x| **x == name).copied())
}

/// `VillagerData` and the rest of the villager's own state.
#[derive(Clone, Debug)]
pub struct VillagerState {
    /// `minecraft:villager_type` entry.
    pub villager_type: &'static str,
    /// `minecraft:villager_profession` entry.
    pub profession: &'static str,
    /// 1 to 5 (no upper clamp, as vanilla).
    pub level: i32,
    pub finalized: bool,
    /// `villagerXp`.
    pub xp: i32,
    pub food_level: i8,
    /// `offers`: `None` until first generated (lazily, on the first interaction).
    pub offers: Option<Vec<MerchantOffer>>,
    /// The player trading with the villager (`tradingPlayer`).
    pub trading_player: Option<i32>,
    /// Set by a click that starts trading: the simulation opens the merchant screen for this
    /// player (`openTradingScreen`) and clears it.
    pub open_for: Option<i32>,
    /// `lastTradedPlayer`: celebrated (entity event 14) on the next AI step.
    pub last_traded_player: Option<i32>,
    /// `DATA_UNHAPPY_COUNTER` (head shaking).
    pub unhappy: i32,
    pub last_restock: i64,
    pub restocks_today: i32,
    pub last_gossip_decay: i64,
    /// `gossips`.
    pub gossips: crate::mob::gossip::Gossips,
    /// `lastGossipTime`.
    pub last_gossip_time: i64,
    /// `lastRestockCheckDay`.
    pub last_restock_check_day: i64,
    /// `getSleepingPos`: the bed the villager lies in.
    pub sleeping_pos: Option<BlockPos>,
    /// A behaviour changed the profession: the brain is rebuilt after this tick (`refreshBrain`).
    pub refresh_brain: bool,
    /// The brain was built since the last tick: its schedule has yet to be read.
    pub schedule_pending: bool,
    /// The 8 slot inventory (`SimpleContainer(8)`).
    pub inventory: Vec<ItemStack>,
}

impl Default for VillagerState {
    fn default() -> Self {
        VillagerState {
            villager_type: "minecraft:plains",
            profession: "minecraft:none",
            level: 1,
            finalized: false,
            xp: 0,
            food_level: 0,
            offers: None,
            trading_player: None,
            open_for: None,
            last_traded_player: None,
            unhappy: 0,
            last_restock: 0,
            restocks_today: 0,
            last_gossip_decay: 0,
            gossips: crate::mob::gossip::Gossips::default(),
            last_gossip_time: 0,
            last_restock_check_day: 0,
            sleeping_pos: None,
            refresh_brain: false,
            schedule_pending: true,
            inventory: (0..8).map(|_| ItemStack::empty()).collect(),
        }
    }
}

impl VillagerState {
    /// `setVillagerData`: a new profession forgets the offers.
    pub fn set_profession(&mut self, profession: &'static str) {
        if profession != self.profession {
            self.offers = None;
        }
        self.profession = profession;
    }

    /// `VillagerProfession.getTrades(level)`: the trade set id, if the profession trades at
    /// that level.
    pub fn trade_set(&self) -> Option<String> {
        let prof = self.profession.strip_prefix("minecraft:")?;
        if prof == "none" || prof == "nitwit" || !(1..=5).contains(&self.level) {
            return None;
        }
        Some(format!("minecraft:{prof}/level_{}", self.level))
    }
}

pub fn state(m: &MobData) -> Option<&VillagerState> {
    ext::state::<VillagerState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut VillagerState> {
    ext::state_mut::<VillagerState>(m)
}

/// `VillagerData.canLevelUp`.
pub fn can_level_up(level: i32) -> bool {
    (1..5).contains(&level)
}

/// `VillagerData.getMaxXpPerLevel`.
pub fn max_xp_per_level(level: i32) -> i32 {
    if can_level_up(level) { XP_THRESHOLDS[level as usize] } else { 0 }
}

/// `Villager.updateTrades`: appends the current level's trade set.
fn update_trades(e: &Entity, st: &mut VillagerState, level: &mut dyn EntityLevel) {
    let Some(set) = st.trade_set() else { return };
    let merchant = TradeMerchant { entity: e.id, pos: e.position(), villager_type: st.villager_type };
    let new = level.trade_offers(&set, &merchant);
    st.offers.get_or_insert_with(Vec::new).extend(new);
}

/// `AbstractVillager.getOffers`: generated on first access.
pub fn offers<'a>(e: &Entity, m: &'a mut MobData, level: &mut dyn EntityLevel) -> &'a [MerchantOffer] {
    let st = state_mut(m).expect("villager");
    if st.offers.is_none() {
        st.offers = Some(Vec::new());
        update_trades(e, st, level);
    }
    st.offers.as_deref().unwrap_or(&[])
}

/// What a completed trade did, for the simulation (the menu refreshes its offers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traded {
    pub leveled_up: bool,
}

/// `AbstractVillager.notifyTrade` + `Villager.rewardTradeXp`: offer `index` was used by the
/// trading player.
pub fn notify_trade(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, index: usize) -> Traded {
    let interval = m.kind.ambient_sound_interval();
    let Some(st) = state_mut(m) else { return Traded { leveled_up: false } };
    let Some(offer) = st.offers.as_mut().and_then(|o| o.get_mut(index)) else { return Traded { leveled_up: false } };
    offer.increase_uses();
    let (offer_xp, reward) = (offer.xp, offer.reward_exp);
    // `onReputationEvent(TRADE, tradingPlayer)`.
    if let Some(u) = st.trading_player.and_then(|id| level.player(id)).map(|p| p.uuid) {
        st.gossips.on_event(crate::mob::gossip::ReputationEvent::Trade, u);
        if st.trading_player.is_some() {
            let hero = st.trading_player.and_then(|id| level.player(id)).and_then(|p| p.hero_of_the_village);
            update_special_prices(st, u, hero);
        }
    }
    // `rewardTradeXp`.
    let mut orb = 3 + e.random.next_int_bounded(4);
    st.xp += offer_xp;
    st.last_traded_player = st.trading_player;
    let mut leveled_up = false;
    if can_level_up(st.level) && st.xp >= max_xp_per_level(st.level) {
        // `increaseMerchantCareer`: same profession, so the offers stay; the next level's are
        // added. (Regeneration for 10 s is not simulated on mobs.)
        st.level += 1;
        update_trades(e, st, level);
        leveled_up = true;
        orb += 5;
    }
    m.ambient_sound_time = -interval;
    if reward {
        let p = e.position();
        mob::award_experience(level, Vec3::new(p.x, p.y + 0.5, p.z), orb);
    }
    Traded { leveled_up }
}

/// `AbstractVillager.notifyTradeUpdated`: the yes/no sound when the result preview changes.
pub fn notify_trade_updated(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, has_result: bool) {
    let interval = m.kind.ambient_sound_interval();
    if m.ambient_sound_time > -interval + 20 {
        m.ambient_sound_time = -interval;
        let s = if has_result { "minecraft:entity.villager.yes" } else { "minecraft:entity.villager.no" };
        mob::make_sound(e, m, level, mob::sound_event(s));
    }
}

/// Runs `f` on a villager entity and its mob data (outside the mob tick).
pub fn with_villager<R>(e: &mut Entity, f: impl FnOnce(&mut Entity, &mut MobData) -> R) -> Option<R> {
    use crate::entity::EntityKind;
    if !mob::data(e).is_some_and(|m| state(m).is_some()) {
        return None;
    }
    let mut kind = std::mem::replace(&mut e.kind, EntityKind::MobTicking { gravity: 0.08 });
    let r = match &mut kind {
        EntityKind::Mob(m) => Some(f(e, m)),
        _ => None,
    };
    e.kind = kind;
    r
}

/// `Villager.setTradingPlayer(null)` (the merchant screen closed).
pub fn stop_trading(m: &mut MobData) {
    if let Some(st) = state_mut(m) {
        st.trading_player = None;
        reset_special_prices(st);
    }
}

/// `resetSpecialPrices`.
fn reset_special_prices(st: &mut VillagerState) {
    for o in st.offers.iter_mut().flatten() {
        o.special_price_diff = 0;
    }
}

/// `updateSpecialPrices(player)`: the player's reputation takes `floor(reputation * price
/// multiplier)` off each first cost; Hero of the Village takes 30% (+6.25% per level) of the
/// base cost more, at least one.
pub fn update_special_prices(st: &mut VillagerState, player: u128, hero: Option<i32>) {
    reset_special_prices(st);
    let reputation = st.gossips.reputation(player);
    let hero_modifier = hero.map_or(0.0, |a| (0.3f32 + 0.0625f32 * a as f32) as f64);
    for o in st.offers.iter_mut().flatten() {
        if reputation != 0 {
            o.special_price_diff += -kiln_javamath::math::floor_f32(reputation as f32 * o.price_multiplier);
        }
        if hero_modifier > 0.0 {
            let reduction = (hero_modifier * o.cost_a.count as f64).floor() as i32;
            o.special_price_diff += -reduction.max(1);
        }
    }
}

/// `Villager.onReputationEventFrom` on villager `m` (the trading player's prices follow).
pub fn reputation_event(m: &mut MobData, event: crate::mob::gossip::ReputationEvent, source: u128) {
    if let Some(st) = state_mut(m) {
        st.gossips.on_event(event, source);
    }
}

/// The UUID of player or entity `id`.
fn uuid_of(level: &dyn EntityLevel, id: i32) -> Option<u128> {
    match level.player(id) {
        Some(p) => Some(p.uuid),
        None => level.entity(id).map(|o| o.uuid),
    }
}

/// `tellWitnessesThatIWasMurdered`: the villagers that could see this one (its visible living
/// entities: within 16 blocks, in line of sight) remember the killer (`VILLAGER_KILLED`).
fn tell_witnesses(e: &Entity, level: &mut dyn EntityLevel, source: &DamageSource) {
    let Some(killer) = source.attacker.and_then(|a| uuid_of(level, a)) else { return };
    let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
    let eye = Vec3::new(e.x(), e.eye_y(), e.z());
    for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
        let Some(o) = level.entity(id) else { continue };
        if o.type_name != "minecraft:villager" || o.position().distance_to_sqr(e.position()) > 256.0 {
            continue;
        }
        let to = Vec3::new(o.x(), o.eye_y(), o.z());
        if mob::clip_blocks(level, eye, to) {
            continue;
        }
        if let Some(om) = level.entity_mut(id).and_then(mob::data_mut) {
            reputation_event(om, crate::mob::gossip::ReputationEvent::VillagerKilled, killer);
        }
    }
}

/// `setUnhappy`: 40 ticks of head shaking and the "no" sound.
fn set_unhappy(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(st) = state_mut(m) {
        st.unhappy = 40;
    }
    mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.villager.no"));
}

// ---------------------------------------------------------------------------- professions

/// The job site `PoiType`s (`#minecraft:acquirable_job_site`): each is also its profession's name.
const JOBS: [&str; 13] = [
    "minecraft:armorer",
    "minecraft:butcher",
    "minecraft:cartographer",
    "minecraft:cleric",
    "minecraft:farmer",
    "minecraft:fisherman",
    "minecraft:fletcher",
    "minecraft:leatherworker",
    "minecraft:librarian",
    "minecraft:mason",
    "minecraft:shepherd",
    "minecraft:toolsmith",
    "minecraft:weaponsmith",
];

/// `profession.heldJobSite().test(poi)`: only the profession's own job site.
pub fn held_job_site(profession: &str, poi: &str) -> bool {
    JOBS.contains(&profession) && profession == poi
}

/// `profession.acquirableJobSite().test(poi)`: an unemployed villager takes any job site, a
/// nitwit none.
pub fn acquirable_job_site(profession: &str, poi: &str) -> bool {
    match profession {
        "minecraft:none" => JOBS.contains(&poi),
        "minecraft:nitwit" => false,
        p => held_job_site(p, poi),
    }
}

/// The profession that holds the job site of `poi`.
pub fn profession_of_job_site(poi: &str) -> Option<&'static str> {
    JOBS.iter().find(|j| **j == poi).copied()
}

/// `profession.secondaryPoi()`: farmland for the farmer.
pub fn secondary_poi_block(profession: &str) -> Option<&'static str> {
    (profession == "minecraft:farmer").then_some("minecraft:farmland")
}

/// `profession.workSound()`.
fn work_sound(profession: &str) -> Option<&'static str> {
    Some(match profession {
        "minecraft:armorer" => "minecraft:entity.villager.work_armorer",
        "minecraft:butcher" => "minecraft:entity.villager.work_butcher",
        "minecraft:cartographer" => "minecraft:entity.villager.work_cartographer",
        "minecraft:cleric" => "minecraft:entity.villager.work_cleric",
        "minecraft:farmer" => "minecraft:entity.villager.work_farmer",
        "minecraft:fisherman" => "minecraft:entity.villager.work_fisherman",
        "minecraft:fletcher" => "minecraft:entity.villager.work_fletcher",
        "minecraft:leatherworker" => "minecraft:entity.villager.work_leatherworker",
        "minecraft:librarian" => "minecraft:entity.villager.work_librarian",
        "minecraft:mason" => "minecraft:entity.villager.work_mason",
        "minecraft:shepherd" => "minecraft:entity.villager.work_shepherd",
        "minecraft:toolsmith" => "minecraft:entity.villager.work_toolsmith",
        "minecraft:weaponsmith" => "minecraft:entity.villager.work_weaponsmith",
        _ => return None,
    })
}

/// `profession.requestedItems()`: the farmer's wheat, seeds and bone meal.
fn requested_items(profession: &str) -> &'static [&'static str] {
    if profession == "minecraft:farmer" {
        &["minecraft:wheat", "minecraft:wheat_seeds", "minecraft:beetroot_seeds", "minecraft:bone_meal"]
    } else {
        &[]
    }
}

/// `GiveGiftToHero.GIFTS` (the loot table of the hero gift).
fn gift_table(profession: &str, baby: bool) -> &'static str {
    if baby {
        return "minecraft:gameplay/hero_of_the_village/baby_gift";
    }
    match profession {
        "minecraft:armorer" => "minecraft:gameplay/hero_of_the_village/armorer_gift",
        "minecraft:butcher" => "minecraft:gameplay/hero_of_the_village/butcher_gift",
        "minecraft:cartographer" => "minecraft:gameplay/hero_of_the_village/cartographer_gift",
        "minecraft:cleric" => "minecraft:gameplay/hero_of_the_village/cleric_gift",
        "minecraft:farmer" => "minecraft:gameplay/hero_of_the_village/farmer_gift",
        "minecraft:fisherman" => "minecraft:gameplay/hero_of_the_village/fisherman_gift",
        "minecraft:fletcher" => "minecraft:gameplay/hero_of_the_village/fletcher_gift",
        "minecraft:leatherworker" => "minecraft:gameplay/hero_of_the_village/leatherworker_gift",
        "minecraft:librarian" => "minecraft:gameplay/hero_of_the_village/librarian_gift",
        "minecraft:mason" => "minecraft:gameplay/hero_of_the_village/mason_gift",
        "minecraft:shepherd" => "minecraft:gameplay/hero_of_the_village/shepherd_gift",
        "minecraft:toolsmith" => "minecraft:gameplay/hero_of_the_village/toolsmith_gift",
        "minecraft:weaponsmith" => "minecraft:gameplay/hero_of_the_village/weaponsmith_gift",
        _ => "minecraft:gameplay/hero_of_the_village/unemployed_gift",
    }
}

// ---------------------------------------------------------------------------- the schedule

/// `minecraft:villager_schedule` `minecraft:gameplay/villager_activity`: the activity at the
/// overworld clock's time of day (keyframes at 10, 2000, 9000, 11000 and 12000 ticks; before
/// the first one the last one holds).
pub fn villager_schedule(level: &dyn EntityLevel) -> Activity {
    match level.day_time().rem_euclid(24000) {
        10..2000 => Activity::Idle,
        2000..9000 => Activity::Work,
        9000..11000 => Activity::Meet,
        11000..12000 => Activity::Idle,
        _ => Activity::Rest,
    }
}

/// `minecraft:gameplay/baby_villager_activity`.
pub fn baby_villager_schedule(level: &dyn EntityLevel) -> Activity {
    match level.day_time().rem_euclid(24000) {
        10..3000 => Activity::Idle,
        3000..6000 => Activity::Play,
        6000..10000 => Activity::Idle,
        10000..12000 => Activity::Play,
        _ => Activity::Rest,
    }
}

// ---------------------------------------------------------------------------- the brain

/// The looks of `getFullLookBehavior`: cats, villagers, players, then the creatures around.
fn look_type(cx: &Cx, id: i32, name: &str) -> bool {
    cx.level.entity(id).map(|o| o.type_name == name).unwrap_or_else(|| cx.level.player(id).is_some() && name == "minecraft:player")
}

fn look_category(cx: &Cx, id: i32, category: mob::Category) -> bool {
    cx.level.entity(id).and_then(mob::data).is_some_and(|d| d.kind.category() == category)
}

fn is_cat(cx: &Cx, id: i32) -> bool {
    look_type(cx, id, "minecraft:cat")
}
fn is_villager(cx: &Cx, id: i32) -> bool {
    look_type(cx, id, "minecraft:villager")
}
fn is_player(cx: &Cx, id: i32) -> bool {
    look_type(cx, id, "minecraft:player")
}
fn is_creature(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::Creature)
}
fn is_water_creature(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::WaterCreature)
}
fn is_axolotl(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::Axolotls)
}
fn is_underground_water_creature(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::UndergroundWaterCreature)
}
fn is_water_ambient(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::WaterAmbient)
}
fn is_monster(cx: &Cx, id: i32) -> bool {
    look_category(cx, id, mob::Category::Monster)
}

type Prio = Vec<(i32, Box<dyn brain::Control>)>;

/// `getFullLookBehavior`.
fn full_look() -> (i32, Box<dyn brain::Control>) {
    (
        5,
        Gate::run_one(vec![
            (set_entity_look_target(is_cat, 8.0), 8),
            (set_entity_look_target(is_villager, 8.0), 2),
            (set_entity_look_target(is_player, 8.0), 2),
            (set_entity_look_target(is_creature, 8.0), 1),
            (set_entity_look_target(is_water_creature, 8.0), 1),
            (set_entity_look_target(is_axolotl, 8.0), 1),
            (set_entity_look_target(is_underground_water_creature, 8.0), 1),
            (set_entity_look_target(is_water_ambient, 8.0), 1),
            (set_entity_look_target(is_monster, 8.0), 1),
            (DoNothing::new(30, 60), 2),
        ]),
    )
}

/// `getMinimalLookBehavior`.
fn minimal_look() -> (i32, Box<dyn brain::Control>) {
    (5, Gate::run_one(vec![(set_entity_look_target(is_villager, 8.0), 2), (set_entity_look_target(is_player, 8.0), 2), (DoNothing::new(30, 60), 8)]))
}

/// `UpdateActivityFromSchedule.create()`.
fn update_activity_from_schedule() -> (i32, Box<dyn brain::Control>) {
    (
        99,
        shot("UpdateActivityFromSchedule", &[], |cx| {
            let t = cx.time;
            cx.b.update_activity_from_schedule(t, &*cx.level);
            true
        }),
    )
}

fn can_breed_self(cx: &Cx) -> bool {
    can_breed(cx.m)
}

fn can_breed_target(cx: &Cx, id: i32) -> bool {
    cx.level.entity(id).and_then(mob::data).is_some_and(can_breed)
}

fn any_self(_: &Cx) -> bool {
    true
}

fn any_target(_: &Cx, _: i32) -> bool {
    true
}

/// `getCorePackage(profession, speed)`.
fn core_package(profession: &'static str, speed: f32) -> Prio {
    use poi::PoiWant;
    vec![
        (0, Swim::new(0.8)),
        (0, stroll::InteractWithDoor::new()),
        (0, LookAtTargetSink::new(45, 90)),
        (0, raid::VillagerPanicTrigger::new()),
        (0, poi::wake_up()),
        (0, raid::react_to_bell()),
        (0, raid::set_raid_status()),
        (0, poi::validate_nearby_poi(PoiWant::HeldJob(profession), Mem::JobSite)),
        (0, poi::validate_nearby_poi(PoiWant::Job(profession), Mem::PotentialJobSite)),
        (1, MoveToTargetSink::new()),
        (2, poi::poi_competitor_scan()),
        (3, social::LookAndFollowTradingPlayerSink::new(speed)),
        (5, village::work::go_to_wanted_item(speed, 4)),
        (6, poi::AcquirePoi::new(PoiWant::Job(profession), Mem::JobSite, Mem::PotentialJobSite, true, None, false)),
        (7, poi::GoToPotentialJobSite::new(speed)),
        (8, poi::yield_job_site(speed)),
        (10, poi::AcquirePoi::new(PoiWant::Home, Mem::Home, Mem::Home, false, Some(14), true)),
        (10, poi::AcquirePoi::new(PoiWant::Meeting, Mem::MeetingPoint, Mem::MeetingPoint, true, Some(14), false)),
        (10, poi::assign_profession_from_job_site()),
        (10, poi::reset_profession()),
    ]
}

/// `getWorkPackage(profession, speed)`.
fn work_package(profession: &'static str, speed: f32) -> Prio {
    let farmer = profession == "minecraft:farmer";
    vec![
        minimal_look(),
        (
            5,
            Gate::run_one(vec![
                (work::WorkAtPoi::new(farmer), 7),
                (stroll::StrollPoi::around(Mem::JobSite, 0.4, 4), 2),
                (stroll::StrollPoi::to(Mem::JobSite, 0.4, 1, 10), 5),
                (stroll::StrollToPoiList::new(speed, 1, 6), 5),
                (work::HarvestFarmland::new(), if farmer { 2 } else { 5 }),
                (work::UseBonemeal::new(), if farmer { 4 } else { 7 }),
            ]),
        ),
        (10, social::ShowTradesToPlayer::new(400, 1600)),
        (10, social::set_look_and_interact("minecraft:player", 4)),
        (2, poi::set_walk_target_from_block_memory(Mem::JobSite, speed, 9, 100, 1200)),
        (3, social::GiveGiftToHero::new(100)),
        update_activity_from_schedule(),
    ]
}

/// `getPlayPackage(speed)`.
fn play_package(speed: f32) -> Prio {
    vec![
        (0, MoveToTargetSink::with_durations(80, 120)),
        full_look(),
        (5, social::play_tag_with_other_kids()),
        (
            5,
            Gate::run_one_when(
                &[(Mem::VisibleVillagerBabies, Status::ValueAbsent)],
                vec![
                    (social::interact_with("minecraft:villager", 8, any_self, any_target, Mem::InteractionTarget, speed, 2), 2),
                    (social::interact_with("minecraft:cat", 8, any_self, any_target, Mem::InteractionTarget, speed, 2), 1),
                    (stroll::village_bound_random_stroll(speed, 10, 7), 1),
                    (set_walk_target_from_look_target(speed, 2), 1),
                    (stroll::JumpOnBed::new(speed), 2),
                    (DoNothing::new(20, 40), 2),
                ],
            ),
        ),
        update_activity_from_schedule(),
    ]
}

/// `getRestPackage(speed)`.
fn rest_package(speed: f32) -> Prio {
    use poi::PoiWant;
    vec![
        (2, poi::set_walk_target_from_block_memory(Mem::Home, speed, 1, 150, 1200)),
        (3, poi::validate_nearby_poi(PoiWant::Home, Mem::Home)),
        (3, poi::SleepInBed::new()),
        (
            5,
            Gate::run_one_when(
                &[(Mem::Home, Status::ValueAbsent)],
                vec![
                    (poi::SetClosestHomeAsWalkTarget::new(speed), 1),
                    (stroll::inside_brownian_walk(speed), 4),
                    (stroll::go_to_closest_village(speed, 4), 2),
                    (DoNothing::new(20, 40), 2),
                ],
            ),
        ),
        minimal_look(),
        update_activity_from_schedule(),
    ]
}

/// `getMeetPackage(speed)`.
fn meet_package(speed: f32) -> Prio {
    use poi::PoiWant;
    vec![
        (2, TriggerGate::one_shuffled(vec![(stroll::StrollPoi::around(Mem::MeetingPoint, 0.4, 40), 2), (social::socialize_at_bell(), 2)])),
        (10, social::ShowTradesToPlayer::new(400, 1600)),
        (10, social::set_look_and_interact("minecraft:player", 4)),
        (2, poi::set_walk_target_from_block_memory(Mem::MeetingPoint, speed, 6, 100, 200)),
        (3, social::GiveGiftToHero::new(100)),
        (3, poi::validate_nearby_poi(PoiWant::Meeting, Mem::MeetingPoint)),
        (3, social::trade_with_villager_gate()),
        full_look(),
        update_activity_from_schedule(),
    ]
}

/// `getIdlePackage(speed)`.
fn idle_package(speed: f32) -> Prio {
    vec![
        (
            2,
            Gate::run_one(vec![
                (social::interact_with("minecraft:villager", 8, any_self, any_target, Mem::InteractionTarget, speed, 2), 2),
                (social::interact_with("minecraft:villager", 8, can_breed_self, can_breed_target, Mem::BreedTarget, speed, 2), 1),
                (social::interact_with("minecraft:cat", 8, any_self, any_target, Mem::InteractionTarget, speed, 2), 1),
                (stroll::village_bound_random_stroll(speed, 10, 7), 1),
                (set_walk_target_from_look_target(speed, 2), 1),
                (stroll::JumpOnBed::new(speed), 1),
                (DoNothing::new(30, 60), 1),
            ]),
        ),
        (3, social::GiveGiftToHero::new(100)),
        (3, social::set_look_and_interact("minecraft:player", 4)),
        (3, social::ShowTradesToPlayer::new(400, 1600)),
        (3, social::trade_with_villager_gate()),
        (3, social::make_love_gate()),
        full_look(),
        update_activity_from_schedule(),
    ]
}

/// `getPanicPackage(speed)`.
fn panic_package(speed: f32) -> Prio {
    let fast = speed * 1.5f32;
    vec![
        (0, raid::villager_calm_down()),
        (1, stroll::walk_away_from_entity(Mem::NearestHostile, fast, 6, false)),
        (1, stroll::walk_away_from_entity(Mem::HurtByEntity, fast, 6, false)),
        (3, stroll::village_bound_random_stroll(fast, 2, 2)),
        minimal_look(),
    ]
}

/// `getPreRaidPackage(speed)`.
fn pre_raid_package(speed: f32) -> Prio {
    vec![
        (0, raid::ring_bell()),
        (
            0,
            TriggerGate::one_shuffled(vec![
                (poi::set_walk_target_from_block_memory(Mem::MeetingPoint, speed * 1.5f32, 2, 150, 200), 6),
                (stroll::village_bound_random_stroll(speed * 1.5f32, 10, 7), 2),
            ]),
        ),
        minimal_look(),
        (99, raid::reset_raid_status()),
    ]
}

/// `getRaidPackage(speed)`.
fn raid_package(speed: f32) -> Prio {
    vec![
        (
            0,
            raid::sequence_if(
                |cx| cx.level.raid_at(cx.e.block_position()).is_some_and(|r| r.over && !r.loss),
                TriggerGate::one_shuffled(vec![(stroll::move_to_sky_seeing_spot(speed), 5), (stroll::village_bound_random_stroll(speed * 1.1f32, 10, 7), 2)]),
            ),
        ),
        (0, raid::CelebrateVillagersSurvivedRaid::new()),
        (
            2,
            raid::sequence_if(
                |cx| cx.level.raid_at(cx.e.block_position()).is_some_and(|r| r.active && !(r.over && !r.loss) && !r.loss),
                raid::LocateHidingPlace::new(24, speed * 1.4f32, 1),
            ),
        ),
        minimal_look(),
        (99, raid::reset_raid_status()),
    ]
}

/// `getHidePackage(speed)`.
fn hide_package(speed: f32) -> Prio {
    vec![(0, raid::SetHiddenState::new(15, 3)), (1, raid::LocateHidingPlace::new(32, speed * 1.25f32, 2)), minimal_look()]
}

/// `Villager.BRAIN_PROVIDER`: the sensors and, by age and profession, the activities.
pub fn build_brain(m: &MobData, random: &mut dyn RandomSource) -> Brain {
    let profession = state(m).map_or("minecraft:none", |s| s.profession);
    let baby = m.baby();
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(village::sensors::NearestItems),
        Box::new(village::sensors::NearestBed),
        Box::new(sensors::HurtBy),
        Box::new(village::sensors::VillagerHostiles),
        Box::new(village::sensors::VillagerBabies),
        Box::new(village::sensors::SecondaryPois),
        Box::new(village::sensors::GolemDetected),
    ];
    let speed = 0.5f32;
    let mut activities = Vec::new();
    if baby {
        activities.push(ActivityData::with_priorities(Activity::Play, play_package(speed)));
    } else {
        activities.push(ActivityData::with_conditions(Activity::Work, work_package(profession, speed), &[(Mem::JobSite, Status::ValuePresent)]));
    }
    activities.push(ActivityData::with_priorities(Activity::Core, core_package(profession, speed)));
    activities.push(ActivityData::with_conditions(Activity::Meet, meet_package(speed), &[(Mem::MeetingPoint, Status::ValuePresent)]));
    activities.push(ActivityData::with_priorities(Activity::Rest, rest_package(speed)));
    activities.push(ActivityData::with_priorities(Activity::Idle, idle_package(speed)));
    activities.push(ActivityData::with_priorities(Activity::Panic, panic_package(speed)));
    activities.push(ActivityData::with_priorities(Activity::PreRaid, pre_raid_package(speed)));
    activities.push(ActivityData::with_priorities(Activity::Raid, raid_package(speed)));
    activities.push(ActivityData::with_priorities(Activity::Hide, hide_package(speed)));
    let mut b = Brain::new(&[], sensors, activities, random);
    b.st.schedule = Some(if baby { baby_villager_schedule } else { villager_schedule });
    b
}

/// `refreshBrain` without a level: a new brain for the villager's age and profession, the
/// memories (those with a codec) carried over. The behaviours that ran are not stopped.
fn rebuild_brain(e: &mut Entity, m: &mut MobData) {
    let Some(old) = m.brain.take() else { return };
    let packed = brain::persist::save(&old);
    let mut new = build_brain(m, &mut e.random);
    brain::persist::load(&mut new, &packed);
    m.brain = Some(Box::new(new));
    if let Some(st) = state_mut(m) {
        st.schedule_pending = true;
    }
}

/// `Villager.refreshBrain(level)`: the running behaviours stop, then the new brain reads the
/// schedule at once.
fn refresh_brain(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(mut old) = m.brain.take() else { return };
    old.stop_all(e, m, level);
    let packed = brain::persist::save(&old);
    let mut new = build_brain(m, &mut e.random);
    brain::persist::load(&mut new, &packed);
    let t = level.game_time();
    new.st.update_activity_from_schedule(t, &*level);
    m.brain = Some(Box::new(new));
}

// ---------------------------------------------------------------------------- inventory

/// `SimpleContainer.canAddItem`.
fn can_add_item(st: &VillagerState, stack: &ItemStack) -> bool {
    st.inventory.iter().any(|s| s.is_empty() || (s.is_same_item_same_components(stack) && s.count() < s.max_stack_size()))
}

/// `SimpleContainer.addItem`: what does not fit comes back.
fn add_item(st: &mut VillagerState, mut stack: ItemStack) -> ItemStack {
    for slot in st.inventory.iter_mut() {
        if stack.is_empty() {
            break;
        }
        if !slot.is_empty() && slot.is_same_item_same_components(&stack) {
            let n = (slot.max_stack_size() - slot.count()).min(stack.count());
            slot.grow(n);
            stack.shrink(n);
        }
    }
    for slot in st.inventory.iter_mut() {
        if stack.is_empty() {
            break;
        }
        if slot.is_empty() {
            *slot = std::mem::replace(&mut stack, ItemStack::empty());
        }
    }
    stack
}

/// `SimpleContainer.countItem`.
pub fn count_item(m: &MobData, name: &str) -> i32 {
    state(m).map_or(0, |s| s.inventory.iter().filter(|x| !x.is_empty() && x.item_name() == name).map(ItemStack::count).sum())
}

fn food_points(stack: &ItemStack) -> i32 {
    stack.get(kiln_item::keys::VILLAGER_FOOD).map_or(0, |f| f.nutrition)
}

/// `countFoodPointsInInventory`.
fn count_food_points(st: &VillagerState) -> i32 {
    st.inventory.iter().filter(|s| !s.is_empty()).map(|s| s.count() * food_points(s)).sum()
}

/// `Villager.hasFarmSeeds`.
pub fn has_farm_seeds(m: &MobData) -> bool {
    state(m).is_some_and(|s| s.inventory.iter().any(|x| !x.is_empty() && mob::item_tag(x.item(), "minecraft:villager_plantable_seeds")))
}

/// `Villager.wantsToPickUp(level, stack)`.
pub fn wants_to_pick_up(m: &MobData, stack: &ItemStack) -> bool {
    let Some(st) = state(m) else { return false };
    let wanted = mob::item_tag(stack.item(), "minecraft:villager_picks_up") || stack.get(kiln_item::keys::VILLAGER_FOOD).is_some() || requested_items(st.profession).contains(&stack.item_name());
    wanted && can_add_item(st, stack)
}

/// `Villager.hasExcessFood` / `wantsMoreFood`.
fn has_excess_food(m: &MobData) -> bool {
    state(m).is_some_and(|s| count_food_points(s) >= 24)
}

fn wants_more_food(m: &MobData) -> bool {
    state(m).is_some_and(|s| count_food_points(s) < 12)
}

/// `Villager.canBreed`.
pub fn can_breed(m: &MobData) -> bool {
    state(m).is_some_and(|s| s.food_level as i32 + count_food_points(s) >= 12 && s.sleeping_pos.is_none()) && m.age == 0
}

/// `Villager.eatUntilFull` then `digestFood(12)`.
pub fn eat_and_digest_food(m: &mut MobData) {
    let Some(st) = state_mut(m) else { return };
    // eatUntilFull
    if (st.food_level as i32) < 12 {
        'slots: for i in 0..st.inventory.len() {
            let Some(nutrition) = st.inventory[i].get(kiln_item::keys::VILLAGER_FOOD).map(|f| f.nutrition) else { continue };
            let count = st.inventory[i].count();
            let mut eaten = 0;
            let mut left = count;
            while left > 0 {
                st.food_level = st.food_level.wrapping_add(nutrition as i8);
                eaten += 1;
                if (st.food_level as i32) >= 12 {
                    st.inventory[i].shrink(eaten);
                    if st.inventory[i].count() <= 0 {
                        st.inventory[i] = ItemStack::empty();
                    }
                    break 'slots;
                }
                left -= 1;
            }
            st.inventory[i].shrink(eaten);
            if st.inventory[i].count() <= 0 {
                st.inventory[i] = ItemStack::empty();
            }
        }
    }
    st.food_level = st.food_level.wrapping_sub(12);
}

/// `eatAndDigestFood` on villager `id`.
pub fn eat_and_digest_food_of(level: &mut dyn EntityLevel, id: i32) {
    if let Some(o) = level.entity_mut(id)
        && let Some(om) = mob::data_mut(o)
    {
        eat_and_digest_food(om);
    }
}

// ---------------------------------------------------------------------------- sleeping

/// `LivingEntity.isSleeping` of a villager.
pub fn is_sleeping(m: &MobData) -> bool {
    state(m).is_some_and(|s| s.sleeping_pos.is_some())
}

fn is_bed(state: u16) -> bool {
    crate::blocks::block_name(state).ends_with("_bed")
}

/// `LivingEntity.startSleeping(pos)`: onto the bed (its top and a little), occupied, lying.
pub fn start_sleeping(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, bed: BlockPos) -> bool {
    let s = level.block(bed);
    if !is_bed(s) {
        return false;
    }
    let info = kiln_data::blocks_types::block_of(s);
    // `getSleepHeight`: the bed's shape reaches 9/16.
    let height = 0.5625;
    e.set_pos(Vec3::new(bed.x as f64 + 0.5, bed.y as f64 + height + 0.125, bed.z as f64 + 0.5));
    if let Some(n) = info.with_property(s, "occupied", "true") {
        level.set_block(bed, n, 3);
    }
    if let Some(st) = state_mut(m) {
        st.sleeping_pos = Some(bed);
    }
    mob::refresh_dimensions(e, m);
    e.delta = Vec3::ZERO;
    e.needs_sync = true;
    true
}

/// `LivingEntity.stopSleeping` and `Villager.stopSleeping` (the wake time goes in the brain: the
/// one that is ticking, when the caller has it).
pub fn stop_sleeping(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, brain: Option<&mut brain::BrainState>) {
    let Some(bed) = state(m).and_then(|s| s.sleeping_pos) else { return };
    let s = level.block(bed);
    if level.is_loaded(bed) && is_bed(s) {
        let info = kiln_data::blocks_types::block_of(s);
        if let Some(n) = info.with_property(s, "occupied", "false") {
            level.set_block(bed, n, 3);
        }
        let dir = bed_facing(s);
        let stand = bed_stand_up(level, bed, dir, e.y_rot).unwrap_or(Vec3::new(bed.x as f64 + 0.5, (bed.y + 1) as f64 + 0.1, bed.z as f64 + 0.5));
        let to_bed = Vec3::new(bed.x as f64 + 0.5 - stand.x, bed.y as f64 - stand.y, bed.z as f64 + 0.5 - stand.z).normalize();
        let yaw = mob::mth::wrap_degrees((mob::mth::atan2(to_bed.z, to_bed.x) * 57.2957763671875 - 90.0) as f32);
        e.set_pos(stand);
        e.y_rot = yaw;
        e.x_rot = 0.0;
    }
    let pos = e.position();
    if let Some(st) = state_mut(m) {
        st.sleeping_pos = None;
    }
    mob::refresh_dimensions(e, m);
    e.set_pos(pos);
    // `Villager.stopSleeping`: `LAST_WOKEN`.
    let now = level.game_time();
    match brain {
        Some(b) => b.mem.set(Mem::LastWoken, brain::Val::Long(now)),
        None => {
            if let Some(b) = m.brain.as_mut() {
                b.st.mem.set(Mem::LastWoken, brain::Val::Long(now));
            }
        }
    }
}

/// `BedBlock.FACING` as a step (x, z).
fn bed_facing(state: u16) -> (i32, i32) {
    match kiln_data::blocks_types::block_of(state).property(state, "facing") {
        Some("north") => (0, -1),
        Some("south") => (0, 1),
        Some("west") => (-1, 0),
        _ => (1, 0),
    }
}

/// `Direction.isFacingAngle(yaw)`.
fn facing_angle(dir: (i32, i32), yaw: f32) -> bool {
    let r = (yaw * 0.017_453_292_f32) as f64;
    let (x, z) = (-mob::mth::sin(r), mob::mth::cos(r));
    dir.0 as f32 * x + dir.1 as f32 * z > 0.0
}

fn dangerous(s: u16) -> bool {
    use kiln_data::blocks::default_state as d;
    let same = |a: u16, b: u16| kiln_data::blocks_types::block_of(a).name == kiln_data::blocks_types::block_of(b).name;
    crate::blocks::has_tag(s, crate::blocks::Tag::Fire)
        || kiln_data::blocks_types::block_of(s).name.ends_with("campfire")
        || [d::MAGMA_BLOCK, d::LAVA, d::WITHER_ROSE, d::SWEET_BERRY_BUSH, d::CACTUS, d::POWDER_SNOW].iter().any(|&b| same(s, b))
}

fn floor_top(s: u16) -> Option<f64> {
    if crate::blocks::has_tag(s, crate::blocks::Tag::Climbable) {
        return None;
    }
    kiln_data::block_props::collision(s).iter().map(|b| b[4] as f64).reduce(f64::max)
}

fn collides(level: &dyn EntityLevel, min: [f64; 3], max: [f64; 3]) -> bool {
    let (x0, y0, z0) = (min[0].floor() as i32, min[1].floor() as i32 - 1, min[2].floor() as i32);
    let (x1, y1, z1) = (max[0].floor() as i32, max[1].floor() as i32, max[2].floor() as i32);
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let s = level.block(BlockPos::new(x, y, z));
                for b in kiln_data::block_props::collision(s) {
                    let bmin = [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64];
                    let bmax = [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64];
                    if (0..3).all(|i| bmin[i] < max[i] - 1.0e-7 && bmax[i] > min[i] + 1.0e-7) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `DismountHelper.findSafeDismountLocation` for a villager (0.6 by 1.95) at block `pos`.
fn safe_dismount(level: &dyn EntityLevel, pos: BlockPos, check_dangerous: bool) -> Option<Vec3> {
    if check_dangerous && dangerous(level.block(pos)) {
        return None;
    }
    let floor = match floor_top(level.block(pos)) {
        Some(t) => t,
        None => match floor_top(level.block(pos.below())) {
            Some(t) if t >= 1.0 => t - 1.0,
            _ => f64::NEG_INFINITY,
        },
    };
    if !(floor.is_finite() && floor < 1.0) {
        return None;
    }
    if check_dangerous && floor <= 0.0 && dangerous(level.block(pos.below())) {
        return None;
    }
    let v = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + floor, pos.z as f64 + 0.5);
    let (min, max) = ([v.x - 0.3, v.y, v.z - 0.3], [v.x + 0.3, v.y + 1.95, v.z + 0.3]);
    if collides(level, min, max) {
        return None;
    }
    Some(v)
}

/// `AbstractBedBlock.findStandUpPosition` (bunk beds are treated as plain beds).
fn bed_stand_up(level: &dyn EntityLevel, head: BlockPos, dir: (i32, i32), yaw: f32) -> Option<Vec3> {
    // `getClockWise` of a horizontal direction: (x, z) -> (-z, x).
    let cw = (-dir.1, dir.0);
    let rot = if facing_angle(cw, yaw) { (-cw.0, -cw.1) } else { cw };
    let (dx, dz) = dir;
    let (rx, rz) = rot;
    let offsets = [
        [rx, rz],
        [rx - dx, rz - dz],
        [rx - dx * 2, rz - dz * 2],
        [-dx * 2, -dz * 2],
        [-rx - dx * 2, -rz - dz * 2],
        [-rx - dx, -rz - dz],
        [-rx, -rz],
        [-rx + dx, -rz + dz],
        [dx, dz],
        [rx + dx, rz + dz],
        [0, 0],
        [-dx, -dz],
    ];
    for check in [true, false] {
        for o in offsets {
            if let Some(v) = safe_dismount(level, BlockPos::new(head.x + o[0], head.y, head.z + o[1]), check) {
                return Some(v);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------- work

/// `Villager.playWorkSound`.
pub fn play_work_sound(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(s) = state(m).and_then(|s| work_sound(s.profession)) {
        mob::make_sound(e, m, level, s);
    }
}

/// `AbstractVillager.playCelebrateSound`.
pub fn play_celebrate_sound(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    mob::make_sound(e, m, level, "minecraft:entity.villager.celebrate");
}

/// `Villager.offers` for the behaviours (generated on first use).
pub fn offers_now(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> Vec<MerchantOffer> {
    offers(e, m, level).to_vec()
}

/// `Villager.shouldRestock(level)`.
pub fn should_restock(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    let now = level.game_time();
    let day = level.day_time().div_euclid(24000);
    let _ = offers(e, m, level);
    let Some(st) = state_mut(m) else { return false };
    let mut reset = now > st.last_restock + 12000;
    reset |= st.last_restock_check_day > 0 && day > st.last_restock_check_day;
    st.last_restock_check_day = day;
    if reset {
        st.last_restock = now;
        // `resetNumberOfRestocks`: `catchUpDemand`, then zero.
        let missed = 2 - st.restocks_today;
        if missed > 0 {
            for o in st.offers.iter_mut().flatten() {
                o.reset_uses();
            }
        }
        for _ in 0..missed.max(0) {
            for o in st.offers.iter_mut().flatten() {
                o.update_demand();
            }
        }
        if let Some(u) = st.trading_player.and_then(|id| level.player(id)).map(|p| p.uuid) {
            let hero = st.trading_player.and_then(|id| level.player(id)).and_then(|p| p.hero_of_the_village);
            update_special_prices(st, u, hero);
        }
        st.restocks_today = 0;
    }
    let allowed = st.restocks_today == 0 || (st.restocks_today < 2 && now > st.last_restock + 2400);
    allowed && st.offers.iter().flatten().any(MerchantOffer::needs_restock)
}

/// `Villager.restock()`.
pub fn restock(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let _ = offers(e, m, level);
    let now = level.game_time();
    let Some(st) = state_mut(m) else { return };
    for o in st.offers.iter_mut().flatten() {
        o.update_demand();
    }
    for o in st.offers.iter_mut().flatten() {
        o.reset_uses();
    }
    if let Some(u) = st.trading_player.and_then(|id| level.player(id)).map(|p| p.uuid) {
        let hero = st.trading_player.and_then(|id| level.player(id)).and_then(|p| p.hero_of_the_village);
        update_special_prices(st, u, hero);
    }
    st.last_restock = now;
    st.restocks_today += 1;
}

/// `WorkAtComposter.useWorkstation`: wheat becomes bread (composting the seeds is not simulated).
pub fn use_composter(cx: &mut Cx) {
    let Some(site) = village::mem_pos(cx, Mem::JobSite) else { return };
    if crate::blocks::block_name(cx.level.block(site)) != "minecraft:composter" {
        return;
    }
    if count_item(cx.m, "minecraft:bread") > 36 {
        return;
    }
    let wheat = count_item(cx.m, "minecraft:wheat");
    let loaves = 3.min(wheat / 3);
    if loaves == 0 {
        return;
    }
    let Some(st) = state_mut(cx.m) else { return };
    // `removeItemType(WHEAT, loaves * 3)`.
    let mut to_remove = loaves * 3;
    for s in st.inventory.iter_mut() {
        if to_remove == 0 {
            break;
        }
        if !s.is_empty() && s.item_name() == "minecraft:wheat" {
            let n = s.count().min(to_remove);
            s.shrink(n);
            to_remove -= n;
            if s.count() <= 0 {
                *s = ItemStack::empty();
            }
        }
    }
    let bread = ItemStack::of("minecraft:bread", loaves).unwrap_or_else(ItemStack::empty);
    let rest = add_item(st, bread);
    if !rest.is_empty() {
        mob::spawn_at_location_offset(cx.e, cx.level, rest, 0.5);
    }
}

/// `BlockItem` of a seed item: the crop it plants.
fn seed_block(item: &str) -> Option<&'static str> {
    Some(match item {
        "minecraft:wheat_seeds" => "minecraft:wheat",
        "minecraft:beetroot_seeds" => "minecraft:beetroots",
        "minecraft:carrot" => "minecraft:carrots",
        "minecraft:potato" => "minecraft:potatoes",
        "minecraft:torchflower_seeds" => "minecraft:torchflower_crop",
        "minecraft:pitcher_pod" => "minecraft:pitcher_crop",
        _ => return None,
    })
}

/// `HarvestFarmland`'s planting: the first plantable seeds in the inventory go into the ground.
pub fn plant_seed(cx: &mut Cx, pos: BlockPos) {
    let Some(st) = state_mut(cx.m) else { return };
    for i in 0..st.inventory.len() {
        let s = &st.inventory[i];
        if s.is_empty() || !mob::item_tag(s.item(), "minecraft:villager_plantable_seeds") {
            continue;
        }
        let Some(block) = seed_block(s.item_name()).and_then(kiln_data::blocks_types::block_by_name) else { continue };
        cx.level.set_block(pos, block.default, 3);
        let id = cx.e.id;
        cx.level.emit(Event::GameEvent { event: "minecraft:block_place", pos: pos.center(), entity: Some(id) });
        cx.level.emit(Event::Sound { pos: pos.to_vec3(), sound: "minecraft:item.crop.plant", source: "blocks", volume: 1.0, pitch: 1.0 });
        st.inventory[i].shrink(1);
        if st.inventory[i].count() <= 0 {
            st.inventory[i] = ItemStack::empty();
        }
        return;
    }
}

/// `BoneMealItem.growCrop` on a crop: the age grows by 2 to 5 (1 to 2 for beetroots), one bone meal
/// is used.
pub fn grow_crop(cx: &mut Cx, pos: BlockPos) -> bool {
    let s = cx.level.block(pos);
    let info = kiln_data::blocks_types::block_of(s);
    let Some(age) = info.property(s, "age").and_then(|a| a.parse::<i32>().ok()) else { return false };
    let max = info.properties.iter().find(|p| p.name == "age").map_or(0, |p| p.values.len() as i32 - 1);
    if age >= max {
        return false;
    }
    let inc = if info.name == "minecraft:beetroots" { cx.rng().next_int_bounded(2) + 1 } else { cx.rng().next_int_bounded(4) + 2 };
    if let Some(n) = info.with_property(s, "age", &(age + inc).min(max).to_string()) {
        cx.level.set_block(pos, n, 2);
    }
    if let Some(st) = state_mut(cx.m) {
        for slot in st.inventory.iter_mut() {
            if !slot.is_empty() && slot.item_name() == "minecraft:bone_meal" {
                slot.shrink(1);
                if slot.count() <= 0 {
                    *slot = ItemStack::empty();
                }
                break;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------- talk, breed, call the golem

/// `Villager.gossip(level, other, gameTime)`.
pub fn gossip(cx: &mut Cx, other: i32) {
    let now = cx.time;
    let Some(o) = cx.level.entity(other) else { return };
    let Some(om) = mob::data(o) else { return };
    let Some(ost) = state(om) else { return };
    let (their_time, their_gossips) = (ost.last_gossip_time, ost.gossips.clone());
    let Some(st) = state_mut(cx.m) else { return };
    if (now >= st.last_gossip_time && now < st.last_gossip_time + 1200) || (now >= their_time && now < their_time + 1200) {
        return;
    }
    let moved = st.gossips.transfer_from(&their_gossips, &mut cx.e.random, 10);
    st.last_gossip_time = now;
    if let Some(o) = cx.level.entity_mut(other)
        && let Some(om) = mob::data_mut(o)
        && let Some(ost) = state_mut(om)
    {
        ost.last_gossip_time = now;
    }
    spawn_golem_if_needed(cx, 5);
    let Some(st) = state_mut(cx.m) else { return };
    if let Some(id) = st.trading_player
        && moved > 0
        && let Some(p) = cx.level.player(id)
    {
        update_special_prices(st, p.uuid, p.hero_of_the_village);
    }
}

/// `TradeWithVillager`'s handing over of food, wheat and what the other wants.
pub fn share_food(cx: &mut Cx, other: i32, trades: &[i32]) {
    let farmer = state(cx.m).is_some_and(|s| s.profession == "minecraft:farmer");
    let other_wants_more = cx.level.entity(other).and_then(mob::data).is_some_and(wants_more_food);
    if has_excess_food(cx.m) && (farmer || other_wants_more) {
        throw_half_stack(cx, other, &|s| s.get(kiln_item::keys::VILLAGER_FOOD).is_some());
    }
    if farmer && count_item(cx.m, "minecraft:wheat") > 32 {
        throw_half_stack(cx, other, &|s| s.item_name() == "minecraft:wheat");
    }
    if !trades.is_empty() && state(cx.m).is_some_and(|s| s.inventory.iter().any(|x| !x.is_empty() && trades.contains(&x.item()))) {
        let t = trades.to_vec();
        throw_half_stack(cx, other, &move |s| t.contains(&s.item()));
    }
}

/// `TradeWithVillager.figureOutWhatIAmWillingToTrade`: what the other one requests that I do not.
pub fn trades_with(cx: &Cx, other: i32) -> Vec<i32> {
    let Some(om) = cx.level.entity(other).and_then(mob::data) else { return Vec::new() };
    let theirs = state(om).map_or(&[][..], |s| requested_items(s.profession));
    let mine = state(cx.m).map_or(&[][..], |s| requested_items(s.profession));
    theirs.iter().filter(|n| !mine.contains(n)).filter_map(|n| kiln_data::builtin_id("minecraft:item", n)).collect()
}

/// `TradeWithVillager.throwHalfStack`.
fn throw_half_stack(cx: &mut Cx, target: i32, pred: &dyn Fn(&ItemStack) -> bool) {
    let mut thrown = ItemStack::empty();
    if let Some(st) = state_mut(cx.m) {
        for slot in st.inventory.iter_mut() {
            if slot.is_empty() || !pred(slot) {
                continue;
            }
            let n = if slot.count() > slot.max_stack_size() / 2 {
                slot.count() / 2
            } else if slot.count() > 24 {
                slot.count() - 24
            } else {
                continue;
            };
            thrown = slot.split(n);
            if slot.count() <= 0 {
                *slot = ItemStack::empty();
            }
            break;
        }
    }
    if !thrown.is_empty()
        && let Some(o) = cx.level.entity(target)
    {
        let to = o.position();
        throw_item(cx, thrown, to);
    }
}

/// `BehaviorUtils.throwItem(entity, stack, target)`.
pub fn throw_item(cx: &mut Cx, stack: ItemStack, target: Vec3) {
    // `new ItemEntity(level, x, y, z, stack)` draws two doubles from the level's random.
    let _ = cx.rng().next_double();
    let _ = cx.rng().next_double();
    let id = cx.level.next_entity_id();
    let seed = cx.level.fresh_seed();
    let mut item = crate::item::new(id, 0, stack, seed);
    let p = Vec3::new(cx.e.x(), cx.e.eye_y() - 0.30000001192092896, cx.e.z());
    item.set_pos(p);
    let d = (target - cx.e.position()).normalize();
    item.delta = Vec3::new(d.x * 0.30000001192092896, d.y * 0.30000001192092896, d.z * 0.30000001192092896);
    if let crate::entity::EntityKind::Item(data) = &mut item.kind {
        data.pickup_delay = 10;
    }
    cx.level.add_entity(item);
}

/// `GiveGiftToHero.throwGift`: the hero-of-the-village loot of the profession (dropped at the villager).
pub fn throw_gift(cx: &mut Cx, _hero: i32) {
    let baby = cx.m.baby();
    let table = gift_table(state(cx.m).map_or("minecraft:none", |s| s.profession), baby);
    let (id, pos) = (cx.e.id, cx.e.position());
    cx.level.emit(Event::GiftLoot { entity: id, table, pos });
}

/// `Villager.wantsToSpawnGolem(time)` for villager data `m`.
fn wants_to_spawn_golem(m: &MobData, now: i64) -> bool {
    let slept = m.brain.as_ref().and_then(|b| b.st.mem.long(Mem::LastSlept));
    slept.is_some_and(|t| now - t < 24000) && !m.brain.as_ref().is_some_and(|b| b.st.mem.has(Mem::GolemDetectedRecently))
}

/// `Villager.spawnGolemIfNeeded(level, gameTime, minVillagerAmount)`.
pub fn spawn_golem_if_needed(cx: &mut Cx, min: usize) {
    let now = cx.time;
    // The brain is out of the mob while it ticks: this villager's memories are in `cx.b`.
    let slept = cx.b.mem.long(Mem::LastSlept);
    let me = slept.is_some_and(|t| now - t < 24000) && !cx.b.mem.has(Mem::GolemDetectedRecently);
    if !me {
        return;
    }
    let area = cx.e.bounding_box().inflate(10.0, 10.0, 10.0);
    let mut wanting = vec![cx.e.id];
    for id in cx.level.entities_in(&area, crate::level::EntityFilter::Living, cx.e.id) {
        if cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager") && cx.level.entity(id).and_then(mob::data).is_some_and(|d| wants_to_spawn_golem(d, now)) {
            wanting.push(id);
        }
    }
    wanting.truncate(5);
    if wanting.len() < min {
        return;
    }
    if !try_spawn_iron_golem(cx) {
        return;
    }
    for id in wanting {
        if id == cx.e.id {
            village::sensors::golem_detected(&mut cx.b.mem);
        } else if let Some(o) = cx.level.entity_mut(id)
            && let Some(om) = mob::data_mut(o)
            && let Some(b) = om.brain.as_mut()
        {
            village::sensors::golem_detected(&mut b.st.mem);
        }
    }
}

/// `SpawnUtil.trySpawnMob(IRON_GOLEM, MOB_SUMMONED, level, pos, 10, 8, 6, LEGACY_IRON_GOLEM)`.
fn try_spawn_iron_golem(cx: &mut Cx) -> bool {
    let origin = cx.e.block_position();
    for _ in 0..10 {
        let dx = cx.rng().next_int_bounded(17) - 8;
        let dz = cx.rng().next_int_bounded(17) - 8;
        let mut p = origin.offset(dx, 6, dz);
        // `moveToPossibleSpawnPosition` (LEGACY_IRON_GOLEM): down to 6 blocks below, onto a solid
        // block with room above.
        let mut prev = cx.level.block(p);
        let mut found = false;
        let mut i = 6;
        while i >= -6 {
            p = p.below();
            let here = cx.level.block(p);
            let above = cx.level.block(p.above());
            if legacy_can_spawn_on(here, above, prev) {
                p = p.above();
                found = true;
                break;
            }
            prev = here;
            i -= 1;
        }
        if !found {
            continue;
        }
        // `checkSpawnRules` and `checkSpawnObstruction`: air for the golem's 4 blocks, on ground.
        let below = cx.level.block(p.below());
        if !kiln_data::block_props::solid_render(below) {
            continue;
        }
        if (0..4).any(|k| !kiln_data::blocks_types::is_air(cx.level.block(p.offset(0, k, 0)))) {
            continue;
        }
        let id = cx.level.next_entity_id();
        let seed = cx.level.fresh_seed();
        let mut g = mob::new(mob::MobKind::IronGolem, id, 0, seed);
        g.set_pos(Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5));
        cx.level.add_entity(g);
        return true;
    }
    false
}

/// `SpawnUtil.Strategy.LEGACY_IRON_GOLEM.canSpawnOn`: a solid, non-glass block, air (or a liquid)
/// above it.
fn legacy_can_spawn_on(floor: u16, above: u16, _prev: u16) -> bool {
    let name = crate::blocks::block_name(floor);
    if matches!(
        name,
        "minecraft:cobweb" | "minecraft:cactus" | "minecraft:glass_pane" | "minecraft:conduit" | "minecraft:ice" | "minecraft:tnt" | "minecraft:glowstone" | "minecraft:beacon" | "minecraft:sea_lantern" | "minecraft:frosted_ice" | "minecraft:tinted_glass" | "minecraft:glass"
    ) || name.ends_with("_glass")
        || name.ends_with("_glass_pane")
        || name.ends_with("_leaves")
    {
        return false;
    }
    (kiln_data::blocks_types::is_air(above) || kiln_data::blocks_types::has_fluid(above)) && (kiln_data::block_logic::is_solid(floor) || name == "minecraft:powder_snow")
}

/// `RingBell`: the bell sounds and the villagers around hear it.
pub fn ring_bell(cx: &mut Cx, pos: BlockPos) {
    let now = cx.time;
    cx.level.emit(Event::Sound { pos: pos.center(), sound: "minecraft:block.bell.use", source: "blocks", volume: 2.0, pitch: 1.0 });
    let area = crate::math::Aabb::of_block(pos).inflate(48.0, 48.0, 48.0);
    for id in cx.level.entities_in(&area, crate::level::EntityFilter::Living, cx.e.id) {
        let Some(o) = cx.level.entity_mut(id) else { continue };
        if !o.is_alive() || !village::closer_to_center_than(pos, o.position(), 32.0) {
            continue;
        }
        if let Some(om) = mob::data_mut(o)
            && let Some(b) = om.brain.as_mut()
            && b.st.mem.is_registered(Mem::HeardBellTime)
        {
            b.st.mem.set(Mem::HeardBellTime, brain::Val::Long(now));
        }
    }
    cx.b.mem.set(Mem::HeardBellTime, brain::Val::Long(now));
}

/// `CelebrateVillagersSurvivedRaid`'s firework (not launched).
pub fn launch_firework(_cx: &mut Cx, _color: i32, _flight: i32) {}

/// `VillagerMakeLove.tryToGiveBirth`: with a free bed, a baby.
pub fn try_to_give_birth(cx: &mut Cx, other: i32) {
    let me = cx.e.id;
    // `takeVacantBed`.
    let center = cx.e.block_position();
    let mut bed = None;
    let candidates = cx.level.poi_in_range(&["minecraft:home"], center, 48, PoiOccupancy::HasSpace);
    for p in candidates {
        let reach = path::create_path(cx.e, cx.m, &*cx.level, p, 1).is_some_and(|q| q.reached);
        if reach && cx.level.poi_take(&["minecraft:home"], p, 1, &|_, q| q == p).is_some() {
            bed = Some(p);
            break;
        }
    }
    let Some(bed) = bed else {
        cx.level.emit(Event::EntityEvent { entity: other, event: 13 });
        cx.level.emit(Event::EntityEvent { entity: me, event: 13 });
        return;
    };
    // `breed`: `getBreedOffspring`.
    let d = cx.e.random.next_double();
    let my_type = state(cx.m).map_or("minecraft:plains", |s| s.villager_type);
    let their_type = cx.level.entity(other).and_then(mob::data).and_then(state).map_or(my_type, |s| s.villager_type);
    let ty = if d < 0.5 {
        type_for_biome(cx.level.biome(cx.e.block_position()).and_then(biome_name).unwrap_or("minecraft:plains"))
    } else if d < 0.75 {
        my_type
    } else {
        their_type
    };
    let id = cx.level.next_entity_id();
    let seed = cx.level.fresh_seed();
    let mut child = mob::new(mob::MobKind::Villager, id, 0, seed);
    if let Some(cm) = mob::data_mut(&mut child)
        && let Some(cst) = state_mut(cm)
    {
        cst.villager_type = ty;
        cst.profession = "minecraft:none";
        cst.finalized = true;
    }
    // `setAge(6000)` for both parents, `setAge(-24000)` for the baby.
    mob::set_age(cx.e, cx.m, 6000);
    if let Some(o) = cx.level.entity_mut(other) {
        let mut om = mob::take(o);
        mob::set_age(o, &mut om, 6000);
        mob::put(o, om);
    }
    {
        let mut cm = mob::take(&mut child);
        mob::set_age(&mut child, &mut cm, -24000);
        // `giveBedToChild`.
        if let Some(b) = cm.brain.as_mut() {
            b.st.mem.set(Mem::Home, village::gpos(bed));
        }
        mob::put(&mut child, cm);
    }
    child.set_pos(cx.e.position());
    child.y_rot = 0.0;
    child.x_rot = 0.0;
    cx.level.add_entity(child);
    cx.level.emit(Event::EntityEvent { entity: id, event: 12 });
}

impl Kind for Villager {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        // `AbstractVillager`: fire maluses; `Villager`: opens doors, floats, picks up loot.
        m.maluses.push((path::PathType::FireInNeighbor, 16.0));
        m.maluses.push((path::PathType::Fire, -1.0));
        m.nav.can_open_doors = true;
        m.nav.can_float = true;
        m.can_pick_up_loot = true;
        Some(Box::new(VillagerState::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(build_brain(m, random))
    }

    /// `Villager.tick` after `super.tick()`: the head shake runs out, the gossip decays daily
    /// (`maybeDecayGossip`); a sleeper's pitch is level.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let now = level.game_time();
        if let Some(st) = state_mut(m) {
            if st.unhappy > 0 {
                st.unhappy -= 1;
            }
            if st.last_gossip_decay == 0 {
                st.last_gossip_decay = now;
            } else if now >= st.last_gossip_decay + 24000 {
                st.gossips.decay();
                st.last_gossip_decay = now;
            }
            if st.sleeping_pos.is_some() {
                e.x_rot = 0.0;
            }
        }
    }

    /// `LivingEntity.tick`: a sleeper whose bed is gone wakes.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(bed) = state(m).and_then(|s| s.sleeping_pos)
            && !is_bed(level.block(bed))
        {
            stop_sleeping(e, m, level, None);
        }
    }

    /// `Villager.customServerAiStep`: the brain, then the trade celebration, the raid check and
    /// the end of a trade without offers.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // A brain built since the last tick reads its schedule (vanilla's `registerBrainGoals`
        // did so when the brain was made, one tick before its first).
        if state(m).is_some_and(|s| s.schedule_pending) {
            if let Some(st) = state_mut(m) {
                st.schedule_pending = false;
            }
            let t = level.game_time();
            if let Some(b) = m.brain.as_mut() {
                b.st.update_activity_from_schedule(t - 1, &*level);
            }
        }
        brain::tick_brain(e, m, level);
        if state(m).is_some_and(|s| s.refresh_brain) {
            if let Some(st) = state_mut(m) {
                st.refresh_brain = false;
            }
            refresh_brain(e, m, level);
        }
        if let Some(st) = state_mut(m)
            && st.last_traded_player.take().is_some()
        {
            level.emit(Event::EntityEvent { entity: e.id, event: 14 });
        }
        // The raid check draws even outside raids.
        if !m.no_ai && e.random.next_int_bounded(100) == 0 && level.raid_at(e.block_position()).is_some_and(|r| r.active && !r.over) {
            level.emit(Event::EntityEvent { entity: e.id, event: 42 });
        }
        if let Some(st) = state_mut(m)
            && st.offers.as_ref().is_none_or(Vec::is_empty)
            && st.trading_player.is_some()
        {
            st.trading_player = None;
        }
    }

    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        // `LivingEntity.hurtServer`: a sleeper wakes when hurt.
        if is_sleeping(m) && !m.is_dead_or_dying() {
            stop_sleeping(e, m, level, None);
        }
        None
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        // `setLastHurtByMob`: the attacker is remembered (`VILLAGER_HURT`), and a player makes
        // the villager angry.
        if hurt
            && let Some(u) = source.attacker.and_then(|a| uuid_of(level, a))
        {
            reputation_event(m, crate::mob::gossip::ReputationEvent::VillagerHurt, u);
        }
        if hurt && source.attacker_is_player && mob::is_alive(e, m) {
            level.emit(Event::EntityEvent { entity: e.id, event: 13 });
        }
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        stop_trading(m);
        tell_witnesses(e, level, source);
        release_all_pois(e, m, level);
        if is_sleeping(m) {
            stop_sleeping(e, m, level, None);
        }
    }

    fn age_boundary_reached(&self, e: &mut Entity, m: &mut MobData) {
        rebuild_brain(e, m);
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        // `finalizeVillagerType`: the biome's type unless already finalized.
        let biome = ctx.biome.and_then(biome_name);
        if let Some(st) = state_mut(m)
            && !st.finalized
        {
            st.villager_type = type_for_biome(biome.unwrap_or("minecraft:plains"));
            st.finalized = true;
        }
        // `AbstractVillager.finalizeSpawn`: `AgeableMobGroupData(false)`, never a baby.
        ext::mob_finalize(m, r);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let data = r.get("VillagerData");
        let finalized = r.bool_or("VillagerDataFinalized", false);
        let food = r.byte_or("FoodLevel", 0);
        let xp = r.int_or("Xp", 0);
        let last_restock = r.num("LastRestock").map_or(0, |v| v as i64);
        let last_decay = r.num("LastGossipDecay").map_or(0, |v| v as i64);
        let restocks = r.int_or("RestocksToday", 0);
        let offers = r.get("Offers").map(trading::offers_from_nbt);
        let gossips = r.get("Gossips").map(crate::mob::gossip::Gossips::load).unwrap_or_default();
        let inventory = r.get("Inventory").and_then(Tag::as_list).map(|l| l.iter().filter_map(|t| ItemStack::from_nbt(t).ok()).collect::<Vec<_>>());
        let sleeping = r.get("sleeping_pos");
        let Some(st) = state_mut(m) else { return };
        st.gossips = gossips;
        st.offers = offers;
        if finalized || data.is_some() {
            st.finalized = true;
            let mut d = VillagerState::default();
            if let Some(Tag::Compound(_)) = data {
                let get = |k: &str| data.and_then(|t| t.get(k));
                d.villager_type = get("type").and_then(Tag::as_str).and_then(|n| registry_name("minecraft:villager_type", n)).unwrap_or("minecraft:plains");
                d.profession = get("profession").and_then(Tag::as_str).and_then(|n| registry_name("minecraft:villager_profession", n)).unwrap_or("minecraft:none");
                d.level = get("level").and_then(Tag::as_f64).map_or(1, |v| (v as i32).max(1));
            }
            // A direct set: the offers stay.
            st.villager_type = d.villager_type;
            st.profession = d.profession;
            st.level = d.level;
        }
        st.food_level = food;
        st.xp = xp;
        st.last_restock = last_restock;
        st.last_gossip_decay = last_decay;
        st.restocks_today = restocks;
        st.inventory = (0..8).map(|_| ItemStack::empty()).collect();
        for s in inventory.into_iter().flatten() {
            add_item(st, s);
        }
        if let Some(Tag::IntArray(p)) = sleeping
            && p.len() == 3
        {
            st.sleeping_pos = Some(BlockPos::new(p[0], p[1], p[2]));
        }
        // `readAdditionalSaveData` ends with `refreshBrain`: the profession's activities.
        rebuild_brain(e, m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if let Some(offers) = &st.offers {
            o.put("Offers", trading::offers_to_nbt(offers));
        }
        o.put(
            "VillagerData",
            Tag::Compound(vec![
                ("level".into(), Tag::Int(st.level)),
                ("profession".into(), Tag::String(st.profession.into())),
                ("type".into(), Tag::String(st.villager_type.into())),
            ]),
        );
        o.put("VillagerDataFinalized", Tag::Byte(st.finalized as i8));
        o.put("FoodLevel", Tag::Byte(st.food_level));
        o.put("Xp", Tag::Int(st.xp));
        o.put("LastRestock", Tag::Long(st.last_restock));
        o.put("Gossips", st.gossips.save());
        o.put("LastGossipDecay", Tag::Long(st.last_gossip_decay));
        o.put("RestocksToday", Tag::Int(st.restocks_today));
        o.put("Inventory", Tag::List(st.inventory.iter().filter(|s| !s.is_empty()).map(ItemStack::to_nbt).collect()));
        if let Some(p) = st.sleeping_pos {
            o.put("sleeping_pos", Tag::IntArray(vec![p.x, p.y, p.z]));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let Some(st) = state(m) else { return };
        d.set(data::abstract_villager::UNHAPPY_COUNTER, &DataValue::Int(st.unhappy));
        let kind = kiln_data::builtin_id("minecraft:villager_type", st.villager_type).unwrap_or(2);
        let profession = kiln_data::builtin_id("minecraft:villager_profession", st.profession).unwrap_or(0);
        d.set(data::villager::VILLAGER_DATA, &DataValue::VillagerData { kind, profession, level: st.level });
        d.set(data::villager::VILLAGER_DATA_FINALIZED, &DataValue::Boolean(st.finalized));
        if let Some(p) = st.sleeping_pos {
            d.set(data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::SLEEPING));
            d.set(data::living_entity::SLEEPING_POS, &DataValue::OptionalBlockPos(Some([p.x, p.y, p.z])));
        }
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let spawn_egg = !stack.is_empty() && mob::item_name(stack) == "minecraft:villager_spawn_egg";
        let trading = state(m).is_some_and(|s| s.trading_player.is_some());
        if spawn_egg || !mob::is_alive(e, m) || trading {
            return None;
        }
        let done = Outcome { success: true, held: HeldChange::None, shear: None, player_sound: None, ride: false };
        if m.baby() {
            set_unhappy(e, m, level);
            return Some(done);
        }
        // Kiln's click carries no hand: treated as the main hand (the client tries it first).
        let no_offers = offers(e, m, level).is_empty();
        if no_offers {
            set_unhappy(e, m, level);
            return Some(done);
        }
        // `startTrading`: special prices for the player, then the screen.
        let player = level.player(who.id);
        if let Some(st) = state_mut(m) {
            if let Some(p) = player {
                update_special_prices(st, p.uuid, p.hero_of_the_village);
            }
            st.trading_player = Some(who.id);
            st.open_for = Some(who.id);
        }
        Some(done)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        // `LivingEntity.SLEEPING_DIMENSIONS`, then the baby's.
        if is_sleeping(m) {
            (0.2, 0.2, 0.2)
        } else if m.baby() {
            (0.49, 0.98, 0.63)
        } else {
            base
        }
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        // `PathfinderMob.getWalkTargetValue`.
        Some(0.0)
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }
}

/// `Villager.releaseAllPois`.
fn release_all_pois(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let profession = state(m).map_or("minecraft:none", |s| s.profession);
    let Some(b) = m.brain.as_ref() else { return };
    for mem in [Mem::JobSite, Mem::PotentialJobSite, Mem::Home, Mem::MeetingPoint] {
        let Some(pos) = b.st.mem.global_pos(mem).map(|g| g.pos) else { continue };
        let Some(ty) = level.poi_type(pos) else { continue };
        let ok = match mem {
            Mem::Home => ty == "minecraft:home",
            Mem::JobSite => held_job_site(profession, ty),
            Mem::PotentialJobSite => acquirable_job_site("minecraft:none", ty),
            _ => ty == "minecraft:meeting",
        };
        if ok {
            level.poi_release(pos);
        }
    }
    let _ = e;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biome_types() {
        assert_eq!(type_for_biome("minecraft:desert"), "minecraft:desert");
        assert_eq!(type_for_biome("minecraft:snowy_plains"), "minecraft:snow");
        assert_eq!(type_for_biome("minecraft:forest"), "minecraft:plains");
        assert_eq!(type_for_biome("minecraft:mangrove_swamp"), "minecraft:swamp");
    }

    #[test]
    fn trade_sets_by_level() {
        let mut st = VillagerState { profession: "minecraft:librarian", level: 3, ..VillagerState::default() };
        assert_eq!(st.trade_set().as_deref(), Some("minecraft:librarian/level_3"));
        st.profession = "minecraft:nitwit";
        assert_eq!(st.trade_set(), None);
        assert_eq!(max_xp_per_level(1), 10);
        assert_eq!(max_xp_per_level(4), 250);
        assert_eq!(max_xp_per_level(5), 0);
    }

    #[test]
    fn schedule_follows_the_timeline() {
        struct L(i64);
        impl EntityLevel for L {
            fn block(&self, _: BlockPos) -> u16 {
                0
            }
            fn set_block(&mut self, _: BlockPos, _: u16, _: u32) -> bool {
                false
            }
            fn random(&mut self) -> &mut kiln_javamath::random::LegacyRandom {
                unimplemented!()
            }
            fn game_time(&self) -> i64 {
                0
            }
            fn min_y(&self) -> i32 {
                0
            }
            fn entities_in(&self, _: &crate::math::Aabb, _: crate::level::EntityFilter, _: i32) -> Vec<i32> {
                Vec::new()
            }
            fn entity_mut(&mut self, _: i32) -> Option<&mut Entity> {
                None
            }
            fn entity(&self, _: i32) -> Option<&Entity> {
                None
            }
            fn add_entity(&mut self, _: Entity) {}
            fn next_entity_id(&mut self) -> i32 {
                0
            }
            fn fresh_seed(&mut self) -> i64 {
                0
            }
            fn emit(&mut self, _: Event) {}
            fn day_time(&self) -> i64 {
                self.0
            }
        }
        for (t, want) in [(0, Activity::Rest), (9, Activity::Rest), (10, Activity::Idle), (2000, Activity::Work), (9000, Activity::Meet), (11000, Activity::Idle), (12000, Activity::Rest), (23999, Activity::Rest), (24010, Activity::Idle)] {
            assert_eq!(villager_schedule(&L(t)), want, "at {t}");
        }
        assert_eq!(baby_villager_schedule(&L(3000)), Activity::Play);
        assert_eq!(baby_villager_schedule(&L(7000)), Activity::Idle);
        assert_eq!(baby_villager_schedule(&L(13000)), Activity::Rest);
    }
}
