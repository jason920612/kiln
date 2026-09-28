//! Villager: type, profession, level and trading.
//!
//! Vanilla drives villagers with a `Brain` (sensors, schedules, points of interest, memories).
//! Kiln approximates it with goals: floating, trading with a player (stands still and looks at
//! them), fleeing nearby zombies and illagers, panicking when hurt, strolling, looking at players
//! and around. Job sites, beds, bells, schedules (work, meet, rest), gossip, golem spawning,
//! farming, item pickup and breeding are not simulated: an unemployed villager stays unemployed
//! (professions come from NBT), and offers only restock through NBT.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, TradeMerchant};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, path, random_pos};
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
    }
}

/// `setUnhappy`: 40 ticks of head shaking and the "no" sound.
fn set_unhappy(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(st) = state_mut(m) {
        st.unhappy = 40;
    }
    mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.villager.no"));
}

/// Hostiles villagers run from (`VillagerHostilesSensor.ACCEPTABLE_DISTANCE_FROM_HOSTILES`).
const HOSTILES: &[(&str, f64)] = &[
    ("minecraft:drowned", 8.0),
    ("minecraft:evoker", 12.0),
    ("minecraft:husk", 8.0),
    ("minecraft:illusioner", 12.0),
    ("minecraft:pillager", 15.0),
    ("minecraft:ravager", 12.0),
    ("minecraft:vex", 8.0),
    ("minecraft:vindicator", 10.0),
    ("minecraft:zoglin", 10.0),
    ("minecraft:zombie", 8.0),
    ("minecraft:zombie_villager", 8.0),
];

/// Kiln's stand-in for the brain's trading behaviour (`LookAndFollowTradingPlayerSink`): while
/// a player trades, the villager stops walking. Named after the wandering trader's goal.
#[derive(Clone, Debug)]
struct TradeWithPlayerGoal;

fn trading_player(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<goals::Living> {
    let p = goals::living(level, state(m)?.trading_player?)?;
    (p.alive && e.position().distance_to_sqr(p.pos) <= 16.0 * 16.0).then_some(p)
}

impl CustomGoal for TradeWithPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TradeWithPlayerGoal"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        mob::is_alive(e, m) && !e.is_in_water() && e.on_ground && trading_player(e, m, level).is_some()
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// Faces the trading player (`LookAtTradingPlayerGoal`).
#[derive(Clone, Debug)]
struct LookAtTradingPlayerGoal;

impl CustomGoal for LookAtTradingPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtTradingPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        trading_player(e, m, level).is_some()
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = trading_player(e, m, level) {
            m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, 10.0, m.kind.max_head_x_rot() as f32);
        }
    }
}

/// Runs from the nearest hostile within its distance (the brain's `VillagerPanicTrigger` with
/// `SetWalkTargetAwayFrom`), at 0.75.
#[derive(Clone, Debug)]
struct AvoidEntityGoal {
    from: Option<i32>,
    to: Vec3,
}

impl AvoidEntityGoal {
    fn nearest_hostile(e: &Entity, level: &dyn EntityLevel) -> Option<(i32, Vec3)> {
        let p = e.position();
        let area = e.bounding_box().inflate(15.0, 4.0, 15.0);
        let mut best: Option<(f64, i32, Vec3)> = None;
        for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
            let Some(o) = level.entity(id) else { continue };
            let Some(&(_, r)) = HOSTILES.iter().find(|(t, _)| *t == o.type_name) else { continue };
            if !o.is_alive() {
                continue;
            }
            let d = o.position().distance_to_sqr(p);
            if d <= r * r && best.is_none_or(|b| d < b.0) {
                best = Some((d, id, o.position()));
            }
        }
        best.map(|(_, id, pos)| (id, pos))
    }
}

impl CustomGoal for AvoidEntityGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "AvoidEntityGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some((id, from)) = Self::nearest_hostile(e, level) else { return false };
        let Some(to) = random_pos::land_pos_away(e, m, level, 16, 7, from) else { return false };
        if from.distance_to_sqr(to) < from.distance_to_sqr(e.position()) {
            return false;
        }
        self.from = Some(id);
        self.to = to;
        true
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.to.x, self.to.y, self.to.z, 0.75);
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.from = None;
    }
}

