//! Fox: red or snow (by biome), sleeps in shelter by day, sits and looks around, stalks
//! chickens and rabbits and pounces on them (faceplanting in snow), hunts fish and baby
//! turtles, carries an item in its mouth (picks up food, eats it after a while, spits out
//! what it drops), picks sweet and glow berries, trusts the players who bred it and defends
//! them, flees untrusting players, wild wolves and polar bears.

use super::common_a::{self, Avoid, AvoidEntityGoal, MoveToBlock, Named, NearestTargetGoal};
use super::rabbit::ClimbOnTopOfPowderSnowGoal;
use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE, TARGET};
use crate::mob::interact::{Interactor, Outcome};
use crate::mob::kinds::wolf::{biome_is, block_in_tag};
use crate::mob::path::{self, PathType};
use crate::mob::{DamageSource, GroupData, MAINHAND, MobData, MobKind, SpawnContext, item_name, item_tag, mth};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Fox;

pub static KIND: Fox = Fox;

static INFO: Info = Info::animal(
    "minecraft:fox",
    &[(MovementSpeed, 0.30000001192092896), (MaxHealth, 10.0), (AttackDamage, 2.0), (SafeFallDistance, 5.0), (FollowRange, 32.0)],
);

pub const RED: i32 = 0;
pub const SNOW: i32 = 1;

const SITTING: u8 = 1;
const CROUCHING: u8 = 4;
const INTERESTED: u8 = 8;
const POUNCING: u8 = 16;
const SLEEPING: u8 = 32;
const FACEPLANTED: u8 = 64;
const DEFENDING: u8 = 128;

#[derive(Clone, Debug)]
pub struct State {
    pub variant: i32,
    pub flags: u8,
    pub trusted: [Option<u128>; 2],
    ticks_since_eaten: i32,
    crouch_amount: f32,
    /// `SleepGoal`'s first countdown (drawn when the goal is made).
    sleep_countdown: i32,
    target_goals: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("fox state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("fox state")
}

fn flag(m: &MobData, f: u8) -> bool {
    st(m).flags & f != 0
}

fn set_flag(m: &mut MobData, f: u8, on: bool) {
    let s = st_mut(m);
    if on {
        s.flags |= f;
    } else {
        s.flags &= !f;
    }
}

pub fn is_sleeping(m: &MobData) -> bool {
    m.kind == MobKind::Fox && flag(m, SLEEPING)
}

fn is_defending(m: &MobData) -> bool {
    flag(m, DEFENDING)
}

/// `canMove`.
fn can_move(m: &MobData) -> bool {
    !flag(m, SLEEPING) && !flag(m, SITTING) && !flag(m, FACEPLANTED)
}

/// `clearStates`.
fn clear_states(m: &mut MobData) {
    st_mut(m).flags &= !(INTERESTED | CROUCHING | SITTING | SLEEPING | DEFENDING | FACEPLANTED);
}

fn trusts(m: &MobData, level: &dyn EntityLevel, id: i32) -> bool {
    let Some(p) = level.player(id) else { return false };
    st(m).trusted.iter().any(|t| *t == Some(p.uuid))
}

fn add_trusted(m: &mut MobData, uuid: u128) {
    let s = st_mut(m);
    if s.trusted[0].is_some() {
        s.trusted[1] = Some(uuid);
    } else {
        s.trusted[0] = Some(uuid);
    }
}

fn is_consumable_food(stack: &ItemStack) -> bool {
    !stack.is_empty() && stack.get(kiln_item::keys::FOOD).is_some() && stack.get(kiln_item::keys::CONSUMABLE).is_some()
}

/// `isPathClear`: nothing but replaceable blocks in the three blocks above the line to `t`.
fn is_path_clear(e: &Entity, level: &dyn EntityLevel, t: &Living) -> bool {
    let zd = t.pos.z - e.z();
    let xd = t.pos.x - e.x();
    let slope = zd / xd;
    for i in 0..6 {
        let z = if slope == 0.0 { 0.0 } else { zd * (i as f32 / 6.0) as f64 };
        let x = if slope == 0.0 { xd * (i as f32 / 6.0) as f64 } else { z / slope };
        for j in 1..4 {
            let p = BlockPos::containing(e.x() + x, e.y() + j as f64, e.z() + z);
            if !crate::physics::can_be_replaced(level.block(p)) {
                return false;
            }
        }
    }
    true
}

/// `FoxAlertableEntitiesSelector`.
fn alertable_selector(m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    if t.type_name == "minecraft:fox" {
        return false;
    }
    if matches!(t.type_name, "minecraft:chicken" | "minecraft:rabbit") || common_a::is_monster_class(t.type_name) {
        return true;
    }
    if let Some(om) = level.entity(t.id).and_then(crate::mob::data)
        && matches!(om.kind, MobKind::Wolf | MobKind::Cat)
    {
        return !super::tame::is_tame(om);
    }
    if t.player && (t.spectator || t.creative) {
        return false;
    }
    if t.player && trusts(m, level, t.id) {
        return false;
    }
    let sleeping = level.entity(t.id).and_then(crate::mob::data).is_some_and(is_sleeping);
    !sleeping && !t.sneaking
}

/// `FoxBehaviorGoal.alertable`: something to watch out for within 12 blocks (6 up and down).
fn alertable(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
    let area = e.bounding_box().inflate(12.0, 6.0, 12.0);
    level.entities_in(&area, EntityFilter::Living, e.id).into_iter().any(|id| {
        let Some(t) = goals::living(level, id) else { return false };
        alertable_selector(m, level, &t) && goals::targeting_ok(e, m, level, &t, true, 12.0, false)
    })
}

/// `FoxBehaviorGoal.hasShelter`.
fn has_shelter(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    let p = BlockPos::containing(e.x(), e.bounding_box().max_y, e.z());
    !level.can_see_sky(p) && crate::mob::walk_target_value(m, level, p) >= 0.0
}

/// `spitOutItem`: thrown a block ahead with a 40-tick pickup delay.
fn spit_out(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel, stack: ItemStack) {
    if stack.is_empty() {
        return;
    }
    let look = look_angle(e);
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, stack, seed);
    item.set_pos(Vec3::new(e.x() + look.x, e.y() + 1.0, e.z() + look.z));
    let dx = item.random.next_double() * 0.2 - 0.1;
    let dz = item.random.next_double() * 0.2 - 0.1;
    item.delta = Vec3::new(dx, 0.2, dz);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 40;
    }
    item.set_old_pos_and_rot();
    common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.spit"), 1.0, 1.0);
    level.add_entity(item);
}

