//! Evoker: a spellcasting illager (`SpellcasterIllager`) that keeps away from players, summons
//! vexes, sends lines or rings of fangs at its target and turns blue sheep red. Also the
//! spellcasting goals the illusioner shares.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Axis, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, LOOK, MOVE};
use crate::mob::kinds::raider::{self, IllagerState};
use crate::mob::kinds::zombie::{IRON_GOLEM, VILLAGERS};
use crate::mob::goals::Wanted;
use crate::mob::mth::{self, reduced_tick_delay};
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext, Species};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Evoker;

pub static KIND: Evoker = Evoker;

static INFO: Info = Info {
    sounds: Some("evoker"),
    ..Info::monster("minecraft:evoker", &[(MovementSpeed, 0.5), (FollowRange, 12.0), (MaxHealth, 24.0)])
};

fn st(m: &MobData) -> &IllagerState {
    raider::illager(m).expect("spellcaster state")
}

fn st_mut(m: &mut MobData) -> &mut IllagerState {
    raider::illager_mut(m).expect("spellcaster state")
}

/// `SpellcasterIllager.IllagerSpell`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spell {
    SummonVex = 1,
    Fangs = 2,
    Wololo = 3,
    Disappear = 4,
    Blindness = 5,
}

/// `isCastingSpell` (the server's: the casting ticks).
pub fn is_casting(m: &MobData) -> bool {
    st(m).spell_ticks > 0
}

impl Kind for Evoker {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(IllagerState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        raider::register_raider_goals(m);
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Custom(Box::new(CastingSpellGoal { name: "EvokerCastingSpellGoal" })));
        g.add(2, Goal::Custom(Box::new(super::cat::AvoidPlayerGoal::new("AvoidEntityGoal", 8.0, 0.6, 1.0))));
        g.add(3, raider::never());
        g.add(4, Goal::Custom(Box::new(UseSpellGoal::new(Spell::SummonVex))));
        g.add(5, Goal::Custom(Box::new(UseSpellGoal::new(Spell::Fangs))));
        g.add(6, Goal::Custom(Box::new(UseSpellGoal::new(Spell::Wololo))));
        g.add(8, Goal::RandomStroll { speed: 0.6, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::LookAtPlayer { dist: 3.0, probability: 1.0, look_at: None, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(raider::LookAtMobGoal::new(8.0))));
        let t = &mut m.targets;
        t.add(1, raider::hurt_by_ignoring_raiders());
        t.add(2, raider::nearest_with_memory(Wanted::Player, true, 300));
        t.add(3, raider::nearest_with_memory(Wanted::Types(VILLAGERS), false, 300));
        t.add(3, super::zombie::nearest(Wanted::Types(IRON_GOLEM), false));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        raider::ai_step_before(e, m, level);
    }

    /// `SpellcasterIllager.customServerAiStep`: the casting runs down.
    fn custom_server_ai_step(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        if s.spell_ticks > 0 {
            s.spell_ticks -= 1;
        }
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        raider::die(e, m, level, source);
    }

    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        Some(raider::remove_when_far_away(m, dist_sqr))
    }

    fn can_attack(&self, _m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        raider::illager_can_attack(level, t)
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        raider::load(m, r);
        st_mut(m).spell_ticks = r.int_or("SpellTicks", 0);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        raider::save(m, o);
        o.put("SpellTicks", Tag::Int(st(m).spell_ticks));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        spellcaster_data(m, d);
    }

    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(experience_with_equipment(e, m, 10))
    }
}

/// The entity data of a spellcasting illager.
pub fn spellcaster_data(m: &MobData, d: &mut EntityData) {
    use kiln_data::entities::data;
    d.set(data::raider::IS_CELEBRATING, &DataValue::Boolean(st(m).raider.celebrating));
    d.set(data::spellcaster_illager::SPELL_CASTING, &DataValue::Byte(st(m).spell as i8));
}

