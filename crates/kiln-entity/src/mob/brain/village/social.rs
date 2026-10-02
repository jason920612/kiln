//! How villagers deal with each other and with players: `InteractWith`, `SetLookAndInteract`,
//! `SocializeAtBell`, `LookAndFollowTradingPlayerSink`, `ShowTradesToPlayer`, `TradeWithVillager`,
//! `VillagerMakeLove`, `PlayTagWithOtherKids`, `GiveGiftToHero`.

use kiln_javamath::random::RandomSource;
use super::closer_to_center_than;
use crate::behavior_boilerplate;
use crate::level::Event;
use crate::mob::brain::behaviors::lock_gaze_and_walk_to_each_other;
use crate::mob::brain::memory::{Tracker, Val, WalkTarget};
use crate::mob::brain::util;
use crate::mob::brain::{Behavior, Control, Cx, Gate, Mem, OrderPolicy, RunningPolicy, Status, Timed, shot};
use crate::mob::kinds::villager;
use crate::mob::random_pos;
use crate::mob::MAINHAND;
use Status::{Registered, ValueAbsent, ValuePresent};

// ---------------------------------------------------------------------------- InteractWith

/// `InteractWith.of(type, maxDistance, selfPredicate, targetPredicate, memory, speed, closeEnough)`:
/// the closest visible entity of `type` (that passes the predicate) becomes the memory's value, the
/// look target, and the walk target.
pub fn interact_with(
    type_name: &'static str,
    max_dist: i32,
    self_pred: fn(&Cx) -> bool,
    target_pred: fn(&Cx, i32) -> bool,
    mem: Mem,
    speed: f32,
    close_enough: i32,
) -> Box<dyn Control> {
    let entry: &'static [(Mem, Status)] = match mem {
        Mem::InteractionTarget => &[
            (Mem::InteractionTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
        ],
        Mem::BreedTarget => &[
            (Mem::BreedTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::WalkTarget, ValueAbsent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
        ],
        _ => panic!("InteractWith over {mem:?}"),
    };
    let max_sqr = (max_dist * max_dist) as f64;
    shot("InteractWith", entry, move |cx| {
        if !self_pred(cx) {
            return false;
        }
        let of_type = |cx: &mut Cx, id: i32| cx.level.entity(id).is_some_and(|o| o.type_name == type_name) && target_pred(cx, id);
        // `contains(type && target predicate)`, then `findClosest(distance && predicate)`.
        if util::find_closest_visible_kind(cx, |k| k == type_name, |cx, id| of_type(cx, id)).is_none() {
            return false;
        }
        let me = cx.e.position();
        let found = util::find_closest_visible_kind(cx, |k| k == type_name, |cx, id| util::living(cx, id).is_some_and(|l| l.pos.distance_to_sqr(me) <= max_sqr) && of_type(cx, id));
        if let Some(id) = found {
            cx.b.mem.set(mem, Val::Entity(id));
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity3(id, false, false), speed, close_enough }));
        }
        true
    })
}

/// `SetLookAndInteract.create(type, maxDist)`: the closest visible entity of `type` within
/// `max_dist` becomes the interaction and look target.
pub fn set_look_and_interact(type_name: &'static str, max_dist: i32) -> Box<dyn Control> {
    let max_sqr = (max_dist * max_dist) as f64;
    shot(
        "SetLookAndInteract",
        &[(Mem::LookTarget, Registered), (Mem::InteractionTarget, ValueAbsent), (Mem::NearestVisibleLivingEntities, ValuePresent)],
        move |cx| {
            let me = cx.e.position();
            // (Only what is of the type is asked: the others fail the predicate anyway.)
            let found = util::find_closest_visible_kind(cx, |k| k == type_name, |cx, id| {
                util::living(cx, id).is_some_and(|l| l.pos.distance_to_sqr(me) <= max_sqr && l.type_name == type_name)
            });
            let Some(id) = found else { return false };
            cx.b.mem.set(Mem::InteractionTarget, Val::Entity(id));
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
            true
        },
    )
}

