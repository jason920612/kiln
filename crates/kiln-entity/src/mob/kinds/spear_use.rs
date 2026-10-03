//! Mobs with spears (26.x `kinetic_weapon`): `SpearUseGoal` (zombies, husks, zombie villagers
//! and zombified piglins charge their target, back off and charge again), the charging weapon's
//! damage each tick of the use (`KineticWeapon.damageEntities` for a mob wielder: the speed
//! conditions at 0.2 of a player's, ranges times the item's `mob_factor`) and
//! `LivingEntity.stabAttack`.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, Vec3};
use crate::mob::attributes::Attr;
use crate::mob::ext::CustomGoal;
use crate::mob::goals::{self, LOOK, Living, MOVE};
use crate::mob::{self, DamageSource, MAINHAND, MobData, MobKind, path, random_pos};
use crate::spear::{self, Candidate, KineticHit};
use kiln_item::component::{AttackRange, KineticWeapon};
use kiln_item::{ItemStack, keys};

/// `Mob.chargeSpeedModifier` of the root vehicle (a zombie horse hurries, a camel husk more).
pub(super) fn charge_speed_modifier(kind: MobKind) -> f32 {
    match kind {
        MobKind::ZombieHorse => 1.4,
        MobKind::CamelHusk => 4.0,
        _ => 1.0,
    }
}

/// The root of the vehicles `e` rides: its entity (as the level has it) and, when it is a mob,
/// its kind. The first hop is the mount a steering rider holds.
pub(super) fn root_vehicle(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<(Vec3, Option<MobKind>)> {
    let (mut speed, mut kind, mut next) = match &m.mount {
        Some(c) => (c.e.last_known_speed, Some(c.m.kind), c.e.vehicle),
        None => {
            let v = level.entity(e.vehicle?)?;
            (v.last_known_speed, mob::data(v).map(|d| d.kind), v.vehicle)
        }
    };
    while let Some(id) = next {
        let Some(v) = level.entity(id) else { break };
        speed = v.last_known_speed;
        kind = mob::data(v).map(|d| d.kind);
        next = v.vehicle;
    }
    Some((speed, kind))
}

/// Id of the root of the vehicles of entity `id` (itself when it rides nothing).
fn root_id(level: &dyn EntityLevel, id: i32, vehicle: Option<i32>) -> i32 {
    let (mut root, mut next) = (id, vehicle);
    while let Some(v) = next {
        root = v;
        next = level.entity(v).and_then(|e| e.vehicle);
    }
    root
}

// ---------------------------------------------------------------------- SpearUseGoal

#[derive(Clone, Debug)]
struct State {
    engage_time: i32,
    fleeing_time: i32,
    away: Option<Vec3>,
    done: bool,
}

/// `SpearUseGoal(mob, speedWhenCharging, speedWhenRepositioning, approachDistance, targetInRangeRadius)`.
#[derive(Clone, Debug)]
pub struct SpearUseGoal {
    charging: f64,
    repositioning: f64,
    approach_sq: f32,
    in_range_sq: f32,
    state: Option<State>,
}

impl SpearUseGoal {
    pub fn new(charging: f64, repositioning: f64, approach: f32, in_range: f32) -> SpearUseGoal {
        SpearUseGoal { charging, repositioning, approach_sq: approach * approach, in_range_sq: in_range * in_range, state: None }
    }
}

/// `Goal.reducedTickDelay`.
fn reduced(ticks: i32) -> i32 {
    (ticks + 1).div_euclid(2)
}

fn able_to_attack(m: &MobData, level: &dyn EntityLevel) -> bool {
    // (The weapon first: most mobs hold none, and the target takes a lookup.)
    m.equipment[MAINHAND].get(keys::KINETIC_WEAPON).is_some() && goals::target(m, level).is_some()
}

/// `MAX_FLEEING_TIME`.
fn max_fleeing_time() -> f64 {
    reduced(100) as f64
}

impl CustomGoal for SpearUseGoal {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SpearUseGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        able_to_attack(m, level) && m.using_item.is_none()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.state.as_ref().is_some_and(|s| !s.done) && able_to_attack(m, level)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.set_aggressive(true);
        self.state = Some(State { engage_time: -1, fleeing_time: -1, away: None, done: false });
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav_mut().stop();
        m.set_aggressive(false);
        self.state = None;
        m.stop_using_item();
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.state.is_none() {
            return;
        }
        let Some(t) = goals::target(m, level) else { return };
        let d = e.position().distance_to_sqr(t.pos);
        let charge = root_vehicle(e, m, level).and_then(|(_, k)| k).map_or(1.0, charge_speed_modifier);
        let extra: i32 = if e.vehicle.is_some() { 2 } else { 0 };
        mob::mob_look_at(e, &t, 30.0, 30.0);
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        let target_block = crate::math::BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
        if self.state.as_ref().unwrap().engage_time < 0 {
            if d > self.approach_sq as f64 {
                path::move_to_entity(e, m, level, target_block, charge as f64 * self.repositioning);
                return;
            }
            let duration = reduced(m.equipment[MAINHAND].get(keys::KINETIC_WEAPON).map_or(0, spear::damage_use_duration));
            self.state.as_mut().unwrap().engage_time = duration;
            m.start_using_item();
        }
        // `tickAndCheckEngagement`.
        let engaged = {
            let s = self.state.as_mut().unwrap();
            if s.engage_time > 0 {
                s.engage_time -= 1;
                s.engage_time == 0
            } else {
                false
            }
        };
        if engaged {
            m.stop_using_item();
            let dist = d.sqrt();
            let away = random_pos::land_pos_away_between(e, m, level, 0.0f64.max((9 + extra) as f64 - dist), 1.0f64.max((11 + extra) as f64 - dist), 7, t.pos);
            let s = self.state.as_mut().unwrap();
            s.away = away;
            s.fleeing_time = 1;
        }
        // `tickAndCheckFleeing`.
        {
            let s = self.state.as_mut().unwrap();
            if s.fleeing_time > 0 {
                s.fleeing_time += 1;
                if s.fleeing_time as f64 > max_fleeing_time() {
                    s.done = true;
                    return;
                }
            }
        }
        let away = self.state.as_ref().unwrap().away;
        if let Some(a) = away {
            path::move_to(e, m, level, a.x, a.y, a.z, charge as f64 * self.repositioning);
            if m.nav_ref().is_done() {
                let s = self.state.as_mut().unwrap();
                if s.fleeing_time > 0 {
                    s.done = true;
                    return;
                }
                s.away = None;
            }
        } else {
            path::move_to_entity(e, m, level, target_block, charge as f64 * self.charging);
            if d < self.in_range_sq as f64 || m.nav_ref().is_done() {
                let dist = d.sqrt();
                let away = random_pos::land_pos_away_between(e, m, level, (6 + extra) as f64 - dist, (7 + extra) as f64 - dist, 7, t.pos);
                self.state.as_mut().unwrap().away = away;
            }
        }
    }
}

