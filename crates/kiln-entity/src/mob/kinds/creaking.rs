//! Creaking: a pale-garden monster that cannot move while a player looks at it (it activates
//! when first looked at within 12 blocks, then hunts that player, freezing whenever watched).
//! One bound to a creaking heart (`home_pos`) cannot be hurt — a hit only makes it sway — and
//! crumbles when its heart is gone.
//!
//! Approximations: vanilla drives it with a `Brain` (`CreakingAi`); here idle wandering and the
//! fight are goals. The creaking heart block entity is not simulated: a heart-bound creaking is
//! protected while a `creaking_heart` block stands at its home, and hearts do not spawn
//! creakings or react to hits.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event, PlayerView};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, LOOK, Living, MOVE};
use crate::mob::{self, DamageSource, MobData, path};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Creaking;

pub static KIND: Creaking = Creaking;

static INFO: Info = Info {
    ..Info::monster("minecraft:creaking", &[(MaxHealth, 1.0), (MovementSpeed, 0.4000000059604645), (AttackDamage, 3.0), (FollowRange, 32.0), (StepHeight, 1.0625)])
};

#[derive(Clone, Debug)]
pub struct State {
    pub can_move: bool,
    pub active: bool,
    pub tearing_down: bool,
    pub home: Option<BlockPos>,
    invulnerability_ticks: i32,
    attack_ticks: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("creaking state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("creaking state")
}

/// Whether its heart protects it (a `creaking_heart` block at home).
fn protected(m: &MobData, level: &dyn EntityLevel) -> bool {
    st(m).home.is_some_and(|h| crate::blocks::block_name(level.block(h)) == "minecraft:creaking_heart")
}

/// `isLookingAtMe(player, 0.5, false, true, eyes, y + 0.5, middle)`.
fn looked_at_by(e: &Entity, level: &dyn EntityLevel, p: &PlayerView) -> bool {
    let look = crate::ext_entity::fireball::view_vector(p.pitch, p.yaw).normalize();
    let eye = p.pos.y + p.eye_height as f64;
    for h in [e.eye_y(), e.y() + 0.5, (e.eye_y() + e.y()) / 2.0] {
        let dir = Vec3::new(e.x() - p.pos.x, h - eye, e.z() - p.pos.z).normalize();
        let dot = look.x * dir.x + look.y * dir.y + look.z * dir.z;
        if dot > 1.0 - 0.5 && !mob::clip_blocks(level, Vec3::new(p.pos.x, eye, p.pos.z), Vec3::new(e.x(), h, e.z())) {
            return true;
        }
    }
    false
}

/// `checkCanMove`: frozen while an attackable player (not in a carved pumpkin, once active)
/// looks at it; the first look within 12 blocks activates it against that player.
fn check_can_move(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    let range = m.attrs.value(FollowRange);
    let players: Vec<PlayerView> = level.players().iter().filter(|p| p.alive && !p.spectator && p.pos.distance_to_sqr(e.position()) <= range * range).copied().collect();
    let active = st(m).active;
    if players.is_empty() {
        if active {
            deactivate(e, m, level);
        }
        return true;
    }
    let pumpkin = kiln_data::builtin_id("minecraft:item", "minecraft:carved_pumpkin");
    let mut potential = false;
    for p in &players {
        let t = goals::living_player(p);
        if !goals::can_attack(m, level, &t) {
            continue;
        }
        potential = true;
        if (!active || Some(p.head) != pumpkin) && looked_at_by(e, level, p) {
            if active {
                return false;
            }
            if p.pos.distance_to_sqr(e.position()) < 144.0 {
                // `activate`.
                mob::set_target(e, m, Some(p.id));
                level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
                mob::make_sound(e, m, level, "minecraft:entity.creaking.activate");
                st_mut(m).active = true;
                return false;
            }
        }
    }
    if !potential && active {
        deactivate(e, m, level);
    }
    true
}

fn deactivate(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    mob::set_target(e, m, None);
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    mob::make_sound(e, m, level, "minecraft:entity.creaking.deactivate");
    st_mut(m).active = false;
}

impl Kind for Creaking {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        Some(Box::new(State { can_move: true, active: false, tearing_down: false, home: None, invulnerability_ticks: 0, attack_ticks: 0 }))
    }

