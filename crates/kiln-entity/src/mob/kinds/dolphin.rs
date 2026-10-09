//! Dolphin (`Dolphin`, an `AgeableWaterCreature`): it swims with `SmoothSwimmingMoveControl`, holds its
//! breath for 4800 ticks and goes up for air below 140, dries out on land (2400 ticks of moistness), jumps
//! out of the water, plays with items it picks up in its mouth, follows boats, swims with swimming players
//! (dolphin's grace) and, fed a fish, leads to the nearest ocean ruin or shipwreck.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::{self, Goal, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::kinds::common_a::{self, Avoid, AvoidEntityGoal};
use crate::mob::kinds::fish::RandomSwimmingGoal;
use crate::mob::kinds::axolotl::{smooth_swim_look, smooth_swim_move};
use crate::mob::{self, Category, DamageSource, GroupData, MobData, MobKind, SpawnContext, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Dolphin;

pub static KIND: Dolphin = Dolphin;

/// `AgeableWaterCreature` on `Mob.createMobAttributes` (health 10, speed 1.2, attack damage 3); the head turns
/// one degree at most (`getMaxHeadXRot`, `getMaxHeadYRot`).
static INFO: Info = Info {
    category: Category::WaterCreature,
    ageable: true,
    head: (1, 1, 10),
    ambient_interval: 120,
    ..Info::misc("minecraft:dolphin", &[(MaxHealth, 10.0), (MovementSpeed, 1.2000000476837158), (AttackDamage, 3.0)])
};

/// `getMaxAirSupply`.
const TOTAL_AIR_SUPPLY: i32 = 4800;
/// `TOTAL_MOISTNESS_LEVEL`.
const TOTAL_MOISTNESS: i32 = 2400;

#[derive(Clone, Debug)]
pub struct State {
    /// `GOT_FISH`: it was fed a fish and wants to show the way to the treasure.
    pub got_fish: bool,
    /// `MOISTNESS_LEVEL`.
    pub moistness: i32,
    /// `treasurePos`.
    pub treasure: Option<BlockPos>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("dolphin state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("dolphin state")
}

pub fn got_fish(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.got_fish)
}

/// `Entity.isInWaterOrRain`.
fn in_water_or_rain(e: &Entity, level: &dyn EntityLevel) -> bool {
    e.is_in_water() || level.is_raining_at(e.block_position()) || level.is_raining_at(BlockPos::containing(e.x(), e.y() + e.height as f64, e.z()))
}

fn water_at(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::physics::fluid_state(level.block(p)).kind.is_water()
}

/// `Direction.fromYRot(yRot)`: the horizontal step (x, z) of the way the entity faces.
fn facing_step(y_rot: f32) -> (i32, i32) {
    match ((y_rot as f64 / 90.0 + 0.5).floor() as i32) & 3 {
        0 => (0, 1),
        1 => (-1, 0),
        2 => (0, -1),
        _ => (1, 0),
    }
}

/// `Vec3i.closerToCenterThan(position, distance)`.
fn closer_to_center_than(p: BlockPos, pos: Vec3, distance: f64) -> bool {
    let (dx, dy, dz) = (pos.x - (p.x as f64 + 0.5), pos.y - (p.y as f64 + 0.5), pos.z - (p.z as f64 + 0.5));
    dx * dx + dy * dy + dz * dz < distance * distance
}

/// `Dolphin.closeToNextPos`.
fn close_to_next_pos(e: &Entity, m: &MobData) -> bool {
    m.nav.target_pos.is_some_and(|p| closer_to_center_than(p, e.position(), 12.0))
}

/// `Dolphin.ALLOWED_ITEMS`: an item entity in the water that can be picked up.
fn allowed_items(level: &dyn EntityLevel, e: &Entity) -> Vec<i32> {
    let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
    level
        .entities_in(&area, EntityFilter::Item, e.id)
        .into_iter()
        .filter(|&id| {
            level.entity(id).is_some_and(|o| match &o.kind {
                EntityKind::Item(d) => d.pickup_delay <= 0 && o.is_alive() && o.is_in_water(),
                _ => false,
            })
        })
        .collect()
}

/// `Mob.setItemSlot(MAINHAND, ...)` with the drop chance `setGuaranteedDrop` leaves.
fn hold(m: &mut MobData, stack: ItemStack) {
    m.equipment[mob::MAINHAND] = stack;
    m.drop_chances[mob::MAINHAND] = 2.0;
}

impl Kind for Dolphin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructors: water costs nothing (`AgeableWaterCreature`), water-bound navigation, the loot pick-up.
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::Water, 0.0));
        m.nav.water_bound = true;
        m.nav.allow_breaching = true;
        m.can_pick_up_loot = true;
        m.air_supply_max = TOTAL_AIR_SUPPLY;
        Some(Box::new(State { got_fish: false, moistness: TOTAL_MOISTNESS, treasure: None }))
    }

    fn register_goals(&self, m: &mut MobData) {
        m.targets.add(1, Goal::Custom(Box::new(HurtByNotGuardians { inner: common_a::hurt_by(true) })));
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(BreathAirGoal)));
        g.add(0, Goal::Custom(Box::new(TryFindWaterGoal)));
        g.add(1, Goal::Custom(Box::new(SwimToTreasureGoal { stuck: false })));
        g.add(2, Goal::Custom(Box::new(SwimWithPlayerGoal { speed: 4.0, player: None })));
        g.add(4, Goal::Custom(Box::new(RandomSwimmingGoal { name: "RandomSwimmingGoal", speed: 1.0, interval: 10, wanted: Vec3::ZERO })));
        g.add(4, common_a::look_around());
        g.add(5, common_a::look(6.0));
        g.add(5, Goal::Custom(Box::new(JumpGoal { interval: mth::reduced_tick_delay(10), breached: false })));
        g.add(6, common_a::melee(1.2000000476837158, true));
        g.add(7, Goal::Custom(Box::new(MoveToItemGoal { cooldown: 0 })));
        g.add(8, Goal::Custom(Box::new(PlayWithItemsGoal)));
        g.add(8, Goal::Custom(Box::new(FollowPlayerRiddenEntityGoal::new(Followed::Boats))));
        g.add(8, Goal::Custom(Box::new(FollowPlayerRiddenEntityGoal::new(Followed::Nautilus))));
        g.add(9, Goal::Custom(Box::new(AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Types(&["minecraft:guardian", "minecraft:elder_guardian"]), 8.0, 1.0, 1.0))));
    }

    /// `Dolphin.finalizeSpawn`: a full breath, level, and the group's babies at one in ten.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        e.air_supply = TOTAL_AIR_SUPPLY;
        e.set_x_rot(0.0);
        ext::ageable_finalize(e, m, r, group, 0.1);
        ext::mob_finalize(m, r);
    }

    /// `Dolphin.tick` after `Mob.tick`: moistness, flopping on land.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if m.no_ai {
            e.air_supply = TOTAL_AIR_SUPPLY;
            return;
        }
        if in_water_or_rain(e, &*level) {
            st_mut(m).moistness = TOTAL_MOISTNESS;
        } else {
            let left = st(m).moistness - 1;
            st_mut(m).moistness = left;
            if left <= 0 {
                mob::hurt(e, m, level, DamageSource::of(DamageKind::DryOut), 1.0);
            }
            if e.on_ground {
                let x = (e.random.next_float() * 2.0 - 1.0) * 0.2;
                let z = (e.random.next_float() * 2.0 - 1.0) * 0.2;
                e.delta = e.delta.add(x as f64, 0.5, z as f64);
                e.y_rot = e.random.next_float() * 360.0;
                e.set_on_ground(&*level, false);
                e.needs_sync = true;
            }
        }
    }

    /// `Mob.aiStep`'s item pickup with `Dolphin.pickUpItem`: the whole stack into its mouth.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !m.can_pick_up_loot || !mob::is_alive(e, m) || !level.mob_griefing() {
            return;
        }
        let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
        for id in level.entities_in(&area, EntityFilter::Item, e.id) {
            let Some(item) = level.entity(id) else { continue };
            let EntityKind::Item(d) = &item.kind else { continue };
            if item.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 {
                continue;
            }
            // `pickUpItem`: only while the mouth is empty.
            if !m.equipment[mob::MAINHAND].is_empty() {
                continue;
            }
            let Some(item) = level.entity_mut(id) else { continue };
            let EntityKind::Item(d) = &mut item.kind else { continue };
            let stack = std::mem::replace(&mut d.stack, ItemStack::empty());
            item.discard();
            hold(m, stack);
        }
    }

    /// `AgeableWaterCreature.isPushedByFluid`.
    fn pushed_by_fluid(&self) -> bool {
        false
    }

    /// `SmoothSwimmingMoveControl(this, 85, 10, 0.02, 0.1, true)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        smooth_swim_move(e, m, 85, 10, 0.02, 0.1, true);
        true
    }

    /// `SmoothSwimmingLookControl(this, 10)`.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        smooth_swim_look(e, m, 10);
        true
    }

    /// `Dolphin.travelInWater`: a light drag, sinking a little without a target.
    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        mob::move_relative(e, m.speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        if m.target.is_none() {
            e.delta = e.delta.add(0.0, -0.005, 0.0);
        }
        true
    }

    /// `Dolphin.increaseAirSupply`: a full breath at once.
    fn increase_air_supply(&self, _current: i32, _max: i32) -> i32 {
        TOTAL_AIR_SUPPLY
    }

    /// `getAmbientSound`.
    fn ambient_sound(&self, e: &mut Entity, _m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(mob::sound_event(if e.is_in_water() { "minecraft:entity.dolphin.ambient_water" } else { "minecraft:entity.dolphin.ambient" })))
    }

    fn swim_sound(&self) -> Option<&'static str> {
        Some("minecraft:entity.dolphin.swim")
    }

    fn splash_sounds(&self) -> Option<(&'static str, &'static str)> {
        Some(("minecraft:entity.dolphin.splash", "minecraft:entity.dolphin.swim"))
    }

    fn experience(&self, e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(1 + e.random.next_int_bounded(3))
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn spawn_ignores_light(&self) -> bool {
        true
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }

    /// `checkSurfaceAgeableWaterCreatureSpawnRules`.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(super::squid::surface_water_rules(view, pos))
    }

    /// `Dolphin.canAttack`: not while a baby.
    fn can_attack(&self, m: &MobData, _level: &dyn EntityLevel, _t: &goals::Living) -> bool {
        !m.baby()
    }

    /// `Dolphin.BABY_DIMENSIONS`: the type's scaled by 0.65, the eyes at 0.09375.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.9f32 * 0.65f32, 0.6f32 * 0.65f32, 0.09375) } else { base }
    }

    /// `Dolphin.mobInteract`: a fish feeds a baby (it grows up faster) or makes an adult want to lead the way.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let _ = who;
        if stack.is_empty() || !mob::item_tag(stack.item(), "minecraft:fishes") {
            return None;
        }
        common_a::play(e, m, level, "minecraft:entity.dolphin.eat", 1.0, 1.0);
        if m.baby() && !m.age_locked {
            let seconds = mob::breed::speed_up_seconds_when_feeding(-m.age);
            mob::age_up(e, m, seconds, true);
        } else {
            st_mut(m).got_fish = true;
        }
        Some(Outcome::success(HeldChange::Consume(1)))
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let got = r.bool_or("GotFish", false);
        let moist = r.int_or("Moistness", TOTAL_MOISTNESS);
        let s = st_mut(m);
        s.got_fish = got;
        s.moistness = moist;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("GotFish", Tag::Byte(s.got_fish as i8));
        o.put("Moistness", Tag::Int(s.moistness));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(kiln_data::entities::data::dolphin::GOT_FISH, &DataValue::Boolean(s.got_fish));
        d.set(kiln_data::entities::data::dolphin::MOISTNESS_LEVEL, &DataValue::Int(s.moistness));
    }
}