// ---------------------------------------------------------------------- charging

/// `ItemStack.onUseTick` of a mob's charging weapon: `KineticWeapon.damageEntities` with the
/// ticks of use so far (the counter, as it stands before this tick adds one).
pub fn kinetic_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let held = m.equipment[MAINHAND].clone();
    let Some(kinetic) = held.get(keys::KINETIC_WEAPON).cloned() else { return };
    let used = m.ticks_using_item();
    if used < kinetic.delay_ticks {
        return;
    }
    let ticks = used - kinetic.delay_ticks;
    let look = e.view_vector();
    let head = crate::ext_entity::fireball::view_vector(e.x_rot, m.y_head_rot);
    let eye = Vec3::new(e.x(), e.eye_y(), e.z());
    let own_speed = root_vehicle(e, m, level).map_or(e.last_known_speed, |(s, _)| s).scale(spear::MOTION_SCALE);
    let my_speed = look.dot(own_speed);
    let range = held.get(keys::ATTACK_RANGE).cloned().unwrap_or_default_range();
    let (min, max) = spear::effective_range(&range, false, false);
    let base_damage = m.attrs.base(Attr::AttackDamage);
    let known = e.delta;
    // The entities near the line of the stab (`PiercingWeapon.canHitEntity`: alive, not invulnerable,
    // pickable, not riding the wielder's vehicle): mobs, players, boats, minecarts and the like, by
    // id. An ender dragon is hit through its parts.
    let end = eye + head.scale(max as f64 + 0.0f64.max(known.dot(head)));
    let margin = range.hitbox_margin as f64;
    // (A dragon's parts reach far from its own box: look wide enough to find the dragon; the stab
    // itself tests each box exactly.)
    let wide = margin + 2.0 + 12.0;
    let area = Aabb::new(eye.x.min(end.x), eye.y.min(end.y), eye.z.min(end.z), eye.x.max(end.x), eye.y.max(end.y), eye.z.max(end.z)).inflate(wide, wide, wide);
    let own_root = root_id(level, e.id, e.vehicle);
    let mut victims: Vec<(Victim, Candidate)> = Vec::new();
    for id in level.entities_in(&area, EntityFilter::Any, e.id) {
        let Some(o) = level.entity(id) else { continue };
        if o.is_removed() || o.invulnerable || root_id(level, id, o.vehicle) == own_root {
            continue;
        }
        match mob::data(o) {
            Some(d) if d.health <= 0.0 || d.dead => continue,
            Some(_) => {
                let Some(l) = goals::living(level, id) else { continue };
                if let Some(dragon) = mob::kinds::ender_dragon::state_of(o) {
                    // (`EnderDragon.isPickable` is false; its parts are what a stab meets.)
                    for i in 0..dragon.parts.len() {
                        victims.push((Victim::Living(l.clone()), Candidate { id, bb: dragon.part_box(i) }));
                    }
                } else {
                    victims.push((Victim::Living(l), Candidate { id, bb: o.bounding_box() }));
                }
            }
            None if spear::pickable_non_mob(o) => {
                victims.push((Victim::Other { id, type_name: o.type_name }, Candidate { id, bb: o.bounding_box() }));
            }
            None => {}
        }
    }
    for p in level.players_in(&area) {
        if !p.alive || p.spectator || root_id(level, p.id, p.vehicle) == own_root {
            continue;
        }
        let l = goals::living_player(&p);
        let bb = l.bb;
        victims.push((Victim::Living(l), Candidate { id: p.id, bb }));
    }
    victims.sort_by_key(|(_, c)| c.id);
    let candidates: Vec<Candidate> = victims.iter().map(|(_, c)| *c).collect();
    let hits = {
        let lv: &dyn EntityLevel = &*level;
        spear::hit_entities_along(eye, head, min, max, known, range.hitbox_margin, &|a, b| spear::clip_collider(lv, a, b), &candidates)
    };
    let now = level.game_time();
    let mut any = false;
    for hit in hits {
        let Some((victim, _)) = victims.iter().find(|(_, c)| c.id == hit.id) else { continue };
        let victim_id = victim.id();
        if m.recent_stabs.iter().any(|&(id, at)| id == victim_id && now - at < kinetic.contact_cooldown_ticks as i64) {
            continue;
        }
        m.recent_stabs.retain(|&(id, _)| id != victim_id);
        m.recent_stabs.push((victim_id, now));
        // `getMotion(target)`: a root vehicle's known speed, a player's own.
        let theirs = if matches!(victim, Victim::Living(t) if t.player) {
            level.known_movement(victim_id).scale(spear::MOTION_SCALE)
        } else {
            let o = level.entity(victim_id);
            let root = o.and_then(|o| o.vehicle).map(|_| root_id(level, victim_id, o.and_then(|o| o.vehicle)));
            match root.and_then(|r| level.entity(r)).or(o) {
                Some(r) => r.last_known_speed.scale(spear::MOTION_SCALE),
                None => Vec3::ZERO,
            }
        };
        let their_speed = look.dot(theirs);
        let Some(h) = spear::kinetic_hit(&kinetic, ticks, my_speed, their_speed, 0.2, base_damage) else { continue };
        let victim = victim.clone();
        // The dragon part the stab reached takes the blow (`EnderDragonPart.hurtServer`).
        if let Some(o) = level.entity_mut(victim_id)
            && mob::kinds::ender_dragon::state_of(o).is_some()
        {
            mob::kinds::ender_dragon::aim_at(o, hit.location);
        }
        any |= match &victim {
            Victim::Living(t) => stab_attack(e, m, level, &held, t, &h),
            Victim::Other { id, type_name } => stab_other(e, m, level, &held, *id, type_name, &h),
        };
    }
    if any {
        level.emit(Event::EntityEvent { entity: e.id, event: 2 });
    }
}

