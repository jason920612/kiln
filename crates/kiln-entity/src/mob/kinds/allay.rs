//! Allay: a flying spirit that takes an item from a player (liking that player), collects
//! matching items lying about into its one-slot inventory and brings them to the player it
//! likes (or to a note block it heard), heals over time and cannot be hurt by that player.
//!
//! Driven by the brain of `AllayAi` (core: swim, panic, look sink, move sink, cooldowns; idle:
//! go to a wanted item, give items to the target, stay close to the target, look at whoever is
//! near, then one of flying about, walking to the look target or nothing) with a
//! `FlyingPathNavigation`, on [`crate::mob::brain`]. The sensors are the nearest living
//! entities, players, `HurtBy` and the nearest wanted item (`NearestItemSensor`).

use crate::behavior_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::memory::GlobalPos;
use crate::mob::brain::persist::OVERWORLD;
use crate::mob::brain::sensors;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Gate, Mem, Sensor, Status, Timed, Tracker, Val, WalkTarget, util};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::fly;
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, MobData};
use crate::persist::{Input, Output};
use crate::sensor_boilerplate;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Allay;

pub static KIND: Allay = Allay;

static INFO: Info = Info {
    ..Info::misc("minecraft:allay", &[(MaxHealth, 20.0), (FlyingSpeed, 0.10000000149011612), (MovementSpeed, 0.10000000149011612), (AttackDamage, 2.0)])
};

/// `Allay.THROW_SOUND_PITCHES`.
const THROW_SOUND_PITCHES: [f32; 16] = [0.5625, 0.625, 0.75, 0.9375, 1.0, 1.0, 1.125, 1.25, 1.5, 1.875, 2.0, 2.25, 2.5, 3.0, 3.75, 4.0];

/// `Allay.DUPLICATION_COOLDOWN_TICKS`.
const DUPLICATION_COOLDOWN_TICKS: i64 = 6000;

#[derive(Clone, Debug, Default)]
pub struct State {
    /// The one-slot inventory (`SimpleContainer(1)`).
    pub inventory: ItemStack,
    /// `duplicationCooldown` (`DATA_CAN_DUPLICATE` while zero).
    pub duplication_cooldown: i64,
    /// `DATA_DANCING` and `jukeboxPos`.
    pub dancing: bool,
    pub jukebox_pos: Option<BlockPos>,
    /// The wanted item as last seen (id, position, eye height): a `NEAREST_VISIBLE_WANTED_ITEM`
    /// entity that has left the level still has a place in vanilla.
    last_item: Option<(i32, Vec3, f64)>,
    /// `LIKED_PLAYER` as the brain's memory has it (a copy for what runs while the brain is
    /// out of the mob: the sensors' targeting tests).
    liked: Option<u128>,
    /// `vibrationData` (`listener` in the save): the note block vibration on its way.
    vibration: crate::vibration::VibrationData,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("allay state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("allay state")
}

fn mem(m: &MobData) -> Option<&brain::Memories> {
    m.brain.as_ref().map(|b| &b.st.mem)
}

/// The liked player's UUID.
fn liked_uuid(m: &MobData) -> Option<u128> {
    mem(m).map_or(st(m).liked, |b| b.uuid(Mem::LikedPlayer))
}

/// `Allay.isOnPickupCooldown`.
fn on_pickup_cooldown(m: &MobData) -> bool {
    mem(m).is_some_and(|b| b.has(Mem::ItemPickupCooldownTicks))
}

/// `Allay.canPickUpLoot`: holding something, off cooldown.
fn can_pick_up_loot(m: &MobData) -> bool {
    !on_pickup_cooldown(m) && !m.equipment[mob::MAINHAND].is_empty()
}

/// `ItemStack.isSameItem` and the same potion contents.
fn considers_equal(a: &ItemStack, b: &ItemStack) -> bool {
    a.is_same_item(b) && a.get(kiln_item::keys::POTION_CONTENTS) == b.get(kiln_item::keys::POTION_CONTENTS)
}

/// `SimpleContainer.canAddItem` for the one slot.
fn can_add_item(inv: &ItemStack, stack: &ItemStack) -> bool {
    inv.is_empty() || (inv.is_same_item_same_components(stack) && inv.count() < inv.max_stack_size())
}

/// `SimpleContainer.addItem`: the part of `stack` that did not fit.
fn add_item(inv: &mut ItemStack, stack: &ItemStack) -> ItemStack {
    let mut rest = stack.clone();
    if inv.is_empty() {
        *inv = std::mem::replace(&mut rest, ItemStack::empty());
        return rest;
    }
    if inv.is_same_item_same_components(&rest) {
        let room = inv.max_stack_size().min(rest.max_stack_size()) - inv.count();
        let n = rest.count().min(room);
        if n > 0 {
            inv.grow(n);
            rest.shrink(n);
        }
    }
    rest
}

/// `Allay.wantsToPickUp`: the same item as the one held, room in the inventory, griefing on.
fn wants(m: &MobData, level: &dyn EntityLevel, stack: &ItemStack) -> bool {
    let held = &m.equipment[mob::MAINHAND];
    !held.is_empty() && level.mob_griefing() && can_add_item(&st(m).inventory, stack) && considers_equal(held, stack)
}

/// `BehaviorUtils.throwItem(thrower, stack, target, velocity, yOffset)`.
pub(crate) fn throw_item(e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack, target: Vec3, velocity: (f64, f64, f64), y_offset: f32) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, stack, seed);
    item.set_pos(Vec3::new(e.x(), e.eye_y() - y_offset as f64, e.z()));
    let v = (target - e.position()).normalize();
    item.delta = Vec3::new(v.x * velocity.0, v.y * velocity.1, v.z * velocity.2);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
        d.thrower = Some(e.uuid);
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
}