/// `Entity.getLookAngle`.
fn look_angle(e: &Entity) -> Vec3 {
    let f = e.x_rot * 0.017453292;
    let g = -e.y_rot * 0.017453292;
    let h = mth::cos(g as f64);
    let i = mth::sin(g as f64);
    let j = mth::cos(f as f64);
    let k = mth::sin(f as f64);
    Vec3::new((i * j) as f64, (-k) as f64, (h * j) as f64)
}

/// `canHoldItem`.
fn can_hold(m: &MobData, stack: &ItemStack) -> bool {
    let held = &m.equipment[MAINHAND];
    held.is_empty() || (st(m).ticks_since_eaten > 0 && is_consumable_food(stack) && !is_consumable_food(held))
}

/// `Mob.aiStep`'s pickup with `Fox.pickUpItem`: one item into the mouth (the rest dropped),
/// the old one spat out.
fn pick_up_items(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !m.can_pick_up_loot || !crate::mob::is_alive(e, m) || !level.mob_griefing() {
        return;
    }
    let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
    for id in level.entities_in(&area, EntityFilter::Item, e.id) {
        let Some(item) = level.entity(id) else { continue };
        let EntityKind::Item(d) = &item.kind else { continue };
        if item.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 || !can_hold(m, &d.stack) {
            continue;
        }
        let Some(item) = level.entity_mut(id) else { continue };
        let EntityKind::Item(d) = &mut item.kind else { continue };
        let mut stack = std::mem::replace(&mut d.stack, ItemStack::empty());
        item.discard();
        let count = stack.count();
        if count > 1 {
            let rest = stack.split(count - 1);
            let nid = level.next_entity_id();
            let seed = level.fresh_seed();
            let mut drop = crate::item::new(nid, 0, rest, seed);
            drop.set_pos(e.position());
            let dx = drop.random.next_double() * 0.2 - 0.1;
            let dz = drop.random.next_double() * 0.2 - 0.1;
            drop.delta = Vec3::new(dx, 0.2, dz);
            drop.set_old_pos_and_rot();
            level.add_entity(drop);
        }
        let old = std::mem::replace(&mut m.equipment[MAINHAND], ItemStack::empty());
        spit_out(e, m, level, old);
        m.equipment[MAINHAND] = stack;
        // `setGuaranteedDrop`.
        m.drop_chances[MAINHAND] = 2.0;
        st_mut(m).ticks_since_eaten = 0;
    }
}

