//! Polar bear: neutral (`NeutralMob`), rears up and roars before a swipe, defends its cubs
//! (adults near a cub go after players; a hurt cub rallies the adults), hunts foxes; cubs
//! panic at any harm, adults only at fire and the like. Not bred with food.

use super::anger::{self, Anger, AngryAtPlayerGoal};
use super::common_a::{self, Avoid, MeleeGoal, Named, NearestTargetGoal, PanicGoal};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::BlockPos;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{Goal, Living};
use crate::mob::kinds::wolf::{biome_is, block_in_tag};
use crate::mob::{GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct PolarBear;

pub static KIND: PolarBear = PolarBear;

static INFO: Info =
    Info::animal("minecraft:polar_bear", &[(MaxHealth, 30.0), (FollowRange, 20.0), (MovementSpeed, 0.25), (AttackDamage, 6.0)]);

#[derive(Clone, Debug, Default)]
pub struct State {
    pub anger: Anger,
    pub standing: bool,
    warning_sound_ticks: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("polar bear state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("polar bear state")
}

fn set_standing(m: &mut MobData, on: bool) {
    st_mut(m).standing = on;
}

/// `playWarningSound`: at most every two seconds.
fn play_warning_sound(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if st(m).warning_sound_ticks <= 0 {
        crate::mob::make_sound(e, m, level, crate::mob::sound_event("minecraft:entity.polar_bear.warning"));
        st_mut(m).warning_sound_ticks = 40;
    }
}

/// `PolarBearMeleeAttackGoal.checkAndPerformAttack`: no swing; within reach plus three blocks the
/// bear stands up and roars as the swipe comes due.
fn bear_attack(next: &mut i32, reset: i32, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    if *next <= 0 && crate::mob::within_melee_range(e, t) && crate::mob::has_line_of_sight_cached(e, m, level, t) {
        *next = reset;
        crate::mob::do_hurt_target(e, m, level, t);
        set_standing(m, false);
    } else {
        let w = (t.bb.max_x - t.bb.min_x) as f32 + 3.0;
        if e.position().distance_to_sqr(t.pos) < (w * w) as f64 {
            if *next <= 0 {
                set_standing(m, false);
                *next = reset;
            }
            if *next <= 10 {
                set_standing(m, true);
                play_warning_sound(e, m, level);
            }
        } else {
            *next = reset;
            set_standing(m, false);
        }
    }
}

impl Kind for PolarBear {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        let mut melee = MeleeGoal::new("PolarBearMeleeAttackGoal", 1.25, true, bear_attack);
        melee.on_stop = Some(|_, m| set_standing(m, false));
        g.add(1, Goal::Custom(Box::new(melee)));
        g.add(
            1,
            Goal::Custom(Box::new(PanicGoal::new("PanicGoal", 2.0, |m| {
                if m.baby() { "minecraft:panic_causes" } else { "minecraft:panic_environmental_causes" }
            }))),
        );
        g.add(4, common_a::follow_parent(1.25));
        g.add(5, Named::new("RandomStrollGoal", Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: None, wanted: crate::math::Vec3::ZERO, force: false }).boxed());
        g.add(6, common_a::look(6.0));
        g.add(7, common_a::look_around());
        let t = &mut m.targets;
        t.add(
            1,
            Named::new("PolarBearHurtByTargetGoal", common_a::hurt_by(false))
                .after_start(|g, e, m, level| {
                    if m.baby() {
                        common_a::alert_others(e, m, level, |o| !o.baby());
                        crate::mob::goals::stop(g, e, m, level);
                    }
                })
                .boxed(),
        );
        t.add(
            2,
            NearestTargetGoal::new("PolarBearAttackPlayersGoal", Avoid::Players, 20, true)
                .scale(0.5)
                .gate(|_, m, _| !m.baby())
                .after(|e, _, level| {
                    let area = e.bounding_box().inflate(8.0, 4.0, 8.0);
                    level.entities_in(&area, crate::level::EntityFilter::Living, i32::MIN).into_iter().any(|id| {
                        level.entity(id).and_then(crate::mob::data).is_some_and(|o| o.kind == MobKind::PolarBear && o.baby())
                    })
                })
                .boxed(),
        );
        t.add(3, Goal::Custom(Box::new(AngryAtPlayerGoal::default())));
        t.add(4, NearestTargetGoal::new("NearestAttackableTargetGoal", Avoid::Types(&["minecraft:fox"]), 10, true).selector(|_, m, _, _| !m.baby()).boxed());
        // `ResetUniversalAngerTargetGoal`: the `universal_anger` game rule is off.
        t.add(5, Goal::Never);
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        if s.warning_sound_ticks > 0 {
            s.warning_sound_ticks -= 1;
        }
        anger::update_persistent_anger(e, m, level);
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(crate::mob::sound_event(if m.baby() { "minecraft:entity.polar_bear.ambient_baby" } else { "minecraft:entity.polar_bear.ambient" })))
    }

    fn water_slow_down(&self, _m: &MobData) -> f32 {
        0.98
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.7, 0.7, 0.34375) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        // An `AgeableMobGroupData(1.0)`: after the first bear, cubs.
        ext::ageable_finalize(e, m, r, group, 1.0);
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        let bright = view.raw_brightness(pos, 0) > 8;
        let below = view.block(pos.below());
        if biome_is(view.biome(pos), "#minecraft:polar_bears_spawn_on_alternate_blocks") {
            Some(bright && block_in_tag(below, "minecraft:polar_bears_spawnable_on_alternate"))
        } else {
            Some(block_in_tag(below, "minecraft:animals_spawnable_on") && bright)
        }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        st_mut(m).anger.end = match r.get("anger_end_time") {
            Some(Tag::Long(t)) => *t,
            _ => -1,
        };
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("anger_end_time", Tag::Long(st(m).anger.end));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(data::polar_bear::STANDING, &DataValue::Boolean(st(m).standing));
    }
}

/// Whether mob `m` is a polar bear that stands up (for viewers).
pub fn standing(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.standing)
}
