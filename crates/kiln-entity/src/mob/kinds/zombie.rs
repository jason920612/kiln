//! What the zombie family shares (vanilla's `Zombie`, which husks, drowned, zombie villagers and
//! zombified piglins extend): the goals, the drowning `ConversionTracker`, the baby flag,
//! `finalizeSpawn` and `handleAttributes`, and the reinforcements a zombie calls on hard
//! difficulty. The plain zombie itself lives in the shared mob code and calls in here.

use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{Aabb, BlockPos};
use crate::mob::attributes::{Attr, Op};
use crate::mob::goals::{Goal, MeleeKind, Wanted};
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext, Species, mth};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `AbstractVillager` (villagers and wandering traders).
pub const VILLAGERS: &[&str] = &["minecraft:villager", "minecraft:wandering_trader"];
/// `IronGolem`.
pub const IRON_GOLEM: &[&str] = &["minecraft:iron_golem"];
/// `AbstractPiglin`.
pub const PIGLINS: &[&str] = &["minecraft:piglin", "minecraft:piglin_brute"];

/// `ConversionTracker`: time spent afflicted (in water, in powder snow), then a countdown to the
/// conversion.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Tracker {
    pub afflicted: i32,
    pub conversion: i32,
    /// The type's `DATA_*_CONVERSION_ID`.
    pub converting: bool,
}

impl Tracker {
    /// `tick` for a living mob with AI: true when the conversion is due.
    pub fn tick(&mut self, afflicted: bool, total_affliction: i32, total_conversion: i32) -> bool {
        if afflicted {
            if self.converting {
                self.conversion -= 1;
                if self.conversion < 0 {
                    return true;
                }
            } else {
                self.afflicted += 1;
                if self.afflicted >= total_affliction {
                    self.start(total_conversion);
                }
            }
        } else {
            self.afflicted = -1;
            self.converting = false;
        }
        false
    }

    /// `startConversion`.
    pub fn start(&mut self, t: i32) {
        self.conversion = t;
        self.converting = true;
    }

    pub fn save(&self, o: &mut Output, afflicted_now: bool, affliction_tag: &str, conversion_tag: &str) {
        o.put(conversion_tag, Tag::Int(if self.converting { self.conversion } else { -1 }));
        o.put(affliction_tag, Tag::Int(if afflicted_now { self.afflicted } else { -1 }));
    }

    pub fn load(&mut self, r: &mut Input, affliction_tag: &'static str, conversion_tag: &'static str) {
        self.afflicted = r.int_or(affliction_tag, 0);
        let t = r.int_or(conversion_tag, -1);
        if t != -1 {
            self.start(t);
        } else {
            self.converting = false;
        }
    }
}

/// The zombie fields of an extension zombie type's state.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ZombieState {
    pub can_break_doors: bool,
    pub drowning: Tracker,
}

/// Extension zombie types' states hold a [`ZombieState`].
pub trait HasZombie: 'static {
    fn zombie(&self) -> &ZombieState;
    fn zombie_mut(&mut self) -> &mut ZombieState;
}

impl HasZombie for ZombieState {
    fn zombie(&self) -> &ZombieState {
        self
    }
    fn zombie_mut(&mut self) -> &mut ZombieState {
        self
    }
}

fn with_zombie<R>(m: &mut MobData, f: impl FnOnce(&mut bool, &mut Tracker) -> R) -> Option<R> {
    if let Species::Zombie { can_break_doors, drowning } = &mut m.species {
        return Some(f(can_break_doors, drowning));
    }
    let z = ext_zombie_mut(m)?;
    Some(f(&mut z.can_break_doors, &mut z.drowning))
}

fn ext_zombie_mut(m: &mut MobData) -> Option<&mut ZombieState> {
    let Species::Ext(s) = &mut m.species else { return None };
    let any = (**s).as_any_mut();
    if any.is::<ZombieState>() {
        return any.downcast_mut::<ZombieState>();
    }
    if any.is::<super::drowned::DrownedState>() {
        return any.downcast_mut::<super::drowned::DrownedState>().map(|s| s.zombie_mut());
    }
    if any.is::<super::zombie_villager::ZombieVillagerState>() {
        return any.downcast_mut::<super::zombie_villager::ZombieVillagerState>().map(|s| s.zombie_mut());
    }
    if any.is::<super::zombified_piglin::PiglinState>() {
        return any.downcast_mut::<super::zombified_piglin::PiglinState>().map(|s| s.zombie_mut());
    }
    None
}