impl Kind for Villager {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        // `AbstractVillager`: fire maluses; `Villager`: picks up loot, floats, opens doors.
        m.maluses.push((path::PathType::FireInNeighbor, 16.0));
        m.maluses.push((path::PathType::Fire, -1.0));
        m.can_pick_up_loot = true;
        Some(Box::new(VillagerState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Custom(Box::new(TradeWithPlayerGoal)));
        g.add(1, Goal::Custom(Box::new(AvoidEntityGoal { from: None, to: Vec3::ZERO })));
        g.add(1, Goal::Custom(Box::new(LookAtTradingPlayerGoal)));
        g.add(1, Goal::Panic { speed: 0.75, pos: Vec3::ZERO });
        g.add(6, Goal::RandomStroll { speed: 0.5, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = e;
        let now = level.game_time();
        if let Some(st) = state_mut(m) {
            // `Villager.tick`: the head shake runs out; gossip decays daily (not simulated, but
            // the clock is kept).
            if st.unhappy > 0 {
                st.unhappy -= 1;
            }
            if st.last_gossip_decay == 0 || now >= st.last_gossip_decay + 24000 {
                st.last_gossip_decay = now;
            }
        }
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // The brain runs here in vanilla (Kiln's goals ran already).
        if let Some(st) = state_mut(m)
            && st.last_traded_player.take().is_some()
        {
            level.emit(Event::EntityEvent { entity: e.id, event: 14 });
        }
        // The raid check draws even outside raids.
        if !m.no_ai {
            let _ = e.random.next_int_bounded(100);
        }
        if let Some(st) = state_mut(m)
            && st.offers.as_ref().is_none_or(Vec::is_empty)
            && st.trading_player.is_some()
        {
            st.trading_player = None;
        }
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        // `setLastHurtByMob`: angry particles when a player hits the villager.
        if hurt && source.attacker_is_player && mob::is_alive(e, m) {
            level.emit(Event::EntityEvent { entity: e.id, event: 13 });
        }
    }

    fn die(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource) {
        stop_trading(m);
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

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let data = r.get("VillagerData");
        let finalized = r.bool_or("VillagerDataFinalized", false);
        let food = r.byte_or("FoodLevel", 0);
        let xp = r.int_or("Xp", 0);
        let last_restock = r.num("LastRestock").map_or(0, |v| v as i64);
        let last_decay = r.num("LastGossipDecay").map_or(0, |v| v as i64);
        let restocks = r.int_or("RestocksToday", 0);
        let offers = r.get("Offers").map(trading::offers_from_nbt);
        let Some(st) = state_mut(m) else { return };
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
        o.put("LastGossipDecay", Tag::Long(st.last_gossip_decay));
        o.put("RestocksToday", Tag::Int(st.restocks_today));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let Some(st) = state(m) else { return };
        d.set(data::abstract_villager::UNHAPPY_COUNTER, &DataValue::Int(st.unhappy));
        let kind = kiln_data::builtin_id("minecraft:villager_type", st.villager_type).unwrap_or(2);
        let profession = kiln_data::builtin_id("minecraft:villager_profession", st.profession).unwrap_or(0);
        d.set(data::villager::VILLAGER_DATA, &DataValue::VillagerData { kind, profession, level: st.level });
        d.set(data::villager::VILLAGER_DATA_FINALIZED, &DataValue::Boolean(st.finalized));
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
        // `startTrading`: special prices from gossip and Hero of the Village are not simulated.
        if let Some(st) = state_mut(m) {
            st.trading_player = Some(who.id);
            st.open_for = Some(who.id);
        }
        Some(done)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.49, 0.98, 0.63) } else { base }
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        // `PathfinderMob.getWalkTargetValue`.
        Some(0.0)
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }
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
}