/// `SocializeAtBell.create()`.
pub fn socialize_at_bell() -> Box<dyn Control> {
    shot(
        "SocializeAtBell",
        &[
            (Mem::WalkTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::MeetingPoint, ValuePresent),
            (Mem::NearestVisibleLivingEntities, ValuePresent),
            (Mem::InteractionTarget, ValueAbsent),
        ],
        |cx| {
            let Some(bell) = super::mem_pos(cx, Mem::MeetingPoint) else { return false };
            if cx.rng().next_int_bounded(100) != 0 {
                return false;
            }
            if !closer_to_center_than(bell, cx.e.position(), 4.0) {
                return false;
            }
            let is_villager = |cx: &mut Cx, id: i32| cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager");
            if util::find_closest_visible(cx, is_villager).is_none() {
                return false;
            }
            let me = cx.e.position();
            let found = util::find_closest_visible(cx, |cx, id| {
                cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager") && util::living(cx, id).is_some_and(|l| l.pos.distance_to_sqr(me) <= 32.0)
            });
            if let Some(id) = found {
                cx.b.mem.set(Mem::InteractionTarget, Val::Entity(id));
                cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
                cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity3(id, false, false), speed: 0.3, close_enough: 1 }));
            }
            true
        },
    )
}

// ---------------------------------------------------------------------------- trading with a player

/// The player trading with the villager, when the trade is still on (`checkExtraStartConditions`
/// of `LookAndFollowTradingPlayerSink`).
fn trading_player_ok(cx: &Cx) -> Option<i32> {
    let id = villager::state(cx.m)?.trading_player?;
    let p = util::living(cx, id)?;
    (crate::mob::is_alive(cx.e, cx.m) && !cx.e.is_in_water() && cx.m.hurt_time <= 0 && cx.e.position().distance_to_sqr(p.pos) <= 16.0).then_some(id)
}

/// `LookAndFollowTradingPlayerSink(speed)`.
#[derive(Clone, Debug)]
pub struct LookAndFollowTradingPlayerSink {
    speed: f32,
}

impl LookAndFollowTradingPlayerSink {
    pub fn new(speed: f32) -> Box<dyn Control> {
        Timed::new(LookAndFollowTradingPlayerSink { speed })
    }

    fn follow(&self, cx: &mut Cx) {
        if let Some(id) = villager::state(cx.m).and_then(|s| s.trading_player) {
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity3(id, false, false), speed: self.speed, close_enough: 2 }));
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
        }
    }
}

impl Behavior for LookAndFollowTradingPlayerSink {
    fn name(&self) -> &'static str {
        "LookAndFollowTradingPlayerSink"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::WalkTarget, Registered), (Mem::LookTarget, Registered)]
    }
    fn duration(&self) -> (i32, i32) {
        (i32::MAX, i32::MAX)
    }
    fn timed_out(&self, _time: i64, _end: i64) -> bool {
        false
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        trading_player_ok(cx).is_some()
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        trading_player_ok(cx).is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        self.follow(cx);
    }
    fn tick(&mut self, cx: &mut Cx) {
        self.follow(cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::LookTarget);
    }
    behavior_boilerplate!();
}

/// `ShowTradesToPlayer(minDuration, maxDuration)`: looks at the player and holds up what it would
/// sell for the item the player holds.
#[derive(Clone, Debug)]
pub struct ShowTradesToPlayer {
    min: i32,
    max: i32,
    /// `playerItemStack` (the item the player held when last looked at).
    player_item: Option<i32>,
    display: Vec<kiln_item::ItemStack>,
    cycle: i32,
    index: usize,
    look_time: i32,
}

impl ShowTradesToPlayer {
    pub fn new(min: i32, max: i32) -> Box<dyn Control> {
        Timed::new(ShowTradesToPlayer { min, max, player_item: None, display: Vec::new(), cycle: 0, index: 0, look_time: 0 })
    }

    fn valid(cx: &Cx) -> bool {
        let Some(id) = cx.b.mem.entity(Mem::InteractionTarget) else { return false };
        let Some(t) = util::living(cx, id) else { return false };
        t.player && crate::mob::is_alive(cx.e, cx.m) && t.alive && !cx.m.baby() && cx.e.position().distance_to_sqr(t.pos) <= 17.0
    }

    fn look_at_target(cx: &mut Cx) -> Option<i32> {
        let id = cx.b.mem.entity(Mem::InteractionTarget)?;
        cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
        Some(id)
    }