/// The zombie fields of any zombie type (a copy).
pub fn zombie_of(m: &MobData) -> Option<ZombieState> {
    match &m.species {
        Species::Zombie { can_break_doors, drowning } => Some(ZombieState { can_break_doors: *can_break_doors, drowning: *drowning }),
        Species::Ext(s) => {
            let any = (**s).as_any();
            if let Some(z) = any.downcast_ref::<ZombieState>() {
                return Some(*z);
            }
            if let Some(s) = any.downcast_ref::<super::drowned::DrownedState>() {
                return Some(*s.zombie());
            }
            if let Some(s) = any.downcast_ref::<super::zombie_villager::ZombieVillagerState>() {
                return Some(*s.zombie());
            }
            any.downcast_ref::<super::zombified_piglin::PiglinState>().map(|s| *s.zombie())
        }
        _ => None,
    }
}

/// `canBreakDoors`.
pub fn can_break_doors(m: &MobData) -> bool {
    zombie_of(m).is_some_and(|z| z.can_break_doors)
}

/// `setCanBreakDoors`: ground navigation opens doors (the break-door goal is not simulated:
/// Kiln has no door breaking yet).
pub fn set_can_break_doors(m: &mut MobData, on: bool) {
    // Every zombie type's navigation `canNavigateGround` (amphibious navigation too).
    with_zombie(m, |d, _| *d = on);
    m.nav.can_open_doors = on;
}

/// `setBaby`: the flag and the speed bonus (dimensions follow).
pub fn set_baby(e: &mut Entity, m: &mut MobData, baby: bool) {
    m.zombie_baby = baby;
    m.attrs.remove_modifier(Attr::MovementSpeed, "minecraft:baby");
    if baby {
        m.attrs.set_modifier(Attr::MovementSpeed, "minecraft:baby", 0.5, Op::AddMultipliedBase);
    }
    mob::refresh_dimensions(e, m);
}

/// `Zombie.BABY_DIMENSIONS` and the types' own (eye heights differ).
pub fn baby_dimensions(kind: MobKind) -> (f32, f32, f32) {
    let eye = match kind {
        MobKind::Husk => 0.825,
        MobKind::ZombieVillager => 0.67,
        MobKind::ZombifiedPiglin => 0.78,
        _ => 0.775,
    };
    (0.49, 0.98, eye)
}

// ---------------------------------------------------------------------- goals

fn melee(speed: f64) -> Goal {
    Goal::Melee { kind: MeleeKind::Zombie, speed, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: crate::math::Vec3::ZERO, raise_arm: 0 }
}

pub fn nearest(wanted: Wanted, must_see: bool) -> Goal {
    Goal::NearestAttackable { wanted, interval: mth::reduced_tick_delay(10), must_see, target: None, unseen: 0, spider: false }
}

pub fn hurt_by(alert: bool) -> Goal {
    Goal::HurtByTarget { timestamp: 0, alert_others: alert, target_mob: None, unseen: 0, unseen_memory: 60 }
}

pub fn stroll(speed: f64, water_avoiding: bool) -> Goal {
    Goal::RandomStroll { speed, interval: 120, check_no_action: true, water_avoiding: water_avoiding.then_some(0.001), wanted: crate::math::Vec3::ZERO, force: false }
}

