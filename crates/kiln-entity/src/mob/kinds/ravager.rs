//! Ravager: the raiders' beast. Bites with a strong knockback (stopping for the attack's ten
//! ticks), tramples through leaves, is stunned when a shield blocks it and then roars, hurting
//! and throwing back what stands around it.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{Goal, Living, MeleeKind, Wanted};
use crate::mob::kinds::raider::{self, IllagerState};
use crate::mob::kinds::zombie::{IRON_GOLEM, VILLAGERS, nearest};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, path};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Ravager;

pub static KIND: Ravager = Ravager;

static INFO: Info = Info {
    sounds: Some("ravager"),
    head: (45, 40, 10),
    ..Info::monster(
        "minecraft:ravager",
        &[
            (MaxHealth, 100.0),
            (MovementSpeed, 0.3),
            (KnockbackResistance, 0.75),
            (AttackDamage, 12.0),
            (AttackKnockback, 1.5),
            (FollowRange, 32.0),
            (StepHeight, 1.0),
        ],
    )
};

fn st(m: &MobData) -> &IllagerState {
    raider::illager(m).expect("ravager state")
}

fn st_mut(m: &mut MobData) -> &mut IllagerState {
    raider::illager_mut(m).expect("ravager state")
}

impl Kind for Ravager {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::Leaves, 0.0));
        Some(Box::new(IllagerState::default()))
    }

    /// `Ravager.updateControlFlags`: raiders on its back leave its goals their flags.
    fn keeps_flags_for_raiders(&self) -> bool {
        true
    }

    fn register_goals(&self, m: &mut MobData) {
        raider::register_raider_goals(m);
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(
            4,
            Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: true, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 },
        );
        g.add(5, Goal::RandomStroll { speed: 0.4, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(6, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(8.0))));
        let t = &mut m.targets;
        t.add(2, raider::hurt_by_ignoring_raiders());
        t.add(3, nearest(Wanted::Player, true));
        t.add(4, nearest(Wanted::Types(VILLAGERS), true));
        t.add(4, nearest(Wanted::Types(IRON_GOLEM), true));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        raider::ai_step_before(e, m, level);
    }

    /// `Raider.updateNoActionTime`: two more every tick, whatever the light.
    fn update_no_action_time(&self, _e: &Entity, m: &mut MobData, _level: &dyn EntityLevel) {
        m.no_action_time += 2;
    }

    /// `Ravager.aiStep` after `super.aiStep()`.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !mob::is_alive(e, m) {
            return;
        }
        let immobile = self.is_immobile(m);
        let base = m.attrs.get(Attr::MovementSpeed).map_or(0.3, |i| i.base);
        let new_base = if immobile {
            0.0
        } else {
            let max = if crate::mob::goals::target(m, level).is_some() { 0.35 } else { 0.3 };
            crate::math::lerp(0.1, base, max)
        };
        if let Some(i) = m.attrs.get_mut(Attr::MovementSpeed) {
            i.base = new_base;
        }
        if e.horizontal_collision && level.mob_griefing() {
            let b = e.bounding_box().inflate(0.2, 0.2, 0.2);
            let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
            let (x1, y1, z1) = (crate::math::floor(b.max_x), crate::math::floor(b.max_y), crate::math::floor(b.max_z));
            let mut destroyed = false;
            // `BlockPos.betweenClosed`: x fastest, then y, then z.
            for z in z0..=z1 {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let p = BlockPos::new(x, y, z);
                        if crate::blocks::block_name(level.block(p)).ends_with("_leaves") {
                            destroyed = level.destroy_block(p, true) || destroyed;
                        }
                    }
                }
            }
            if !destroyed && e.on_ground {
                let power = m.attrs.value(Attr::JumpStrength) as f32 * e.block_jump_factor(level);
                if power > 1.0e-5 {
                    e.delta = Vec3::new(e.delta.x, (power as f64).max(e.delta.y), e.delta.z);
                    e.needs_sync = true;
                }
            }
        }
        let s = st_mut(m);
        if s.roar_tick > 0 {
            s.roar_tick -= 1;
            if s.roar_tick == 10 {
                roar(e, m, level);
            }
        }
        let s = st_mut(m);
        if s.attack_tick > 0 {
            s.attack_tick -= 1;
        }
        if s.stunned_tick > 0 {
            s.stunned_tick -= 1;
            // `stunEffect`: particles at the head, one tick in six.
            if e.random.next_int_bounded(6) == 0 {
                let _ = e.random.next_double();
                let _ = e.random.next_double();
            }
            if st(m).stunned_tick == 0 {
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event("minecraft:entity.ravager.roar"), source: "hostile", volume: 1.0, pitch: 1.0 });
                }
                st_mut(m).roar_tick = 20;
            }
        }
    }

    fn is_immobile(&self, m: &MobData) -> bool {
        let s = st(m);
        s.attack_tick > 0 || s.stunned_tick > 0 || s.roar_tick > 0
    }

    fn can_see(&self, m: &MobData) -> bool {
        st(m).stunned_tick <= 0 && st(m).roar_tick <= 0
    }

    /// `Ravager.doHurtTarget`: the attack animation, then `Mob.doHurtTarget` with the attack
    /// knockback (the ravager slows to 60% of its own motion).
    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        st_mut(m).attack_tick = 10;
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event("minecraft:entity.ravager.attack"), source: "hostile", volume: 1.0, pitch: 1.0 });
        }
        let hurt = mob::do_hurt_target_base(e, m, level, t);
        if hurt {
            let kb = (m.attrs.value(Attr::AttackKnockback) as f32) / 2.0;
            if kb > 0.0 {
                let yaw = e.y_rot * 0.017453292;
                let (s, c) = (mob::mth::sin(yaw as f64), mob::mth::cos(yaw as f64));
                knockback_target(level, t.id, kb as f64, s as f64, -c as f64);
                e.delta = e.delta.multiply(0.6, 1.0, 0.6);
            }
        }
        Some(hurt)
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        raider::die(e, m, level, source);
    }

    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        Some(raider::remove_when_far_away(m, dist_sqr))
    }

    fn can_attack(&self, _m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        // The villager target's selector (`!target.isBaby()`); golems and players always.
        !(raider::is_villager_type(t.type_name) && raider::is_baby(level, t.id))
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        raider::load(m, r);
        let s = st_mut(m);
        s.attack_tick = r.int_or("AttackTick", 0);
        s.stunned_tick = r.int_or("StunTick", 0);
        s.roar_tick = r.int_or("RoarTick", 0);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        raider::save(m, o);
        let s = st(m);
        o.put("AttackTick", Tag::Int(s.attack_tick));
        o.put("StunTick", Tag::Int(s.stunned_tick));
        o.put("RoarTick", Tag::Int(s.roar_tick));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::raider::IS_CELEBRATING, &DataValue::Boolean(st(m).raider.celebrating));
    }

    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(super::evoker::experience_with_equipment(e, m, 20))
    }
}

