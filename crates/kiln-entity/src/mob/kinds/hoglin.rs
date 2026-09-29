//! Hoglin: attacks players and throws them, breeds with crimson fungus, flees warped fungus.
//!
//! Vanilla drives hoglins with a `Brain` (`HoglinAi`: fight, avoid and idle activities). Kiln
//! approximates it with goals: attacking the nearest visible player (the hit's random damage and
//! the upward throw are vanilla's `HoglinBase`), anger at attackers shared with nearby hoglins,
//! breeding and following parents, fleeing hoglin repellents (warped fungus, portals, respawn
//! anchors) within 8 blocks and staying peaceful for 10 s after, strolling and looking around.
//! Retreating from outnumbering piglins and babies avoiding piglins are not simulated. There is
//! no zoglin type yet, so hoglins outside the nether count their time but do not convert.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, Living, MOVE, MeleeKind, TARGET};
use crate::mob::interact::{Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, mth, random_pos};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Hoglin;

pub static KIND: Hoglin = Hoglin;

static INFO: Info = Info {
    animal: true,
    ageable: true,
    sounds: Some("hoglin"),
    sound_source: "hostile",
    extends_monster: false,
    ..Info::monster("minecraft:hoglin", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (KnockbackResistance, 0.6000000238418579), (AttackKnockback, 1.0), (AttackDamage, 6.0)])
};

#[derive(Clone, Debug, Default)]
pub struct HoglinState {
    pub immune_to_zombification: bool,
    pub time_in_overworld: i32,
    pub cannot_be_hunted: bool,
    pub attack_animation: i32,
    /// `PACIFIED` ticks left (a repellent was near).
    pub pacified: i32,
    /// The nearest repellent block (`NEAREST_REPELLENT`), refreshed every second.
    pub repellent: Option<BlockPos>,
}

pub fn state(m: &MobData) -> Option<&HoglinState> {
    ext::state::<HoglinState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut HoglinState> {
    ext::state_mut::<HoglinState>(m)
}

/// `HoglinSpecificSensor.findNearestRepellent`: a `#minecraft:hoglin_repellents` block within 8
/// blocks horizontally and 4 vertically, nearest first.
fn find_repellent(e: &Entity, level: &dyn EntityLevel) -> Option<BlockPos> {
    let c = e.block_position();
    let mut best: Option<(i32, BlockPos)> = None;
    for dy in -4..=4 {
        for dx in -8..=8 {
            for dz in -8..=8 {
                let p = BlockPos::new(c.x + dx, c.y + dy, c.z + dz);
                if !is_repellent(level.block(p)) {
                    continue;
                }
                let d = dx * dx + dy * dy + dz * dz;
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
        }
    }
    best.map(|b| b.1)
}

fn is_repellent(state: u16) -> bool {
    let name = crate::blocks::block_name(state);
    matches!(name, "minecraft:warped_fungus" | "minecraft:potted_warped_fungus" | "minecraft:nether_portal" | "minecraft:respawn_anchor")
}

/// `HoglinBase.throwTarget`: the target flies up and away, turned by a random angle (given to
/// `Vec3.yRot` in degrees-sized radians, as vanilla does).
pub fn throw_target(e: &Entity, m: &MobData, level: &mut dyn EntityLevel, target: i32, resistance: f64) {
    let strength = m.attrs.value(AttackKnockback) - resistance;
    if strength <= 0.0 {
        return;
    }
    let Some(t) = level.entity(target).map(|t| t.position()).or_else(|| level.player(target).map(|p| p.pos)) else { return };
    let (dx, dz) = (t.x - e.x(), t.z - e.z());
    let r = level.random();
    let angle = (r.next_int_bounded(21) - 10) as f32;
    let horiz = strength * (r.next_float() * 0.5 + 0.2) as f64;
    let v = Vec3::new(dx, 0.0, dz).normalize().scale(horiz);
    let (c, s) = (mth::cos(angle as f64) as f64, mth::sin(angle as f64) as f64);
    let v = Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s);
    let up = strength * r.next_float() as f64 * 0.5;
    if let Some(o) = level.entity_mut(target) {
        o.delta = o.delta.add(v.x, up, v.z);
        o.needs_sync = true;
    }
}

/// Targets the nearest visible attackable player (the brain's `StartAttacking`), unless
/// pacified or breeding.
#[derive(Clone, Debug)]
struct HoglinTargetGoal;

fn attackable_player(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, id: i32) -> Option<f64> {
    let p = level.player(id)?;
    if !p.alive || p.spectator || p.creative {
        return None;
    }
    let t = goals::living(level, id)?;
    let d = e.position().distance_to_sqr(p.pos);
    let range = m.attrs.value(FollowRange);
    (d <= range * range && mob::has_line_of_sight_cached(e, m, level, &t)).then_some(d)
}

impl CustomGoal for HoglinTargetGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "HoglinTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.in_love > 0 || state(m).is_none_or(|s| s.pacified > 0) {
            return false;
        }
        let mut best: Option<(f64, i32)> = None;
        for p in level.players() {
            if let Some(d) = attackable_player(e, m, level, p.id)
                && best.is_none_or(|b| d < b.0)
            {
                best = Some((d, p.id));
            }
        }
        let Some((_, id)) = best else { return false };
        m.target = Some(id);
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = m.target else { return false };
        state(m).is_some_and(|s| s.pacified == 0) && attackable_player(e, m, level, t).is_some()
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
    }
}