/// `Mob.getBaseExperienceReward` with an `xpReward` of `base`.
pub fn experience_with_equipment(e: &mut Entity, m: &MobData, base: i32) -> i32 {
    let mut xp = base;
    for i in 0..6 {
        if !m.equipment[i].is_empty() && m.drop_chances[i] <= 1.0 {
            xp += 1 + e.random.next_int_bounded(3);
        }
    }
    xp
}

/// `SpellcasterIllager.setIsCastingSpell`.
fn set_spell(m: &mut MobData, spell: Option<Spell>) {
    st_mut(m).spell = spell.map_or(0, |s| s as u8);
}

// ---------------------------------------------------------------------- goals

/// `SpellcasterCastingSpellGoal` (and the evoker's, which also looks at its wololo sheep).
#[derive(Clone, Debug)]
pub struct CastingSpellGoal {
    pub name: &'static str,
}

impl CustomGoal for CastingSpellGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        st(m).spell_ticks > 0
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_spell(m, None);
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let (max_y, max_x) = (m.kind.max_head_y_rot() as f32, m.max_head_x_rot() as f32);
        let look = match m.target {
            Some(t) => goals::living(level, t),
            None if self.name == "EvokerCastingSpellGoal" => st(m).wololo_target.and_then(|id| goals::living(level, id)),
            None => None,
        };
        if let Some(t) = look {
            m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, max_y, max_x);
        }
    }
}

/// `SpellcasterUseSpellGoal` with each spell's settings and casting.
#[derive(Clone, Debug)]
pub struct UseSpellGoal {
    spell: Spell,
    warmup: i32,
    next_attack_tick: i32,
    /// `IllusionerBlindnessSpellGoal.lastTargetId`.
    last_target: i32,
}

impl UseSpellGoal {
    pub fn new(spell: Spell) -> UseSpellGoal {
        UseSpellGoal { spell, warmup: 0, next_attack_tick: 0, last_target: 0 }
    }

    /// (`getCastWarmupTime`, `getCastingTime`, `getCastingInterval`).
    fn timing(&self) -> (i32, i32, i32) {
        match self.spell {
            Spell::SummonVex => (20, 100, 340),
            Spell::Fangs => (20, 40, 100),
            Spell::Wololo => (40, 60, 140),
            Spell::Disappear => (20, 20, 340),
            Spell::Blindness => (20, 20, 180),
        }
    }

    fn prepare_sound(&self) -> &'static str {
        match self.spell {
            Spell::SummonVex => "minecraft:entity.evoker.prepare_summon",
            Spell::Fangs => "minecraft:entity.evoker.prepare_attack",
            Spell::Wololo => "minecraft:entity.evoker.prepare_wololo",
            Spell::Disappear => "minecraft:entity.illusioner.prepare_mirror",
            Spell::Blindness => "minecraft:entity.illusioner.prepare_blindness",
        }
    }

    /// `SpellcasterUseSpellGoal.canUse`.
    fn base_can_use(&self, e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
        let Some(t) = goals::target(m, level) else { return false };
        t.alive && !is_casting(m) && e.tick_count >= self.next_attack_tick
    }
}