/// `LivingEntity.knockback` of a hit target (a mob, or a player through its stand-in).
fn knockback_target(level: &mut dyn EntityLevel, id: i32, strength: f64, dx: f64, dz: f64) {
    raider::knockback_other(level, id, strength, dx, dz);
}

/// `Ravager.blockedByItem`: a shield blocked its bite. Half the time it is stunned for two
/// seconds (entity event 39), else the defender is thrown back.
pub fn blocked_by_shield(e: &mut Entity, level: &mut dyn EntityLevel, defender: i32) {
    let Some(m) = mob::data(e) else { return };
    if raider::illager(m).is_none_or(|s| s.roar_tick != 0) {
        return;
    }
    if e.random.next_double() < 0.5 {
        if let Some(s) = mob::data_mut(e).and_then(raider::illager_mut) {
            s.stunned_tick = 40;
        }
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event("minecraft:entity.ravager.stunned"), source: "hostile", volume: 1.0, pitch: 1.0 });
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 39 });
        // `defender.push(this)`.
        if let Some(d) = level.entity(defender).map(|d| d.position()) {
            let (dx, dz) = (e.x() - d.x, e.z() - d.z);
            let mut dd = dx.abs().max(dz.abs());
            if dd >= 0.009999999776482582 {
                dd = dd.sqrt();
                let f = (1.0 / dd).min(1.0);
                let (x, z) = (dx / dd * f * 0.05000000074505806, dz / dd * f * 0.05000000074505806);
                raider::push_other(level, defender, -x, 0.0, -z);
                e.delta = e.delta.add(x, 0.0, z);
            }
        }
    } else {
        strong_knockback(e, level, defender);
    }
}

/// `strongKnockback`: a push of 4 over the squared horizontal distance, and 0.2 up.
fn strong_knockback(e: &Entity, level: &mut dyn EntityLevel, id: i32) {
    let Some(p) = level.entity(id).map(|o| o.position()) else { return };
    let (dx, dz) = (p.x - e.x(), p.z - e.z());
    let d = (dx * dx + dz * dz).max(0.001);
    raider::push_other(level, id, dx / d * 4.0, 0.2, dz / d * 4.0);
}

/// `Ravager.roar`: everything living within 4 (but other ravagers) is hurt for 6 unless an
/// illager, and thrown back unless a player.
fn roar(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !mob::is_alive(e, m) {
        return;
    }
    let griefing = level.mob_griefing();
    let area = e.bounding_box().inflate(4.0, 4.0, 4.0);
    for id in level.entities_in(&area, EntityFilter::Living, e.id) {
        let Some(t) = crate::mob::goals::living(level, id) else { continue };
        if !t.alive || t.type_name == "minecraft:ravager" || (!griefing && t.type_name == "minecraft:armor_stand") {
            continue;
        }
        if !raider::is_illager(level.entity(id).and_then(mob::data).map_or(crate::mob::MobKind::Pig, |om| om.kind)) {
            let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
            mob::hurt_living(level, &t, source, 6.0);
        }
        if !t.player {
            strong_knockback(e, level, id);
        }
    }
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    level.emit(Event::EntityEvent { entity: e.id, event: 69 });
}