impl Kind for Fox {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        super::tame::set_malus(m, PathType::DamagingInNeighbor, 0.0);
        super::tame::set_malus(m, PathType::Damaging, 0.0);
        m.can_pick_up_loot = true;
        m.nav.required_path_length = 32.0;
        // `SleepGoal`'s constructor.
        let sleep_countdown = random.next_int_bounded(mth::reduced_tick_delay(140));
        Some(Box::new(State { variant: RED, flags: 0, trusted: [None, None], ticks_since_eaten: 0, crouch_amount: 0.0, sleep_countdown, target_goals: false }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let countdown = st(m).sleep_countdown;
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(FoxFloatGoal)));
        g.add(0, Goal::Custom(Box::new(ClimbOnTopOfPowderSnowGoal)));
        g.add(1, Goal::Custom(Box::new(FaceplantGoal { countdown: 0 })));
        g.add(2, Named::new("FoxPanicGoal", common_a::panic(2.2)).gate(|_, m, _| !is_defending(m)).boxed());
        g.add(
            3,
            Named::new("FoxBreedGoal", common_a::breed(1.0))
                .after_start(|g, _e, m, level| {
                    clear_states(m);
                    if let Goal::Breed { partner: Some(p), .. } = g
                        && let Some(pm) = level.entity_mut(*p).and_then(crate::mob::data_mut)
                        && pm.kind == MobKind::Fox
                    {
                        clear_states(pm);
                    }
                })
                .boxed(),
        );
        g.add(
            4,
            Goal::Custom(Box::new(
                AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Players, 16.0, 1.6, 1.4)
                    .gate(|m, _| !is_defending(m))
                    .filter(|m, level, t| !t.sneaking && !trusts(m, level, t.id)),
            )),
        );
        g.add(
            4,
            Goal::Custom(Box::new(
                AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Types(&["minecraft:wolf"]), 8.0, 1.6, 1.4)
                    .gate(|m, _| !is_defending(m))
                    .filter(|_, level, t| !level.entity(t.id).and_then(crate::mob::data).is_some_and(super::tame::is_tame)),
            )),
        );
        g.add(4, Goal::Custom(Box::new(AvoidEntityGoal::new("AvoidEntityGoal", Avoid::Types(&["minecraft:polar_bear"]), 8.0, 1.6, 1.4).gate(|m, _| !is_defending(m)))));
        g.add(5, Goal::Custom(Box::new(StalkPreyGoal)));
        g.add(6, Goal::Custom(Box::new(FoxPounceGoal)));
        g.add(6, Goal::Custom(Box::new(SeekShelterGoal { speed: 1.25, interval: mth::reduced_tick_delay(100), wanted: Vec3::ZERO })));
        let mut melee = common_a::MeleeGoal::new("FoxMeleeAttackGoal", 1.2, true, |next, reset, e, m, level, t| {
            if *next <= 0 && crate::mob::within_melee_range(e, m, t) && crate::mob::has_line_of_sight_cached(e, m, level, t) {
                *next = reset;
                crate::mob::do_hurt_target(e, m, level, t);
                common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.bite"), 1.0, 1.0);
            }
        });
        melee.gate = Some(|m| !flag(m, SITTING) && !flag(m, SLEEPING) && !flag(m, CROUCHING) && !flag(m, FACEPLANTED));
        melee.on_start = Some(|m| set_flag(m, INTERESTED, false));
        g.add(7, Goal::Custom(Box::new(melee)));
        g.add(7, Goal::Custom(Box::new(SleepGoal { countdown })));
        g.add(
            8,
            Named::new("FoxFollowParentGoal", common_a::follow_parent(1.25))
                .gate(|_, m, _| !is_defending(m))
                .keep(|_, m, _| !is_defending(m))
                .after_start(|_, _, m, _| clear_states(m))
                .boxed(),
        );
        g.add(9, Goal::Custom(Box::new(FoxStrollThroughVillageGoal { interval: mth::reduced_tick_delay(200) })));
        let mut berries = MoveToBlock::new(1.2000000476837158, 12, 1);
        berries.vrange = 1;
        g.add(10, Goal::Custom(Box::new(FoxEatBerriesGoal { mtb: berries, waited: 0 })));
        g.add(10, Goal::LeapAtTarget { yd: 0.4, target: None });
        g.add(11, common_a::stroll(1.0));
        g.add(11, Goal::Custom(Box::new(FoxSearchForItemsGoal)));
        g.add(
            12,
            Named::new("FoxLookAtPlayerGoal", common_a::look(24.0))
                .post_gate(|_, m, _| !flag(m, FACEPLANTED) && !flag(m, INTERESTED))
                .keep(|_, m, _| !flag(m, FACEPLANTED) && !flag(m, INTERESTED))
                .boxed(),
        );
        g.add(13, Goal::Custom(Box::new(PerchAndSearchGoal { rel_x: 0.0, rel_z: 0.0, look_time: 0, looks_remaining: 0 })));
        m.targets.add(3, Goal::Custom(Box::new(DefendTrustedTargetGoal { interval: mth::reduced_tick_delay(10), target: None, timestamp: 0, candidate: 0, unseen: 0 })));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if crate::mob::is_alive(e, m) && !m.no_ai {
            st_mut(m).ticks_since_eaten += 1;
            let held = m.equipment[MAINHAND].clone();
            if is_consumable_food(&held) && m.target.is_none() && e.on_ground && !flag(m, SLEEPING) {
                if st(m).ticks_since_eaten > 600 {
                    eat(e, m, level);
                    st_mut(m).ticks_since_eaten = 0;
                } else if st(m).ticks_since_eaten > 560 && e.random.next_float() < 0.1 {
                    common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.eat"), 1.0, 1.0);
                    level.emit(Event::EntityEvent { entity: e.id, event: 45 });
                }
            }
            if goals::living(level, m.target.unwrap_or(i32::MIN)).is_none_or(|t| !t.alive) {
                set_flag(m, CROUCHING, false);
                set_flag(m, INTERESTED, false);
            }
        }
        if flag(m, SLEEPING) || m.is_dead_or_dying() {
            m.jumping = false;
            m.xxa = 0.0;
            m.zza = 0.0;
        }
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        pick_up_items(e, m, level);
        if is_defending(m) && e.random.next_float() < 0.05 {
            common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.aggro"), 1.0, 1.0);
        }
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !m.no_ai {
            let in_water = e.is_in_water();
            if in_water || m.target.is_some() {
                set_flag(m, SLEEPING, false);
            }
            if in_water || flag(m, SLEEPING) {
                set_flag(m, SITTING, false);
            }
            if flag(m, FACEPLANTED) && level.random().next_float() < 0.2 {
                let p = e.block_position();
                let s = level.block(p);
                level.emit(Event::LevelEvent { event: 2001, pos: p, data: s as i32 });
            }
        }
        let crouching = flag(m, CROUCHING);
        let s = st_mut(m);
        if crouching {
            s.crouch_amount = (s.crouch_amount + 0.2).min(5.0);
        } else {
            s.crouch_amount = 0.0;
        }
    }

    fn on_set_target(&self, _e: &mut Entity, m: &mut MobData, target: Option<i32>) {
        if is_defending(m) && target.is_none() {
            set_flag(m, DEFENDING, false);
        }
    }

    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if flag(m, SLEEPING) {
            return true;
        }
        // `LookControl.tick` with `resetXRotOnTick` only while not pouncing, crouching,
        // interested or faceplanted.
        if !flag(m, POUNCING) && !flag(m, CROUCHING) && !flag(m, INTERESTED) && !flag(m, FACEPLANTED) {
            e.x_rot = 0.0;
        }
        if m.look.cooldown > 0 {
            m.look.cooldown -= 1;
            let [wx, wy, wz] = m.look.wanted;
            let (dx, dz) = (wx - e.x(), wz - e.z());
            if dz.abs() > 9.999999747378752e-6 || dx.abs() > 9.999999747378752e-6 {
                let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
                m.y_head_rot = mth::rotate_towards(m.y_head_rot, yaw, m.look.y_max_rot_speed);
            }
            let dy = wy - e.eye_y();
            let h = (dx * dx + dz * dz).sqrt();
            if dy.abs() > 9.999999747378752e-6 || h.abs() > 9.999999747378752e-6 {
                let pitch = (-(mth::atan2(dy, h) * 57.2957763671875)) as f32;
                e.x_rot = mth::rotate_towards(e.x_rot, pitch, m.look.x_max_rot_angle);
            }
        } else {
            m.y_head_rot = mth::rotate_towards(m.y_head_rot, m.y_body_rot, 10.0);
        }
        if !m.nav.is_done() {
            m.y_head_rot = mth::rotate_if_necessary(m.y_head_rot, m.y_body_rot, m.kind.max_head_y_rot() as f32);
        }
        true
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if can_move(m) {
            crate::mob::control::tick_move(e, m, level);
        }
        true
    }

    fn ambient_sound(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        // `playAmbientSound`: a screech plays loud; anything else goes through
        // `Mob.playAmbientSound`, which asks `getAmbientSound` again.
        let first = ambient(e, m, level);
        if first == "minecraft:entity.fox.screech" {
            return Some(Some(crate::mob::sound_event(first)));
        }
        Some(Some(crate::mob::sound_event(ambient(e, m, level))))
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:fox_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (base.0 * 0.6, base.1 * 0.6, 0.34375) } else { base }
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource) {
        // `dropAllDeathLoot`: the mouth item (the shared drop skips a guaranteed one already taken).
        let held = std::mem::replace(&mut m.equipment[MAINHAND], ItemStack::empty());
        crate::mob::spawn_at_location(e, level, held);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        let by_biome = if ctx.biome.is_some_and(|b| biome_is(b, "#minecraft:spawns_snow_foxes")) { SNOW } else { RED };
        let mut baby = false;
        let variant = match group.variant {
            Some(v) => {
                baby = group.ageable_group_size >= 2;
                v
            }
            None => {
                group.variant = Some(by_biome);
                by_biome
            }
        };
        st_mut(m).variant = variant;
        if baby {
            crate::mob::set_age(e, m, crate::mob::breed::BABY_START_AGE);
        }
        set_target_goals(m);
        // `populateDefaultEquipmentSlots`.
        if r.next_float() < 0.2 {
            let odds = r.next_float();
            let item = if odds < 0.05 {
                "minecraft:emerald"
            } else if odds < 0.2 {
                "minecraft:egg"
            } else if odds < 0.4 {
                if r.next_bool() { "minecraft:rabbit_foot" } else { "minecraft:rabbit_hide" }
            } else if odds < 0.6 {
                "minecraft:wheat"
            } else if odds < 0.8 {
                "minecraft:leather"
            } else {
                "minecraft:feather"
            };
            m.equipment[MAINHAND] = ItemStack::of(item, 1).unwrap_or_else(ItemStack::empty);
        }
        // `FoxGroupData` never spawns babies through `AgeableMob`.
        group.ageable_group_size += 1;
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(block_in_tag(view.block(pos.below()), "minecraft:foxes_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    /// `Fox.onOffspringSpawnedFromEgg`: the baby trusts the player who used the egg.
    fn offspring_from_egg(&self, _m: &mut MobData, baby: &mut Entity, level: &mut dyn EntityLevel, player: i32) {
        if let Some(uuid) = level.player(player).map(|p| p.uuid)
            && let Some(bm) = crate::mob::data_mut(baby)
        {
            add_trusted(bm, uuid);
        }
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        st_mut(child).variant = if e.random.next_bool() { st(m).variant } else { ext::state::<State>(partner).map_or(RED, |s| s.variant) };
        // `FoxBreedGoal.breed`: the players who fed the parents are trusted.
        let a = m.love_cause.and_then(|id| level.player(id)).map(|p| p.uuid);
        let b = partner.love_cause.and_then(|id| level.player(id)).map(|p| p.uuid);
        if let Some(a) = a {
            add_trusted(child, a);
        }
        if let Some(b) = b
            && Some(b) != a
        {
            add_trusted(child, b);
        }
    }

    fn interact(&self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel, _who: &Interactor, _stack: &ItemStack) -> Option<Outcome> {
        None
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let trusted: Vec<u128> = match r.get("Trusted") {
            Some(Tag::List(v)) => v.iter().filter_map(crate::persist::uuid_from_tag).collect(),
            _ => Vec::new(),
        };
        st_mut(m).trusted = [None, None];
        for u in trusted {
            add_trusted(m, u);
        }
        let sleeping = r.bool_or("Sleeping", false);
        let variant = match r.get("Type").and_then(Tag::as_str) {
            Some("snow") => SNOW,
            _ => RED,
        };
        let sitting = r.bool_or("Sitting", false);
        let crouching = r.bool_or("Crouching", false);
        set_flag(m, SLEEPING, sleeping);
        st_mut(m).variant = variant;
        set_flag(m, SITTING, sitting);
        set_flag(m, CROUCHING, crouching);
        set_target_goals(m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("Trusted", Tag::List(s.trusted.iter().flatten().map(|u| crate::persist::uuid_to_tag(*u)).collect()));
        o.put("Sleeping", Tag::Byte(flag(m, SLEEPING) as i8));
        o.put("Type", Tag::String(if s.variant == SNOW { "snow" } else { "red" }.into()));
        o.put("Sitting", Tag::Byte(flag(m, SITTING) as i8));
        o.put("Crouching", Tag::Byte(flag(m, CROUCHING) as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::fox::TYPE, &DataValue::Int(s.variant));
        d.set(data::fox::FLAGS, &DataValue::Byte(s.flags as i8));
        d.set(data::fox::TRUSTED_ID_0, &DataValue::OptionalEntityReference(s.trusted[0].map(uuid::Uuid::from_u128)));
        d.set(data::fox::TRUSTED_ID_1, &DataValue::OptionalEntityReference(s.trusted[1].map(uuid::Uuid::from_u128)));
    }
}

/// `getAmbientSound`: asleep, sleeping sounds; at night with no player within 16 blocks,
/// sometimes a screech.
fn ambient(e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> &'static str {
    if flag(m, SLEEPING) {
        return "minecraft:entity.fox.sleep";
    }
    if !level.is_bright_outside() && e.random.next_float() < 0.1 {
        let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
        let h = |p: &crate::level::PlayerView| if p.sneaking { 1.5 } else { 1.8 };
        let players = level.players_in(&area).iter().any(|p| {
            !p.spectator && crate::math::Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h(p), p.pos.z + 0.3).intersects(&area)
        });
        if !players {
            return "minecraft:entity.fox.screech";
        }
    }
    "minecraft:entity.fox.ambient"
}

/// `setTargetGoals`: red foxes hunt on land first, snow foxes fish first.
fn set_target_goals(m: &mut MobData) {
    if st(m).target_goals {
        return;
    }
    st_mut(m).target_goals = true;
    let land = || {
        NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:chicken", "minecraft:rabbit"]), 10, false).boxed()
    };
    let turtle = || {
        NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:turtle"]), 10, false)
            .selector(|_, _, level, t| level.entity(t.id).is_some_and(|o| !o.is_in_water() && crate::mob::data(o).is_some_and(|om| om.baby())))
            .boxed()
    };
    let fish = || NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:cod", "minecraft:salmon", "minecraft:tropical_fish"]), 20, false).boxed();
    if st(m).variant == RED {
        m.targets.add(4, land());
        m.targets.add(4, turtle());
        m.targets.add(6, fish());
    } else {
        m.targets.add(4, fish());
        m.targets.add(6, land());
        m.targets.add(6, turtle());
    }
}

/// `finishUsingItem` on the mouth item: the eating sound and crumbs, one item gone.
fn eat(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let stack = m.equipment[MAINHAND].clone();
    // `Consumable.emitParticlesAndSounds(random, user, stack, 16)`.
    let volume = if e.random.next_bool() { 0.5 } else { 1.0 };
    let pitch = 1.0 + 0.2 * (e.random.next_float() - e.random.next_float());
    let _drink = 0.9 + e.random.next_float() * 0.1;
    for _ in 0..16 {
        e.random.next_float();
        e.random.next_float();
        e.random.next_float();
        e.random.next_float();
    }
    common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.eat"), volume, pitch);
    // `FoodProperties.onConsume`: the food's own sound with a triangle pitch.
    let p2 = 1.0 + 0.4 * (e.random.next_float() - e.random.next_float());
    common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.generic.eat"), 1.0, p2);
    level.emit(Event::GameEvent { event: "minecraft:eat", pos: e.position(), entity: Some(e.id) });
    let mut left = stack;
    left.shrink(1);
    if item_name(&m.equipment[MAINHAND]) == "minecraft:honey_bottle" || item_name(&m.equipment[MAINHAND]).ends_with("_stew") {
        left = ItemStack::empty();
    }
    m.equipment[MAINHAND] = left;
}

/// `FoxFloatGoal`: swims when a quarter block deep.
#[derive(Clone, Debug)]
struct FoxFloatGoal;

impl CustomGoal for FoxFloatGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FoxFloatGoal"
    }
    fn flags(&self) -> u8 {
        JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        e.fluid_height_water() > 0.25 || e.is_in_lava()
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        clear_states(m);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if e.random.next_float() < 0.8 {
            m.jump.jump = true;
        }
    }
}

/// `FaceplantGoal`: stuck head first in snow for two seconds.
#[derive(Clone, Debug)]
struct FaceplantGoal {
    countdown: i32,
}

impl CustomGoal for FaceplantGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FaceplantGoal"
    }
    fn flags(&self) -> u8 {
        LOOK | JUMP | MOVE
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        flag(m, FACEPLANTED)
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        flag(m, FACEPLANTED) && self.countdown > 0
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.countdown = mth::reduced_tick_delay(40);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, FACEPLANTED, false);
    }
    fn tick(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.countdown -= 1;
    }
}

