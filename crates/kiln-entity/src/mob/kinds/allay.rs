//! Allay: a flying spirit that takes an item from a player (liking that player), collects
//! matching items lying about into its one-slot inventory and brings them to the player it
//! likes, heals over time and cannot be hurt by that player.
//!
//! Approximation: vanilla drives it with a `Brain` (`AllayAi`) and flies with a
//! `FlyingPathNavigation`; here the behaviours are goals (panicking, fetching wanted items,
//! delivering them, staying near the liked player, fluttering about) and the flight goes
//! straight at its target ([`crate::mob::fly::DirectFlight`]). Note blocks, jukebox dancing and
//! duplication with amethyst (which needs the dancing) are not simulated.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::fly::{self, DirectFlight};
use crate::mob::goals::{self, Goal, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, MobData};
use crate::persist::{Input, Output, uuid_to_tag};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Allay;

pub static KIND: Allay = Allay;

static INFO: Info = Info {
    ..Info::misc("minecraft:allay", &[(MaxHealth, 20.0), (FlyingSpeed, 0.10000000149011612), (MovementSpeed, 0.10000000149011612), (AttackDamage, 2.0)])
};

#[derive(Clone, Debug, Default)]
pub struct State {
    /// The one-slot inventory.
    pub inventory: ItemStack,
    /// `LIKED_PLAYER` (the player's UUID) and its network id while it is around.
    pub liked_player: Option<u128>,
    /// `ITEM_PICKUP_COOLDOWN_TICKS`.
    pickup_cooldown: i32,
    duplication_cooldown: i64,
    flight: DirectFlight,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("allay state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("allay state")
}

/// The liked player, if in the level.
fn liked(m: &MobData, level: &dyn EntityLevel) -> Option<crate::level::PlayerView> {
    let u = st(m).liked_player?;
    level.player_by_uuid(u).filter(|p| p.alive)
}

/// `wantsToPickUp`: the same item as the one held, room in the inventory, griefing on.
fn wants(m: &MobData, level: &dyn EntityLevel, stack: &ItemStack) -> bool {
    let held = &m.equipment[mob::MAINHAND];
    if held.is_empty() || stack.is_empty() || !level.mob_griefing() || held.item() != stack.item() {
        return false;
    }
    let inv = &st(m).inventory;
    inv.is_empty() || (inv.item() == stack.item() && inv.count() < inv.max_stack_size())
}

/// `BehaviorUtils.throwItem` toward `target` (0.3 a tick, from 0.3 below the eyes).
fn throw_item(e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack, target: Vec3) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, stack, seed);
    item.set_pos(Vec3::new(e.x(), e.eye_y() - 0.30000001192092896, e.z()));
    let v = (target - e.position()).normalize();
    item.delta = Vec3::new(v.x * 0.30000001192092896, v.y * 0.30000001192092896, v.z * 0.30000001192092896);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
        d.thrower = Some(e.uuid);
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
}

