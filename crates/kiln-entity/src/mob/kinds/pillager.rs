//! Pillager: an illager with a crossbow (`RangedCrossbowAttackGoal`: walks into range, charges
//! the crossbow for 25 ticks, waits 1-2 seconds and shoots an arrow at its target), holds its
//! ground while patrolling, and carries up to five items (banners in raids). Also the mob side
//! of crossbows (`CrossbowItem` charging and `performShooting` for mobs), shared with other
//! crossbow users.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, Wanted, LOOK, MOVE};
use crate::mob::kinds::zombie::{IRON_GOLEM, VILLAGERS};
use crate::mob::kinds::raider::{self, IllagerState};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, OFFHAND, SpawnContext, path};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Pillager;

pub static KIND: Pillager = Pillager;

static INFO: Info = Info {
    sounds: Some("pillager"),
    ..Info::monster(
        "minecraft:pillager",
        &[(MovementSpeed, 0.3499999940395355), (MaxHealth, 24.0), (AttackDamage, 5.0), (FollowRange, 32.0)],
    )
};

fn st(m: &MobData) -> &IllagerState {
    raider::illager(m).expect("pillager state")
}

fn st_mut(m: &mut MobData) -> &mut IllagerState {
    raider::illager_mut(m).expect("pillager state")
}

impl Kind for Pillager {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(IllagerState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        raider::register_raider_goals(m);
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, raider::never());
        g.add(2, Goal::Custom(Box::new(raider::HoldGroundAttackGoal { hostile_radius_sqr: 100.0 })));
        g.add(3, Goal::Custom(Box::new(RangedCrossbowAttackGoal::new(1.0, 8.0))));
        g.add(8, Goal::RandomStroll { speed: 0.6, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::LookAtPlayer { dist: 15.0, probability: 1.0, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(15.0))));
        let t = &mut m.targets;
        t.add(1, raider::hurt_by_ignoring_raiders());
        t.add(2, super::zombie::nearest(Wanted::Player, true));
        t.add(3, super::zombie::nearest(Wanted::Types(VILLAGERS), false));
        t.add(3, super::zombie::nearest(Wanted::Types(IRON_GOLEM), true));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        raider::ai_step_before(e, m, level);
    }

    /// `Raider.updateNoActionTime`: two more every tick, whatever the light.
    fn update_no_action_time(&self, _e: &Entity, m: &mut MobData, _level: &dyn EntityLevel) {
        m.no_action_time += 2;
    }

    fn update_using_item(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        crossbow_use_tick(e, m, level);
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        raider::die(e, m, level, source);
    }

    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        Some(raider::remove_when_far_away(m, dist_sqr))
    }

    fn can_attack(&self, _m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        raider::illager_can_attack(level, t)
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        // `populateDefaultEquipmentSlots`, `populateDefaultEquipmentEnchantments` (the enchanting
        // rolls; enchantment providers are not applied), `enchantSpawnedWeapon`'s 1 in 300.
        m.equipment[MAINHAND] = ItemStack::of("minecraft:crossbow", 1).unwrap_or_else(ItemStack::empty);
        super::zombie::populate_enchantments(m, r, ctx);
        let _ = r.next_int_bounded(300);
        raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        raider::load(m, r);
        if let Some(Tag::List(items)) = r.get("Inventory") {
            st_mut(m).inventory = items.iter().filter_map(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).collect();
        }
        // `readAdditionalSaveData`: a loaded pillager picks up loot.
        m.can_pick_up_loot = true;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        raider::save(m, o);
        let items: Vec<Tag> = st(m).inventory.iter().map(ItemStack::to_nbt).collect();
        o.put("Inventory", Tag::List(items));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        d.set(data::raider::IS_CELEBRATING, &DataValue::Boolean(st(m).raider.celebrating));
        d.set(data::pillager::IS_CHARGING_CROSSBOW, &DataValue::Boolean(st(m).charging));
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn max_spawn_cluster(&self) -> i32 {
        1
    }
}

/// `Raider.applyRaidBuffs` of a pillager: with the raid's enchant odds a fresh crossbow
/// (enchanted after wave 3 and 5 by vanilla's providers; the enchanting is not applied).
pub fn apply_raid_buffs(e: &mut Entity, m: &mut MobData, wave: i32, enchant_odds: f32, normal_groups: i32, easy_groups: i32) {
    if e.random.next_float() <= enchant_odds && wave > easy_groups {
        let _ = normal_groups;
        m.equipment[MAINHAND] = ItemStack::of("minecraft:crossbow", 1).unwrap_or_else(ItemStack::empty);
    }
}

// ---------------------------------------------------------------------- crossbows