/// `StalkPreyGoal`: creeps up on a chicken or rabbit, crouching within six blocks.
#[derive(Clone, Debug)]
struct StalkPreyGoal;

impl CustomGoal for StalkPreyGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "StalkPreyGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if flag(m, SLEEPING) {
            return false;
        }
        let Some(t) = goals::target(m, level) else { return false };
        t.alive && matches!(t.type_name, "minecraft:chicken" | "minecraft:rabbit") && e.position().distance_to_sqr(t.pos) > 36.0 && !flag(m, CROUCHING) && !flag(m, INTERESTED) && !m.jumping
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, SITTING, false);
        set_flag(m, FACEPLANTED, false);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        match goals::target(m, level) {
            Some(t) if is_path_clear(e, level, &t) => {
                set_flag(m, INTERESTED, true);
                set_flag(m, CROUCHING, true);
                m.nav.stop();
                let (y, x) = (m.kind.max_head_y_rot() as f32, m.max_head_x_rot() as f32);
                m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, y, x);
            }
            _ => {
                set_flag(m, INTERESTED, false);
                set_flag(m, CROUCHING, false);
            }
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        let (y, x) = (m.kind.max_head_y_rot() as f32, m.max_head_x_rot() as f32);
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, y, x);
        if e.position().distance_to_sqr(t.pos) <= 36.0 {
            set_flag(m, INTERESTED, true);
            set_flag(m, CROUCHING, true);
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.5);
        }
    }
}