/// `AllayAi.getLikedPlayer`: the liked player while it plays (not a spectator) within 64 blocks.
fn liked_player(cx: &Cx) -> Option<PlayerView> {
    let u = cx.b.mem.uuid(Mem::LikedPlayer)?;
    let p = cx.level.player_by_uuid(u)?;
    let d = p.pos.distance_to_sqr(cx.e.position());
    (!p.spectator && d < 64.0 * 64.0).then_some(p)
}

/// `AllayAi.shouldDepositItemsAtLikedNoteblock`.
fn should_deposit_at_noteblock(cx: &Cx, g: &GlobalPos) -> bool {
    let near = &*g.dim == OVERWORLD && util::dist_sqr_pos(g.pos, cx.e.block_position()) <= 1024.0 * 1024.0;
    near && crate::blocks::block_name(cx.level.block(g.pos)) == "minecraft:note_block" && cx.b.mem.has(Mem::LikedNoteblockCooldownTicks)
}

/// `AllayAi.getItemDepositPosition`: the liked note block (forgetting it when it no longer
/// counts), else the liked player.
fn item_deposit_position(cx: &mut Cx) -> Option<Tracker> {
    if let Some(g) = cx.b.mem.global_pos(Mem::LikedNoteblockPosition).cloned() {
        if should_deposit_at_noteblock(cx, &g) {
            return Some(Tracker::block(g.pos.above()));
        }
        cx.b.mem.erase(Mem::LikedNoteblockPosition);
    }
    liked_player(cx).map(|p| Tracker::entity(p.id, true))
}

/// `AllayAi.hearNoteblock`: remember the note block (for 600 ticks of interest).
pub fn hear_noteblock(m: &mut MobData, pos: BlockPos) {
    let Some(b) = m.brain.as_mut() else { return };
    let g = GlobalPos::new(OVERWORLD, pos);
    match b.st.mem.global_pos(Mem::LikedNoteblockPosition) {
        None => {
            b.st.mem.set(Mem::LikedNoteblockPosition, Val::Pos(g));
            b.st.mem.set(Mem::LikedNoteblockCooldownTicks, Val::Int(600));
        }
        Some(cur) if *cur == g => b.st.mem.set(Mem::LikedNoteblockCooldownTicks, Val::Int(600)),
        _ => {}
    }
}

