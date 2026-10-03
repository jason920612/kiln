//! Slime, and what it shares with the magma cube (`AbstractCubeMob`, an `AgeableMob` in 26.x
//! that never is a baby): the size (entity data after the ageable fields, `Size` NBT minus
//! one), dimensions and attributes by size, the hopping move control and its goals, the
//! landing squish, touch damage and the split into 2 to 4 smaller cubes when a dead one is
//! removed.

use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr;
use crate::mob::control::{Operation, set_speed};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView, state, state_mut};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE, Wanted};
use crate::mob::{self, DamageSource, GroupData, MobData, SpawnContext, mth};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Slime;

pub static KIND: Slime = Slime;

static INFO: Info = Info { ageable: true, head: (75, 0, 10), extends_monster: false, ..Info::monster("minecraft:slime", &[]) };

/// The cube state (`AbstractCubeMob` fields and its `CubeMobMoveControl`).
#[derive(Clone, Debug)]
pub struct Cube {
    pub size: i32,
    pub target_squish: f32,
    pub squish: f32,
    pub o_squish: f32,
    pub was_on_ground: bool,
    /// `CubeMobMoveControl.yRot`, `jumpDelay`, `isAggressive`.
    pub move_y_rot: f32,
    pub jump_delay: i32,
    pub move_aggressive: bool,
}

fn magma(m: &MobData) -> bool {
    m.kind == mob::MobKind::MagmaCube
}

pub fn size(m: &MobData) -> i32 {
    state::<Cube>(m).map_or(1, |c| c.size)
}

fn set_base(m: &mut MobData, a: Attr, v: f64) {
    if let Some(i) = m.attrs.get_mut(a) {
        i.base = v;
    }
}

fn sound(e: &Entity, level: &mut dyn EntityLevel, name: &str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: mob::sound_event(name), source: "hostile", volume, pitch });
    }
}

/// `getSoundPitch`: two draws, 1.4 for tiny cubes, else 0.8.
fn sound_pitch(e: &mut Entity, size: i32) -> f32 {
    let base = if size <= 1 { 1.4 } else { 0.8 };
    let a = e.random.next_float();
    let b = e.random.next_float();
    ((a - b) * 0.2 + 1.0) * base
}

/// The type's sound: `<cube>.<what>` or its `_small` form for tiny cubes where one exists.
fn cube_sound(m: &MobData, what: &str) -> String {
    let base = if magma(m) { "magma_cube" } else { "slime" };
    let small = size(m) <= 1 && !(magma(m) && what == "jump");
    format!("minecraft:entity.{base}.{what}{}", if small { "_small" } else { "" })
}

/// `AbstractCubeMob.setSize` with the types' additions.
pub fn set_size(e: &mut Entity, m: &mut MobData, size: i32, update_health: bool) {
    let i = size.clamp(1, 127);
    let Some(c) = state_mut::<Cube>(m) else { return };
    let changed = c.size != i;
    c.size = i;
    mob::refresh_dimensions(e, m);
    if changed {
        // `onSyncedDataUpdated(ID_SIZE)`: turned to the head; a splash in water (its particles
        // and sound are not drawn: approximation).
        e.y_rot = m.y_head_rot;
        m.y_body_rot = m.y_head_rot;
        if e.is_in_water() {
            let _ = e.random.next_int_bounded(20);
        }
    }
    set_base(m, Attr::MaxHealth, (i * i) as f64);
    set_base(m, Attr::MovementSpeed, (0.2f32 + 0.1 * i as f32) as f64);
    if update_health {
        m.health = m.max_health();
    }
    set_base(m, Attr::AttackDamage, i as f64);
    if magma(m) {
        set_base(m, Attr::Armor, (size * 3) as f64);
    }
}

pub fn new_state(m: &mut MobData) -> Option<Box<dyn MobExt>> {
    let c = Cube { size: 1, target_squish: 0.0, squish: 0.0, o_squish: 0.0, was_on_ground: false, move_y_rot: 0.0, jump_delay: 0, move_aggressive: false };
    m.nav.can_float = true;
    Some(Box::new(c))
}