/// `FoxPounceGoal`: from a full crouch, a leap at the prey; landing in snow faceplants.
#[derive(Clone, Debug)]
struct FoxPounceGoal;

impl CustomGoal for FoxPounceGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FoxPounceGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if st(m).crouch_amount != 5.0 {
            return false;
        }
        let Some(t) = goals::target(m, level).filter(|t| t.alive) else { return false };
        let clear = is_path_clear(e, level, &t);
        if !clear {
            path::create_path_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 0);
            set_flag(m, CROUCHING, false);
            set_flag(m, INTERESTED, false);
        }
        clear
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if goals::target(m, level).is_none_or(|t| !t.alive) {
            return false;
        }
        let yd = e.delta.y;
        (!(yd * yd < 0.05f32 as f64) || !(e.x_rot.abs() < 15.0) || !e.on_ground) && !flag(m, FACEPLANTED)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.jumping = true;
        set_flag(m, POUNCING, true);
        set_flag(m, INTERESTED, false);
        if let Some(t) = goals::target(m, level) {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 60.0, 30.0);
            let uv = Vec3::new(t.pos.x - e.x(), t.pos.y - e.y(), t.pos.z - e.z()).normalize();
            e.delta = e.delta.add(uv.x * 0.8, 0.9, uv.z * 0.8);
        }
        m.nav.stop();
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, CROUCHING, false);
        st_mut(m).crouch_amount = 0.0;
        set_flag(m, INTERESTED, false);
        set_flag(m, POUNCING, false);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let t = goals::target(m, level);
        if let Some(t) = &t {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 60.0, 30.0);
        }
        if !flag(m, FACEPLANTED) {
            let v = e.delta;
            if v.y * v.y < 0.03f32 as f64 && e.x_rot != 0.0 {
                e.x_rot = e.x_rot + 0.2 * mth::wrap_degrees(0.0 - e.x_rot);
            } else {
                let dir = (v.x * v.x + v.z * v.z).sqrt();
                let bias = if m.jumping && v.y > 0.0 { 6.5f32 } else { 1.0 };
                let by = v.y * bias as f64;
                let len = (dir * dir + by * by).sqrt();
                if len > 1.0e-5f32 as f64 {
                    // `Mth.RAD_TO_DEG` (a float).
                    let r = (-by).signum() * kiln_javamath::strict::acos(dir / len) * 57.2957763671875;
                    e.x_rot = r as f32;
                }
            }
        }
        if let Some(t) = t
            && e.position().distance_to_sqr(t.pos).sqrt() as f32 <= 2.0
        {
            crate::mob::do_hurt_target(e, m, level, &t);
        } else if e.x_rot > 0.0 && e.on_ground && e.delta.y as f32 != 0.0 && crate::blocks::block_name(level.block(e.block_position())) == "minecraft:snow" {
            e.x_rot = 60.0;
            crate::mob::set_target(e, m, None);
            set_flag(m, FACEPLANTED, true);
        }
    }
}

