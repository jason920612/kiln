//! Wandering traders (`WanderingTrader`): a merchant that walks to the place the spawner chose
//! (a village bell, or where the player stood), trades the vanilla trade sets
//! (`wandering_trader/buying`, `uncommon` and `common`), drinks an invisibility potion at night
//! and milk by day, runs from the monsters that hunt villagers, and leaves after its despawn
//! delay (48000 ticks, not counting the ticks of a trade). Its two trader llamas follow it on
//! leads (see [`super::llama`]) and live as long as it does.
//!
//! The trade screen and the offers' bookkeeping are the simulation's (`kiln-sim`'s trading), as
//! for villagers; this module holds the merchant's state and what the trader itself does.

use super::common_a::{Avoid, AvoidEntityGoal};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, TradeMerchant};
use crate::math::{BlockPos, Vec3};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, Category, GroupData, MobData, SpawnContext, effects, path, random_pos};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_item::trading::MerchantOffer;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct WanderingTrader;

pub static KIND: WanderingTrader = WanderingTrader;

static INFO: Info = Info { category: Category::Creature, ageable: true, ..Info::misc("minecraft:wandering_trader", &[]) };

/// The trade sets `updateTrades` adds, in order.
pub const TRADE_SETS: [&str; 3] = ["minecraft:wandering_trader/buying", "minecraft:wandering_trader/uncommon", "minecraft:wandering_trader/common"];

/// `WanderingTrader`'s and `AbstractVillager`'s state.
#[derive(Clone, Debug)]
pub struct State {
    /// `wanderTarget`: where it walks to.
    pub wander_target: Option<BlockPos>,
    /// `despawnDelay`: ticks left (0: stays).
    pub despawn_delay: i32,
    /// `offers`: `None` until first generated (on the first click).
    pub offers: Option<Vec<MerchantOffer>>,
    /// `tradingPlayer` (entity id).
    pub trading_player: Option<i32>,
    /// Set by a click that starts trading: the simulation opens the merchant screen for it.
    pub open_for: Option<i32>,
    /// `AbstractVillager.DATA_UNHAPPY_COUNTER`.
    pub unhappy: i32,
    /// `InventoryCarrier.inventory` (8 slots; kept as it was loaded).
    pub inventory: Vec<ItemStack>,
}

impl State {
    fn new() -> State {
        State { wander_target: None, despawn_delay: 0, offers: None, trading_player: None, open_for: None, unhappy: 0, inventory: Vec::new() }
    }
}

pub fn state(m: &MobData) -> Option<&State> {
    ext::state::<State>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut State> {
    ext::state_mut::<State>(m)
}

/// `WanderingTrader.getDespawnDelay`.
pub fn despawn_delay(m: &MobData) -> i32 {
    state(m).map_or(0, |s| s.despawn_delay)
}

/// `WanderingTrader.setDespawnDelay`.
pub fn set_despawn_delay(m: &mut MobData, delay: i32) {
    if let Some(s) = state_mut(m) {
        s.despawn_delay = delay;
    }
}

/// `WanderingTrader.setWanderTarget`.
pub fn set_wander_target(m: &mut MobData, target: Option<BlockPos>) {
    if let Some(s) = state_mut(m) {
        s.wander_target = target;
    }
}

/// `Mob.setHomeTo` (the radius is 16 for a spawned trader).
pub fn set_home_to(m: &mut MobData, pos: BlockPos, radius: i32) {
    m.home = Some((pos, radius));
}

/// `AbstractVillager.getOffers`: the trade sets' offers, rolled on first access.
pub fn offers<'a>(e: &Entity, m: &'a mut MobData, level: &mut dyn EntityLevel) -> &'a [MerchantOffer] {
    if state(m).is_some_and(|s| s.offers.is_none()) {
        let merchant = TradeMerchant { entity: e.id, pos: e.position(), villager_type: "minecraft:plains", entity_type: "minecraft:wandering_trader" };
        let mut all = Vec::new();
        for set in TRADE_SETS {
            all.extend(level.trade_offers(set, &merchant));
        }
        if let Some(s) = state_mut(m) {
            s.offers = Some(all);
        }
    }
    state(m).and_then(|s| s.offers.as_deref()).unwrap_or(&[])
}

