//! Piglin: admires gold, barters, attacks players without gold armor, zombifies outside the
//! nether.
//!
//! Vanilla drives piglins with a `Brain` (`PiglinAi`: activities for admiring, fighting,
//! avoiding, celebrating, riding hoglins). Kiln approximates it with goals: admiring a gold
//! item held out or picked up (standing still for 119 ticks, then bartering a gold ingot through
//! `minecraft:gameplay/piglin_bartering`), walking to gold on the ground, attacking the nearest
//! player not wearing `#minecraft:piglin_safe_armor` (melee, also for crossbow holders: piglin
//! crossbows are not simulated), anger at attackers shared with nearby piglins, babies fleeing
//! when hurt, strolling and looking around. Hoglin hunting, celebrating, dancing, riding,
//! avoiding zombified piglins and soul fire, eating, and equipping picked-up gear (it goes to
//! the inventory) are not simulated. Bartered items drop at the piglin instead of being thrown
//! toward the nearest player.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::attributes::Op;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, LOOK, MOVE, MeleeKind, TARGET};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, MobKind, OFFHAND, SpawnContext, path};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Piglin;

pub static KIND: Piglin = Piglin;

static INFO: Info = Info { sounds: Some("piglin"), ..Info::monster("minecraft:piglin", &[(MaxHealth, 16.0), (MovementSpeed, 0.3499999940395355), (AttackDamage, 5.0)]) };

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE_DURATION: i32 = 119;
/// `AbstractPiglin`: ticks outside the nether before zombifying.
pub const CONVERSION_TIME: i32 = 300;
/// `PiglinAi.BARTERING_ITEM`.
pub const BARTERING_ITEM: &str = "minecraft:gold_ingot";
pub const BARTERING_TABLE: &str = "minecraft:gameplay/piglin_bartering";

#[derive(Clone, Debug, Default)]
pub struct PiglinState {
    pub immune_to_zombification: bool,
    /// `timeInOverworld`.
    pub time_in_overworld: i32,
    pub cannot_hunt: bool,
    /// Ticks left of `ADMIRING_ITEM` (0: not admiring).
    pub admiring: i32,
    /// Ticks left of `ADMIRING_DISABLED` (after a player hit it).
    pub admiring_disabled: i32,
    /// `ANGRY_AT` and ticks left of it.
    pub angry_at: Option<i32>,
    pub anger: i32,
    /// `HUNTED_RECENTLY` ticks left (set at spawn).
    pub hunted_recently: i32,
    /// The 8-slot inventory (`InventoryCarrier`).
    pub inventory: Vec<ItemStack>,
}