impl Kind for Allay {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// `createNavigation`: a `FlyingPathNavigation` that floats, opens no doors and plans 48 long.
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.fly = true;
        m.nav.can_float = true;
        m.nav.can_open_doors = false;
        m.nav.required_path_length = 48.0;
        Some(Box::new(State::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    /// The brain, then `AllayAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        if let Some(b) = m.brain.as_mut() {
            b.st.set_active_activity_to_first_valid(&[Activity::Idle]);
        }
    }

    /// `FlyingMoveControl(20, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        fly::tick_move(e, m, 20.0, true);
        true
    }

    /// `travelFlying(input, getSpeed())`.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let speed = m.speed;
        fly::travel(e, level, input, speed);
        true
    }

    /// `considersEntityAsAlly`: the liked player is an ally, never a target.
    fn can_attack(&self, m: &MobData, level: &dyn EntityLevel, t: &crate::mob::goals::Living) -> bool {
        let liked = liked_uuid(m);
        !(t.player && liked.is_some() && level.player(t.id).map(|p| p.uuid) == liked)
    }

    /// `PathfinderMob.getWalkTargetValue`: every spot is worth the same.
    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    /// `Mob.aiStep`'s looting, then `Allay.aiStep`: healing every 10 ticks, the dance, the
    /// duplication cooldown.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `Mob.aiStep`: `canPickUpLoot`, alive, griefing; reach (1, 1, 1).
        if can_pick_up_loot(m) && mob::is_alive(e, m) && !m.dead && level.mob_griefing() {
            let area = e.bounding_box().inflate(1.0, 1.0, 1.0);
            for id in level.entities_in(&area, EntityFilter::Item, e.id) {
                let Some(it) = level.entity(id) else { continue };
                let EntityKind::Item(d) = &it.kind else { continue };
                if it.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 || !wants(m, &*level, &d.stack) {
                    continue;
                }
                // `InventoryCarrier.pickUpItem`.
                let stack = d.stack.clone();
                let (pos, eye) = (it.position(), it.eye_y());
                if !can_add_item(&st(m).inventory, &stack) {
                    continue;
                }
                let count = stack.count();
                let rest = add_item(&mut st_mut(m).inventory, &stack);
                let Some(it) = level.entity_mut(id) else { continue };
                let EntityKind::Item(d) = &mut it.kind else { continue };
                if rest.is_empty() {
                    it.discard();
                } else {
                    d.stack.set_count(rest.count());
                }
                let _ = count;
                st_mut(m).last_item = Some((id, pos, eye));
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.item.pickup", source: "neutral", volume: 0.2, pitch: 1.0 });
                }
            }
        }
        if mob::is_alive(e, m) && e.tick_count % 10 == 0 && m.health > 0.0 {
            let h = m.health + 1.0;
            m.set_health(h);
        }
        // `shouldStopDancing`, checked every 20 ticks.
        let s = st(m);
        if s.dancing && e.tick_count % 20 == 0 && should_stop_dancing(e, m, &*level) {
            let s = st_mut(m);
            s.dancing = false;
            s.jukebox_pos = None;
        }
        let s = st_mut(m);
        if s.duplication_cooldown > 0 {
            s.duplication_cooldown -= 1;
        }
    }

    /// `Allay.tick`: the vibration ticker, and a panicking allay stops dancing.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        tick_vibrations(e, m, level);
        if mem(m).is_some_and(|b| b.has(Mem::IsPanicking)) {
            st_mut(m).dancing = false;
        }
        update_listener(e, m, level);
    }

    /// `hurtServer`: the liked player cannot hurt it.
    fn hurt(&self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        let liked = liked_uuid(m);
        let by_liked = source.attacker.and_then(|a| level.player(a)).is_some_and(|p| Some(p.uuid) == liked);
        by_liked.then_some(false)
    }

    /// Giving it an item (it likes the giver), taking its item back, duplicating it with
    /// amethyst while it dances.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let held = m.equipment[mob::MAINHAND].clone();
        let sound = |level: &mut dyn EntityLevel, e: &Entity, s: &'static str| {
            level.emit(Event::Sound { pos: e.position(), sound: s, source: "neutral", volume: 2.0, pitch: 1.0 });
        };
        if st(m).dancing && !stack.is_empty() && mob::item_tag(stack.item(), "minecraft:duplicates_allays") && st(m).duplication_cooldown == 0 {
            duplicate(e, m, level);
            level.emit(Event::EntityEvent { entity: e.id, event: 18 });
            sound(level, e, "minecraft:block.amethyst_block.chime");
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if held.is_empty() && !stack.is_empty() {
            let mut one = stack.clone();
            one.set_count(1);
            m.equipment[mob::MAINHAND] = one;
            sound(level, e, "minecraft:entity.allay.item_given");
            if let Some(p) = level.player(who.id) {
                if let Some(b) = m.brain.as_mut() {
                    b.st.mem.set(Mem::LikedPlayer, Val::Uuid(p.uuid));
                }
                st_mut(m).liked = Some(p.uuid);
            }
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if !held.is_empty() && stack.is_empty() {
            m.equipment[mob::MAINHAND] = ItemStack::empty();
            sound(level, e, "minecraft:entity.allay.item_taken");
            m.swing = true;
            let inv = std::mem::replace(&mut st_mut(m).inventory, ItemStack::empty());
            if !inv.is_empty() {
                throw_item(e, level, inv, e.position(), (0.30000001192092896, 0.30000001192092896, 0.30000001192092896), 0.3);
            }
            if let Some(b) = m.brain.as_mut() {
                b.st.mem.erase(Mem::LikedPlayer);
            }
            st_mut(m).liked = None;
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
        let liked = r.get("Brain").and_then(|b| b.get("memories")).and_then(|mm| mm.get("minecraft:liked_player")).and_then(|v| v.get("value")).and_then(crate::persist::uuid_from_tag);
        let vibration = crate::vibration::VibrationData::from_nbt(r.get("listener"));
        let s = st_mut(m);
        s.liked = liked;
        s.vibration = vibration;
        if let Some(i) = inv {
            s.inventory = i;
        }
        s.duplication_cooldown = cooldown;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        let inv = if s.inventory.is_empty() { Vec::new() } else { vec![s.inventory.to_nbt()] };
        o.put("Inventory", Tag::List(inv));
        o.put("DuplicationCooldown", Tag::Long(s.duplication_cooldown));
        o.put("listener", s.vibration.to_nbt());
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let s = st(m);
        d.set(data::allay::DANCING, &DataValue::Boolean(s.dancing));
        d.set(data::allay::CAN_DUPLICATE, &DataValue::Boolean(s.duplication_cooldown == 0));
    }
}

/// `VibrationSystem.Ticker.tick` for the allay: what the dispatcher heard reaches its selector,
/// the current vibration travels and arrives.
fn tick_vibrations(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let heard = level.take_allay_vibrations(e.id);
    let now = level.game_time();
    let eyes = Vec3::new(e.x(), e.eye_y(), e.z());
    let liked = mem(m).and_then(|b| b.global_pos(Mem::LikedNoteblockPosition)).map(|g| g.pos);
    let no_ai = m.no_ai;
    let s = st_mut(m);
    let mut jukeboxes: Vec<(BlockPos, bool)> = Vec::new();
    for h in heard {
        let at = BlockPos::containing(h.from.x, h.from.y, h.from.z);
        match h.event {
            // `JukeboxListener.handleGameEvent`.
            "minecraft:jukebox_play" => jukeboxes.push((at, true)),
            "minecraft:jukebox_stop_play" => jukeboxes.push((at, false)),
            // `canReceiveVibration`: AI on, and no liked note block or this one.
            _ if !no_ai && liked.is_none_or(|p| p == at) => s.vibration.schedule(h.event, h.from, h.to, h.source, None, h.tick),
            _ => {}
        }
    }
    for (at, playing) in jukeboxes {
        set_jukebox_playing(m, at, playing);
    }
    let s = st_mut(m);
    if s.vibration.current.is_none() && s.vibration.selector.current.is_none() {
        return;
    }
    let t = s.vibration.tick(now, eyes, crate::vibration::travel_time);
    let eye_height = e.eye_height;
    for (from, ticks) in t.particles {
        level.vibration_particle(from, e.id, eye_height, ticks);
    }
    if !t.arrived {
        return;
    }
    let Some(info) = st(m).vibration.current.clone() else { return };
    // `AllayVibrationUser.onReceiveVibration`.
    if info.event == "minecraft:note_block_play" {
        hear_noteblock(m, BlockPos::containing(info.pos.x, info.pos.y, info.pos.z));
    }
    st_mut(m).vibration.received();
}

/// The allay's `DynamicGameEventListener` after its tick.
fn update_listener(e: &Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if e.is_removed() || m.is_dead_or_dying() {
        level.set_allay_listener(e.id, None);
        return;
    }
    let ear = crate::vibration::Ear { pos: Vec3::new(e.x(), e.eye_y(), e.z()), busy: st(m).vibration.current.is_some(), can_hear: !m.no_ai };
    level.set_allay_listener(e.id, Some(ear));
}

/// `Allay.shouldStopDancing`: no jukebox within its radius (10) playing.
fn should_stop_dancing(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    let Some(p) = st(m).jukebox_pos else { return true };
    let c = Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
    !(c.distance_to_sqr(e.position()) < 10.0 * 10.0 && crate::blocks::block_name(level.block(p)) == "minecraft:jukebox")
}

/// `Allay.setJukeboxPlaying`: a jukebox nearby starts or stops the dance.
pub fn set_jukebox_playing(m: &mut MobData, pos: BlockPos, playing: bool) {
    let panicking = mem(m).is_some_and(|b| b.has(Mem::IsPanicking));
    let s = st_mut(m);
    if playing {
        if !s.dancing {
            s.jukebox_pos = Some(pos);
            // `setDancing(true)`: only an effective AI that is not panicking.
            if !panicking && !m.no_ai {
                st_mut(m).dancing = true;
            }
        }
    } else if s.jukebox_pos == Some(pos) || s.jukebox_pos.is_none() {
        s.jukebox_pos = None;
        if !m.no_ai {
            st_mut(m).dancing = false;
        }
    }
}

/// A jukebox game event at `at` for allay `e` if it is within the listener's 10 blocks
/// (`JukeboxListener`): for callers that deliver game events themselves (the parity harness).
pub fn hear_jukebox(e: &Entity, m: &mut MobData, playing: bool, at: BlockPos) {
    let c = BlockPos::containing(e.x(), e.eye_y(), e.z());
    let d = [(at.x - c.x) as i64, (at.y - c.y) as i64, (at.z - c.z) as i64];
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= 10 * 10 {
        set_jukebox_playing(m, at, playing);
    }
}

/// `duplicateAllay`: a copy at its place, both start a 5 minute cooldown.
fn duplicate(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut c = mob::new(mob::MobKind::Allay, id, 0, seed);
    // `snapTo(position())` keeps the copy's own random yaw.
    c.set_pos(e.position());
    if let Some(cm) = mob::data_mut(&mut c) {
        cm.persistence_required = true;
        st_mut(cm).duplication_cooldown = DUPLICATION_COOLDOWN_TICKS;
    }
    st_mut(m).duplication_cooldown = DUPLICATION_COOLDOWN_TICKS;
    level.add_entity(c);
}

/// `NearestItemSensor`: the closest item the allay wants and can see, within 32 blocks.
#[derive(Clone, Debug)]
struct NearestItems;

impl Sensor for NearestItems {
    fn name(&self) -> &'static str {
        "NearestItemSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleWantedItem]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let area = cx.e.bounding_box().inflate(32.0, 16.0, 32.0);
        let ids = cx.level.entities_in(&area, EntityFilter::Item, cx.e.id);
        let here = cx.e.position();
        let mut items: Vec<(f64, i32)> = ids.into_iter().filter_map(|id| Some((cx.level.entity(id)?.position().distance_to_sqr(here), id))).collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let eye = Vec3::new(cx.e.x(), cx.e.eye_y(), cx.e.z());
        let mut found = None;
        for (d, id) in items {
            let Some(it) = cx.level.entity(id) else { continue };
            let EntityKind::Item(data) = &it.kind else { continue };
            // wantsToPickUp, closerThan(32), hasLineOfSight.
            if !wants(cx.m, &*cx.level, &data.stack) || !(d < 32.0 * 32.0) {
                continue;
            }
            let to = Vec3::new(it.x(), it.eye_y(), it.z());
            if to.distance_to_sqr(eye).sqrt() > 128.0 || mob::clip_blocks(&*cx.level, eye, to) {
                continue;
            }
            found = Some(id);
            break;
        }
        cx.b.mem.set_opt(Mem::NearestVisibleWantedItem, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

/// `AllayAi.getActivities` and the sensors of `Allay.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn Sensor>> = vec![Box::new(sensors::NearestLivingEntities), Box::new(sensors::Players), Box::new(sensors::HurtBy), Box::new(NearestItems)];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            Swim::new(0.8),
            AnimalPanic::new(2.5),
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            CountDownCooldownTicks::new(Mem::LikedNoteblockCooldownTicks),
            CountDownCooldownTicks::new(Mem::ItemPickupCooldownTicks),
        ],
    );
    let idle = ActivityData::create(
        Activity::Idle,
        0,
        vec![
            go_to_wanted_item(),
            Timed::new(GoAndGiveItemsToTarget),
            stay_close_to_target(),
            SetEntityLookTargetSometimes::new(None, 6.0, (30, 60)),
            Gate::run_one(vec![
                (stroll(1.0, StrollKind::Fly), 2),
                (set_walk_target_from_look_target(1.0, 3), 2),
                (DoNothing::new(30, 60), 1),
            ]),
        ],
    );
    Brain::new(&[Mem::LikedPlayer, Mem::LikedNoteblockPosition, Mem::LikedNoteblockCooldownTicks], sensors, vec![core, idle], random)
}