/// `AbstractVillager.notifyTrade` + `WanderingTrader.rewardTradeXp`: offer `index` was used.
pub fn notify_trade(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, index: usize) {
    let interval = m.kind.ambient_sound_interval();
    let Some(st) = state_mut(m) else { return };
    let Some(offer) = st.offers.as_mut().and_then(|o| o.get_mut(index)) else { return };
    offer.increase_uses();
    let reward = offer.reward_exp;
    m.ambient_sound_time = -interval;
    if reward {
        // `rewardTradeXp`: 3 + nextInt(4) experience a half block up.
        let n = 3 + e.random.next_int_bounded(4);
        let p = e.position();
        mob::award_experience(level, Vec3::new(p.x, p.y + 0.5, p.z), n);
    }
}

/// `AbstractVillager.notifyTradeUpdated`: the yes or no of a trade that was set up.
pub fn notify_trade_updated(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, has_result: bool) {
    let interval = m.kind.ambient_sound_interval();
    if m.ambient_sound_time > -interval + 20 {
        m.ambient_sound_time = -interval;
        let s = if has_result { "minecraft:entity.wandering_trader.yes" } else { "minecraft:entity.wandering_trader.no" };
        mob::make_sound(e, m, level, mob::sound_event(s));
    }
}

/// `AbstractVillager.stopTrading` (the merchant screen closed).
pub fn stop_trading(m: &mut MobData) {
    if let Some(st) = state_mut(m) {
        st.trading_player = None;
    }
}