    fn register_goals(&self, m: &mut MobData) {
        m.goals.add(0, Goal::Float);
        m.goals.add(1, Goal::Custom(Box::new(CreakingAttack { cooldown: 0, recalc: 0 })));
        m.goals.add(5, Goal::RandomStroll { speed: 0.3, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        m.goals.add(6, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
    }

    /// `Creaking.tick`: without its heart a bound creaking dies.
    fn pre_tick(&self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).home.is_some() && !protected(m, level) {
            m.set_health(0.0);
        }
    }

    /// `Creaking.aiStep` before `LivingEntity.aiStep`: the animation timers and freezing.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        s.invulnerability_ticks = (s.invulnerability_ticks - 1).max(0);
        s.attack_ticks = (s.attack_ticks - 1).max(0);
        let was = st(m).can_move;
        let now = check_can_move(e, m, level);
        if now != was {
            level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
            if now {
                mob::make_sound(e, m, level, "minecraft:entity.creaking.unfreeze");
            } else {
                // `stopInPlace`.
                m.nav.stop();
                m.xxa = 0.0;
                m.yya = 0.0;
                mob::control::set_speed(m, 0.0);
                e.delta = Vec3::new(0.0, e.delta.y, 0.0);
                mob::make_sound(e, m, level, "minecraft:entity.creaking.freeze");
            }
        }
        st_mut(m).can_move = now;
    }

    /// Frozen: no AI, no steps (`CreakingMoveControl`, `CreakingLookControl`, `CreakingJumpControl`).
    fn is_immobile(&self, m: &MobData) -> bool {
        !st(m).can_move
    }

    fn tick_body(&self, _e: &mut Entity, m: &mut MobData) -> bool {
        !st(m).can_move
    }

    /// Heart-bound: fire cannot hurt it.
    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        st(m).home.is_some() && kind.is_tag("minecraft:is_fire")
    }

    /// `hurtServer`: a heart-bound creaking sways instead of taking damage; a frozen one takes
    /// no knockback.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        if st(m).home.is_none() || source.kind.is_tag("minecraft:bypasses_invulnerability") {
            let delta = e.delta;
            let r = mob::hurt_base(e, m, level, *source, amount);
            if !st(m).can_move {
                e.delta = delta;
            }
            return Some(r);
        }
        if st(m).invulnerability_ticks > 0 || m.is_dead_or_dying() {
            return Some(false);
        }
        let living_or_player = source.direct.or(source.attacker).is_some_and(|d| goals::living(level, d).is_some() || level.entity(d).is_some());
        if !living_or_player && !source.attacker_is_player {
            return Some(false);
        }
        st_mut(m).invulnerability_ticks = 8;
        level.emit(Event::EntityEvent { entity: e.id, event: 66 });
        level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
        if protected(m, level) {
            mob::make_sound(e, m, level, "minecraft:entity.creaking.sway");
        }
        Some(true)
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        st_mut(m).attack_ticks = 15;
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        Some(mob::do_hurt_target_base(e, m, level, t))
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(if st(m).active { None } else { Some("minecraft:entity.creaking.ambient") })
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(0)
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    /// Heart-bound creakings do not despawn with the others.
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        st(m).home.is_some().then_some(false)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let home = match r.get("home_pos") {
            Some(Tag::IntArray(a)) if a.len() == 3 => Some(BlockPos::new(a[0], a[1], a[2])),
            _ => None,
        };
        if home.is_some() {
            // `setTransient`: hazards cost less to a creaking that cannot be hurt.
            m.maluses.push((path::PathType::Damaging, 8.0));
            m.maluses.push((path::PathType::PowderSnow, 8.0));
            m.maluses.push((path::PathType::Lava, 8.0));
            m.maluses.push((path::PathType::Fire, 0.0));
            m.maluses.push((path::PathType::FireInNeighbor, 0.0));
        }
        st_mut(m).home = home;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        if let Some(h) = st(m).home {
            o.put("home_pos", Tag::IntArray(vec![h.x, h.y, h.z]));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::creaking as f;
        let s = st(m);
        d.set(f::CAN_MOVE, &DataValue::Boolean(s.can_move));
        d.set(f::IS_ACTIVE, &DataValue::Boolean(s.active));
        d.set(f::IS_TEARING_DOWN, &DataValue::Boolean(s.tearing_down));
        d.set(f::HOME_POS, &DataValue::OptionalBlockPos(s.home.map(|h| [h.x, h.y, h.z])));
    }
}

/// The fight activity: walks at its target (`SetWalkTargetFromAttackTargetIfTargetOutOfReach`)
/// and hits it every 40 ticks when it can move (`MeleeAttack`), dropping a target it can no
/// longer see.
#[derive(Clone, Debug)]
struct CreakingAttack {
    cooldown: i32,
    recalc: i32,
}

impl CustomGoal for CreakingAttack {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CreakingAttack"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        st(m).can_move && goals::target(m, level).is_some()
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = goals::target(m, level) else { return };
        if !mob::has_line_of_sight_cached(e, m, level, &t) {
            mob::set_target(e, m, None);
            return;
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 45.0, 90.0);
        self.cooldown = (self.cooldown - 1).max(0);
        if mob::within_melee_range(e, &t) {
            m.nav.stop();
            if self.cooldown == 0 {
                self.cooldown = 40;
                m.swing = true;
                mob::do_hurt_target(e, m, level, &t);
            }
        } else {
            self.recalc -= 1;
            if self.recalc <= 0 {
                self.recalc = 10;
                path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.0);
            }
        }
    }
}