pub fn register_goals(m: &mut MobData) {
    let g = &mut m.goals;
    g.add(1, Goal::Custom(Box::new(CubeFloat)));
    g.add(2, Goal::Custom(Box::new(CubeAttack { grow_tired: 0 })));
    g.add(4, Goal::Custom(Box::new(CubeRandomDirection { chosen: 0.0, next_randomize: 0 })));
    g.add(5, Goal::Custom(Box::new(CubeKeepOnJumping)));
    let nearest = |wanted| Goal::NearestAttackable { wanted, interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false };
    m.targets.add(1, nearest(Wanted::PlayerWithinDy(4)));
    m.targets.add(3, nearest(Wanted::Types(&["minecraft:iron_golem"])));
}

/// `AbstractCubeMob.tick` before `super.tick()`.
pub fn pre_tick(m: &mut MobData) {
    if let Some(c) = state_mut::<Cube>(m) {
        c.o_squish = c.squish;
        c.squish += (c.target_squish - c.squish) * 0.5;
    }
}

/// `AbstractCubeMob.tick` after `super.tick()`: the landing squish (particle positions draw from
/// the mob's random on the server too).
pub fn post_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(c) = state::<Cube>(m) else { return };
    let (was, size) = (c.was_on_ground, c.size);
    let mut target = c.target_squish;
    if e.on_ground && !was {
        let width = e.width * 2.0;
        let mut j = 0;
        while (j as f32) < width * 16.0 {
            e.random.next_float();
            e.random.next_float();
            j += 1;
        }
        let a = e.random.next_float();
        let b = e.random.next_float();
        let pitch = ((a - b) * 0.2 + 1.0) / 0.8;
        let s = cube_sound(m, "squish");
        sound(e, level, &s, 0.4 * size as f32, pitch);
        target = -0.5;
    } else if !e.on_ground && was {
        target = 1.0;
    }
    let decay = if magma(m) { 0.9 } else { 0.6 };
    let c = state_mut::<Cube>(m).unwrap();
    c.was_on_ground = e.on_ground;
    c.target_squish = target * decay;
}

/// `MoveControl.rotlerp`.
fn rotlerp(from: f32, to: f32, max: f32) -> f32 {
    let d = mth::wrap_degrees(to - from).clamp(-max, max);
    let mut r = from + d;
    if r < 0.0 {
        r += 360.0;
    } else if r > 360.0 {
        r -= 360.0;
    }
    r
}

/// `CubeMobMoveControl.setDirection`.
fn set_direction(m: &mut MobData, y_rot: f32, aggressive: bool) {
    if let Some(c) = state_mut::<Cube>(m) {
        c.move_y_rot = y_rot;
        c.move_aggressive = aggressive;
    }
}

/// `CubeMobMoveControl.setWantedMovement`.
fn set_wanted_movement(m: &mut MobData, speed: f64) {
    m.mov.speed_modifier = speed;
    m.mov.operation = Operation::MoveTo;
}

/// `CubeMobMoveControl.tick`.
pub fn tick_move(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let Some(c) = state::<Cube>(m) else { return };
    let (target_rot, size) = (c.move_y_rot, c.size);
    e.y_rot = rotlerp(e.y_rot, target_rot, 90.0);
    m.y_head_rot = e.y_rot;
    m.y_body_rot = e.y_rot;
    if m.mov.operation != Operation::MoveTo {
        m.zza = 0.0;
        return;
    }
    m.mov.operation = Operation::Wait;
    let speed = (m.mov.speed_modifier * m.attrs.value(Attr::MovementSpeed)) as f32;
    set_speed(m, speed);
    if !e.on_ground {
        return;
    }
    let c = state_mut::<Cube>(m).unwrap();
    let d = c.jump_delay;
    c.jump_delay -= 1;
    if d <= 0 {
        // `getJumpDelay`.
        let mut delay = e.random.next_int_bounded(20) + 10;
        if m.kind == mob::MobKind::MagmaCube {
            delay *= 4;
        }
        let c = state_mut::<Cube>(m).unwrap();
        if c.move_aggressive {
            delay /= 3;
        }
        c.jump_delay = delay;
        m.jump.jump = true;
        if size > 0 {
            let pitch = sound_pitch(e, size);
            let s = cube_sound(m, "jump");
            sound(e, level, &s, 0.4 * size as f32, pitch);
        }
    } else {
        m.xxa = 0.0;
        m.zza = 0.0;
        set_speed(m, 0.0);
    }
}