/// Runs `f` on a wandering trader's entity and mob data (outside the mob tick).
pub fn with_trader<R>(e: &mut Entity, f: impl FnOnce(&mut Entity, &mut MobData) -> R) -> Option<R> {
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

/// `Level.isDarkOutside`.
fn is_dark_outside(level: &dyn EntityLevel) -> bool {
    !level.is_bright_outside()
}

impl Kind for WanderingTrader {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::new()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        // Invisible at night, back in the open by day.
        g.add(
            0,
            Goal::Custom(Box::new(UseItemGoal {
                potion: true,
                sound: "minecraft:entity.wandering_trader.disappeared",
            })),
        );
        g.add(
            0,
            Goal::Custom(Box::new(UseItemGoal {
                potion: false,
                sound: "minecraft:entity.wandering_trader.reappeared",
            })),
        );
        g.add(1, Goal::Custom(Box::new(TradeWithPlayerGoal)));
        for (types, dist) in [
            (&["minecraft:zombie", "minecraft:husk", "minecraft:drowned", "minecraft:zombie_villager", "minecraft:zombified_piglin"][..], 8.0f32),
            (&["minecraft:evoker"][..], 12.0),
            (&["minecraft:vindicator"][..], 8.0),
            (&["minecraft:vex"][..], 8.0),
            (&["minecraft:pillager"][..], 15.0),
            (&["minecraft:illusioner"][..], 12.0),
            (&["minecraft:zoglin"][..], 10.0),
        ] {
            g.add(1, Goal::Custom(Box::new(AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Types(types), dist, 0.5, 0.5))));
        }
        g.add(1, Goal::Panic { speed: 0.5, pos: Vec3::ZERO });
        g.add(1, Goal::Custom(Box::new(LookAtTradingPlayerGoal::new())));
        g.add(2, Goal::Custom(Box::new(WanderToPositionGoal { stop_distance: 2.0, speed: 0.35 })));
        g.add(4, Goal::Custom(Box::new(MoveTowardsRestrictionGoal { speed: 0.35, wanted: Vec3::ZERO })));
        g.add(8, Goal::RandomStroll { speed: 0.35, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::Custom(Box::new(InteractGoal::new())));
        g.add(10, Goal::Custom(Box::new(super::raider::LookAtMobGoal::new(8.0))));
    }

    /// `WanderingTrader.aiStep`: the despawn countdown (paused by a trade).
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let Some(st) = state_mut(m) else { return };
        if st.despawn_delay > 0 && st.trading_player.is_none() {
            st.despawn_delay -= 1;
            if st.despawn_delay == 0 {
                e.discard();
            }
        }
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    /// `AbstractVillager.finalizeSpawn`: `AgeableMobGroupData(false)`, never a baby.
    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        ext::mob_finalize(m, r);
    }

    fn die(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &mob::DamageSource) {
        stop_trading(m);
    }

    fn is_food(&self, _item: i32) -> bool {
        false
    }

    /// `WanderingTrader.mobInteract`: a click starts a trade unless the trader is busy, a baby,
    /// or has nothing to sell (the click is taken, nothing else happens).
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let spawn_egg = !stack.is_empty() && mob::item_name(stack) == "minecraft:villager_spawn_egg";
        let trading = state(m).is_some_and(|s| s.trading_player.is_some());
        if spawn_egg || !mob::is_alive(e, m) || trading || m.baby() {
            return None;
        }
        let done = Outcome { success: true, held: HeldChange::None, shear: None, player_sound: None, ride: false, open_container: false, sheared: None };
        if offers(e, m, level).is_empty() {
            return Some(done);
        }
        if let Some(st) = state_mut(m) {
            st.trading_player = Some(who.id);
            st.open_for = Some(who.id);
        }
        Some(done)
    }

    /// `WanderingTrader.getAmbientSound`: the trading sound while trading.
    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if state(m).is_some_and(|s| s.trading_player.is_some()) {
            return Some(Some("minecraft:entity.wandering_trader.trade"));
        }
        None
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        // `PathfinderMob.getWalkTargetValue`.
        Some(0.0)
    }

    fn update_using_item(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        drink_tick(e, m, level);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let delay = r.int_or("DespawnDelay", 0);
        let target = match r.get("wander_target") {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        };
        let offers = r.get("Offers").map(kiln_item::trading::offers_from_nbt);
        let inventory = r.get("Inventory").and_then(Tag::as_list).map(|l| l.iter().filter_map(|t| ItemStack::from_nbt(t).ok()).collect::<Vec<_>>());
        if let Some(st) = state_mut(m) {
            st.despawn_delay = delay;
            st.wander_target = target;
            st.offers = offers;
            st.inventory = inventory.unwrap_or_default();
        }
        // `readAdditionalSaveData` ends with `setAge(max(0, getAge()))`: never a baby.
        if m.age < 0 {
            m.age = 0;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if let Some(offers) = &st.offers {
            o.put("Offers", kiln_item::trading::offers_to_nbt(offers));
        }
        o.put("Inventory", Tag::List(st.inventory.iter().filter(|s| !s.is_empty()).map(ItemStack::to_nbt).collect()));
        o.put("DespawnDelay", Tag::Int(st.despawn_delay));
        if let Some(p) = st.wander_target {
            o.put("wander_target", Tag::IntArray(vec![p.x, p.y, p.z]));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let Some(st) = state(m) else { return };
        d.set(data::abstract_villager::UNHAPPY_COUNTER, &DataValue::Int(st.unhappy));
        // `LivingEntity.DATA_LIVING_ENTITY_FLAGS`: using an item (its hand is the main one).
        if m.using_item.is_some() {
            d.set(data::living_entity::LIVING_ENTITY_FLAGS, &DataValue::Byte(1));
        }
    }
}

// ----------------------------------------------------------------------- drinking

/// The ticks a potion or a bucket of milk takes to drink (`Consumable.consumeTicks`).
const DRINK_TICKS: i32 = 32;

/// `UseItemGoal`: drinks `item` (an invisibility potion or milk) when its condition holds, until
/// it is finished (the item is gone then, and the finish sound plays).
#[derive(Clone, Debug)]
struct UseItemGoal {
    /// The invisibility potion (dark outside, visible) or the milk bucket (bright outside, invisible).
    potion: bool,
    sound: &'static str,
}

impl UseItemGoal {
    fn stack(&self) -> ItemStack {
        if self.potion {
            let mut s = ItemStack::of("minecraft:potion", 1).unwrap_or_else(ItemStack::empty);
            s.insert(
                kiln_item::keys::POTION_CONTENTS,
                kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id("minecraft:invisibility"), ..Default::default() },
            );
            s
        } else {
            ItemStack::of("minecraft:milk_bucket", 1).unwrap_or_else(ItemStack::empty)
        }
    }
}