pub fn state(m: &MobData) -> Option<&PiglinState> {
    ext::state::<PiglinState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut PiglinState> {
    ext::state_mut::<PiglinState>(m)
}

fn is_item(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && mob::item_name(stack) == name
}

/// `PiglinAi.isLovedItem`.
pub fn is_loved(stack: &ItemStack) -> bool {
    !stack.is_empty() && mob::item_tag(stack.item(), "minecraft:piglin_loved")
}

/// `Piglin.setBaby`: the flag and the speed bonus.
pub fn set_baby(e: &mut Entity, m: &mut MobData, baby: bool) {
    m.zombie_baby = baby;
    m.attrs.remove_modifier(MovementSpeed, "minecraft:baby");
    if baby {
        m.attrs.set_modifier(MovementSpeed, "minecraft:baby", 0.20000000298023224, Op::AddMultipliedBase);
    }
    mob::refresh_dimensions(e, m);
}

/// `InventoryCarrier` / `SimpleContainer.addItem`: what does not fit comes back.
fn add_to_inventory(st: &mut PiglinState, mut stack: ItemStack) -> ItemStack {
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

fn can_add_to_inventory(st: &PiglinState, stack: &ItemStack) -> bool {
    st.inventory.iter().any(|s| s.is_empty() || (s.is_same_item_same_components(stack) && s.count() < s.max_stack_size()))
}

/// `PiglinAi.admireGoldItem` + `stopWalking`.
fn admire(m: &mut MobData) {
    if let Some(st) = state_mut(m) {
        st.admiring = ADMIRE_DURATION;
    }
    m.nav.stop();
}

/// `PiglinAi.holdInOffhand`: the previous off hand item drops; gold being bartered does not
/// make the piglin persistent.
fn hold_in_offhand(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: ItemStack) {
    if !m.equipment[OFFHAND].is_empty() {
        let old = std::mem::replace(&mut m.equipment[OFFHAND], ItemStack::empty());
        mob::spawn_at_location(e, level, old);
    }
    if !is_item(&stack, BARTERING_ITEM) {
        m.persistence_required = true;
    }
    m.equipment[OFFHAND] = stack;
    m.drop_chances[OFFHAND] = 2.0;
}

/// `PiglinAi.stopHoldingOffHandItem`: an adult barters a held gold ingot (or deletes it when
/// hurt while admiring); other items go to the inventory.
fn stop_holding_offhand(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, barter: bool) {
    let off = std::mem::replace(&mut m.equipment[OFFHAND], ItemStack::empty());
    if off.is_empty() {
        return;
    }
    let currency = is_item(&off, BARTERING_ITEM);
    if m.baby() || !currency {
        let Some(st) = state_mut(m) else { return };
        let left = add_to_inventory(st, off);
        mob::spawn_at_location(e, level, left);
        return;
    }
    if barter {
        // `throwItems` toward the nearest player (or a random spot): Kiln drops the loot two
        // blocks that way instead of throwing it.
        m.swing = true;
        let p = e.position();
        let toward = level.players().into_iter().filter(|v| v.alive && !v.spectator).map(|v| v.pos).min_by(|a, b| a.distance_to_sqr(p).total_cmp(&b.distance_to_sqr(p)));
        let dir = match toward {
            Some(t) => Vec3::new(t.x - p.x, 0.0, t.z - p.z).normalize(),
            None => {
                let a = e.random.next_float() as f64 * std::f64::consts::TAU;
                Vec3::new(a.cos(), 0.0, a.sin())
            }
        };
        level.emit(Event::GiftLoot { entity: e.id, table: BARTERING_TABLE, pos: Vec3::new(p.x + dir.x * 2.0, p.y + 1.0, p.z + dir.z * 2.0) });
    }
}

/// `PiglinAi.wantsToPickup` (gold and loved items; food and gear are not simulated).
fn wants_to_pick_up(m: &MobData, stack: &ItemStack) -> bool {
    let Some(st) = state(m) else { return false };
    if m.baby() && mob::item_tag(stack.item(), "minecraft:ignored_by_piglin_babies") {
        return false;
    }
    if mob::item_tag(stack.item(), "minecraft:piglin_repellents") {
        return false;
    }
    if st.admiring_disabled > 0 && m.target.is_some() {
        return false;
    }
    let holding_loved = is_loved(&m.equipment[OFFHAND]);
    if is_item(stack, BARTERING_ITEM) {
        return !holding_loved;
    }
    let fits = can_add_to_inventory(st, stack);
    if is_item(stack, "minecraft:gold_nugget") {
        return fits;
    }
    is_loved(stack) && !holding_loved && fits
}

/// `Mob.aiStep`'s item pickup (reach 1 horizontally) with `PiglinAi.pickUpItem`.
fn pick_up_items(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !m.can_pick_up_loot || !mob::is_alive(e, m) || !level.mob_griefing() {
        return;
    }
    let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
    for id in level.entities_in(&area, EntityFilter::Item, e.id) {
        let Some(item) = level.entity(id) else { continue };
        let EntityKind::Item(d) = &item.kind else { continue };
        if item.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 || !wants_to_pick_up(m, &d.stack) {
            continue;
        }
        // `pickUpItem`: a gold nugget stack whole, anything else one at a time.
        let Some(item) = level.entity_mut(id) else { continue };
        let EntityKind::Item(d) = &mut item.kind else { continue };
        let taken = if is_item(&d.stack, "minecraft:gold_nugget") { std::mem::replace(&mut d.stack, ItemStack::empty()) } else { d.stack.split(1) };
        if d.stack.is_empty() {
            item.discard();
        }
        m.nav.stop();
        if is_loved(&taken) {
            hold_in_offhand(e, m, level, taken);
            admire(m);
            return;
        }
        let Some(st) = state_mut(m) else { return };
        let left = add_to_inventory(st, taken);
        mob::spawn_at_location(e, level, left);
    }
}

/// `AbstractPiglin.finishConversion`: a zombified piglin takes the piglin's place (equipment,
/// position, baby, flags), the inventory drops.
fn zombify(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.piglin.converted_to_zombified"));
    if let Some(st) = state_mut(m) {
        st.admiring = 0;
        let items = std::mem::take(&mut st.inventory);
        for s in items {
            mob::spawn_at_location(e, level, s);
        }
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut z = mob::new(MobKind::ZombifiedPiglin, id, 0, seed);
    z.set_pos(e.position());
    z.y_rot = e.y_rot;
    z.x_rot = e.x_rot;
    z.set_old_pos_and_rot();
    z.delta = e.delta;
    z.on_ground = e.on_ground;
    z.fall_distance = e.fall_distance;
    z.silent = e.silent;
    z.no_gravity = e.no_gravity;
    let baby = m.baby();
    let mut kind = std::mem::replace(&mut z.kind, EntityKind::MobTicking { gravity: 0.08 });
    if let EntityKind::Mob(zm) = &mut kind {
        for i in 0..6 {
            if !m.equipment[i].is_empty() {
                zm.equipment[i] = std::mem::replace(&mut m.equipment[i], ItemStack::empty());
                zm.drop_chances[i] = m.drop_chances[i];
            }
        }
        zm.y_body_rot = m.y_body_rot;
        zm.y_head_rot = m.y_head_rot;
        zm.hurt_time = m.hurt_time;
        zm.absorption = m.absorption;
        zm.last_hurt_by_player_memory = m.last_hurt_by_player_memory;
        zm.can_pick_up_loot = m.can_pick_up_loot;
        zm.left_handed = m.left_handed;
        zm.no_ai = m.no_ai;
        zm.persistence_required = m.persistence_required;
        if baby {
            // `Zombie.setBaby`.
            zm.zombie_baby = true;
            zm.attrs.set_modifier(Attr::MovementSpeed, "minecraft:baby", 0.5, Op::AddMultipliedBase);
            mob::refresh_dimensions(&mut z, zm);
        }
    }
    z.kind = kind;
    level.add_entity(z);
    e.discard();
}

/// Stands still while admiring (the brain's `ADMIRE_ITEM` activity without an item to reach).
#[derive(Clone, Debug)]
struct PiglinAdmireGoal;

impl CustomGoal for PiglinAdmireGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PiglinAdmireGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        state(m).is_some_and(|s| s.admiring > 0) && !m.equipment[OFFHAND].is_empty()
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// Walks to a wanted item on the ground within 9 blocks (`GoToWantedItem` of the brain).
#[derive(Clone, Debug)]
struct PiglinGoToWantedItemGoal {
    item: Option<i32>,
}

impl PiglinGoToWantedItemGoal {
    fn find(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<(i32, Vec3)> {
        let area = e.bounding_box().inflate(9.0, 4.0, 9.0);
        let p = e.position();
        let mut best: Option<(f64, i32, Vec3)> = None;
        for id in level.entities_in(&area, EntityFilter::Item, e.id) {
            let Some(o) = level.entity(id) else { continue };
            let EntityKind::Item(d) = &o.kind else { continue };
            if o.is_removed() || !wants_to_pick_up(m, &d.stack) {
                continue;
            }
            let dist = o.position().distance_to_sqr(p);
            if dist <= 81.0 && best.is_none_or(|b| dist < b.0) {
                best = Some((dist, id, o.position()));
            }
        }
        best.map(|(_, id, pos)| (id, pos))
    }
}

impl CustomGoal for PiglinGoToWantedItemGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PiglinGoToWantedItemGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !m.can_pick_up_loot || m.target.is_some() || state(m).is_none_or(|s| s.admiring > 0) {
            return false;
        }
        let Some((id, pos)) = Self::find(e, m, level) else { return false };
        self.item = Some(id);
        path::move_to(e, m, level, pos.x, pos.y, pos.z, 1.0)
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done() && self.item.and_then(|id| level.entity(id)).is_some_and(|o| !o.is_removed())
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.item = None;
        m.nav.stop();
    }
}

/// Targets the nearest visible player within 16 blocks that wears no gold armor (the brain's
/// `StartAttacking` with `findNearestValidAttackTarget`), or the player it is angry at.
#[derive(Clone, Debug)]
struct PiglinTargetGoal;

fn valid_player(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, id: i32, need_unsafe: bool) -> Option<f64> {
    let p = level.player(id)?;
    if !p.alive || p.spectator || p.creative || (need_unsafe && p.piglin_safe_armor) {
        return None;
    }
    let t = goals::living(level, id)?;
    let d = e.position().distance_to_sqr(p.pos);
    (d <= 16.0 * 16.0 && mob::has_line_of_sight_cached(e, m, level, &t)).then_some(d)
}

impl CustomGoal for PiglinTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PiglinTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.baby() || state(m).is_none_or(|s| s.admiring > 0) {
            return false;
        }
        if let Some(a) = state(m).and_then(|s| s.angry_at)
            && valid_player(e, m, level, a, false).is_some()
        {
            m.target = Some(a);
            return true;
        }
        let mut best: Option<(f64, i32)> = None;
        for p in goals::players_around(e, level, 16.0).iter() {
            if let Some(d) = valid_player(e, m, level, p.id, true)
                && best.is_none_or(|b| d < b.0)
            {
                best = Some((d, p.id));
            }
        }
        let Some((_, id)) = best else { return false };
        m.target = Some(id);
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = m.target else { return false };
        let angry = state(m).and_then(|s| s.angry_at) == Some(t);
        let admiring = state(m).is_some_and(|s| s.admiring > 0);
        !admiring && valid_player(e, m, level, t, !angry).is_some()
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
    }
}