/// `canDealDamage`: not tiny (magma cubes always).
fn can_deal_damage(m: &MobData) -> bool {
    magma(m) || size(m) > 1
}

/// `dealDamage`: a hit on a touching target in reach and sight.
fn deal_damage(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    if !mob::is_alive(e, m) || !mob::within_melee_range(e, m, t) || !mob::has_line_of_sight_cached(e, m, level, t) {
        return;
    }
    let mut damage = m.attrs.value(Attr::AttackDamage) as f32;
    if magma(m) {
        damage += 2.0;
    }
    let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    let hurt = if t.player { level.hurt_player(t.id, source, damage) } else { hurt_other(level, t.id, source, damage) };
    if hurt {
        let a = e.random.next_float();
        let b = e.random.next_float();
        sound(e, level, "minecraft:entity.slime.attack", 1.0, (a - b) * 0.2 + 1.0);
    }
}

/// Hurts another mob of the level.
pub fn hurt_other(level: &mut dyn EntityLevel, id: i32, source: DamageSource, amount: f32) -> bool {
    let Some(o) = level.entity_mut(id) else { return false };
    let marker = Entity::new("minecraft:marker", 0, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0);
    let mut o2 = std::mem::replace(o, marker);
    let r = mob::hurt_entity(&mut o2, level, source, amount);
    if let Some(slot) = level.entity_mut(id) {
        *slot = o2;
    }
    r
}

pub fn player_touch(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, p: &Living) {
    if can_deal_damage(m) && !m.no_ai {
        deal_damage(e, m, level, p);
    }
}

/// `push(entity)` for iron golems: the pushing side of `pushEntities` (approximation: checked
/// once per tick against touching golems).
pub fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !can_deal_damage(m) || m.no_ai {
        return;
    }
    let bb = e.bounding_box();
    for id in level.entities_in(&bb, crate::level::EntityFilter::Living, e.id) {
        if level.entity(id).is_some_and(|o| o.type_name == "minecraft:iron_golem")
            && let Some(t) = goals::living(level, id)
        {
            deal_damage(e, m, level, &t);
        }
    }
}

/// `jumpFromGround` (the magma cube adds a tenth per size).
pub fn jump_from_ground(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let mut power = (m.attrs.value(Attr::JumpStrength) as f32) * e.block_jump_factor(level);
    if magma(m) {
        power += size(m) as f32 * 0.1;
    }
    e.delta = Vec3::new(e.delta.x, power as f64, e.delta.z);
    e.needs_sync = true;
}