impl CustomGoal for UseItemGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "UseItemGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let invisible = effects::invisible(m);
        if self.potion { is_dark_outside(level) && !invisible } else { level.is_bright_outside() && invisible }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        m.using_item.is_some()
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.equipment[mob::MAINHAND] = self.stack();
        m.start_using_item();
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.equipment[mob::MAINHAND] = ItemStack::empty();
        if !e.silent {
            let pitch = e.random.next_float() * 0.2 + 0.9;
            level.emit(Event::Sound { pos: e.position(), sound: self.sound, source: "neutral", volume: 1.0, pitch });
        }
    }
}

/// `Consumable.emitParticlesAndSounds` of a drink: the draws, and the drinking sound.
fn emit_drink(e: &mut Entity, level: &mut dyn EntityLevel, milk: bool) {
    let _volume_roll = e.random.next_bool();
    let _triangle = {
        let a = e.random.next_float();
        let b = e.random.next_float();
        1.0 + (a - b) * 0.2
    };
    let pitch = {
        let f = e.random.next_float();
        0.9 + f * (1.0 - 0.9)
    };
    if !e.silent {
        let sound = if milk { "minecraft:entity.wandering_trader.drink_milk" } else { "minecraft:entity.wandering_trader.drink_potion" };
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 0.5, pitch });
    }
}

/// `LivingEntity.updatingUsingItem` for the trader's drink: every fourth tick after the first
/// seven the drinking sounds; at the last the item takes effect.
fn drink_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let held = m.equipment[mob::MAINHAND].clone();
    if held.is_empty() {
        m.stop_using_item();
        return;
    }
    let milk = mob::item_name(&held) == "minecraft:milk_bucket";
    let remaining = DRINK_TICKS - m.ticks_using_item();
    // `Consumable.shouldEmitParticlesAndSounds`.
    let elapsed = DRINK_TICKS - remaining;
    if elapsed > (DRINK_TICKS as f32 * 0.21875f32) as i32 && remaining % 4 == 0 {
        emit_drink(e, level, milk);
    }
    if remaining - 1 == 0 {
        // `completeUsingItem` → `Consumable.onConsume`.
        emit_drink(e, level, milk);
        if milk {
            effects::remove_all(m);
            m.equipment[mob::MAINHAND] = ItemStack::of("minecraft:bucket", 1).unwrap_or_else(ItemStack::empty);
        } else {
            if let Some(fx) = crate::effect::Effect::named("minecraft:invisibility", 3600, 0) {
                effects::add(e, m, level, fx, Some(e.id));
            }
            m.equipment[mob::MAINHAND] = ItemStack::of("minecraft:glass_bottle", 1).unwrap_or_else(ItemStack::empty);
        }
        level.emit(Event::GameEvent { event: "minecraft:drink", pos: e.position(), entity: Some(e.id) });
        m.stop_using_item();
        // (The counter does not tick on: `stopUsingItem` ended it.)
        m.using_item = None;
    }
}

// ----------------------------------------------------------------------- goals

/// `TradeWithPlayerGoal`: stands still for the player it trades with.
#[derive(Clone, Debug)]
struct TradeWithPlayerGoal;

impl CustomGoal for TradeWithPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TradeWithPlayerGoal"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !mob::is_alive(e, m) || e.is_in_water() || !e.on_ground || m.hurt_time > 0 {
            return false;
        }
        let Some(p) = state(m).and_then(|s| s.trading_player).and_then(|id| level.player(id)) else { return false };
        e.position().distance_to_sqr(p.pos) <= 16.0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.nav_mut().stop();
        let _ = (e, level);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        stop_trading(m);
    }
}