/// `SeekShelterGoal`: out of the sun (and the storm) now and then.
#[derive(Clone, Debug)]
struct SeekShelterGoal {
    speed: f64,
    interval: i32,
    wanted: Vec3,
}

impl SeekShelterGoal {
    /// `FleeSunGoal.getHidePos`.
    fn set_wanted_pos(&mut self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
        let base = e.block_position();
        for _ in 0..10 {
            let dx = e.random.next_int_bounded(20) - 10;
            let dy = e.random.next_int_bounded(6) - 3;
            let dz = e.random.next_int_bounded(20) - 10;
            let p = base.offset(dx, dy, dz);
            if !level.can_see_sky(p) && crate::mob::walk_target_value(m, level, p) < 0.0 {
                self.wanted = Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
                return true;
            }
        }
        false
    }
}

impl CustomGoal for SeekShelterGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SeekShelterGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if flag(m, SLEEPING) || m.target.is_some() {
            return false;
        }
        // (Thunderstorms are not visible to entities here.)
        if self.interval > 0 {
            self.interval -= 1;
            return false;
        }
        self.interval = 100;
        let p = e.block_position();
        level.is_bright_outside() && level.can_see_sky(p) && self.set_wanted_pos(e, m, level)
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        clear_states(m);
        path::move_to(e, m, level, self.wanted.x, self.wanted.y, self.wanted.z, self.speed);
    }
}

/// `SleepGoal`: by day in shelter with nothing alarming around, curls up.
#[derive(Clone, Debug)]
struct SleepGoal {
    countdown: i32,
}