/// `remove(KILLED)`: a dead cube larger than 1 splits into 2 to 4 of half its size
/// (`convertTo(SPLIT_ON_DEATH)` + `setUpSplitCube`, draws from the parent's random).
pub fn on_killed_removal(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let size = size(m);
    if size <= 1 || !m.is_dead_or_dying() {
        return;
    }
    let half = e.width / 2.0;
    let child_size = size / 2;
    let count = 2 + e.random.next_int_bounded(3);
    for l in 0..count {
        let ox = ((l % 2) as f32 - 0.5) * half;
        let oz = ((l / 2) as f32 - 0.5) * half;
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut child = mob::new(m.kind, id, 0, seed);
        let mut cm = mob::take(&mut child);
        // `ConversionType.SPLIT_ON_DEATH.convertCommon`.
        cm.absorption = m.absorption;
        cm.left_handed = m.left_handed;
        cm.no_ai = m.no_ai;
        cm.persistence_required = m.persistence_required;
        child.invulnerable = e.invulnerable;
        child.invulnerable_time = e.invulnerable_time;
        child.no_gravity = e.no_gravity;
        child.silent = e.silent;
        if e.is_on_fire() {
            child.remaining_fire_ticks = 1;
        }
        set_size(&mut child, &mut cm, child_size, true);
        // `snapTo(x + ox, y + 0.5, z + oz, random * 360, 0)`.
        let yaw = e.random.next_float() * 360.0;
        child.set_pos(Vec3::new(e.x() + ox as f64, e.y() + 0.5, e.z() + oz as f64));
        child.y_rot = yaw;
        child.x_rot = 0.0;
        child.set_old_pos_and_rot();
        pin_move_yaw(&child, &mut cm);
        mob::put(&mut child, cm);
        level.add_entity(child);
    }
}

/// The move control's remembered yaw as its constructor would see the current one (the
/// constructor's own yaw is random and not reproducible: loading and splitting pin it).
fn pin_move_yaw(e: &Entity, m: &mut MobData) {
    if let Some(c) = state_mut::<Cube>(m) {
        c.move_y_rot = 180.0 * e.y_rot / std::f32::consts::PI;
    }
}

pub fn finalize_spawn(e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
    // `AgeableMobGroupData(false)`: no baby roll.
    group.ageable_group_size += 1;
    ext::mob_finalize(m, r);
    // `setSpawnSize` from the level's random.
    let mut i = r.next_int_bounded(3);
    if i < 2 && r.next_float() < 0.5 * ctx.special_multiplier {
        i += 1;
    }
    set_size(e, m, 1 << i, true);
}

pub fn load(e: &mut Entity, m: &mut MobData, r: &mut Input) {
    // `setSize(Size + 1, false)` runs before the living fields are read: the saved health
    // (else the new maximum) wins.
    let health = r.num("Health");
    set_size(e, m, r.int_or("Size", 0) + 1, false);
    m.health = health.map_or(m.max_health(), |h| h as f32);
    let was = r.bool_or("wasOnGround", false);
    if let Some(c) = state_mut::<Cube>(m) {
        c.was_on_ground = was;
    }
    pin_move_yaw(e, m);
}

pub fn save(m: &MobData, o: &mut Output) {
    let c = state::<Cube>(m);
    o.put("Size", Tag::Int(c.map_or(0, |c| c.size - 1)));
    o.put("wasOnGround", Tag::Byte(c.is_some_and(|c| c.was_on_ground) as i8));
}

pub fn entity_data(m: &MobData, d: &mut EntityData) {
    d.set(kiln_data::entities::data::abstract_cube_mob::ID_SIZE, &DataValue::Int(size(m)));
}

pub fn dimensions(m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
    let s = size(m) as f32;
    (base.0 * s, base.1 * s, base.2 * s)
}

/// Whether biome `id` is in the `minecraft:worldgen/biome` tag `tag`.
pub fn biome_in_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:worldgen/biome")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

/// `WorldgenRandom.seedSlimeChunk(x, z, seed, 987234911).nextInt(10) == 0`.
pub fn is_slime_chunk(world_seed: i64, cx: i32, cz: i32) -> bool {
    let seed = world_seed
        .wrapping_add(cx.wrapping_mul(cx).wrapping_mul(4987142) as i64)
        .wrapping_add(cx.wrapping_mul(5947611) as i64)
        .wrapping_add((cz.wrapping_mul(cz) as i64).wrapping_mul(4392871))
        .wrapping_add(cz.wrapping_mul(389711) as i64)
        ^ 987234911;
    LegacyRandom::new(seed).next_int_bounded(10) == 0
}

/// `Mob.checkMobSpawnRules` (not from a spawner): a valid spawn block below.
pub fn check_mob_spawn_rules(view: &dyn SpawnView, pos: BlockPos) -> bool {
    mob::path::valid_spawn(view.block(pos.below()), false)
}

