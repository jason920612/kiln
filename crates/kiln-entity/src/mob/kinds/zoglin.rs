//! Zoglin: a hoglin turned undead (a hoglin outside the nether for 300 ticks). Attacks anything
//! in sight but zoglins and creepers, throws its victims like a hoglin.
//!
//! Driven by the small brain of `Zoglin` (core: looking and moving; idle: attacking the
//! closest visible attackable mob, looking, strolling; fight: melee every 40 ticks, 15 for
//! babies), on [`crate::mob::brain`].

use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::{melee_attack, set_walk_target_from_attack_target_if_out_of_reach, start_attacking, stop_attacking_if_target_invalid_default};
use crate::mob::brain::memory::Val;
use crate::mob::brain::nether::trigger_if;
use crate::mob::brain::sensors;
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Cx, Gate, Mem, Status};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::Living;
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Zoglin;

pub static KIND: Zoglin = Zoglin;

static INFO: Info = Info {
    sounds: Some("zoglin"),
    ..Info::monster("minecraft:zoglin", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (KnockbackResistance, 0.6000000238418579), (AttackKnockback, 1.0), (AttackDamage, 6.0)])
};

#[derive(Clone, Debug, Default)]
pub struct ZoglinState {
    pub attack_animation: i32,
    /// Attackers whose hits the zoglin reacts to at the start of its next brain tick.
    pub pending_hurt: Vec<i32>,
}

fn state_mut(m: &mut MobData) -> Option<&mut ZoglinState> {
    ext::state_mut::<ZoglinState>(m)
}

/// `Zoglin.setBaby`: the flag (`DATA_BABY_ID`) and a baby's attack damage of 0.5.
pub fn set_baby(e: &mut Entity, m: &mut MobData, baby: bool) {
    m.zombie_baby = baby;
    if baby && let Some(a) = m.attrs.get_mut(AttackDamage) {
        a.base = 0.5;
    }
    mob::refresh_dimensions(e, m);
}

fn is_adult(cx: &mut Cx) -> bool {
    !cx.m.baby()
}

fn is_baby(cx: &mut Cx) -> bool {
    cx.m.baby()
}

/// `Zoglin.findNearestValidAttackTarget`: not a zoglin or a creeper, and attackable.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    util::find_closest_visible(cx, |cx, id| {
        let Some(l) = util::living(cx, id) else { return false };
        l.type_name != "minecraft:zoglin" && l.type_name != "minecraft:creeper" && util::is_entity_attackable(cx, &l)
    })
}

/// `Zoglin.setAttackTarget`.
fn set_attack_target(cx: &mut Cx, target: &Living) {
    cx.b.mem.erase(Mem::CantReachWalkTargetSince);
    cx.b.mem.set_expiring(Mem::AttackTarget, Val::Entity(target.id), 200);
}

fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![Box::new(sensors::NearestLivingEntities), Box::new(sensors::Players)];
    let core = ActivityData::create(Activity::Core, 0, vec![LookAtTargetSink::new(45, 90), MoveToTargetSink::new()]);
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![
            start_attacking(|_| true, find_nearest_valid_attack_target),
            SetEntityLookTargetSometimes::new(None, 8.0, (30, 60)),
            Gate::run_one(vec![
                (stroll(0.4, StrollKind::Land { avoid_water: true }), 2),
                (set_walk_target_from_look_target(0.4, 3), 2),
                (DoNothing::new(30, 60), 1),
            ]),
        ],
    );
    let fight = ActivityData::full(
        Activity::Fight,
        super::hoglin::numbered(
            10,
            vec![
                set_walk_target_from_attack_target_if_out_of_reach(|_| 1.0),
                trigger_if(is_adult, melee_attack(40)),
                trigger_if(is_baby, melee_attack(15)),
                stop_attacking_if_target_invalid_default(),
            ],
        ),
        &[(Mem::AttackTarget, Status::ValuePresent)],
        &[Mem::AttackTarget],
    );
    Brain::new(&[], sensors, vec![core, idle, fight], random)
}

fn process_pending_hurt(cx: &mut Cx) {
    let pending = state_mut(cx.m).map(|s| std::mem::take(&mut s.pending_hurt)).unwrap_or_default();
    for id in pending {
        if let Some(a) = util::living(cx, id) {
            was_hurt_by(cx, &a);
        }
    }
}

/// `Zoglin.hurtServer`'s reaction: attack an attacker it can attack, unless the current target is
/// much closer.
fn was_hurt_by(cx: &mut Cx, attacker: &Living) {
    if crate::mob::goals::can_attack(cx.m, &*cx.level, attacker) && !util::other_target_much_further(cx, attacker, 4.0) {
        set_attack_target(cx, attacker);
    }
}

fn update_activity(cx: &mut Cx) {
    let old = cx.b.active_non_core();
    cx.b.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
    let new = cx.b.active_non_core();
    if new == Some(Activity::Fight) && old != Some(Activity::Fight) {
        mob::make_sound(cx.e, cx.m, cx.level, mob::sound_event("minecraft:entity.zoglin.angry"));
    }
    let aggressive = cx.b.mem.has(Mem::AttackTarget);
    cx.m.set_aggressive(aggressive);
}

fn sync_target(m: &mut MobData) {
    m.target = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
}

impl Kind for Zoglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(ZoglinState::default()))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn ai_step_before(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if let Some(st) = state_mut(m)
            && st.attack_animation > 0
        {
            st.attack_animation -= 1;
        }
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                process_pending_hurt(&mut cx);
            }
            m.brain = Some(b);
        }
        brain::tick_brain(e, m, level);
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                update_activity(&mut cx);
            }
            m.brain = Some(b);
        }
        sync_target(m);
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if !hurt {
            return;
        }
        let Some(a) = source.attacker else { return };
        match crate::mob::goals::living(&*level, a) {
            None => {
                if let Some(s) = state_mut(m) {
                    s.pending_hurt.push(a);
                }
            }
            Some(attacker) => {
                if let Some(mut brain) = m.brain.take() {
                    let time = level.game_time();
                    {
                        let mut cx = Cx { e, m, level, b: &mut brain.st, time };
                        was_hurt_by(&mut cx, &attacker);
                    }
                    m.brain = Some(brain);
                }
            }
        }
        sync_target(m);
    }

    /// `Zoglin.doHurtTarget`.
    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        if let Some(st) = state_mut(m) {
            st.attack_animation = 10;
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.zoglin.attack"));
        Some(super::hoglin::hurt_and_throw_target(e, m, level, t))
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        if r.next_float() < 0.2 {
            set_baby(e, m, true);
        }
        ext::mob_finalize(m, r);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let baby = r.bool_or("IsBaby", false);
        set_baby(e, m, baby);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("IsBaby", Tag::Byte(m.baby() as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::zoglin::BABY, &DataValue::Boolean(m.baby()));
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.75, 0.85, 0.625) } else { base }
    }

    /// `Zoglin.getAmbientSound`: angry with a target.
    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let target = m.brain.as_ref()?.st.mem.has(Mem::AttackTarget);
        Some(Some(mob::sound_event(if target { "minecraft:entity.zoglin.angry" } else { "minecraft:entity.zoglin.ambient" })))
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(5)
    }
}