// ---------------------------------------------------------------------- goals

/// `BreathAirGoal`: below 140 ticks of air, up to the nearest spot with air above.
#[derive(Clone, Debug)]
struct BreathAirGoal;

impl BreathAirGoal {
    /// `givesAir`: no fluid (or a bubble column) and a place to stand in.
    fn gives_air(level: &dyn EntityLevel, p: BlockPos) -> bool {
        let state = level.block(p);
        let fluid_free = crate::physics::fluid_state(state).is_empty() || crate::blocks::kind(state) == crate::blocks::Kind::BubbleColumn;
        fluid_free && path::pathfindable_land(state)
    }

    /// `findAirPosition`: `BlockPos.neighborColumn(x, y, z, y + 8)` is the column and then each side
    /// (north, east, south, west) from the mob's height up.
    fn find_air_position(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let bp = e.block_position();
        let (x, y, z) = (bp.x, bp.y, bp.z);
        let mut found: Option<BlockPos> = None;
        'search: for (dx, dz) in [(0, 0), (0, -1), (1, 0), (0, 1), (-1, 0)] {
            for step in 0..=8 {
                let p = BlockPos::new(x + dx, y + step, z + dz);
                if Self::gives_air(&*level, p) {
                    found = Some(p);
                    break 'search;
                }
            }
        }
        let p = found.unwrap_or_else(|| BlockPos::containing(e.x(), e.y() + 8.0, e.z()));
        path::move_to(e, m, &*level, p.x as f64, (p.y + 1) as f64, p.z as f64, 1.0);
    }
}