impl Kind for Slime {
    fn touches_players(&self) -> bool {
        true
    }
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        new_state(m)
    }
    fn register_goals(&self, m: &mut MobData) {
        register_goals(m);
    }
    fn pre_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        pre_tick(m);
    }
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        post_tick(e, m, level);
    }
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        ai_step(e, m, level);
    }
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        tick_move(e, m, level);
        true
    }
    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        jump_from_ground(e, m, level);
        true
    }
    fn player_touch(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: &Living) {
        player_touch(e, m, level, player);
    }
    fn on_killed_removal(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        on_killed_removal(e, m, level);
    }
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        finalize_spawn(e, m, r, ctx, group);
    }
    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        load(e, m, r);
    }
    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        save(m, o);
    }
    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        entity_data(m, d);
    }
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        dimensions(m, base)
    }
    fn experience(&self, _e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(size(m))
    }
    fn spawn_ignores_light(&self) -> bool {
        true
    }
    /// `checkSlimeSpawnRules`: swamp surfaces at night by the moon (the
    /// `minecraft:gameplay/surface_slime_spawn_chance` attribute: its moon timeline is half the
    /// moon brightness), or slime chunks below y 40.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        if view.difficulty() == 0 {
            return Some(false);
        }
        if biome_in_tag(view.biome(pos), "minecraft:allows_surface_slime_spawns") && pos.y > 50 && pos.y < 70 {
            let chance = 0.5 * view.moon_brightness();
            if r.next_float() < chance && view.raw_brightness(pos, view.sky_darken()) <= r.next_int_bounded(8) {
                return Some(check_mob_spawn_rules(view, pos));
            }
        }
        let slime_chunk = is_slime_chunk(view.world_seed(), pos.x >> 4, pos.z >> 4);
        if r.next_int_bounded(10) == 0 && slime_chunk && pos.y < 40 {
            return Some(check_mob_spawn_rules(view, pos));
        }
        Some(false)
    }
}

// ---------------------------------------------------------------------- goals

fn in_liquid(e: &Entity) -> bool {
    e.is_in_water() || e.is_in_lava()
}

/// `CubeMobFloatGoal`.
#[derive(Clone, Debug)]
struct CubeFloat;

impl CustomGoal for CubeFloat {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CubeMobFloatGoal"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        in_liquid(e)
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if e.random.next_float() < 0.8 {
            m.jump.jump = true;
        }
        set_wanted_movement(m, 1.2);
    }
}

/// `CubeMobAttackGoal`.
#[derive(Clone, Debug)]
struct CubeAttack {
    grow_tired: i32,
}

impl CustomGoal for CubeAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CubeMobAttackGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if goals::target(m, level).is_none() {
            return false;
        }
        self.grow_tired -= 1;
        self.grow_tired > 0
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.grow_tired = mth::reduced_tick_delay(300);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(t) = goals::target(m, level) {
            mob::mob_look_at(e, &t, 10.0, 10.0);
        }
        let deal = can_deal_damage(m);
        set_direction(m, e.y_rot, deal);
    }
}

/// `CubeMobRandomDirectionGoal`.
#[derive(Clone, Debug)]
struct CubeRandomDirection {
    chosen: f32,
    next_randomize: i32,
}

impl CustomGoal for CubeRandomDirection {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CubeMobRandomDirectionGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_none() && (e.on_ground || in_liquid(e))
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.next_randomize -= 1;
        if self.next_randomize <= 0 {
            self.next_randomize = mth::reduced_tick_delay(40 + e.random.next_int_bounded(60));
            self.chosen = e.random.next_int_bounded(360) as f32;
        }
        set_direction(m, self.chosen, false);
    }
}

/// `CubeMobKeepOnJumpingGoal`.
#[derive(Clone, Debug)]
struct CubeKeepOnJumping;

impl CustomGoal for CubeKeepOnJumping {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "CubeMobKeepOnJumpingGoal"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        true
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_wanted_movement(m, 1.0);
    }
}