    fn display_item(cx: &mut Cx, stack: kiln_item::ItemStack) {
        cx.m.equipment[MAINHAND] = stack;
        cx.m.drop_chances[MAINHAND] = 0.0;
    }

    fn clear_held(cx: &mut Cx) {
        cx.m.equipment[MAINHAND] = kiln_item::ItemStack::empty();
        cx.m.drop_chances[MAINHAND] = 0.085;
    }
}

impl Behavior for ShowTradesToPlayer {
    fn name(&self) -> &'static str {
        "ShowTradesToPlayer"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::InteractionTarget, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (self.min, self.max)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        Self::valid(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        Self::valid(cx) && self.look_time > 0 && cx.b.mem.has(Mem::InteractionTarget)
    }
    fn start(&mut self, cx: &mut Cx) {
        Self::look_at_target(cx);
        self.cycle = 0;
        self.index = 0;
        self.look_time = 40;
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(id) = Self::look_at_target(cx) else { return };
        // `findItemsToDisplay`.
        let held = cx.level.player(id).map_or(0, |p| p.main_hand);
        let mut changed = false;
        if self.player_item != Some(held) {
            self.player_item = Some(held);
            changed = true;
            self.display.clear();
        }
        if changed && held != 0 {
            // `updateDisplayItems`: what it sells for the item (in stock).
            let offers: Vec<kiln_item::trading::MerchantOffer> = villager::offers_now(cx.e, cx.m, cx.level).to_vec();
            for o in offers {
                if !o.is_out_of_stock() && (o.cost_a().item() == held || o.cost_b().item() == held) {
                    self.display.push(o.result.clone());
                }
            }
            if !self.display.is_empty() {
                self.look_time = 900;
                Self::display_item(cx, self.display[0].clone());
            }
        }
        if !self.display.is_empty() {
            // `displayCyclingItems`.
            if self.display.len() >= 2 {
                self.cycle += 1;
                if self.cycle >= 40 {
                    self.index += 1;
                    self.cycle = 0;
                    if self.index > self.display.len() - 1 {
                        self.index = 0;
                    }
                    Self::display_item(cx, self.display[self.index].clone());
                }
            }
        } else {
            Self::clear_held(cx);
            self.look_time = self.look_time.min(40);
        }
        self.look_time -= 1;
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::InteractionTarget);
        Self::clear_held(cx);
        self.player_item = None;
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- villagers together

/// `BehaviorUtils.targetIsValid(brain, memory, VILLAGER)`: the memory's entity is a living,
/// visible villager.
fn target_is_villager(cx: &mut Cx, mem: Mem) -> Option<i32> {
    let id = cx.b.mem.entity(mem)?;
    let ok = cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager" && o.is_alive()) && util::entity_is_visible(cx, id);
    ok.then_some(id)
}

/// `TradeWithVillager`: the two exchange gossip, and what one has too much of and the other wants.
#[derive(Clone, Debug, Default)]
pub struct TradeWithVillager {
    /// What I would give the other one (items it requests and I do not).
    trades: Vec<i32>,
}

impl Behavior for TradeWithVillager {
    fn name(&self) -> &'static str {
        "TradeWithVillager"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::InteractionTarget, ValuePresent), (Mem::NearestVisibleLivingEntities, ValuePresent)]
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        target_is_villager(cx, Mem::InteractionTarget).is_some()
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        target_is_villager(cx, Mem::InteractionTarget).is_some()
    }
    fn start(&mut self, cx: &mut Cx) {
        if let Some(other) = cx.b.mem.entity(Mem::InteractionTarget) {
            lock_gaze_and_walk_to_each_other(cx, other, 0.5, 2);
            self.trades = villager::trades_with(cx, other);
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(other) = cx.b.mem.entity(Mem::InteractionTarget) else { return };
        let Some(o) = util::living(cx, other) else { return };
        if cx.e.position().distance_to_sqr(o.pos) > 5.0 {
            return;
        }
        lock_gaze_and_walk_to_each_other(cx, other, 0.5, 2);
        villager::gossip(cx, other);
        villager::share_food(cx, other, &self.trades);
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::InteractionTarget);
    }
    behavior_boilerplate!();
}