impl CustomGoal for BreathAirGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BreathAirGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        e.air_supply < 140
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.nav.stop();
        Self::find_air_position(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        Self::find_air_position(e, m, level);
        let input = Vec3::new(m.xxa as f64, m.yya as f64, m.zza as f64);
        mob::move_relative(e, 0.02, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
    }
}

/// `TryFindLiquidGoal(this, #minecraft:dolphin_tries_to_find)`: stranded on the ground out of (source) water, it
/// heads for water within reach.
#[derive(Clone, Debug)]
struct TryFindWaterGoal;

fn source_water(level: &dyn EntityLevel, p: BlockPos) -> bool {
    crate::physics::fluid_state(level.block(p)).kind == crate::physics::FluidKind::Water
}

impl CustomGoal for TryFindWaterGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "TryFindLiquidGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        e.on_ground && !source_water(&*level, e.block_position())
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `BlockPos.betweenClosed(x - 2, y - 2, z - 2, x + 2, blockY, z + 2)`: x fastest, then y, then z.
        let (x0, y0, z0) = ((e.x() - 2.0).floor() as i32, (e.y() - 2.0).floor() as i32, (e.z() - 2.0).floor() as i32);
        let (x1, y1, z1) = ((e.x() + 2.0).floor() as i32, e.block_position().y, (e.z() + 2.0).floor() as i32);
        let mut found = None;
        'search: for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = BlockPos::new(x, y, z);
                    if source_water(&*level, p) {
                        found = Some(p);
                        break 'search;
                    }
                }
            }
        }
        if let Some(p) = found {
            m.mov.set_wanted_position(p.x as f64, p.y as f64, p.z as f64, 1.0);
        }
    }
}