impl SleepGoal {
    fn can_sleep(&mut self, e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        if self.countdown > 0 {
            self.countdown -= 1;
            return false;
        }
        level.is_bright_outside() && has_shelter(e, m, level) && !alertable(e, m, level) && !e.is_in_powder_snow
    }
}

impl CustomGoal for SleepGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SleepGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.xxa == 0.0 && m.yya == 0.0 && m.zza == 0.0 { self.can_sleep(e, m, level) || flag(m, SLEEPING) } else { false }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_sleep(e, m, level)
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.countdown = e.random.next_int_bounded(mth::reduced_tick_delay(140));
        clear_states(m);
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, SITTING, false);
        set_flag(m, CROUCHING, false);
        set_flag(m, INTERESTED, false);
        m.jumping = false;
        set_flag(m, SLEEPING, true);
        m.nav.stop();
        m.mov.set_wanted_position(e.x(), e.y(), e.z(), 0.0);
    }
}

/// `FoxStrollThroughVillageGoal`: at night near villages (Kiln has no village sections: the
/// interval draw only).
#[derive(Clone, Debug)]
struct FoxStrollThroughVillageGoal {
    interval: i32,
}

impl CustomGoal for FoxStrollThroughVillageGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FoxStrollThroughVillageGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if level.is_bright_outside() {
            return false;
        }
        let _ = e.random.next_int_bounded(self.interval) != 0;
        false
    }
}

/// `FoxEatBerriesGoal`: sniffs its way to ripe sweet berries or glow berries and picks them.
#[derive(Clone, Debug)]
struct FoxEatBerriesGoal {
    mtb: MoveToBlock,
    waited: i32,
}

fn berries_ready(state: u16) -> bool {
    let info = kiln_data::blocks_types::block_of(state);
    match info.name {
        "minecraft:sweet_berry_bush" => info.property(state, "age").and_then(|a| a.parse::<i32>().ok()).is_some_and(|a| a >= 2),
        "minecraft:cave_vines" | "minecraft:cave_vines_plant" => info.property(state, "berries") == Some("true"),
        _ => false,
    }
}

impl CustomGoal for FoxEatBerriesGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FoxEatBerriesGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if flag(m, SLEEPING) {
            return false;
        }
        let lv: &dyn EntityLevel = level;
        self.mtb.can_use(e, |p| berries_ready(lv.block(p)))
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.mtb.in_time() && berries_ready(level.block(self.mtb.block))
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.waited = 0;
        set_flag(m, SITTING, false);
        self.mtb.start(e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.mtb.reached {
            if self.waited >= 40 {
                self.on_reached(e, m, level);
            } else {
                self.waited += 1;
            }
        } else if e.random.next_float() < 0.05 {
            common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.sniff"), 1.0, 1.0);
        }
        // `MoveToBlockGoal.tick` with accepted distance 2 and a new path every 100 ticks.
        let t = self.mtb.block.above();
        let c = Vec3::new(t.x as f64 + 0.5, t.y as f64 + 0.5, t.z as f64 + 0.5);
        if c.distance_to_sqr(e.position()) >= 4.0 {
            self.mtb.reached = false;
            self.mtb.try_ticks += 1;
            if self.mtb.try_ticks % 100 == 0 {
                path::move_to(e, m, level, t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5, self.mtb.speed);
            }
        } else {
            self.mtb.reached = true;
            self.mtb.try_ticks -= 1;
        }
    }
}

impl FoxEatBerriesGoal {
    fn on_reached(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !level.mob_griefing() {
            return;
        }
        let p = self.mtb.block;
        let state = level.block(p);
        let info = kiln_data::blocks_types::block_of(state);
        let pos = Vec3::new(p.x as f64, p.y as f64, p.z as f64);
        if info.name == "minecraft:sweet_berry_bush" {
            let age: i32 = info.property(state, "age").and_then(|a| a.parse().ok()).unwrap_or(0);
            let mut count = 1 + level.random().next_int_bounded(2) + (age == 3) as i32;
            if m.equipment[MAINHAND].is_empty() {
                m.equipment[MAINHAND] = ItemStack::of("minecraft:sweet_berries", 1).unwrap_or_else(ItemStack::empty);
                count -= 1;
            }
            if count > 0 {
                pop_resource(level, p, ItemStack::of("minecraft:sweet_berries", count).unwrap_or_else(ItemStack::empty));
            }
            common_a::play(e, m, level, "minecraft:block.sweet_berry_bush.pick_berries", 1.0, 1.0);
            let picked = info.with_property(state, "age", "1").unwrap_or(state);
            level.set_block(p, picked, 2);
            level.emit(Event::GameEvent { event: "minecraft:block_change", pos, entity: Some(e.id) });
        } else {
            // `CaveVines.use`: the glow berry drops, the vine goes bare.
            pop_resource(level, p, ItemStack::of("minecraft:glow_berries", 1).unwrap_or_else(ItemStack::empty));
            let pitch = 0.8 + level.random().next_float() * 0.4;
            level.emit(Event::Sound { pos: pos.add(0.5, 0.5, 0.5), sound: "minecraft:block.cave_vines.pick_berries", source: "blocks", volume: 1.0, pitch });
            let bare = info.with_property(state, "berries", "false").unwrap_or(state);
            level.set_block(p, bare, 2);
            level.emit(Event::GameEvent { event: "minecraft:block_change", pos, entity: Some(e.id) });
        }
    }
}