impl CustomGoal for UseSpellGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        match self.spell {
            Spell::SummonVex => "EvokerSummonSpellGoal",
            Spell::Fangs => "EvokerAttackSpellGoal",
            Spell::Wololo => "EvokerWololoSpellGoal",
            Spell::Disappear => "IllusionerMirrorSpellGoal",
            Spell::Blindness => "IllusionerBlindnessSpellGoal",
        }
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match self.spell {
            Spell::Wololo => {
                if m.target.is_some() || is_casting(m) || e.tick_count < self.next_attack_tick || !level.mob_griefing() {
                    return false;
                }
                let sheep = blue_sheep(e, m, level);
                if sheep.is_empty() {
                    return false;
                }
                let i = e.random.next_int_bounded(sheep.len() as i32) as usize;
                st_mut(m).wololo_target = Some(sheep[i]);
                true
            }
            Spell::SummonVex => {
                if !self.base_can_use(e, m, level) {
                    return false;
                }
                let vexes = nearby_vexes(e, level);
                e.random.next_int_bounded(8) + 1 > vexes
            }
            Spell::Disappear => {
                // `!hasEffect(INVISIBILITY)`: mobs keep no effects in Kiln yet.
                self.base_can_use(e, m, level)
            }
            Spell::Blindness => {
                if !self.base_can_use(e, m, level) {
                    return false;
                }
                let Some(t) = m.target else { return false };
                t != self.last_target && level.effective_difficulty(e.block_position()) > 2.0
            }
            Spell::Fangs => self.base_can_use(e, m, level),
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.spell == Spell::Wololo {
            return st(m).wololo_target.is_some() && self.warmup > 0;
        }
        goals::target(m, level).is_some_and(|t| t.alive) && self.warmup > 0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let (warmup, casting, interval) = self.timing();
        self.warmup = reduced_tick_delay(warmup);
        st_mut(m).spell_ticks = casting;
        self.next_attack_tick = e.tick_count + interval;
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event(self.prepare_sound()), source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
        }
        set_spell(m, Some(self.spell));
        if self.spell == Spell::Blindness
            && let Some(t) = m.target
        {
            self.last_target = t;
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if self.spell == Spell::Wololo {
            st_mut(m).wololo_target = None;
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.warmup -= 1;
        if self.warmup != 0 {
            return;
        }
        match self.spell {
            Spell::SummonVex => summon_vexes(e, m, level),
            Spell::Fangs => cast_fangs(e, m, level),
            Spell::Wololo => {
                if let Some(id) = st(m).wololo_target
                    && let Some(sheep) = level.entity_mut(id)
                    && sheep.is_alive()
                    && let Some(sm) = mob::data_mut(sheep)
                    && sm.health > 0.0
                    && let Species::Sheep { color, .. } = &mut sm.species
                {
                    *color = 14;
                }
            }
            Spell::Disappear => {
                level.add_effect(e.id, "minecraft:invisibility", 1200, 0, None);
            }
            Spell::Blindness => {
                if let Some(t) = m.target {
                    level.add_effect(t, "minecraft:blindness", 400, 0, Some(e.id));
                }
            }
        }
        if !e.silent {
            let sound = match m.kind {
                MobKind::Illusioner => "minecraft:entity.illusioner.cast_spell",
                _ => "minecraft:entity.evoker.cast_spell",
            };
            level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event(sound), source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
        }
    }
}

/// Blue sheep within 16 that the evoker can see (`wololoTargeting`).
fn blue_sheep(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> Vec<i32> {
    let area = e.bounding_box().inflate(16.0, 4.0, 16.0);
    let mut out = Vec::new();
    for id in level.entities_in(&area, EntityFilter::Living, e.id) {
        let blue = level.entity(id).and_then(mob::data).is_some_and(|sm| matches!(sm.species, Species::Sheep { color: 11, .. }));
        if !blue {
            continue;
        }
        let Some(t) = goals::living(level, id) else { continue };
        if goals::targeting_ok(e, m, level, &t, false, 16.0, true) {
            out.push(id);
        }
    }
    out
}

/// Vexes within 16 (`vexCountTargeting`: no line of sight needed).
fn nearby_vexes(e: &Entity, level: &dyn EntityLevel) -> i32 {
    let area = e.bounding_box().inflate(16.0, 16.0, 16.0);
    let mut n = 0;
    for id in level.entities_in(&area, EntityFilter::Living, e.id) {
        let Some(o) = level.entity(id) else { continue };
        let alive = mob::data(o).is_some_and(|vm| vm.kind == MobKind::Vex && vm.health > 0.0) && o.is_alive();
        if alive && o.position().distance_to_sqr(e.position()) <= 256.0 {
            n += 1;
        }
    }
    n
}

/// `EvokerSummonSpellGoal.performSpellCasting`: three vexes around the evoker, bound to where
/// they appeared, living 30 to 119 seconds.
fn summon_vexes(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let _ = m;
    for _ in 0..3 {
        let dx = -2 + e.random.next_int_bounded(5);
        let dz = -2 + e.random.next_int_bounded(5);
        let pos = e.block_position().offset(dx, 1, dz);
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut vex = mob::new(MobKind::Vex, id, 0, seed);
        vex.set_pos(Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5));
        vex.y_rot = 0.0;
        vex.x_rot = 0.0;
        vex.set_old_pos_and_rot();
        let eff = level.effective_difficulty(pos);
        let ctx = SpawnContext {
            biome: None,
            moon_brightness: 1.0,
            special_multiplier: super::zombie::special_multiplier(eff),
            effective_difficulty: eff,
            hard: level.difficulty() == 3,
            halloween: false,
        };
        mob::finalize_spawn(&mut vex, level.random(), &ctx, &mut GroupData::default(), false);
        let life = 20 * (30 + e.random.next_int_bounded(90));
        if let Some(vm) = mob::data_mut(&mut vex) {
            super::vex::bind(vm, e.id, e.uuid, pos, life);
        }
        level.add_entity(vex);
        level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: Vec3::new(pos.x as f64, pos.y as f64, pos.z as f64), entity: Some(e.id) });
    }
}