/// `GoToWantedItem.create(always, 1.75, true, 32)`: walk to the wanted item when it can be
/// picked up.
fn go_to_wanted_item() -> Box<dyn brain::Control> {
    brain::shot(
        "GoToWantedItem",
        &[(Mem::LookTarget, Status::Registered), (Mem::WalkTarget, Status::Registered), (Mem::NearestVisibleWantedItem, Status::ValuePresent), (Mem::ItemPickupCooldownTicks, Status::Registered)],
        |cx| {
            let Some(id) = cx.b.mem.entity(Mem::NearestVisibleWantedItem) else { return false };
            if cx.b.mem.has(Mem::ItemPickupCooldownTicks) {
                return false;
            }
            let live = cx.level.entity(id).map(|it| (it.position(), it.eye_y()));
            let pos = match live {
                Some((p, eye)) => {
                    st_mut(cx.m).last_item = Some((id, p, eye));
                    p
                }
                None => match st(cx.m).last_item {
                    Some((i, p, _)) if i == id => p,
                    _ => return false,
                },
            };
            if !(pos.distance_to_sqr(cx.e.position()) < 32.0 * 32.0) || !can_pick_up_loot(cx.m) {
                return false;
            }
            cx.b.mem.set(Mem::LookTarget, Val::Look(Tracker::entity(id, true)));
            cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: Tracker::entity(id, false), speed: 1.75, close_enough: 0 }));
            true
        },
    )
}