/// `Dolphin.DolphinSwimToTreasureGoal`: after a fish, swim toward the nearest ocean ruin or shipwreck.
#[derive(Clone, Debug)]
struct SwimToTreasureGoal {
    stuck: bool,
}

impl SwimToTreasureGoal {
    fn arrived(e: &Entity, treasure: BlockPos) -> bool {
        closer_to_center_than(BlockPos::containing(treasure.x as f64, e.y(), treasure.z as f64), e.position(), 4.0)
    }
}

impl CustomGoal for SwimToTreasureGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DolphinSwimToTreasureGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        got_fish(m) && e.air_supply >= 100
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let Some(t) = st(m).treasure else { return false };
        !Self::arrived(e, t) && !self.stuck && e.air_supply >= 100
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.stuck = false;
        m.nav.stop();
        match level.find_nearest_map_structure("minecraft:dolphin_located", e.block_position(), 50) {
            Some(p) => st_mut(m).treasure = Some(p),
            None => {
                self.stuck = true;
                return;
            }
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 38 });
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let t = st(m).treasure;
        if t.is_none_or(|t| Self::arrived(e, t)) || self.stuck {
            st_mut(m).got_fish = false;
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = st(m).treasure else { return };
        if close_to_next_pos(e, m) || m.nav.is_done() {
            let center = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
            let mut dest = random_pos::default_pos_towards(e, m, &*level, 16, 1, center, 0.39269909262657166);
            if dest.is_none() {
                dest = random_pos::default_pos_towards(e, m, &*level, 8, 4, center, 1.5707963705062866);
            }
            if let Some(d) = dest {
                let p = BlockPos::containing(d.x, d.y, d.z);
                // `getFluidState(pos).is(WATER) && isPathfindable(WATER)`.
                if !water_at(&*level, p) {
                    dest = random_pos::default_pos_towards(e, m, &*level, 8, 5, center, 1.5707963705062866);
                }
            }
            let Some(d) = dest else {
                self.stuck = true;
                return;
            };
            m.look.set_look_at(d.x, d.y, d.z, (m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
            path::move_to(e, m, &*level, d.x, d.y, d.z, 1.3);
            if level.random().next_int_bounded(mth::reduced_tick_delay(80)) == 0 {
                level.emit(Event::EntityEvent { entity: e.id, event: 38 });
            }
        }
    }
}

/// `Dolphin.DolphinSwimWithPlayerGoal`: next to a swimming player within 10 blocks, giving it dolphin's grace.
#[derive(Clone, Debug)]
struct SwimWithPlayerGoal {
    speed: f64,
    player: Option<i32>,
}

impl CustomGoal for SwimWithPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DolphinSwimWithPlayerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        // `forNonCombat().range(10.0).ignoreLineOfSight()`.
        self.player = goals::nearest_player(e, m, &*level, false, 10.0, false, |_| true).map(|t| t.id);
        let Some(id) = self.player else { return false };
        level.player(id).is_some_and(|p| p.swimming) && m.target != Some(id)
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return false };
        p.swimming && e.position().distance_to_sqr(p.pos) < 256.0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(id) = self.player {
            level.add_effect(id, "minecraft:dolphins_grace", 100, 0, Some(e.id));
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.player = None;
        m.nav.stop();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return };
        m.look.set_look_at(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z, (m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
        if e.position().distance_to_sqr(p.pos) < 6.25 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, &*level, BlockPos::containing(p.pos.x, p.pos.y, p.pos.z), self.speed);
        }
        if p.swimming && level.random().next_int_bounded(6) == 0 {
            level.add_effect(p.id, "minecraft:dolphins_grace", 100, 0, Some(e.id));
        }
    }
}