/// `LookAtTradingPlayerGoal`: a `LookAtPlayerGoal(8)` that looks at the player it trades with.
#[derive(Clone, Debug)]
struct LookAtTradingPlayerGoal {
    inner: Goal,
}

impl LookAtTradingPlayerGoal {
    fn new() -> LookAtTradingPlayerGoal {
        LookAtTradingPlayerGoal { inner: Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 } }
    }
}

impl CustomGoal for LookAtTradingPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtTradingPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if let Some(p) = state(m).and_then(|s| s.trading_player) {
            if let Goal::LookAtPlayer { look_at, .. } = &mut self.inner {
                *look_at = Some(p);
            }
            return true;
        }
        false
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

/// `InteractGoal(this, Player.class, 3.0F, 1.0F)`: a `LookAtPlayerGoal` that also holds the
/// mob still (MOVE) while it looks at a player within 3 blocks.
#[derive(Clone, Debug)]
struct InteractGoal {
    inner: Goal,
}

impl InteractGoal {
    fn new() -> InteractGoal {
        InteractGoal { inner: Goal::LookAtPlayer { dist: 3.0, probability: 1.0, look_at: None, look_time: 0 } }
    }
}

impl CustomGoal for InteractGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "InteractGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_use(&mut self.inner, e, m, level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

/// `BlockPos.closerToCenterThan(position, distance)`.
fn closer_to_center_than(pos: BlockPos, at: Vec3, dist: f64) -> bool {
    let (dx, dy, dz) = (pos.x as f64 + 0.5 - at.x, pos.y as f64 + 0.5 - at.y, pos.z as f64 + 0.5 - at.z);
    dx * dx + dy * dy + dz * dz < dist * dist
}

/// `WanderingTrader.WanderToPositionGoal`: walks to the chosen place (in steps of 10 blocks when
/// far), then forgets it.
#[derive(Clone, Debug)]
struct WanderToPositionGoal {
    stop_distance: f64,
    speed: f64,
}

impl WanderToPositionGoal {
    fn too_far(e: &Entity, target: BlockPos, d: f64) -> bool {
        !closer_to_center_than(target, e.position(), d)
    }
}

impl CustomGoal for WanderToPositionGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "WanderToPositionGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        state(m).and_then(|s| s.wander_target).is_some_and(|t| Self::too_far(e, t, self.stop_distance))
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_wander_target(m, None);
        m.nav_mut().stop();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(target) = state(m).and_then(|s| s.wander_target) else { return };
        if !m.nav_ref().is_done() {
            return;
        }
        if Self::too_far(e, target, 10.0) {
            let v = Vec3::new(target.x as f64 - e.x(), target.y as f64 - e.y(), target.z as f64 - e.z()).normalize();
            let to = v.scale(10.0).add(e.x(), e.y(), e.z());
            path::move_to(e, m, level, to.x, to.y, to.z, self.speed);
        } else {
            path::move_to(e, m, level, target.x as f64, target.y as f64, target.z as f64, self.speed);
        }
    }
}

/// `MoveTowardsRestrictionGoal`: back toward the home when outside it.
#[derive(Clone, Debug)]
pub struct MoveTowardsRestrictionGoal {
    pub speed: f64,
    pub wanted: Vec3,
}

impl CustomGoal for MoveTowardsRestrictionGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveTowardsRestrictionGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let home = m.home;
        if random_pos::within_home(home, e.block_position()) {
            return false;
        }
        let Some((c, _)) = home else { return false };
        let to = Vec3::new(c.x as f64 + 0.5, c.y as f64, c.z as f64 + 0.5);
        match random_pos::default_pos_towards_home(e, m, level, 16, 7, to, std::f32::consts::FRAC_PI_2 as f64, home) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav_ref().is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, self.speed);
    }
}