/// `StayCloseToTarget.create(depositPosition, no wanted item, 4, 16, 2.25)`: walk to the
/// liked player (or note block) when it is 16 blocks away or more.
fn stay_close_to_target() -> Box<dyn brain::Control> {
    brain::shot("StayCloseToTarget", &[(Mem::LookTarget, Status::Registered), (Mem::WalkTarget, Status::Registered)], |cx| {
        let Some(t) = item_deposit_position(cx) else { return false };
        if cx.b.mem.has(Mem::NearestVisibleWantedItem) {
            return false;
        }
        let Some(p) = util::tracker_pos(cx, &t) else { return false };
        if cx.e.position().distance_to_sqr(p) < 16.0 * 16.0 {
            return false;
        }
        cx.b.mem.set(Mem::LookTarget, Val::Look(t));
        cx.b.mem.set(Mem::WalkTarget, Val::Walk(WalkTarget { target: t, speed: 2.25, close_enough: 4 }));
        true
    })
}

/// `GoAndGiveItemsToTarget(depositPosition, 2.25, 20, throwItem, ITEM_PICKUP_COOLDOWN_TICKS, 60,
/// hasItem)`.
#[derive(Clone, Debug)]
struct GoAndGiveItemsToTarget;

impl GoAndGiveItemsToTarget {
    fn can_throw(cx: &mut Cx) -> bool {
        !st(cx.m).inventory.is_empty() && item_deposit_position(cx).is_some()
    }
}