/// `DolphinJumpGoal(this, 10)`: a leap out of the water when the water ahead is clear and the air above it too.
#[derive(Clone, Debug)]
struct JumpGoal {
    interval: i32,
    breached: bool,
}

impl JumpGoal {
    const STEPS: [i32; 6] = [0, 1, 4, 5, 6, 7];
}

impl CustomGoal for JumpGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DolphinJumpGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | goals::JUMP
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_int_bounded(self.interval) != 0 {
            return false;
        }
        let (dx, dz) = facing_step(e.y_rot);
        let pos = e.block_position();
        for step in Self::STEPS {
            let at = BlockPos::new(pos.x + dx * step, pos.y, pos.z + dz * step);
            // `waterIsClear`: water that does not block the jump.
            let water_clear = water_at(&*level, at) && !crate::blocks::has_tag(level.block(at), crate::blocks::Tag::BlocksDolphinJump);
            // `surfaceIsClear`: air in the two blocks over it.
            let surface_clear = kiln_data::blocks_types::is_air(level.block(at.above())) && kiln_data::blocks_types::is_air(level.block(at.above().above()));
            if !water_clear || !surface_clear {
                return false;
            }
        }
        true
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        let yd = e.delta.y;
        if yd * yd < 0.03f32 as f64 && e.x_rot != 0.0 && e.x_rot.abs() < 10.0 && e.is_in_water() {
            return false;
        }
        !e.on_ground
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let (dx, dz) = facing_step(e.y_rot);
        e.delta = e.delta.add(dx as f64 * 0.6, 0.7, dz as f64 * 0.6);
        m.nav.stop();
    }
    fn stop(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        e.set_x_rot(0.0);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let was = self.breached;
        if !self.breached {
            self.breached = water_at(&*level, e.block_position());
        }
        if self.breached && !was {
            common_a::play(e, m, level, "minecraft:entity.dolphin.jump", 1.0, 1.0);
        }
        let v = e.delta;
        if v.y * v.y < 0.03f32 as f64 && e.x_rot != 0.0 {
            // `Mth.rotLerp(0.2, xRot, 0)`.
            let x = e.x_rot;
            e.set_x_rot(x + 0.2 * mth::wrap_degrees(0.0 - x));
        } else if v.length() > 9.999999747378752E-6 {
            let h = v.horizontal_distance();
            let angle = kiln_javamath::atan::atan2(-v.y, h) * 57.2957763671875;
            e.set_x_rot(angle as f32);
        }
    }
}