/// The `GateBehavior` (ordered, run one) that holds `TradeWithVillager`.
pub fn trade_with_villager_gate() -> Box<dyn Control> {
    Gate::new("GateBehavior", &[], &[Mem::InteractionTarget], OrderPolicy::Ordered, RunningPolicy::RunOne, vec![(Timed::new(TradeWithVillager::default()), 1)])
}

/// The `GateBehavior` that holds `VillagerMakeLove`.
pub fn make_love_gate() -> Box<dyn Control> {
    Gate::new("GateBehavior", &[], &[Mem::BreedTarget], OrderPolicy::Ordered, RunningPolicy::RunOne, vec![(Timed::new(VillagerMakeLove::default()), 1)])
}

/// `VillagerMakeLove`: two villagers that can breed walk to each other; after 275 to 324 ticks
/// a baby is born if there is a free bed.
#[derive(Clone, Debug, Default)]
pub struct VillagerMakeLove {
    birth_timestamp: i64,
}

impl VillagerMakeLove {
    fn breeding_possible(cx: &mut Cx) -> bool {
        let Some(id) = cx.b.mem.entity(Mem::BreedTarget) else { return false };
        // The target is a villager (`AgeableMob` of type villager), alive and visible.
        if !cx.level.entity(id).is_some_and(|o| o.type_name == "minecraft:villager") {
            return false;
        }
        if target_is_villager(cx, Mem::BreedTarget).is_none() {
            return false;
        }
        villager::can_breed(cx.m) && cx.level.entity(id).and_then(crate::mob::data).is_some_and(villager::can_breed)
    }
}

impl Behavior for VillagerMakeLove {
    fn name(&self) -> &'static str {
        "VillagerMakeLove"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::BreedTarget, ValuePresent), (Mem::NearestVisibleLivingEntities, ValuePresent)]
    }
    fn duration(&self) -> (i32, i32) {
        (350, 350)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        Self::breeding_possible(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        cx.time <= self.birth_timestamp && Self::breeding_possible(cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        let Some(other) = cx.b.mem.entity(Mem::BreedTarget) else { return };
        lock_gaze_and_walk_to_each_other(cx, other, 0.5, 2);
        let me = cx.e.id;
        cx.level.emit(Event::EntityEvent { entity: other, event: 18 });
        cx.level.emit(Event::EntityEvent { entity: me, event: 18 });
        let d = 275 + cx.e.random.next_int_bounded(50);
        self.birth_timestamp = cx.time + d as i64;
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(other) = cx.b.mem.entity(Mem::BreedTarget) else { return };
        let Some(o) = util::living(cx, other) else { return };
        if cx.e.position().distance_to_sqr(o.pos) > 5.0 {
            return;
        }
        lock_gaze_and_walk_to_each_other(cx, other, 0.5, 2);
        if cx.time >= self.birth_timestamp {
            villager::eat_and_digest_food(cx.m);
            villager::eat_and_digest_food_of(cx.level, other);
            villager::try_to_give_birth(cx, other);
        } else if cx.e.random.next_int_bounded(35) == 0 {
            let me = cx.e.id;
            cx.level.emit(Event::EntityEvent { entity: other, event: 12 });
            cx.level.emit(Event::EntityEvent { entity: me, event: 12 });
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        cx.b.mem.erase(Mem::BreedTarget);
    }
    behavior_boilerplate!();
}

// ---------------------------------------------------------------------------- play

/// `PlayTagWithOtherKids.create()`: the babies chase each other.
pub fn play_tag_with_other_kids() -> Box<dyn Control> {
    shot(
        "PlayTagWithOtherKids",
        &[(Mem::VisibleVillagerBabies, ValuePresent), (Mem::WalkTarget, ValueAbsent), (Mem::LookTarget, Registered), (Mem::InteractionTarget, Registered)],
        |cx| {
            if cx.rng().next_int_bounded(10) != 0 {
                return false;
            }
            let kids: Vec<i32> = cx.b.mem.entities(Mem::VisibleVillagerBabies).to_vec();
            let me = cx.e.id;
            // A friend chasing me?
            let chased_by_friend = kids.iter().any(|&k| interaction_target_of(cx, k) == Some(me));
            if chased_by_friend {
                for _ in 0..10 {
                    if let Some(v) = random_pos::land_pos(cx.e, cx.m, &*cx.level, 20, 8)
                        && cx.level.is_village(crate::math::BlockPos::containing(v.x, v.y, v.z))
                    {
                        cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget::vec(v, 0.6, 0)));
                        break;
                    }
                }
                return true;
            }
            // `findSomeoneBeingChased`: kids being chased by 1..=5, the least chased first.
            let mut counts: Vec<(i32, i32)> = Vec::new();
            for &k in &kids {
                if let Some(t) = interaction_target_of(cx, k) {
                    match counts.iter_mut().find(|(id, _)| *id == t) {
                        Some((_, n)) => *n += 1,
                        None => counts.push((t, 1)),
                    }
                }
            }
            counts.sort_by_key(|(_, n)| *n);
            let chased = counts.iter().find(|(_, n)| *n > 0 && *n <= 5).map(|(id, _)| *id);
            let chosen = chased.or_else(|| kids.first().copied());
            if let Some(k) = chosen {
                chase_kid(cx, k);
            }
            true
        },
    )
}