trait DefaultRange {
    fn unwrap_or_default_range(self) -> AttackRange;
}

impl DefaultRange for Option<AttackRange> {
    /// `AttackRange.defaultFor(entity)`: the entity interaction range (3), no margin.
    fn unwrap_or_default_range(self) -> AttackRange {
        self.unwrap_or(AttackRange { min_reach: 0.0, max_reach: 3.0, min_creative_reach: 0.0, max_creative_reach: 3.0, hitbox_margin: 0.0, mob_factor: 1.0 })
    }
}

/// Whom a stab reached: a living thing, or another entity that can be hit (a boat, a minecart...).
#[derive(Clone, Debug)]
enum Victim {
    Living(Living),
    Other { id: i32, type_name: &'static str },
}

impl Victim {
    fn id(&self) -> i32 {
        match self {
            Victim::Living(t) => t.id,
            Victim::Other { id, .. } => *id,
        }
    }
}

/// `LivingEntity.stabAttack(slot, target, amount, damage, knockback, dismount)` of a mob at a
/// living target: the damage through the weapon's enchantments, the hit, the push (0.4 and the
/// attack knockback with the weapon's enchantments), the dismount, the weapon's post-attack
/// effects.
fn stab_attack(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, weapon: &ItemStack, t: &Living, h: &KineticHit) -> bool {
    // `ItemStack.getDamageSource`: the weapon's damage type, else a mob attack.
    let kind = match weapon.get(keys::DAMAGE_TYPE).and_then(|d| kiln_item::registry::DAMAGE_TYPE.name(d.0)) {
        Some(name) => DamageKind::of_type(name),
        None => DamageKind::MobAttack,
    };
    let source = DamageSource { kind, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    let amount = mob::weapon::modify_damage(level, e, m, t, kind, h.amount);
    let old = level.motion(t.id);
    let mut landed = h.knockback;
    let hurt = h.damage && mob::hurt_living_by(level, e, m, t, source, amount);
    landed |= hurt;
    if h.knockback {
        mob::weapon::cause_extra_knockback(e, level, t, 0.4, old);
        let strength = mob::weapon::attack_knockback(level, e, m, t, kind);
        mob::weapon::cause_extra_knockback(e, level, t, strength, old);
    }
    if h.dismount {
        let passenger = if t.player { level.player(t.id).is_some_and(|p| p.vehicle.is_some()) } else { level.entity(t.id).is_some_and(|o| o.vehicle.is_some()) };
        if passenger && !mob::entity_type_tag(t.type_name, "minecraft:cannot_be_dismounted_by_item_usage") {
            landed = true;
            level.stop_riding(t.id);
        }
    }
    if hurt {
        mob::weapon::post_attack(level, e, m, t, kind);
    }
    if !landed {
        return false;
    }
    m.last_hurt_mob = Some(t.id);
    true
}

/// `LivingEntity.stabAttack` at an entity that is not living (a boat, a minecart, an end crystal):
/// it takes the hit (a fireball or wind charge only has `markHurt` happen), a rider is thrown
/// off, and there is no push.
fn stab_other(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, weapon: &ItemStack, id: i32, type_name: &'static str, h: &KineticHit) -> bool {
    let kind = match weapon.get(keys::DAMAGE_TYPE).and_then(|d| kiln_item::registry::DAMAGE_TYPE.name(d.0)) {
        Some(name) => DamageKind::of_type(name),
        None => DamageKind::MobAttack,
    };
    let source = DamageSource { kind, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    // (`modifyDamage` has the weapon's enchantments weigh the target as an entity.)
    let target = Living { id, type_name, pos: level.entity(id).map_or(Vec3::ZERO, |o| o.position()), eye_y: 0.0, alive: true, player: false, creative: false, spectator: false, invulnerable: false, sneaking: false, invisible: false, armor_cover: 0.0, bb: level.entity(id).map_or(Aabb::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0), |o| o.bounding_box()) };
    let amount = mob::weapon::modify_damage(level, e, m, &target, kind, h.amount);
    let mut landed = false;
    // `Projectile.hurtServer` (fireballs, wind charges) does nothing but `markHurt`.
    if h.damage && !spear::redirectable_projectile(type_name) {
        landed |= crate::ext_entity::fireball::hurt(level, id, source, amount);
    }
    if h.dismount && level.entity(id).is_some_and(|o| o.vehicle.is_some()) && !mob::entity_type_tag(type_name, "minecraft:cannot_be_dismounted_by_item_usage") {
        landed = true;
        level.stop_riding(id);
    }
    if !landed {
        return false;
    }
    m.last_hurt_mob = Some(id);
    true
}

/// The kinetic weapon `m` charges with (its main hand), if any.
pub fn charging_weapon(m: &MobData) -> Option<&KineticWeapon> {
    m.equipment[MAINHAND].get(keys::KINETIC_WEAPON)
}