/// `Dolphin.ItemGoal.dropItem`: the item in its mouth is thrown out, flying where the dolphin looks.
fn drop_item(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    let held = std::mem::replace(&mut m.equipment[mob::MAINHAND], ItemStack::empty());
    if held.is_empty() {
        return false;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, held, seed);
    item.set_pos(Vec3::new(e.x(), e.eye_y() - 0.30000001192092896, e.z()));
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 40;
        d.thrower = Some(e.uuid);
    }
    let g = e.random.next_float() * 6.2831855;
    let h = 0.02 * e.random.next_float();
    let (yr, xr) = (e.y_rot * 0.017453292, e.x_rot * 0.017453292);
    let dx = (0.3 * -mth::sin(yr as f64) * mth::cos(xr as f64)) + mth::cos(g as f64) * h;
    let dy = 0.3 * mth::sin(xr as f64) * 1.5;
    let dz = (0.3 * mth::cos(yr as f64) * mth::cos(xr as f64)) + mth::sin(g as f64) * h;
    item.delta = Vec3::new(dx as f64, dy as f64, dz as f64);
    item.set_old_pos_and_rot();
    level.add_entity(item);
    true
}

/// `Dolphin.MoveToItemGoal`: swimming to items in the water nearby (and a play sound), then dropping what it holds.
#[derive(Clone, Debug)]
struct MoveToItemGoal {
    cooldown: i32,
}

impl MoveToItemGoal {
    fn go(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let items = allowed_items(&*level, e);
        let Some(&first) = items.first() else { return false };
        if let Some(at) = level.entity(first).map(|o| o.block_position()) {
            path::move_to_entity(e, m, &*level, at, 1.2000000476837158);
        }
        true
    }
}

impl CustomGoal for MoveToItemGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "MoveToItemGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.cooldown > e.tick_count {
            return false;
        }
        !allowed_items(&*level, e).is_empty()
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if drop_item(e, m, level) {
            self.cooldown = e.tick_count + e.random.next_int_bounded(100);
        }
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.go(e, m, level) {
            common_a::play(e, m, level, "minecraft:entity.dolphin.play", 1.0, 1.0);
        }
        self.cooldown = 0;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.go(e, m, level);
    }
}

/// `Dolphin.PlayWithItemsGoal`: with something in its mouth it spits it out again.
#[derive(Clone, Debug)]
struct PlayWithItemsGoal;

impl CustomGoal for PlayWithItemsGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PlayWithItemsGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.equipment[mob::MAINHAND].is_empty()
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        drop_item(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        drop_item(e, m, level);
    }
}

/// What a `FollowPlayerRiddenEntityGoal` follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Followed {
    /// `AbstractBoat` (boats and rafts, with or without a chest).
    Boats,
    /// `AbstractNautilus`.
    Nautilus,
}

/// `FollowPlayerRiddenEntityGoal`: following a player on a boat (or a nautilus) that is moving, to its side first
/// and then ahead of it.
#[derive(Clone, Debug)]
struct FollowPlayerRiddenEntityGoal {
    followed: Followed,
    time_to_recalc_path: i32,
    following: Option<i32>,
    /// `FollowEntityGoal.GO_IN_ENTITY_DIRECTION` (else `GO_TO_ENTITY`).
    go_in_direction: bool,
}

impl FollowPlayerRiddenEntityGoal {
    fn new(followed: Followed) -> FollowPlayerRiddenEntityGoal {
        FollowPlayerRiddenEntityGoal { followed, time_to_recalc_path: 0, following: None, go_in_direction: false }
    }