impl Kind for Allay {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(AllayFly { what: Fly::Panic, until: 0 })));
        g.add(1, Goal::Custom(Box::new(AllayFly { what: Fly::WantedItem, until: 0 })));
        g.add(2, Goal::Custom(Box::new(AllayFly { what: Fly::GiveItems, until: 0 })));
        g.add(3, Goal::Custom(Box::new(AllayFly { what: Fly::StayClose, until: 0 })));
        g.add(4, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(5, Goal::Custom(Box::new(AllayFly { what: Fly::Stroll, until: 0 })));
    }

    /// The flight target before the move control; then `FlyingMoveControl(20, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let mut f = st(m).flight;
        f.tick(e, m);
        st_mut(m).flight = f;
        fly::tick_move(e, m, 20.0, true);
        true
    }

    /// `travelFlying(input, getSpeed())`.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let speed = m.speed;
        fly::travel(e, level, input, speed);
        true
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    /// `Allay.aiStep` after `Mob.aiStep`: picking up wanted items, healing every 10 ticks.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        if s.pickup_cooldown > 0 {
            s.pickup_cooldown -= 1;
        }
        if s.duplication_cooldown > 0 {
            s.duplication_cooldown -= 1;
        }
        // `Mob.aiStep`: `canPickUpLoot` (holding something, off cooldown), reach (1, 1, 1).
        if st(m).pickup_cooldown == 0 && !m.equipment[mob::MAINHAND].is_empty() && mob::is_alive(e, m) {
            let area = e.bounding_box().inflate(1.0, 1.0, 1.0);
            for id in level.entities_in(&area, EntityFilter::Item, e.id) {
                let Some(it) = level.entity(id) else { continue };
                let EntityKind::Item(d) = &it.kind else { continue };
                if it.is_removed() || d.pickup_delay > 0 || !wants(m, level, &d.stack) {
                    continue;
                }
                let room = { let inv = &st(m).inventory; if inv.is_empty() { d.stack.max_stack_size() } else { inv.max_stack_size() - inv.count() } };
                let Some(it) = level.entity_mut(id) else { continue };
                let EntityKind::Item(d) = &mut it.kind else { continue };
                let taken = d.stack.split(room.min(d.stack.count()));
                if d.stack.is_empty() {
                    it.discard();
                }
                let inv = &mut st_mut(m).inventory;
                if inv.is_empty() {
                    *inv = taken;
                } else {
                    let c = inv.count() + taken.count();
                    inv.set_count(c);
                }
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.item.pickup", source: "neutral", volume: 0.2, pitch: 1.0 });
            }
        }
        if mob::is_alive(e, m) && e.tick_count % 10 == 0 {
            let h = m.health + 1.0;
            m.set_health(h);
        }
    }

    /// `hurtServer`: the liked player cannot hurt it.
    fn hurt(&self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        let by_liked = source.attacker.and_then(|a| level.player(a)).is_some_and(|p| Some(p.uuid) == st(m).liked_player);
        by_liked.then_some(false)
    }

    /// Giving it an item (it likes the giver), or taking its item back.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let held = m.equipment[mob::MAINHAND].clone();
        let sound = |level: &mut dyn EntityLevel, e: &Entity, s: &'static str| {
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: s, source: "neutral", volume: 2.0, pitch: 1.0 });
            }
        };
        if held.is_empty() && !stack.is_empty() {
            let mut one = stack.clone();
            one.set_count(1);
            m.equipment[mob::MAINHAND] = one;
            sound(level, e, "minecraft:entity.allay.item_given");
            st_mut(m).liked_player = level.player(who.id).map(|p| p.uuid);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if !held.is_empty() && stack.is_empty() {
            m.equipment[mob::MAINHAND] = ItemStack::empty();
            sound(level, e, "minecraft:entity.allay.item_taken");
            m.swing = true;
            let inv = std::mem::replace(&mut st_mut(m).inventory, ItemStack::empty());
            if !inv.is_empty() {
                throw_item(e, level, inv, e.position());
            }
            st_mut(m).liked_player = None;
            // The held item goes to the player (`Player.addItem`).
            return Some(Outcome::success(HeldChange::Fill(held)));
        }
        None
    }

    /// `dropEquipment`: the inventory and the held item.
    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource) {
        let inv = std::mem::replace(&mut st_mut(m).inventory, ItemStack::empty());
        mob::spawn_at_location(e, level, inv);
        let held = std::mem::replace(&mut m.equipment[mob::MAINHAND], ItemStack::empty());
        mob::spawn_at_location(e, level, held);
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(false)
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(if m.equipment[mob::MAINHAND].is_empty() { "minecraft:entity.allay.ambient_without_item" } else { "minecraft:entity.allay.ambient_with_item" }))
    }

    fn sound_volume(&self, _m: &MobData) -> f32 {
        0.4
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let inv = match r.get("Inventory") {
            Some(Tag::List(l)) => l.first().and_then(|t| ItemStack::from_nbt(t).ok()),
            _ => None,
        };
        let cooldown = r.num("DuplicationCooldown").map_or(0, |v| v as i64);
        let liked = match r.get("Brain").and_then(|b| b.get("memories")).and_then(|mm| mm.get("minecraft:liked_player")).and_then(|v| v.get("value")) {
            Some(t) => crate::persist::uuid_from_tag(t),
            None => None,
        };
        let s = st_mut(m);
        if let Some(i) = inv {
            s.inventory = i;
        }
        s.duplication_cooldown = cooldown;
        s.liked_player = liked;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        let inv = if s.inventory.is_empty() { Vec::new() } else { vec![s.inventory.to_nbt()] };
        o.put("Inventory", Tag::List(inv));
        o.put("DuplicationCooldown", Tag::Long(s.duplication_cooldown));
        let mut memories = Vec::new();
        if let Some(u) = s.liked_player {
            memories.push(("minecraft:liked_player".to_owned(), Tag::Compound(vec![("value".into(), uuid_to_tag(u))])));
        }
        o.put("Brain", Tag::Compound(vec![("memories".into(), Tag::Compound(memories))]));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        d.set(data::allay::DANCING, &DataValue::Boolean(false));
        d.set(data::allay::CAN_DUPLICATE, &DataValue::Boolean(st(m).duplication_cooldown == 0));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fly {
    /// `AnimalPanic(2.5)`.
    Panic,
    /// `GoToWantedItem(1.75, 32)`.
    WantedItem,
    /// `GoAndGiveItemsToTarget(2.25)`.
    GiveItems,
    /// `StayCloseToTarget(liked player, 4, 16, 2.25)`.
    StayClose,
    /// `RandomStroll.fly(1.0)`.
    Stroll,
}

/// One of the allay's brain behaviours as a goal steering its [`DirectFlight`].
#[derive(Clone, Debug)]
struct AllayFly {
    what: Fly,
    until: i32,
}

impl AllayFly {
    fn target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> Option<(Vec3, f64)> {
        match self.what {
            Fly::Panic => {
                if !goals::should_panic(m, level) {
                    return None;
                }
                let d = Vec3::new(e.random.next_int_bounded(11) as f64 - 5.0, e.random.next_int_bounded(9) as f64 - 4.0, e.random.next_int_bounded(11) as f64 - 5.0);
                Some((e.position() + d, 2.5))
            }
            Fly::WantedItem => {
                if st(m).pickup_cooldown > 0 {
                    return None;
                }
                let area = e.bounding_box().inflate(32.0, 16.0, 32.0);
                let mut best: Option<(f64, Vec3)> = None;
                for id in level.entities_in(&area, EntityFilter::Item, e.id) {
                    let Some(it) = level.entity(id) else { continue };
                    let EntityKind::Item(d) = &it.kind else { continue };
                    if !wants(m, level, &d.stack) {
                        continue;
                    }
                    let dist = it.position().distance_to_sqr(e.position());
                    if best.is_none_or(|(b, _)| dist < b) {
                        best = Some((dist, it.position()));
                    }
                }
                best.map(|(_, p)| (p, 1.75))
            }
            Fly::GiveItems => {
                if st(m).inventory.is_empty() {
                    return None;
                }
                let p = liked(m, level)?;
                Some((p.pos, 2.25))
            }
            Fly::StayClose => {
                let p = liked(m, level)?;
                (p.pos.distance_to_sqr(e.position()) > 16.0).then_some((p.pos, 2.25))
            }
            Fly::Stroll => {
                if e.random.next_int_bounded(mob::mth::reduced_tick_delay(120)) != 0 {
                    return None;
                }
                let d = Vec3::new(e.random.next_int_bounded(21) as f64 - 10.0, e.random.next_int_bounded(15) as f64 - 7.0, e.random.next_int_bounded(21) as f64 - 10.0);
                let to = e.position() + d;
                let air = kiln_data::blocks_types::is_air(level.block(BlockPos::containing(to.x, to.y, to.z)));
                air.then_some((to, 1.0))
            }
        }
    }
}

impl CustomGoal for AllayFly {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        match self.what {
            Fly::Panic => "AnimalPanic",
            Fly::WantedItem => "GoToWantedItem",
            Fly::GiveItems => "GoAndGiveItemsToTarget",
            Fly::StayClose => "StayCloseToTarget",
            Fly::Stroll => "RandomStroll",
        }
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match self.target(e, m, level) {
            Some((to, speed)) => {
                st_mut(m).flight.fly_to(to, speed);
                self.until = 100;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.until <= 0 {
            return false;
        }
        match self.what {
            Fly::GiveItems => {
                let Some(p) = liked(m, level) else { return false };
                if st(m).inventory.is_empty() {
                    return false;
                }
                // Close enough: the items go to the player.
                if p.pos.distance_to_sqr(e.position()) < 9.0 {
                    let inv = std::mem::replace(&mut st_mut(m).inventory, ItemStack::empty());
                    let at = Vec3::new(p.pos.x, p.pos.y + 1.0, p.pos.z);
                    throw_item(e, level, inv, at);
                    if !e.silent {
                        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.allay.item_thrown", source: "neutral", volume: 1.0, pitch: 1.0 });
                    }
                    st_mut(m).pickup_cooldown = 60;
                    return false;
                }
                st_mut(m).flight.fly_to(p.pos, 2.25);
                true
            }
            Fly::StayClose => liked(m, level).is_some_and(|p| {
                let far = p.pos.distance_to_sqr(e.position()) > 16.0;
                if far {
                    st_mut(m).flight.fly_to(p.pos, 2.25);
                }
                far
            }),
            _ => !st(m).flight.is_done(),
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).flight.stop();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.until -= 1;
        if let Some(t) = st(m).flight.target {
            m.look.set_look_at(t.x, t.y + 0.5, t.z, 45.0, 90.0);
        }
        let _ = e;
    }
}
