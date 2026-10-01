//! PiglinBrute: the bastion guard. Fights whoever it is angry at, any visible attackable player
//! and wither skeletons or withers, strolls around its home (where it spawned), never hunts,
//! barters or admires.
//!
//! Driven by the brain of `PiglinBruteAi` (core: looking, moving, doors, calming down; idle:
//! attacking, looking, wandering and home strolls; fight: melee every 20 ticks), on
//! [`crate::mob::brain`]. `AbstractPiglin`'s zombification is the piglin's.

use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::{melee_attack, set_walk_target_from_attack_target_if_out_of_reach, start_attacking, stop_attacking_if_target_invalid};
use crate::mob::brain::memory::{GlobalPos, Val};
use crate::mob::brain::nether::*;
use crate::mob::brain::sensors;
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Control, Cx, Gate, Mem, Status};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Living;
use crate::mob::kinds::piglin::{self, PiglinState};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use crate::math::BlockPos;

pub struct PiglinBrute;

pub static KIND: PiglinBrute = PiglinBrute;

static INFO: Info = Info { sounds: Some("piglin_brute"), ..Info::monster("minecraft:piglin_brute", &[(MaxHealth, 50.0), (MovementSpeed, 0.3499999940395355), (AttackDamage, 7.0), (FollowRange, 12.0)]) };

/// `PiglinBrute.wantsToPickUp`: only a golden axe.
fn sensor_wants(cx: &Cx, stack: &ItemStack) -> bool {
    !stack.is_empty() && mob::item_name(stack) == "minecraft:golden_axe" && cx.level.mob_griefing() && cx.m.can_pick_up_loot
}

/// `PiglinBruteAi.findNearestValidAttackTarget`.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    if let Some(a) = living_from_uuid_memory(cx, Mem::AngryAt)
        && util::is_entity_attackable_ignoring_los(cx, &a)
    {
        return Some(a.id);
    }
    if let Some(p) = cx.b.mem.entity(Mem::NearestVisibleAttackablePlayer) {
        return Some(p);
    }
    cx.b.mem.entity(Mem::NearestVisibleNemesis)
}

fn is_player(cx: &Cx, id: i32) -> bool {
    living_now(cx, id).is_some_and(|l| l.type_name == PLAYER)
}

fn is_piglin(cx: &Cx, id: i32) -> bool {
    living_now(cx, id).is_some_and(|l| l.type_name == PIGLIN)
}

fn is_brute(cx: &Cx, id: i32) -> bool {
    living_now(cx, id).is_some_and(|l| l.type_name == PIGLIN_BRUTE)
}

fn any(_cx: &Cx, _id: i32) -> bool {
    true
}

fn idle_look_behaviors() -> Box<dyn Control> {
    Gate::run_one(vec![
        (set_entity_look_target(is_player, 8.0), 1),
        (set_entity_look_target(is_piglin, 8.0), 1),
        (set_entity_look_target(is_brute, 8.0), 1),
        (set_entity_look_target(any, 8.0), 1),
        (DoNothing::new(30, 60), 1),
    ])
}

fn idle_movement_behaviors() -> Box<dyn Control> {
    Gate::run_one(vec![
        (stroll(0.6, StrollKind::Land { avoid_water: true }), 2),
        (interact_with(PIGLIN, 8, 0.6, 2), 2),
        (interact_with(PIGLIN_BRUTE, 8, 0.6, 2), 2),
        (StrollToPoi::new(Mem::Home, 0.6, 2, 100), 2),
        (StrollAroundPoi::new(Mem::Home, 0.6, 5), 2),
        (DoNothing::new(30, 60), 1),
    ])
}

fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(NearestItems { wants: sensor_wants }),
        Box::new(sensors::HurtBy),
        Box::new(PiglinBruteSpecific),
    ];
    let core = ActivityData::create(Activity::Core, 0, vec![LookAtTargetSink::new(45, 90), MoveToTargetSink::new(), InteractWithDoor::new(), stop_being_angry_if_target_dead()]);
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![start_attacking(|_| true, find_nearest_valid_attack_target), idle_look_behaviors(), idle_movement_behaviors(), set_look_and_interact(PLAYER, 4)],
    );
    let fight = ActivityData::full(
        Activity::Fight,
        super::hoglin::numbered(
            10,
            vec![
                stop_attacking_if_target_invalid(|cx, t| find_nearest_valid_attack_target(cx) != Some(t.id), |_, _| {}, true),
                set_walk_target_from_attack_target_if_out_of_reach(|_| 1.0),
                melee_attack(20),
            ],
        ),
        &[(Mem::AttackTarget, Status::ValuePresent)],
        &[Mem::AttackTarget],
    );
    Brain::new(&[Mem::NearestVisibleAdultPiglins], sensors, vec![core, idle, fight], random)
}