    /// The players that control the entities of the class within 5 blocks, in the order the entities are found.
    fn riders(&self, e: &Entity, level: &dyn EntityLevel) -> Vec<crate::level::PlayerView> {
        let area = e.bounding_box().inflate(5.0, 5.0, 5.0);
        let mut out = Vec::new();
        for id in level.entities_in(&area, EntityFilter::Any, e.id) {
            let Some(o) = level.entity(id) else { continue };
            let class = match self.followed {
                Followed::Boats => o.type_name.ends_with("_boat") || o.type_name.ends_with("_raft") || o.type_name.ends_with("_chest_boat") || o.type_name.ends_with("_chest_raft"),
                Followed::Nautilus => matches!(o.type_name, "minecraft:nautilus" | "minecraft:zombie_nautilus"),
            };
            if !class {
                continue;
            }
            // `getControllingPassenger() instanceof Player`.
            let driver = match self.followed {
                Followed::Boats => o.passengers.first().and_then(|&p| level.player(p)),
                Followed::Nautilus => o.passengers.first().and_then(|&p| level.player(p)).filter(|_| mob::data(o).is_some_and(|nm| nm.kind.ext().is_some_and(|k| k.controlling_player(o, nm, level).is_some()))),
            };
            if let Some(p) = driver {
                out.push(p);
            }
        }
        out
    }
}

impl CustomGoal for FollowPlayerRiddenEntityGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FollowPlayerRiddenEntityGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.following.and_then(|id| level.player(id)).is_some_and(|p| p.moved_horizontally) {
            return true;
        }
        self.riders(e, &*level).iter().any(|p| p.moved_horizontally)
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.following.and_then(|id| level.player(id)).is_some_and(|p| p.vehicle.is_some() && p.moved_horizontally)
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = self.riders(e, &*level).first() {
            self.following = Some(p.id);
        }
        self.time_to_recalc_path = 0;
        self.go_in_direction = false;
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.following = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let speed = if self.go_in_direction { 0.01 } else { 0.015 };
        let input = Vec3::new(m.xxa as f64, m.yya as f64, m.zza as f64);
        mob::move_relative(e, speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        self.time_to_recalc_path -= 1;
        if self.time_to_recalc_path > 0 {
            return;
        }
        self.time_to_recalc_path = mth::reduced_tick_delay(10);
        let Some(p) = self.following.and_then(|id| level.player(id)) else { return };
        let pb = BlockPos::containing(p.pos.x, p.pos.y, p.pos.z);
        let (dx, dz) = facing_step(p.yaw);
        let dist = e.position().distance_to_sqr(p.pos).sqrt() as f32;
        if !self.go_in_direction {
            // The block behind the player, one lower.
            let at = BlockPos::new(pb.x - dx, pb.y - 1, pb.z - dz);
            path::move_to(e, m, &*level, at.x as f64, at.y as f64, at.z as f64, 1.0);
            if dist < 4.0 {
                self.time_to_recalc_path = 0;
                self.go_in_direction = true;
            }
        } else {
            let at = BlockPos::new(pb.x + dx * 10, pb.y, pb.z + dz * 10);
            path::move_to(e, m, &*level, at.x as f64, (at.y - 1) as f64, at.z as f64, 1.0);
            if dist > 12.0 {
                self.time_to_recalc_path = 0;
                self.go_in_direction = false;
            }
        }
    }
}

/// `HurtByTargetGoal(this, Guardian.class).setAlertOthers()`: guardians that hurt it are not fought back.
#[derive(Clone, Debug)]
struct HurtByNotGuardians {
    inner: Goal,
}

fn hurt_timestamp(g: &Goal) -> i32 {
    match g {
        Goal::HurtByTarget { timestamp, .. } => *timestamp,
        _ => 0,
    }
}

impl CustomGoal for HurtByNotGuardians {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "HurtByTargetGoal"
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.last_hurt_by_mob_timestamp != hurt_timestamp(&self.inner)
            && let Some(by) = m.last_hurt_by_mob
            && level.entity(by).and_then(mob::data).is_some_and(|om| matches!(om.kind, MobKind::Guardian | MobKind::ElderGuardian))
        {
            return false;
        }
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