/// `Block.popResource`: an item at a random spot in the block.
fn pop_resource(level: &mut dyn EntityLevel, p: BlockPos, stack: ItemStack) {
    if stack.is_empty() {
        return;
    }
    let r = level.random();
    // `Mth.nextDouble(random, -0.25, 0.25)`.
    let x = p.x as f64 + 0.5 + (r.next_double() * 0.5 - 0.25);
    let y = p.y as f64 + 0.5 + (r.next_double() * 0.5 - 0.25) - 0.125;
    let z = p.z as f64 + 0.5 + (r.next_double() * 0.5 - 0.25);
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new(id, 0, stack, seed);
    item.set_pos(Vec3::new(x, y, z));
    let dx = item.random.next_double() * 0.2 - 0.1;
    let dz = item.random.next_double() * 0.2 - 0.1;
    item.delta = Vec3::new(dx, 0.2, dz);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
}

/// `FoxSearchForItemsGoal`: with an empty mouth, walks to items lying around.
#[derive(Clone, Debug)]
struct FoxSearchForItemsGoal;

fn items_near(e: &Entity, level: &dyn EntityLevel) -> Vec<(i32, BlockPos)> {
    let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
    level
        .entities_in(&area, EntityFilter::Item, e.id)
        .into_iter()
        .filter_map(|id| {
            let o = level.entity(id)?;
            let EntityKind::Item(d) = &o.kind else { return None };
            (d.pickup_delay == 0 && o.is_alive()).then(|| (id, o.block_position()))
        })
        .collect()
}

impl CustomGoal for FoxSearchForItemsGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FoxSearchForItemsGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !m.equipment[MAINHAND].is_empty() || m.target.is_some() || m.last_hurt_by_mob.is_some() || !can_move(m) {
            return false;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(10)) != 0 {
            return false;
        }
        !items_near(e, level).is_empty() && m.equipment[MAINHAND].is_empty()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(&(_, p)) = items_near(e, level).first() {
            path::move_to_entity(e, m, level, p, 1.2000000476837158);
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let items = items_near(e, level);
        if m.equipment[MAINHAND].is_empty()
            && let Some(&(_, p)) = items.first()
        {
            path::move_to_entity(e, m, level, p, 1.2000000476837158);
        }
    }
}

/// `PerchAndSearchGoal`: sits and looks around a few times.
#[derive(Clone, Debug)]
struct PerchAndSearchGoal {
    rel_x: f64,
    rel_z: f64,
    look_time: i32,
    looks_remaining: i32,
}

impl PerchAndSearchGoal {
    fn reset_look(&mut self, e: &mut Entity) {
        let r = std::f64::consts::TAU * e.random.next_double();
        self.rel_x = kiln_javamath::trig::cos(r);
        self.rel_z = kiln_javamath::trig::sin(r);
        self.look_time = mth::reduced_tick_delay(80 + e.random.next_int_bounded(20));
    }
}

impl CustomGoal for PerchAndSearchGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PerchAndSearchGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        m.last_hurt_by_mob.is_none()
            && e.random.next_float() < 0.02
            && !flag(m, SLEEPING)
            && m.target.is_none()
            && m.nav.is_done()
            && !alertable(e, m, level)
            && !flag(m, POUNCING)
            && !flag(m, CROUCHING)
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        self.looks_remaining > 0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.reset_look(e);
        self.looks_remaining = 2 + e.random.next_int_bounded(3);
        set_flag(m, SITTING, true);
        m.nav.stop();
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, SITTING, false);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time -= 1;
        if self.look_time <= 0 {
            self.looks_remaining -= 1;
            self.reset_look(e);
        }
        let (y, x) = (m.kind.max_head_y_rot() as f32, m.max_head_x_rot() as f32);
        m.look.set_look_at(e.x() + self.rel_x, e.eye_y(), e.z() + self.rel_z, y, x);
    }
}

/// `DefendTrustedTargetGoal`: goes after whoever hurt a trusted player.
#[derive(Clone, Debug)]
struct DefendTrustedTargetGoal {
    interval: i32,
    target: Option<i32>,
    timestamp: i32,
    /// The trusted player's hurt timestamp seen by `canUse`.
    candidate: i32,
    unseen: i32,
}

impl CustomGoal for DefendTrustedTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "DefendTrustedTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.interval > 0 && e.random.next_int_bounded(self.interval) != 0 {
            return false;
        }
        for uuid in st(m).trusted.into_iter().flatten() {
            let Some(p) = level.player_by_uuid(uuid) else { continue };
            self.target = p.last_hurt_by_mob;
            if p.last_hurt_by_mob_time == self.timestamp {
                return false;
            }
            self.candidate = p.last_hurt_by_mob_time;
            let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return false };
            let follow = m.attrs.value(FollowRange);
            // `TRUSTED_TARGET_SELECTOR` and not trusted itself.
            let hurt_someone = if t.player { level.player(t.id).is_some_and(|q| q.last_hurt_mob.is_some()) } else { level.entity(t.id).and_then(crate::mob::data).is_some_and(|om| om.last_hurt_mob.is_some()) };
            return hurt_someone && !(t.player && trusts(m, level, t.id)) && goals::targeting_ok(e, m, level, &t, true, follow, false);
        }
        false
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let follow = m.attrs.value(FollowRange);
        common_a::continue_target(e, m, level, self.target, false, &mut self.unseen, 60, follow)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, self.target);
        self.timestamp = self.candidate;
        common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.fox.aggro"), 1.0, 1.0);
        set_flag(m, DEFENDING, true);
        set_flag(m, SLEEPING, false);
        self.unseen = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        crate::mob::set_target(e, m, None);
        self.target = None;
    }
}