/// `PiglinBruteAi.wasHurtBy`: piglins do not anger a brute.
pub fn was_hurt_by(cx: &mut Cx, attacker: &Living) {
    if attacker.type_name == PIGLIN || attacker.type_name == PIGLIN_BRUTE {
        return;
    }
    piglin::maybe_retaliate(cx, attacker);
}

/// `PiglinBruteAi.playActivitySound`: the angry grunt of a fight.
fn play_activity_sound(cx: &mut Cx) {
    if cx.b.active_non_core() == Some(Activity::Fight) {
        mob::make_sound(cx.e, cx.m, cx.level, mob::sound_event("minecraft:entity.piglin_brute.angry"));
    }
}

/// `PiglinBruteAi.updateActivity` and `maybePlayActivitySound` (a draw from the level's random
/// every tick).
fn update_activity(cx: &mut Cx) {
    let old = cx.b.active_non_core();
    cx.b.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
    let new = cx.b.active_non_core();
    if old != new {
        play_activity_sound(cx);
    }
    let aggressive = cx.b.mem.has(Mem::AttackTarget);
    cx.m.set_aggressive(aggressive);
    if (cx.rng().next_float() as f64) < 0.0125 {
        play_activity_sound(cx);
    }
}

fn process_pending_hurt(cx: &mut Cx) {
    let pending = piglin::state_mut(cx.m).map(|s| std::mem::take(&mut s.pending_hurt)).unwrap_or_default();
    for id in pending {
        if let Some(a) = living_now(cx, id) {
            was_hurt_by(cx, &a);
        }
    }
}

impl Kind for PiglinBrute {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.can_pick_up_loot = true;
        m.nav.can_open_doors = true;
        crate::mob::kinds::tame::set_malus(m, crate::mob::path::PathType::FireInNeighbor, 16.0);
        crate::mob::kinds::tame::set_malus(m, crate::mob::path::PathType::Fire, -1.0);
        Some(Box::new(PiglinState::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        piglin::pick_up_loot(e, m, level);
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                // The home was set where the brute spawned, in whatever dimension it turned out to be.
                if let Some(h) = cx.b.mem.global_pos(Mem::Home).cloned()
                    && h.dim.is_empty()
                {
                    let dim = dimension_of(&*cx.level);
                    cx.b.mem.set(Mem::Home, Val::Pos(GlobalPos::new(dim, h.pos)));
                }
                process_pending_hurt(&mut cx);
            }
            m.brain = Some(b);
        }
        set_ticking(Some((&*e, &*m)));
        brain::tick_brain(e, m, level);
        set_ticking(None);
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                update_activity(&mut cx);
            }
            m.brain = Some(b);
        }
        piglin::sync_target(m, &*level);
        piglin::tick_conversion(e, m, level);
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        piglin::on_hurt(e, m, level, source, hurt);
        piglin::sync_target(m, &*level);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        // `PiglinBruteAi.initMemories`: home is where it stands; then its golden axe.
        let pos = e.block_position();
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.set(Mem::Home, Val::Pos(GlobalPos::new("", pos)));
        }
        if let Some(s) = ItemStack::of("minecraft:golden_axe", 1) {
            m.equipment[MAINHAND] = s;
        }
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let immune = r.bool_or("IsImmuneToZombification", false);
        let time = r.int_or("TimeInOverworld", 0);
        m.can_pick_up_loot = r.bool_or("CanPickUpLoot", true);
        if let Some(st) = piglin::state_mut(m) {
            st.immune_to_zombification = immune;
            st.time_in_overworld = time;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = piglin::state(m) else { return };
        if st.immune_to_zombification {
            o.put("IsImmuneToZombification", Tag::Byte(1));
        }
        o.put("TimeInOverworld", Tag::Int(st.time_in_overworld));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let immune = piglin::state(m).is_some_and(|s| s.immune_to_zombification);
        d.set(kiln_data::entities::data::abstract_piglin::IMMUNE_TO_ZOMBIFICATION, &DataValue::Boolean(immune));
    }

    /// `AbstractPiglin.playAmbientSound`: only while idle.
    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let b = m.brain.as_ref()?;
        if b.st.is_active(Activity::Idle) { None } else { Some(None) }
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(20)
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}