impl Behavior for GoAndGiveItemsToTarget {
    fn name(&self) -> &'static str {
        "GoAndGiveItemsToTarget"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::LookTarget, Status::Registered), (Mem::WalkTarget, Status::Registered), (Mem::ItemPickupCooldownTicks, Status::Registered)]
    }
    fn duration(&self) -> (i32, i32) {
        (20, 20)
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        Self::can_throw(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        Self::can_throw(cx)
    }
    fn start(&mut self, cx: &mut Cx) {
        if let Some(t) = item_deposit_position(cx) {
            util::set_walk_and_look(cx, t, 2.25, 3);
        }
    }
    fn tick(&mut self, cx: &mut Cx) {
        let Some(t) = item_deposit_position(cx) else { return };
        let Some(p) = util::tracker_pos(cx, &t) else { return };
        let eye = Vec3::new(cx.e.x(), cx.e.eye_y(), cx.e.z());
        if p.distance_to_sqr(eye).sqrt() < 3.0 {
            throw_at(cx, p);
            cx.b.mem.set(Mem::ItemPickupCooldownTicks, Val::Int(60));
        }
    }
    behavior_boilerplate!();
}

/// `AllayAi.throwItem`: one item from the inventory at the target, and now and then a sound.
fn throw_at(cx: &mut Cx, target: Vec3) {
    let mut one = st(cx.m).inventory.clone();
    if one.is_empty() {
        return;
    }
    let taken = one.split(1);
    st_mut(cx.m).inventory = one;
    throw_item(cx.e, cx.level, taken, target.add(0.0, 1.0, 0.0), (0.20000000298023224, 0.30000001192092896, 0.20000000298023224), 0.2);
    if cx.time % 7 == 0 && cx.rng().next_double() < 0.9 {
        let pitch = THROW_SOUND_PITCHES[cx.rng().next_int_bounded(THROW_SOUND_PITCHES.len() as i32) as usize];
        let pos = cx.e.position();
        cx.level.emit(Event::Sound { pos, sound: "minecraft:entity.allay.item_thrown", source: "neutral", volume: 1.0, pitch });
    }
}