/// `EvokerAttackSpellGoal.performSpellCasting`: a ring of fangs around the evoker when the
/// target is close, else a line of 16 toward it.
fn cast_fangs(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(t) = goals::target(m, level) else { return };
    let min_y = t.pos.y.min(e.y());
    let max_y = t.pos.y.max(e.y()) + 1.0;
    let angle = mth::atan2(t.pos.z - e.z(), t.pos.x - e.x()) as f32;
    if e.position().distance_to_sqr(t.pos) < 9.0 {
        for i in 0..5 {
            let a = angle + i as f32 * std::f32::consts::PI * 0.4;
            fang(e, level, e.x() + mth::cos(a as f64) as f64 * 1.5, e.z() + mth::sin(a as f64) as f64 * 1.5, min_y, max_y, a, 0);
        }
        for i in 0..8 {
            let a = angle + i as f32 * std::f32::consts::PI * 2.0 / 8.0 + (std::f64::consts::PI * 2.0 / 5.0) as f32;
            fang(e, level, e.x() + mth::cos(a as f64) as f64 * 2.5, e.z() + mth::sin(a as f64) as f64 * 2.5, min_y, max_y, a, 3);
        }
    } else {
        for i in 0..16 {
            let reach = 1.25 * (i + 1) as f64;
            fang(e, level, e.x() + mth::cos(angle as f64) as f64 * reach, e.z() + mth::sin(angle as f64) as f64 * reach, min_y, max_y, angle, i);
        }
    }
}

/// `createSpellEntity`: fangs on the first sturdy floor going down from `max_y`.
#[allow(clippy::too_many_arguments)]
fn fang(e: &Entity, level: &mut dyn EntityLevel, x: f64, z: f64, min_y: f64, max_y: f64, angle: f32, delay: i32) {
    let mut pos = BlockPos::containing(x, max_y, z);
    let mut top = 0.0;
    let mut found = false;
    loop {
        let below = pos.below();
        if crate::physics::is_face_sturdy(level.block(below), crate::math::Direction::Up) {
            let s = level.block(pos);
            if !kiln_data::blocks_types::is_air(s) {
                let (shape, _) = crate::collision::collision_shape(s, pos, &crate::collision::CollisionContext::EMPTY);
                if !shape.is_empty() {
                    top = shape.max(Axis::Y, 0.0);
                }
            }
            found = true;
            break;
        }
        pos = pos.below();
        if pos.y < crate::math::floor(min_y) - 1 {
            break;
        }
    }
    if !found {
        return;
    }
    let at = Vec3::new(x, pos.y as f64 + top, z);
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    level.add_entity(crate::ext_entity::evoker_fangs::new(id, at, angle, delay, e, seed));
    level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: at, entity: Some(e.id) });
}