/// `Zombie.registerGoals` before `addBehaviourGoals`.
pub fn register_base_goals(m: &mut MobData) {
    let g = &mut m.goals;
    g.add(4, Goal::RemoveTurtleEgg { next_start: 0, block: BlockPos::default(), try_ticks: 0, max_stay: 0, reached: false, since_reached: 0 });
    g.add(8, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
    g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
}

/// `Zombie.registerGoals` with `Zombie.addBehaviourGoals` (zombies, husks, zombie villagers).
pub fn register_goals(m: &mut MobData) {
    register_base_goals(m);
    let g = &mut m.goals;
    // `MoveThroughVillageGoal` (no villages) never starts.
    g.add(2, Goal::Custom(Box::new(super::spear_use::SpearUseGoal::new(1.0, 1.0, 10.0, 2.0))));
    g.add(3, melee(1.0));
    g.add(6, Goal::Never);
    g.add(7, stroll(1.0, true));
    let t = &mut m.targets;
    t.add(1, hurt_by(true));
    t.add(2, nearest(Wanted::Player, true));
    t.add(3, nearest(Wanted::Types(VILLAGERS), false));
    t.add(3, nearest(Wanted::Types(IRON_GOLEM), true));
    // Baby turtles on land (turtles are not simulated).
    t.add(5, nearest(Wanted::BabyTurtlesOnLand, true));
}

// ---------------------------------------------------------------------- tick

/// `Zombie.tick`'s drowning tracker (after `Mob.tick`) for types that `convertsInWater`:
/// zombies become drowned, husks become zombies.
pub fn tick_drowning(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let (to, sound) = match m.kind {
        MobKind::Zombie => (MobKind::Drowned, 1040),
        MobKind::Husk => (MobKind::Zombie, 1041),
        _ => return,
    };
    if !mob::is_alive(e, m) || m.no_ai {
        return;
    }
    let eye = e.fluid.is_eye_in_water();
    let due = with_zombie(m, |_, t| t.tick(eye, 600, 300)).unwrap_or(false);
    if due {
        mob::convert::convert_to(e, m, level, to, true, true, |ne, nm, level| {
            // `handleAttributes(specialMultiplier, CONVERSION)` from the new mob's random.
            let special = special_multiplier(level.effective_difficulty(ne.block_position()));
            handle_attributes(ne, nm, special, true);
            if !ne.silent {
                level.emit(Event::LevelEvent { event: sound, pos: ne.block_position(), data: 0 });
            }
        });
    }
}

/// `DifficultyInstance.getSpecialMultiplier` from the effective difficulty.
pub fn special_multiplier(effective: f32) -> f32 {
    if effective < 2.0 {
        0.0
    } else if effective > 4.0 {
        1.0
    } else {
        (effective - 2.0) / 2.0
    }
}

// ---------------------------------------------------------------------- spawning

/// `Zombie.finalizeSpawn` (draws from `r`, the level's random; `handleAttributes` from the
/// mob's own). `conversion`: `EntitySpawnReason.CONVERSION`.
pub fn finalize(e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData, conversion: bool) {
    super::super::ext::mob_finalize(m, r);
    let special = ctx.special_multiplier;
    if !conversion {
        m.can_pick_up_loot = r.next_float() < 0.55 * special;
    }
    // (`new ZombieGroupData(getSpawnAsBabyOdds(random), true)` for the first of a group.)
    if group.zombie_baby.is_none() {
        group.zombie_baby = Some(r.next_float() < 0.05);
        group.zombie_can_jockey = true;
    }
    let baby = group.zombie_baby.unwrap_or(false);
    if baby {
        set_baby(e, m, true);
        if group.zombie_can_jockey {
            // Chicken jockeys: with 5% an unridden chicken nearby is ridden (the caller looks
            // for it), else with 5% a new chicken carries it.
            if (r.next_float() as f64) < 0.05 {
                group.nearby_chicken = true;
            } else if (r.next_float() as f64) < 0.05 {
                let mut chicken = mob::new_jockey(e, MobKind::Chicken);
                mob::finalize_spawn(&mut chicken, r, ctx, &mut GroupData::default(), false);
                if let Some(cm) = mob::data_mut(&mut chicken) {
                    cm.chicken_jockey = true;
                }
                group.companions.push(mob::Companion { entity: chicken, seat: mob::Seat::UnderMob });
            }
        }
    }
    let doors = r.next_float() < special * 0.1;
    set_can_break_doors(m, doors);
    if !conversion {
        populate_equipment(m, r, ctx);
        populate_enchantments(m, r, ctx);
    }
    if ctx.halloween && m.equipment[mob::HEAD].is_empty() && r.next_float() < 0.25 {
        let name = if r.next_float() < 0.1 { "minecraft:jack_o_lantern" } else { "minecraft:carved_pumpkin" };
        if let Some(s) = ItemStack::of(name, 1) {
            m.equipment[mob::HEAD] = s;
            m.drop_chances[mob::HEAD] = 0.0;
        }
    }
    handle_attributes(e, m, special, conversion);
}

/// `populateDefaultEquipmentSlots` of the zombie types.
fn populate_equipment(m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext) {
    match m.kind {
        MobKind::Drowned => {
            // No armor, no zombie weapon.
            if (r.next_float() as f64) > 0.9 {
                let name = if r.next_int_bounded(16) < 10 { "minecraft:trident" } else { "minecraft:fishing_rod" };
                set_item(m, mob::MAINHAND, name);
            }
        }
        MobKind::ZombifiedPiglin => {
            let name = if r.next_int_bounded(20) == 0 { "minecraft:golden_spear" } else { "minecraft:golden_sword" };
            set_item(m, mob::MAINHAND, name);
        }
        _ => {
            populate_armor(m, r, ctx);
            if r.next_float() < if ctx.hard { 0.05 } else { 0.01 } {
                let name = match r.next_int_bounded(6) {
                    0 => "minecraft:iron_sword",
                    1 => "minecraft:iron_spear",
                    _ => "minecraft:iron_shovel",
                };
                set_item(m, mob::MAINHAND, name);
            }
        }
    }
}

fn set_item(m: &mut MobData, slot: usize, name: &str) {
    if let Some(s) = ItemStack::of(name, 1) {
        m.equipment[slot] = s;
    }
}

/// `Mob.populateDefaultEquipmentSlots`: a random armor set (leather to diamond).
pub fn populate_armor(m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext) {
    if r.next_float() < 0.15 * ctx.special_multiplier {
        let mut tier = r.next_int_bounded(3);
        for _ in 1..=3 {
            if r.next_float() < 0.1087 {
                tier += 1;
            }
        }
        let stop = if ctx.hard { 0.1 } else { 0.25 };
        let mut first = true;
        for slot in [mob::HEAD, mob::CHEST, mob::LEGS, mob::FEET] {
            if !first && r.next_float() < stop {
                break;
            }
            first = false;
            if m.equipment[slot].is_empty() {
                let material = ["leather", "copper", "golden", "chainmail", "iron", "diamond"].get(tier as usize).copied();
                let piece = ["boots", "leggings", "chestplate", "helmet"][slot - mob::FEET];
                if let Some(material) = material {
                    set_item(m, slot, &format!("minecraft:{material}_{piece}"));
                }
            }
        }
    }
}

/// `Mob.populateDefaultEquipmentEnchantments`: `enchantSpawnedWeapon`, then each armor slot in
/// `EquipmentSlot` order, each a roll (only for a filled slot) of 25% (weapon) or 50% (armor)
/// of the special multiplier, a success enchanting the item from `minecraft:mob_spawn_equipment`
/// ([`crate::enchanting`]).
pub fn populate_enchantments(m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext) {
    enchant_spawned_weapon(m, r, ctx);
    for slot in [mob::FEET, mob::LEGS, mob::CHEST, mob::HEAD] {
        crate::enchanting::enchant_spawned_equipment(&mut m.equipment[slot], 0.5, ctx.special_multiplier, r);
    }
}

/// `Mob.enchantSpawnedWeapon` (a pillager's also gives a crossbow piercing in 1 of 300 cases).
pub fn enchant_spawned_weapon(m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext) {
    crate::enchanting::enchant_spawned_equipment(&mut m.equipment[mob::MAINHAND], 0.25, ctx.special_multiplier, r);
    if m.kind == MobKind::Pillager && r.next_int_bounded(300) == 0 && m.equipment[mob::MAINHAND].item_name() == "minecraft:crossbow" {
        crate::enchanting::enchant_from_provider(&mut m.equipment[mob::MAINHAND], "minecraft:pillager_spawn_crossbow", ctx.special_multiplier, r);
    }
}

/// `Zombie.handleAttributes` (from the mob's own random; zombified piglins do not randomize
/// their reinforcement chance).
pub fn handle_attributes(e: &mut Entity, m: &mut MobData, special: f32, conversion: bool) {
    let rr = &mut e.random;
    if m.kind == MobKind::ZombifiedPiglin {
        if let Some(i) = m.attrs.get_mut(Attr::SpawnReinforcements) {
            i.base = 0.0;
        }
    } else {
        let reinf = rr.next_double() * 0.10000000149011612;
        if let Some(i) = m.attrs.get_mut(Attr::SpawnReinforcements) {
            i.base = reinf;
        }
    }
    let kb = rr.next_double() * 0.05000000074505806;
    m.attrs.set_modifier(Attr::KnockbackResistance, "minecraft:random_spawn_bonus", kb, Op::AddValue);
    let d = rr.next_double() * 1.5 * special as f64;
    if d > 1.0 {
        m.attrs.set_modifier(Attr::FollowRange, "minecraft:zombie_random_spawn_bonus", d, Op::AddMultipliedTotal);
    }
    if rr.next_float() < special * 0.05 {
        let a = rr.next_double() * 0.25 + 0.5;
        m.attrs.set_modifier(Attr::SpawnReinforcements, "minecraft:leader_zombie_bonus", a, Op::AddValue);
        let b = rr.next_double() * 3.0 + 1.0;
        m.attrs.set_modifier(Attr::MaxHealth, "minecraft:leader_zombie_bonus", b, Op::AddMultipliedTotal);
        if !conversion {
            m.health = m.max_health();
        }
        set_can_break_doors(m, true);
    }
}

// ---------------------------------------------------------------------- reinforcements

/// `Zombie.hurtServer` after a hit that landed: on hard difficulty, with the reinforcement
/// chance, a zombie of the same type appears 7 to 40 blocks away (up to 50 tries of the spawn
/// placement and rules) and goes after the target.
pub fn reinforcements(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
    let target = m.target.or_else(|| source.attacker.filter(|a| mob::goals::living(level, *a).is_some()));
    let Some(target) = target else { return };
    if level.difficulty() != 3 {
        return;
    }
    if (e.random.next_float() as f64) >= m.attrs.value(Attr::SpawnReinforcements) {
        return;
    }
    let (x, y, z) = (mth_floor(e.x()), mth_floor(e.y()), mth_floor(e.z()));
    let kind = m.kind;
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let uuid = (seed as u64 as u128) << 64 | 0x5a5a;
    let mut ne = mob::new(kind, id, uuid, seed);
    for _ in 0..50 {
        let mut off = || {
            let d = mth::next_int_between(&mut e.random, 7, 40);
            d * mth::next_int_between(&mut e.random, -1, 1)
        };
        let (rx, ry, rz) = (x + off(), y + off(), z + off());
        let pos = BlockPos::new(rx, ry, rz);
        if !spawn_position_ok(kind, level, pos) || !spawn_rules_ok(kind, level, pos) {
            continue;
        }
        ne.set_pos(crate::math::Vec3::new(rx as f64, ry as f64, rz as f64));
        let near_area = crate::math::Aabb::new(rx as f64 - 7.0, ry as f64 - 7.0, rz as f64 - 7.0, rx as f64 + 7.0, ry as f64 + 7.0, rz as f64 + 7.0);
        let near_player = level.players_in(&near_area).iter().any(|p| !p.spectator && p.alive && p.pos.distance_to_sqr(ne.position()) < 49.0);
        let ctx = ne.collision_context();
        let bb = ne.bounding_box();
        let free = crate::collision::no_collision(level, &ctx, ne.id, &bb);
        let liquid = kind != MobKind::Drowned && contains_any_liquid(level, &bb);
        if near_player || !free || liquid {
            continue;
        }
        ne.set_old_pos_and_rot();
        let mut nm = mob::take(&mut ne);
        mob::set_target(&mut ne, &mut nm, Some(target));
        mob::put(&mut ne, nm);
        let eff = level.effective_difficulty(ne.block_position());
        let sctx = SpawnContext { biome: None, moon_brightness: 1.0, special_multiplier: special_multiplier(eff), effective_difficulty: eff, hard: true, halloween: false };
        mob::finalize_spawn(&mut ne, level.random(), &sctx, &mut GroupData::default(), false);
        if let Some(nm) = mob::data_mut(&mut ne) {
            callee_charge(nm);
        }
        level.add_entity(ne);
        m.attrs.get_mut(Attr::SpawnReinforcements).map(|i| {
            let old = i.modifiers.iter().find(|md| md.id == "minecraft:reinforcement_caller_charge").map_or(0.0, |md| md.amount);
            i.modifiers.retain(|md| md.id != "minecraft:reinforcement_caller_charge");
            i.modifiers.push(crate::mob::attributes::Modifier { id: "minecraft:reinforcement_caller_charge".into(), amount: old - 0.05, op: Op::AddValue });
        });
        return;
    }
}

/// `ZOMBIE_REINFORCEMENT_CALLEE_CHARGE` on a new reinforcement.
fn callee_charge(m: &mut MobData) {
    m.attrs.set_modifier(Attr::SpawnReinforcements, "minecraft:reinforcement_callee_charge", -0.05000000074505806, Op::AddValue);
}

fn mth_floor(v: f64) -> i32 {
    crate::math::floor(v)
}

/// `SpawnPlacements.isSpawnPositionOk` for the zombie types (on the ground; drowned in water).
fn spawn_position_ok(kind: MobKind, level: &dyn EntityLevel, pos: BlockPos) -> bool {
    if kind == MobKind::Drowned {
        let water = crate::physics::fluid_state(level.block(pos)).kind.is_water();
        return water && !crate::mob::path::collision_full_block(level.block(pos.above()));
    }
    crate::mob::path::valid_spawn(level.block(pos.below()), false)
        && crate::mob::path::valid_empty_spawn(level.block(pos), false)
        && crate::mob::path::valid_empty_spawn(level.block(pos.above()), false)
}

/// `SpawnPlacements.checkSpawnRules` with `EntitySpawnReason.REINFORCEMENT` (the level's
/// random): monsters need the dark; drowned need water below and here.
fn spawn_rules_ok(kind: MobKind, level: &mut dyn EntityLevel, pos: BlockPos) -> bool {
    match kind {
        MobKind::ZombifiedPiglin => {
            level.difficulty() != 0 && crate::blocks::block_name(level.block(pos.below())) != "minecraft:nether_wart_block"
        }
        MobKind::Drowned => {
            let water = |level: &dyn EntityLevel, p: BlockPos| crate::physics::fluid_state(level.block(p)).kind.is_water();
            if !water(level, pos.below()) {
                return false;
            }
            level.difficulty() != 0 && dark_enough(level, pos) && water(level, pos)
        }
        _ => level.difficulty() != 0 && dark_enough(level, pos) && crate::mob::path::valid_spawn(level.block(pos.below()), false),
    }
}

/// `Monster.isDarkEnoughToSpawn` (overworld limits: no block light, raw brightness up to a
/// uniform 0..7).
pub fn dark_enough(level: &mut dyn EntityLevel, pos: BlockPos) -> bool {
    let sky = level.sky_light(pos);
    if sky > level.random().next_int_bounded(32) {
        return false;
    }
    if level.raw_brightness(pos, 15) > 0 {
        return false;
    }
    let light = level.raw_brightness(pos, level.sky_darken());
    light <= level.random().next_int_bounded(8)
}

fn contains_any_liquid(level: &dyn EntityLevel, b: &Aabb) -> bool {
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::ceil(b.max_x), crate::math::ceil(b.max_y), crate::math::ceil(b.max_z));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if !crate::physics::fluid_state(level.block(BlockPos::new(x, y, z))).is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------- persistence

/// `Zombie.readAdditionalSaveData` for an extension zombie type.
pub fn load(e: &mut Entity, m: &mut MobData, r: &mut Input) {
    let baby = r.bool_or("IsBaby", false);
    set_baby(e, m, baby);
    let doors = r.bool_or("CanBreakDoors", false);
    set_can_break_doors(m, doors);
    with_zombie(m, |_, t| t.load(r, "InWaterTime", "DrownedConversionTime"));
}

/// `Zombie.addAdditionalSaveData` for an extension zombie type.
pub fn save(e: &Entity, m: &MobData, o: &mut Output) {
    o.put("IsBaby", Tag::Byte(m.zombie_baby as i8));
    o.put("CanBreakDoors", Tag::Byte(can_break_doors(m) as i8));
    let eye = e.fluid.is_eye_in_water();
    zombie_of(m).unwrap_or_default().drowning.save(o, eye, "InWaterTime", "DrownedConversionTime");
}

/// `Zombie`'s entity data: the baby flag and the drowning conversion (shaking).
pub fn entity_data(_e: &Entity, m: &MobData, d: &mut kiln_proto::packets::entity::EntityData) {
    use kiln_data::entities::data::zombie;
    use kiln_proto::packets::entity::DataValue;
    if m.zombie_baby {
        d.set(zombie::BABY, &DataValue::Boolean(true));
    }
    if zombie_of(m).is_some_and(|z| z.drowning.converting) {
        d.set(zombie::DROWNED_CONVERSION, &DataValue::Boolean(true));
    }
}

/// `Monster.checkMonsterSpawnRules` for natural spawning: not peaceful, dark enough
/// (`isDarkEnoughToSpawn`, two draws), a valid spawn block below.
pub fn monster_rules(view: &dyn crate::mob::ext::SpawnView, pos: BlockPos, r: &mut kiln_javamath::random::LegacyRandom) -> bool {
    view.difficulty() != 0 && dark_enough_view(view, pos, r) && crate::mob::path::valid_spawn(view.block(pos.below()), false)
}

/// `Monster.isDarkEnoughToSpawn` over a [`SpawnView`](crate::mob::ext::SpawnView).
pub fn dark_enough_view(view: &dyn crate::mob::ext::SpawnView, pos: BlockPos, r: &mut kiln_javamath::random::LegacyRandom) -> bool {
    if view.sky_light(pos) > r.next_int_bounded(32) {
        return false;
    }
    if view.block_light(pos) > 0 {
        return false;
    }
    view.raw_brightness(pos, view.sky_darken()) <= r.next_int_bounded(8)
}