/// Runs from a repellent (`SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, true)`).
#[derive(Clone, Debug)]
struct HoglinAvoidRepellentGoal {
    to: Vec3,
}

impl CustomGoal for HoglinAvoidRepellentGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "HoglinAvoidRepellentGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(r) = state(m).and_then(|s| s.repellent) else { return false };
        let from = Vec3::new(r.x as f64 + 0.5, r.y as f64, r.z as f64 + 0.5);
        if e.position().distance_to_sqr(from) > 8.0 * 8.0 {
            return false;
        }
        let Some(to) = random_pos::land_pos_away(e, m, level, 16, 7, from) else { return false };
        self.to = to;
        true
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        mob::path::move_to(e, m, level, self.to.x, self.to.y, self.to.z, 1.0);
    }
}

/// `ageBoundaryReached`: babies hit for 0.5.
fn update_attack_damage(m: &mut MobData) {
    let base = if m.baby() { 0.5 } else { 6.0 };
    if let Some(a) = m.attrs.get_mut(AttackDamage) {
        a.base = base;
    }
}

impl Kind for Hoglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(HoglinState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(HoglinAvoidRepellentGoal { to: Vec3::ZERO })));
        g.add(2, Goal::Breed { speed: 0.6, partner: None, love_time: 0 });
        g.add(3, Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(5, Goal::FollowParent { speed: 0.6, parent: None, recalc: 0 });
        g.add(6, Goal::RandomStroll { speed: 0.4, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(HoglinTargetGoal)));
    }

    fn ai_step_before(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if let Some(st) = state_mut(m)
            && st.attack_animation > 0
        {
            st.attack_animation -= 1;
        }
    }

    fn post_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        update_attack_damage(m);
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // The sensor looks for repellents; one nearby pacifies for 200 ticks.
        if e.tick_count % 20 == 0 {
            let r = find_repellent(e, level);
            if let Some(st) = state_mut(m) {
                st.repellent = r;
            }
        }
        let converting = level.piglins_zombify() && !m.no_ai;
        if let Some(st) = state_mut(m) {
            if st.repellent.is_some() {
                st.pacified = 200;
            } else if st.pacified > 0 {
                st.pacified -= 1;
            }
            let converting = converting && !st.immune_to_zombification;
            // Zoglins are not simulated: the time counts, the conversion never comes.
            st.time_in_overworld = if converting { st.time_in_overworld + 1 } else { 0 };
        }
        m.set_aggressive(m.target.is_some());
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        // `HoglinAi.wasHurtBy`: no longer pacified; an adult retaliates at once (the attack
        // target is set even without AI) against an attackable attacker in follow range.
        if !hurt {
            return;
        }
        if let Some(st) = state_mut(m) {
            st.pacified = 0;
        }
        let Some(a) = source.attacker else { return };
        if m.baby() {
            return;
        }
        let range = m.attrs.value(FollowRange);
        let ok = match level.player(a) {
            Some(p) => p.alive && !p.creative && !p.spectator && e.position().distance_to_sqr(p.pos) <= range * range,
            None => level.entity(a).is_some_and(|o| o.type_name != "minecraft:hoglin" && o.is_alive() && e.position().distance_to_sqr(o.position()) <= range * range),
        };
        if ok {
            m.target = Some(a);
        }
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        if let Some(st) = state_mut(m) {
            st.attack_animation = 10;
        }
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        mob::make_sound(e, m, level, mob::sound_event("minecraft:entity.hoglin.attack"));
        // `HoglinBase.hurtAndThrowTarget`.
        let base = m.attrs.value(AttackDamage) as f32;
        let damage = if !m.baby() && base as i32 > 0 { base / 2.0 + level.random().next_int_bounded(base as i32) as f32 } else { base };
        let source = DamageSource { kind: crate::level::DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        let hurt = if t.player {
            level.hurt_player(t.id, source, damage)
        } else {
            match level.entity_mut(t.id) {
                Some(o) => {
                    let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", 0, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0));
                    let r = mob::hurt_entity(&mut o2, level, source, damage);
                    if let Some(slot) = level.entity_mut(t.id) {
                        *slot = o2;
                    }
                    r
                }
                None => false,
            }
        };
        if hurt {
            m.last_hurt_mob = Some(t.id);
            if !m.baby() {
                let resistance = level.entity(t.id).and_then(mob::data).map_or(0.0, |o| o.attrs.value(KnockbackResistance));
                throw_target(e, m, level, t.id, resistance);
            }
        }
        Some(hurt)
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        if r.next_float() < 0.2 {
            mob::set_age(e, m, mob::breed::BABY_START_AGE);
        }
        ext::ageable_finalize(e, m, r, group, 0.05);
        update_attack_damage(m);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let immune = r.bool_or("IsImmuneToZombification", false);
        let time = r.int_or("TimeInOverworld", 0);
        let hunted = r.bool_or("CannotBeHunted", false);
        update_attack_damage(m);
        let Some(st) = state_mut(m) else { return };
        st.immune_to_zombification = immune;
        st.time_in_overworld = time;
        st.cannot_be_hunted = hunted;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if st.immune_to_zombification {
            o.put("IsImmuneToZombification", Tag::Byte(1));
        }
        o.put("TimeInOverworld", Tag::Int(st.time_in_overworld));
        if st.cannot_be_hunted {
            o.put("CannotBeHunted", Tag::Byte(1));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let immune = state(m).is_some_and(|s| s.immune_to_zombification);
        d.set(kiln_data::entities::data::hoglin::IMMUNE_TO_ZOMBIFICATION, &DataValue::Boolean(immune));
    }

    fn interact(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `Hoglin.mobInteract`: feeding (the shared animal code) also makes it persistent.
        if !stack.is_empty() && self.is_food(stack.item()) && ((m.age == 0 && m.in_love <= 0) || (m.age < 0 && !m.age_locked)) {
            m.persistence_required = true;
        }
        None
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:hoglin_food")
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.75, 0.85, 0.625) } else { base }
    }

    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        Some(if crate::blocks::block_name(level.block(p.below())) == "minecraft:crimson_nylium" { 10.0 } else { 0.0 })
    }

    fn experience(&self, _e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(if m.baby() { 3 } else { 5 })
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        Some(true)
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}