/// `ProjectileUtil.getWeaponHoldingHand(mob, CROSSBOW)`: the main hand unless only the off hand
/// holds one.
fn crossbow_slot(m: &MobData) -> usize {
    if mob::item_name(&m.equipment[MAINHAND]) == "minecraft:crossbow" { MAINHAND } else { OFFHAND }
}

/// `isHolding(CROSSBOW)`.
pub fn holding_crossbow(m: &MobData) -> bool {
    [MAINHAND, OFFHAND].iter().any(|&i| mob::item_name(&m.equipment[i]) == "minecraft:crossbow")
}

/// `CrossbowItem.getChargeDuration` (quick charge is not applied).
pub fn charge_duration(_stack: &ItemStack) -> i32 {
    crate::math::floor((1.25f32 * 20.0) as f64)
}

/// `CrossbowItem.isCharged`.
pub fn is_charged(stack: &ItemStack) -> bool {
    stack.get(kiln_item::keys::CHARGED_PROJECTILES).is_some_and(|c| !c.0.is_empty())
}

fn set_charged(stack: &mut ItemStack, arrow: Option<ItemStack>) {
    let list = arrow.iter().map(kiln_item::stack::ItemStackTemplate::from_stack).collect();
    stack.insert(kiln_item::keys::CHARGED_PROJECTILES, kiln_item::component::ChargedProjectiles(list));
}

/// `CrossbowItem.onUseTick` for a mob charging a crossbow `ticks` into the use: the loading
/// sounds and, once fully drawn, one arrow loaded (`tryLoadProjectiles`: a mob always has one).
pub fn crossbow_use_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let slot = crossbow_slot(m);
    if mob::item_name(&m.equipment[slot]) != "minecraft:crossbow" {
        return;
    }
    let duration = charge_duration(&m.equipment[slot]);
    let pct = m.ticks_using_item() as f32 / duration as f32;
    let pos = e.position();
    let sound = |level: &mut dyn EntityLevel, sound: &str, source: &'static str, volume: f32, pitch: f32| {
        level.emit(Event::Sound { pos, sound: mob::sound_event(sound), source, volume, pitch });
    };
    // The start and middle sounds follow the use time (vanilla keeps the played flags on the
    // item, shared by every crossbow user).
    if m.ticks_using_item() == crate::math::floor((duration as f32 * 0.2) as f64).max(1) {
        sound(level, "minecraft:item.crossbow.loading_start", "players", 0.5, 1.0);
    }
    if m.ticks_using_item() == crate::math::ceil((duration as f32 * 0.5) as f64) {
        sound(level, "minecraft:item.crossbow.loading_middle", "players", 0.5, 1.0);
    }
    if pct >= 1.0 && !is_charged(&m.equipment[slot]) {
        let arrow = ItemStack::of("minecraft:arrow", 1);
        set_charged(&mut m.equipment[slot], arrow);
        let pitch = 1.0 / (level.random().next_float() * 0.5 + 1.0) + 0.2;
        sound(level, "minecraft:item.crossbow.loading_end", m.kind.sound_source(), 1.0, pitch);
    }
}

/// `LivingEntity.releaseUsingItem` with a crossbow (`useOnRelease`: one more use tick first).
pub fn release_crossbow(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if m.using_item.is_some() {
        crossbow_use_tick(e, m, level);
        if let Some(t) = m.using_item.as_mut() {
            *t += 1;
        }
    }
    m.stop_using_item();
}

/// JOML's `Vector3f.normalize` (floats, `1 / sqrt`).
fn normalize_f(x: f64, y: f64, z: f64) -> (f32, f32, f32) {
    let (x, y, z) = (x as f32, y as f32, z as f32);
    let len2 = x * x + (y * y + z * z);
    let s = 1.0f32 / (len2 as f64).sqrt() as f32;
    (x * s, y * s, z * s)
}