/// The `INTERACTION_TARGET` of kid `id`'s brain.
fn interaction_target_of(cx: &Cx, id: i32) -> Option<i32> {
    let o = cx.level.entity(id)?;
    crate::mob::data(o)?.brain.as_ref()?.st.mem.entity(Mem::InteractionTarget)
}

fn chase_kid(cx: &mut Cx, id: i32) {
    cx.b.mem.set(Mem::InteractionTarget, Val::Entity(id));
    cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
    cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity3(id, false, false), speed: 0.6, close_enough: 1 }));
}

// ---------------------------------------------------------------------------- gifts

/// `GiveGiftToHero(minDuration = maxDuration)`: a hero of the village nearby gets a gift thrown
/// at them now and then.
#[derive(Clone, Debug)]
pub struct GiveGiftToHero {
    duration: i32,
    time_until_next: i32,
    given: bool,
    time_since_start: i64,
}

impl GiveGiftToHero {
    pub fn new(duration: i32) -> Box<dyn Control> {
        Timed::new(GiveGiftToHero { duration, time_until_next: 600, given: false, time_since_start: 0 })
    }

    fn hero(cx: &Cx) -> Option<i32> {
        let id = cx.b.mem.entity(Mem::NearestVisiblePlayer)?;
        cx.level.player_effect(id, "minecraft:hero_of_the_village").map(|_| id)
    }
}

impl Behavior for GiveGiftToHero {
    fn name(&self) -> &'static str {
        "GiveGiftToHero"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[
            (Mem::WalkTarget, Registered),
            (Mem::LookTarget, Registered),
            (Mem::InteractionTarget, Registered),
            (Mem::NearestVisiblePlayer, ValuePresent),
        ]
    }
    fn duration(&self) -> (i32, i32) {
        (self.duration, self.duration)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        if Self::hero(cx).is_none() {
            return false;
        }
        if self.time_until_next > 0 {
            self.time_until_next -= 1;
            return false;
        }
        true
    }
    fn start(&mut self, cx: &mut Cx) {
        self.given = false;
        self.time_since_start = cx.time;
        if let Some(h) = Self::hero(cx) {
            cx.b.mem.set(Mem::InteractionTarget, Val::Entity(h));
            util::look_at_entity(cx, h);
        }
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        Self::hero(cx).is_some() && !self.given
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(h) = Self::hero(cx) else { return };
        util::look_at_entity(cx, h);
        let Some(l) = util::living(cx, h) else { return };
        let mine = cx.e.block_position();
        let theirs = crate::math::BlockPos::containing(l.pos.x, l.pos.y, l.pos.z);
        if util::dist_sqr_pos(theirs, mine) < 25.0 {
            if cx.time - self.time_since_start > 20 {
                villager::throw_gift(cx, h);
                self.given = true;
            }
        } else {
            util::set_walk_and_look(cx, Tracker::entity(h, true), 0.5, 5);
        }
    }
    fn stop(&mut self, cx: &mut Cx) {
        self.time_until_next = 600 + cx.rng().next_int_bounded(6001);
        cx.b.mem.erase(Mem::InteractionTarget);
        cx.b.mem.erase(Mem::WalkTarget);
        cx.b.mem.erase(Mem::LookTarget);
    }
    behavior_boilerplate!();
}