/// `PiglinAi.createSpawnWeapon` (from the piglin's own random).
fn spawn_weapon(random: &mut LegacyRandom) -> &'static str {
    if (random.next_float() as f64) < 0.5 {
        return "minecraft:crossbow";
    }
    if random.next_int_bounded(10) == 0 { "minecraft:golden_spear" } else { "minecraft:golden_sword" }
}

impl Kind for Piglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.can_pick_up_loot = true;
        Some(Box::new(PiglinState { inventory: vec![ItemStack::empty(); 8], ..PiglinState::default() }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Custom(Box::new(PiglinAdmireGoal)));
        g.add(1, Goal::Panic { speed: 1.0, pos: Vec3::ZERO });
        g.add(2, Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(3, Goal::Custom(Box::new(PiglinGoToWantedItemGoal { item: None })));
        g.add(6, Goal::RandomStroll { speed: 0.6, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(PiglinTargetGoal)));
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        pick_up_items(e, m, level);
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // The brain's memories run out.
        let mut stop_holding = false;
        if let Some(st) = state_mut(m) {
            if st.admiring > 0 {
                st.admiring -= 1;
            }
            st.admiring_disabled = (st.admiring_disabled - 1).max(0);
            st.hunted_recently = (st.hunted_recently - 1).max(0);
            if st.anger > 0 {
                st.anger -= 1;
                if st.anger == 0 {
                    st.angry_at = None;
                }
            }
            stop_holding = st.admiring == 0;
        }
        // `StopHoldingItemIfNoLongerAdmiring` (shields stay).
        if stop_holding && !m.equipment[OFFHAND].is_empty() && !m.equipment[OFFHAND].has(kiln_item::component::ids::BLOCKS_ATTACKS) {
            stop_holding_offhand(e, m, level, true);
        }
        // `AbstractPiglin.customServerAiStep`: zombification outside the nether.
        let converting = level.piglins_zombify() && state(m).is_some_and(|s| !s.immune_to_zombification) && !m.no_ai;
        let Some(st) = state_mut(m) else { return };
        if converting {
            st.time_in_overworld += 1;
        } else {
            st.time_in_overworld = 0;
        }
        if st.time_in_overworld > CONVERSION_TIME {
            zombify(e, m, level);
        }
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        // `PiglinAi.wasHurtBy`.
        if !hurt || source.attacker.is_none() {
            return;
        }
        if !m.equipment[OFFHAND].is_empty() {
            stop_holding_offhand(e, m, level, false);
        }
        let Some(st) = state_mut(m) else { return };
        st.admiring = 0;
        if source.attacker_is_player {
            st.admiring_disabled = 400;
            st.angry_at = source.attacker;
            st.anger = 600;
        }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        // Not a structure spawn: a baby, or an adult with a weapon.
        if r.next_float() < 0.2 {
            set_baby(e, m, true);
        } else if let Some(s) = ItemStack::of(spawn_weapon(&mut e.random), 1) {
            m.equipment[MAINHAND] = s;
        }
        // `PiglinAi.initMemories`: hunted recently for 30-120 s.
        let hunted = 600 + r.next_int_bounded(1801);
        if let Some(st) = state_mut(m) {
            st.hunted_recently = hunted;
        }
        // `populateDefaultEquipmentSlots`: each gold armor piece at 10%.
        if !m.baby() {
            for (slot, name) in [(mob::HEAD, "minecraft:golden_helmet"), (mob::CHEST, "minecraft:golden_chestplate"), (mob::LEGS, "minecraft:golden_leggings"), (mob::FEET, "minecraft:golden_boots")] {
                if r.next_float() < 0.1
                    && let Some(s) = ItemStack::of(name, 1)
                {
                    m.equipment[slot] = s;
                }
            }
        }
        // `populateDefaultEquipmentEnchantments`: one draw per worn item (weapon 25%, armor 50%
        // of the special multiplier); the enchanting itself is not simulated.
        let _ = !m.equipment[MAINHAND].is_empty() && r.next_float() < 0.25 * ctx.special_multiplier;
        for slot in [mob::FEET, mob::LEGS, mob::CHEST, mob::HEAD] {
            let _ = !m.equipment[slot].is_empty() && r.next_float() < 0.5 * ctx.special_multiplier;
        }
        ext::mob_finalize(m, r);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let baby = r.bool_or("IsBaby", false);
        let cannot_hunt = r.bool_or("CannotHunt", false);
        let immune = r.bool_or("IsImmuneToZombification", false);
        let time = r.int_or("TimeInOverworld", 0);
        let inventory = r.get("Inventory").and_then(Tag::as_list).map(|l| l.iter().filter_map(|t| ItemStack::from_nbt(t).ok()).collect::<Vec<_>>());
        set_baby(e, m, baby);
        let Some(st) = state_mut(m) else { return };
        st.cannot_hunt = cannot_hunt;
        st.immune_to_zombification = immune;
        st.time_in_overworld = time;
        if let Some(items) = inventory {
            st.inventory = vec![ItemStack::empty(); 8];
            for s in items {
                let _ = add_to_inventory(st, s);
            }
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if st.immune_to_zombification {
            o.put("IsImmuneToZombification", Tag::Byte(1));
        }
        o.put("TimeInOverworld", Tag::Int(st.time_in_overworld));
        if m.baby() {
            o.put("IsBaby", Tag::Byte(1));
        }
        if st.cannot_hunt {
            o.put("CannotHunt", Tag::Byte(1));
        }
        o.put("Inventory", Tag::List(st.inventory.iter().filter(|s| !s.is_empty()).map(ItemStack::to_nbt).collect()));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let immune = state(m).is_some_and(|s| s.immune_to_zombification);
        d.set(data::abstract_piglin::IMMUNE_TO_ZOMBIFICATION, &DataValue::Boolean(immune));
        d.set(data::piglin::BABY, &DataValue::Boolean(m.baby()));
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `PiglinAi.mobInteract`: an adult not admiring takes one gold ingot.
        let st = state(m)?;
        if st.admiring_disabled > 0 || st.admiring > 0 || m.baby() || !is_item(stack, BARTERING_ITEM) {
            return None;
        }
        hold_in_offhand(e, m, level, stack.with_count(1));
        admire(m);
        Some(Outcome { success: true, held: HeldChange::Consume(1), shear: None, player_sound: None, ride: false, open_container: false })
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.49, 0.98, 0.78) } else { base }
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        // `checkPiglinSpawnRules`: anywhere but on nether wart blocks.
        Some(crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> crate::memory::MemoryLevel {
        let mut level = crate::memory::MemoryLevel::new(-64, 1);
        let stone = kiln_data::blocks::default_state::STONE;
        for x in -8..=8 {
            for z in -8..=8 {
                level.blocks.insert(BlockPos::new(x, 99, z), stone);
            }
        }
        level
    }

    fn piglin_at(level: &mut crate::memory::MemoryLevel, x: f64, nbt: Tag) -> i32 {
        let mut e = mob::new(MobKind::Piglin, 10, 0, 5);
        e.set_pos(Vec3::new(x, 100.0, 0.5));
        mob::persist::apply_nbt(&mut e, &nbt);
        level.insert(e);
        10
    }

    #[test]
    fn targets_players_without_gold_and_zombifies() {
        let mut level = world();
        let mut p = crate::level::PlayerView::new(1, Vec3::new(4.5, 100.0, 0.5));
        level.players.push(p);
        let id = piglin_at(&mut level, 0.5, Tag::Compound(vec![]));
        for _ in 0..40 {
            level.game_time += 1;
            level.tick();
        }
        let m = mob::data(level.entities().find(|e| e.id == id).unwrap()).unwrap();
        assert_eq!(m.target, Some(1), "goals {:?}", m.running_goals());
        // Gold armor: left alone.
        let mut level = world();
        p.piglin_safe_armor = true;
        level.players.push(p);
        piglin_at(&mut level, 0.5, Tag::Compound(vec![]));
        for _ in 0..320 {
            level.game_time += 1;
            level.tick();
            level.flush_spawned();
        }
        assert!(level.entities().any(|e| e.type_name == "minecraft:zombified_piglin"), "converted after 300 ticks");
        assert!(!level.entities().any(|e| e.type_name == "minecraft:piglin" && !e.is_removed()));
    }

    #[test]
    fn admires_then_barters() {
        let mut level = world();
        let mut p = crate::level::PlayerView::new(1, Vec3::new(3.5, 100.0, 0.5));
        p.piglin_safe_armor = true;
        level.players.push(p);
        let id = piglin_at(&mut level, 0.5, Tag::Compound(vec![("IsImmuneToZombification".into(), Tag::Byte(1))]));
        let who = Interactor { id: 1, creative: false, sneaking: false };
        let gold = ItemStack::of(BARTERING_ITEM, 5).unwrap();
        let mut out = None;
        level.tick_one(0, |e, level| out = Some(crate::mob::interact::interact(e, level, &who, &gold)));
        assert_eq!(out.unwrap().held, HeldChange::Consume(1));
        for t in 0..130 {
            level.game_time += 1;
            level.tick();
            let bartered = level.events.iter().any(|ev| matches!(ev, Event::GiftLoot { table: BARTERING_TABLE, .. }));
            assert_eq!(bartered, t >= ADMIRE_DURATION - 1, "tick {t}");
            if bartered {
                break;
            }
        }
        let m = mob::data(level.entities().find(|e| e.id == id).unwrap()).unwrap();
        assert!(m.equipment[OFFHAND].is_empty());
    }

    #[test]
    fn inventory_fills_and_overflows() {
        let mut st = PiglinState { inventory: vec![ItemStack::empty(); 8], ..PiglinState::default() };
        for _ in 0..8 {
            assert!(add_to_inventory(&mut st, ItemStack::of("minecraft:gold_nugget", 64).unwrap()).is_empty());
        }
        assert!(!can_add_to_inventory(&st, &ItemStack::of("minecraft:gold_nugget", 1).unwrap()));
        assert_eq!(add_to_inventory(&mut st, ItemStack::of("minecraft:gold_nugget", 3).unwrap()).count(), 3);
    }
}