/// `CrossbowAttackMob.performCrossbowAttack(body, 1.6)` → `CrossbowItem.performShooting`: the
/// loaded arrow flies at the target from the mob's eyes (`MOB_ARROW_POWER`, the difficulty's
/// uncertainty); the crossbow takes a point of wear.
pub fn perform_crossbow_attack(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    let slot = crossbow_slot(m);
    if mob::item_name(&m.equipment[slot]) == "minecraft:crossbow" && is_charged(&m.equipment[slot]) {
        set_charged(&mut m.equipment[slot], None);
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
        let mut arrow = crate::arrow::new(id, 0, "minecraft:arrow", pos, Vec3::ZERO, Some(e.id), seed);
        let dx = t.pos.x - e.x();
        let dz = t.pos.z - e.z();
        let dist = (dx * dx + dz * dz).sqrt();
        let dy = t.pos.y + (t.bb.max_y - t.bb.min_y) * 0.3333333333333333 - arrow.y() + dist * 0.20000000298023224;
        let (x, y, z) = normalize_f(dx, dy, dz);
        let uncertainty = (14 - level.difficulty() as i32 * 4) as f32;
        mob::species::shoot(&mut arrow, x as f64, y as f64, z as f64, 1.6, uncertainty);
        arrow.set_old_pos_and_rot();
        level.add_entity(arrow);
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event("minecraft:item.crossbow.shoot"), source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
        }
        // `hurtAndBreak(1, shooter, hand)`.
        let stack = &mut m.equipment[slot];
        if stack.is_damageable_item() {
            let d = stack.damage() + 1;
            if d >= stack.max_damage() {
                *stack = ItemStack::empty();
                level.emit(Event::EntityEvent { entity: e.id, event: if slot == MAINHAND { 47 } else { 48 } });
            } else {
                stack.insert(kiln_item::keys::DAMAGE, d);
            }
        }
    }
    // `onCrossbowAttackPerformed`.
    m.no_action_time = 0;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrossbowState {
    Uncharged,
    Charging,
    Charged,
    ReadyToAttack,
}

/// `RangedCrossbowAttackGoal`.
#[derive(Clone, Debug)]
pub struct RangedCrossbowAttackGoal {
    state: CrossbowState,
    speed: f64,
    radius_sqr: f32,
    see_time: i32,
    attack_delay: i32,
    update_path_delay: i32,
}

impl RangedCrossbowAttackGoal {
    pub fn new(speed: f64, radius: f32) -> RangedCrossbowAttackGoal {
        RangedCrossbowAttackGoal { state: CrossbowState::Uncharged, speed, radius_sqr: radius * radius, see_time: 0, attack_delay: 0, update_path_delay: 0 }
    }
}

fn valid_target(m: &MobData, level: &dyn EntityLevel) -> Option<Living> {
    goals::target(m, level).filter(|t| t.alive)
}

impl CustomGoal for RangedCrossbowAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RangedCrossbowAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        valid_target(m, level).is_some() && holding_crossbow(m)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        valid_target(m, level).is_some() && (self.can_use(e, m, level) || !m.nav_ref().is_done()) && holding_crossbow(m)
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.set_aggressive(false);
        mob::set_target(e, m, None);
        self.see_time = 0;
        if m.using_item.is_some() {
            m.stop_using_item();
            st_mut_any(m).charging = false;
            let slot = crossbow_slot(m);
            set_charged(&mut m.equipment[slot], None);
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        let sees = mob::has_line_of_sight_cached(e, m, level, &t);
        if sees != (self.see_time > 0) {
            self.see_time = 0;
        }
        if sees {
            self.see_time += 1;
        } else {
            self.see_time -= 1;
        }
        let d = e.position().distance_to_sqr(t.pos);
        let needs_to_move = (d > self.radius_sqr as f64 || self.see_time < 5) && self.attack_delay == 0;
        if needs_to_move {
            self.update_path_delay -= 1;
            if self.update_path_delay <= 0 {
                let speed = if self.state == CrossbowState::Uncharged { self.speed } else { self.speed * 0.5 };
                path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), speed);
                // `TimeUtil.rangeOfSeconds(1, 2).sample`.
                self.update_path_delay = 20 + e.random.next_int_bounded(21);
            }
        } else {
            self.update_path_delay = 0;
            m.nav_mut().stop();
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        match self.state {
            CrossbowState::Uncharged => {
                if !needs_to_move {
                    m.start_using_item();
                    self.state = CrossbowState::Charging;
                    st_mut_any(m).charging = true;
                }
            }
            CrossbowState::Charging => {
                if m.using_item.is_none() {
                    self.state = CrossbowState::Uncharged;
                }
                let slot = crossbow_slot(m);
                if m.ticks_using_item() >= charge_duration(&m.equipment[slot]) {
                    release_crossbow(e, m, level);
                    self.state = CrossbowState::Charged;
                    self.attack_delay = 20 + e.random.next_int_bounded(20);
                    st_mut_any(m).charging = false;
                }
            }
            CrossbowState::Charged => {
                self.attack_delay -= 1;
                if self.attack_delay == 0 {
                    self.state = CrossbowState::ReadyToAttack;
                }
            }
            CrossbowState::ReadyToAttack => {
                if sees {
                    perform_crossbow_attack(e, m, level, &t);
                    self.state = CrossbowState::Uncharged;
                }
            }
        }
    }
}

/// The charging flag of any crossbow user with an illager state.
fn st_mut_any(m: &mut MobData) -> &mut IllagerState {
    st_mut(m)
}
