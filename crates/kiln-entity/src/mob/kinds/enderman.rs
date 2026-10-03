//! Enderman: teleports (randomly, in daylight, when wet, away from projectiles and toward far
//! targets), carries blocks, angers when stared at (`NeutralMob` persistent anger).

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityFilter, EntityLevel, Event, PlayerView};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::attributes::Op;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, MeleeKind, Wanted, JUMP, MOVE, TARGET};
use crate::mob::{self, mth, path, DamageSource, MobData};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};
use std::sync::OnceLock;

pub struct Enderman;

pub static KIND: Enderman = Enderman;

static INFO: Info = Info::monster("minecraft:enderman", &[(MaxHealth, 40.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 7.0), (FollowRange, 64.0), (StepHeight, 1.0)]);

/// `SPEED_MODIFIER_ATTACKING`.
const ATTACKING: &str = "minecraft:attacking";

/// An `EntityReference<LivingEntity>`: the entity id when known, and its UUID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reference {
    pub id: Option<i32>,
    pub uuid: u128,
}

impl Reference {
    pub fn of(level: &dyn EntityLevel, id: i32) -> Reference {
        let uuid = level.player(id).map(|p| p.uuid).or_else(|| level.entity(id).map(|e| e.uuid)).unwrap_or(0);
        Reference { id: Some(id), uuid }
    }

    pub fn matches(&self, level: &dyn EntityLevel, id: i32) -> bool {
        match self.id {
            Some(i) => i == id,
            None => self.uuid != 0 && Reference::of(level, id).uuid == self.uuid,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EndermanState {
    /// `DATA_CARRY_STATE`.
    pub carried: Option<u16>,
    /// `DATA_CREEPY` and `DATA_STARED_AT`.
    pub creepy: bool,
    pub stared_at: bool,
    pub target_change_time: i32,
    /// The target the last `setTarget` saw (Kiln's goals assign the target directly; the
    /// enderman's `setTarget` override runs when it notices the change).
    pub seen_target: Option<i32>,
    /// `NeutralMob`: `persistentAngerEndTime` and `persistentAngerTarget`.
    pub anger_end: i64,
    pub angry_at: Option<Reference>,
}

fn st(m: &MobData) -> &EndermanState {
    ext::state::<EndermanState>(m).expect("enderman state")
}

fn st_mut(m: &mut MobData) -> &mut EndermanState {
    ext::state_mut::<EndermanState>(m).expect("enderman state")
}

impl Kind for Enderman {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::Water, -1.0));
        Some(Box::new(EndermanState {
            carried: None,
            creepy: false,
            stared_at: false,
            target_change_time: 0,
            seen_target: None,
            anger_end: 0,
            angry_at: None,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Custom(Box::new(FreezeWhenLookedAt { target: None })));
        g.add(2, Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(7, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.0), wanted: Vec3::ZERO, force: false });
        g.add(8, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        g.add(10, Goal::Custom(Box::new(LeaveBlock)));
        g.add(11, Goal::Custom(Box::new(TakeBlock)));
        let t = &mut m.targets;
        t.add(1, Goal::Custom(Box::new(LookForPlayer { pending: None, target: None, aggro_time: 0, teleport_time: 0, unseen: 0 })));
        t.add(2, Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(3, Goal::NearestAttackable { wanted: Wanted::Types(&["minecraft:endermite"]), interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false });
        t.add(4, Goal::Custom(Box::new(ResetUniversalAnger { last_hurt_by_player_timestamp: 0 })));
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        m.jumping = false;
        update_persistent_anger(e, m, level, true);
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // The end of `LivingEntity.aiStep`: `isSensitiveToWater` mobs take drowning damage
        // in water or rain.
        if in_water_or_rain(e, level) {
            mob::hurt(e, m, level, DamageSource::of(crate::level::DamageKind::Drown), 1.0);
        }
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        sync_target(e, m);
        if level.is_bright_outside() && e.tick_count >= st(m).target_change_time + 600 {
            let br = mob::light_magic_value(e, level);
            if br > 0.5 && level.can_see_sky(e.block_position()) && e.random.next_float() * 30.0 < (br - 0.4) * 2.0 {
                set_target(e, m, None);
                teleport(e, m, level);
            }
        }
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        sync_target(e, m);
    }

    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        if source.kind.is_tag("minecraft:is_projectile") {
            repeatedly_try_to_teleport(e, m, level);
            return Some(false);
        }
        let hurt = mob::hurt_base(e, m, level, *source, amount);
        let by_living = source.attacker.is_some_and(|a| goals::living(level, a).is_some());
        if !by_living && e.random.next_int_bounded(10) != 0 {
            teleport(e, m, level);
        }
        Some(hurt)
    }

    /// `Enderman.dropCustomDeathLoot`: the carried block's drops, mined with a diamond axe
    /// enchanted from `minecraft:enderman_loot_drop` (silk touch) and rolled from its loot table.
    fn drop_custom_death_loot(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource) {
        let Some(carried) = st(m).carried else { return };
        let Some(mut tool) = kiln_item::ItemStack::of("minecraft:diamond_axe", 1) else { return };
        let special = super::zombie::special_multiplier(level.effective_difficulty(e.block_position()));
        level.enchant_from_provider(&mut tool, "minecraft:enderman_loot_drop", special, &mut e.random);
        for stack in level.block_loot(carried, e.position(), &tool, e.id) {
            mob::spawn_at_location(e, level, stack);
        }
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        st(m).creepy.then(|| Some(mob::sound_event("minecraft:entity.enderman.scream")))
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        // `requiresCustomPersistence`: an enderman carrying a block stays.
        st(m).carried.is_some().then_some(false)
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let carried = r.get("carriedBlockState").and_then(crate::persist::state_from_tag).filter(|s| !kiln_data::blocks_types::is_air(*s));
        let end = r.num("anger_end_time").map(|v| v as i64);
        let legacy = r.num("AngerTime");
        let angry = r.uuid("angry_at");
        let s = st_mut(m);
        s.carried = carried;
        s.anger_end = match (end, legacy) {
            (Some(t), _) => t,
            // Relative to the load time; Kiln has no clock here (approximation: from 0).
            (None, Some(t)) => t as i64,
            _ => -1,
        };
        s.angry_at = angry.map(|uuid| Reference { id: None, uuid });
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        if let Some(c) = s.carried {
            o.put("carriedBlockState", crate::persist::state_to_tag(c));
        }
        o.put("anger_end_time", Tag::Long(s.anger_end));
        if let Some(r) = s.angry_at
            && r.uuid != 0
        {
            o.put("angry_at", crate::persist::uuid_to_tag(r.uuid));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::enderman as f;
        let s = st(m);
        d.set(f::CARRY_STATE, &DataValue::OptionalBlockState(s.carried.map(|c| c as i32)));
        d.set(f::CREEPY, &DataValue::Boolean(s.creepy));
        d.set(f::STARED_AT, &DataValue::Boolean(s.stared_at));
    }
}

// ---------------------------------------------------------------------- target and anger

/// `Enderman.setTarget`'s additions, when the target changed.
fn sync_target(e: &Entity, m: &mut MobData) {
    let t = m.target;
    if st(m).seen_target == t {
        return;
    }
    let tick = e.tick_count;
    let s = st_mut(m);
    s.seen_target = t;
    if t.is_none() {
        s.target_change_time = 0;
        s.creepy = false;
        s.stared_at = false;
        m.attrs.remove_modifier(Attr::MovementSpeed, ATTACKING);
    } else {
        s.target_change_time = tick;
        s.creepy = true;
        if !m.attrs.get(Attr::MovementSpeed).is_some_and(|i| i.has_modifier(ATTACKING)) {
            m.attrs.set_modifier(Attr::MovementSpeed, ATTACKING, 0.15000000596046448, Op::AddValue);
        }
    }
}

/// `Enderman.setTarget`.
fn set_target(e: &Entity, m: &mut MobData, t: Option<i32>) {
    m.target = t;
    sync_target(e, m);
}

/// `NeutralMob.isAngry`.
fn is_angry(m: &MobData, level: &dyn EntityLevel) -> bool {
    let end = st(m).anger_end;
    end > 0 && end - level.game_time() > 0
}

/// `NeutralMob.isValidPlayerTarget`.
fn valid_player_target(level: &dyn EntityLevel, t: &Living) -> bool {
    t.player && !t.creative && !t.spectator && level.difficulty() != 0
}

/// `NeutralMob.stopBeingAngry`.
fn stop_being_angry(e: &Entity, m: &mut MobData) {
    m.last_hurt_by_mob = None;
    m.last_hurt_by_mob_timestamp = e.tick_count;
    st_mut(m).angry_at = None;
    set_target(e, m, None);
    st_mut(m).anger_end = -1;
}

/// `Enderman.startPersistentAngerTimer`: 20 to 39 seconds.
fn start_persistent_anger_timer(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let t = mth::next_int_between(&mut e.random, 400, 780);
    st_mut(m).anger_end = level.game_time() + t as i64;
}

/// `NeutralMob.updatePersistentAnger`.
fn update_persistent_anger(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, always_refresh: bool) {
    let r = st(m).angry_at;
    if let Some(u) = m.target
        && let Some(l) = goals::living(level, u)
        && !l.alive
        && !l.player
        && r.is_some_and(|r| r.matches(level, u))
    {
        stop_being_angry(e, m);
        return;
    }
    let target = goals::target(m, level);
    if let Some(t) = target {
        let changed = r.is_none_or(|r| !r.matches(level, t.id));
        if changed {
            st_mut(m).angry_at = Some(Reference::of(level, t.id));
        }
        if changed || always_refresh {
            start_persistent_anger_timer(e, m, level);
        }
    }
    if r.is_some() && !is_angry(m, level) && (target.is_none_or(|t| !valid_player_target(level, &t)) || !always_refresh) {
        stop_being_angry(e, m);
    }
    let resolved = r.and_then(|r| r.id.or_else(|| level.player_by_uuid(r.uuid).map(|p| p.id)));
    if let Some(p) = resolved.and_then(|id| level.player(id))
        && (p.creative || p.spectator || level.difficulty() == 0)
    {
        stop_being_angry(e, m);
    }
}

/// `NeutralMob.isAngryAt` (universal anger is off).
fn is_angry_at(m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    if !goals::can_attack(m, level, t) {
        return false;
    }
    st(m).angry_at.is_some_and(|r| r.matches(level, t.id))
}

/// `LivingEntity.calculateViewVector(xRot, yRot)`.
pub fn view_vector(x_rot: f32, y_rot: f32) -> Vec3 {
    let f = x_rot * 0.017453292;
    let f1 = -y_rot * 0.017453292;
    let f2 = mth::cos(f1 as f64);
    let f3 = mth::sin(f1 as f64);
    let f4 = mth::cos(f as f64);
    let f5 = mth::sin(f as f64);
    Vec3::new((f3 * f4) as f64, (-f5) as f64, (f2 * f4) as f64)
}

/// `Enderman.isBeingStaredBy`: `isLookingAtMe(player, 0.025, true, false, eyeY)` unless the
/// player wears a gaze disguise (a carved pumpkin).
fn is_being_stared_by(e: &Entity, level: &dyn EntityLevel, p: &PlayerView) -> bool {
    if mob::item_tag(p.head, "minecraft:gaze_disguise_equipment") {
        return false;
    }
    let view = view_vector(p.pitch, p.yaw).normalize();
    let eye = Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z);
    let y = e.eye_y();
    let to_me = Vec3::new(e.x() - eye.x, y - eye.y, e.z() - eye.z);
    let len = to_me.length();
    let n = to_me.normalize();
    let dot = view.x * n.x + view.y * n.y + view.z * n.z;
    if dot > 1.0 - 0.025 / len {
        let to = Vec3::new(e.x(), y, e.z());
        return to.distance_to_sqr(eye).sqrt() <= 128.0 && !mob::clip_blocks(level, eye, to);
    }
    false
}

fn stared_by(e: &Entity, level: &dyn EntityLevel, id: i32) -> bool {
    level.player(id).is_some_and(|p| is_being_stared_by(e, level, &p))
}

/// `isInWaterOrRain`.
fn in_water_or_rain(e: &Entity, level: &dyn EntityLevel) -> bool {
    e.is_in_water() || level.is_raining_at(e.block_position()) || level.is_raining_at(BlockPos::containing(e.x(), e.bounding_box().max_y, e.z()))
}

// ---------------------------------------------------------------------- teleporting

/// A `minecraft:block` tag as per-state flags.
fn tag_flags(tag: &str) -> Vec<bool> {
    let mut out = vec![false; kiln_data::blocks::STATE_COUNT as usize];
    let ids = kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:block")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .map_or(&[][..], |(_, ids)| *ids);
    let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
    for &id in ids {
        if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
            out[info.first as usize..=info.last as usize].fill(true);
        }
    }
    out
}

pub fn can_teleport_to(state: u16) -> bool {
    static F: OnceLock<Vec<bool>> = OnceLock::new();
    F.get_or_init(|| tag_flags("minecraft:entities_can_teleport_to"))[state as usize]
}

pub fn enderman_does_not_teleport_to(state: u16) -> bool {
    static F: OnceLock<Vec<bool>> = OnceLock::new();
    F.get_or_init(|| tag_flags("minecraft:enderman_does_not_teleport_to"))[state as usize]
}

pub fn enderman_holdable(state: u16) -> bool {
    static F: OnceLock<Vec<bool>> = OnceLock::new();
    F.get_or_init(|| tag_flags("minecraft:enderman_holdable"))[state as usize]
}

/// `LivingEntity.randomTeleport(x, y, z, broadcast, forbidden)`: down to the first block
/// entities can teleport onto, if the box there is free. `can_go` is `canRandomlyTeleportTo`.
/// Approximation: no world border clamp.
pub fn random_teleport(
    e: &mut Entity,
    m: &mut MobData,
    level: &mut dyn EntityLevel,
    x: f64,
    y: f64,
    z: f64,
    broadcast: bool,
    forbidden: fn(u16) -> bool,
    can_go: &dyn Fn(&dyn EntityLevel, f64, f64, f64) -> bool,
) -> bool {
    let mut ty = y;
    let mut pos = BlockPos::containing(x, y, z);
    if !level.is_loaded(pos) {
        return false;
    }
    while pos.y > level.min_y() {
        pos = pos.below();
        let ground = level.block(pos);
        if can_teleport_to(ground) {
            if forbidden(ground) {
                return false;
            }
            let b = e.make_bounding_box(Vec3::new(x, ty, z));
            let ctx = crate::collision::CollisionContext::EMPTY;
            if !crate::collision::no_collision(level, &ctx, e.id, &b) || contains_any_liquid(level, &b) || any_block(level, &b, forbidden) {
                return false;
            }
            if !can_go(level, x, ty, z) {
                return false;
            }
            // `teleportTo` → `snapTo`.
            e.set_pos(Vec3::new(x, ty, z));
            e.set_old_pos_and_rot();
            if broadcast {
                level.emit(Event::EntityEvent { entity: e.id, event: 46 });
            }
            m.nav.stop();
            return true;
        }
        ty -= 1.0;
    }
    false
}

fn contains_any_liquid(level: &dyn EntityLevel, b: &Aabb) -> bool {
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::ceil(b.max_x), crate::math::ceil(b.max_y), crate::math::ceil(b.max_z));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if !crate::physics::fluid_state(level.block(BlockPos::new(x, y, z))).is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

/// `BlockGetter.findBlocksIn(box).filterState(f).anyMatched()`: the blocks the box touches.
fn any_block(level: &dyn EntityLevel, b: &Aabb, f: fn(u16) -> bool) -> bool {
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::floor(b.max_x), crate::math::floor(b.max_y), crate::math::floor(b.max_z));
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                if f(level.block(BlockPos::new(x, y, z))) {
                    return true;
                }
            }
        }
    }
    false
}

/// `Enderman.canRandomlyTeleportTo`: not onto water.
fn not_onto_water(level: &dyn EntityLevel, x: f64, y: f64, z: f64) -> bool {
    let below = BlockPos::containing(x, y, z).below();
    !crate::physics::fluid_state(level.block(below)).kind.is_water()
}

/// `Enderman.teleport(x, y, z)`.
fn teleport_to(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, x: f64, y: f64, z: f64) -> bool {
    let old = e.position();
    let ok = random_teleport(e, m, level, x, y, z, true, enderman_does_not_teleport_to, &not_onto_water);
    if ok {
        level.emit(Event::GameEvent { event: "minecraft:teleport", pos: old, entity: Some(e.id) });
        if !e.silent {
            let from = BlockPos::containing(old.x, old.y, old.z);
            let to = e.block_position();
            let d = |a: i32, b: i32| (b - a).clamp(-127, 127);
            let packed = ((d(from.x, to.x) + 127) & 255) << 16 | ((d(from.y, to.y) + 127) & 255) << 8 | ((d(from.z, to.z) + 127) & 255);
            level.emit(Event::LevelEvent { event: 2018, pos: from, data: packed });
            let sound = "minecraft:entity.enderman.teleport";
            level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume: 1.0, pitch: 1.0 });
            level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume: 1.0, pitch: 1.0 });
        }
    }
    ok
}

/// `Enderman.teleport()`: somewhere within 32 blocks.
fn teleport(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    if !mob::is_alive(e, m) {
        return false;
    }
    let x = e.x() + (e.random.next_double() - 0.5) * 64.0;
    let y = e.y() + (e.random.next_int_bounded(64) - 32) as f64;
    let z = e.z() + (e.random.next_double() - 0.5) * 64.0;
    teleport_to(e, m, level, x, y, z)
}

/// `Enderman.teleportTowards(entity)`.
fn teleport_towards(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> bool {
    let dir = Vec3::new(e.x() - t.pos.x, e.y() + e.height as f64 * 0.5 - t.eye_y, e.z() - t.pos.z).normalize();
    let x = e.x() + (e.random.next_double() - 0.5) * 8.0 - dir.x * 16.0;
    let y = e.y() + (e.random.next_int_bounded(16) - 8) as f64 - dir.y * 16.0;
    let z = e.z() + (e.random.next_double() - 0.5) * 8.0 - dir.z * 16.0;
    teleport_to(e, m, level, x, y, z)
}

fn repeatedly_try_to_teleport(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    for _ in 0..64 {
        if teleport(e, m, level) {
            return;
        }
    }
}

// ---------------------------------------------------------------------- goals

/// `EndermanFreezeWhenLookedAt`.
#[derive(Clone, Debug)]
struct FreezeWhenLookedAt {
    target: Option<i32>,
}

impl CustomGoal for FreezeWhenLookedAt {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "EndermanFreezeWhenLookedAt"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let t = goals::target(m, level);
        self.target = t.map(|t| t.id);
        let Some(t) = t.filter(|t| t.player) else { return false };
        if t.pos.distance_to_sqr(e.position()) > 256.0 {
            return false;
        }
        stared_by(e, level, t.id)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(t) = self.target.and_then(|id| goals::living(level, id)) {
            crate::mob::control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
        }
    }
}

/// `EndermanLookForPlayerGoal` (a `NearestAttackableTargetGoal<Player>`).
#[derive(Clone, Debug)]
struct LookForPlayer {
    pending: Option<i32>,
    target: Option<i32>,
    aggro_time: i32,
    teleport_time: i32,
    unseen: i32,
}

/// `isAngerInducing`.
fn anger_inducing(e: &Entity, m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    t.player && (stared_by(e, level, t.id) || is_angry_at(m, level, t))
}

impl CustomGoal for LookForPlayer {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "EndermanLookForPlayerGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let range = m.attrs.value(FollowRange);
        let mm: &MobData = m;
        // (only the players in range can be picked below)
        let wanted: Vec<i32> = goals::players_around(e, level, range).iter().map(goals::living_player).filter(|t| anger_inducing(e, mm, level, t)).map(|t| t.id).collect();
        self.pending = goals::nearest_player(e, m, level, true, range, true, |p| wanted.contains(&p.id)).map(|t| t.id);
        self.pending.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if let Some(p) = self.pending {
            let Some(t) = goals::living(level, p) else { return false };
            if !anger_inducing(e, m, level, &t) {
                return false;
            }
            mob::mob_look_at(e, &t, 10.0, 10.0);
            return true;
        }
        if let Some(t) = self.target.and_then(|id| goals::living(level, id))
            && goals::targeting_ok(e, m, level, &t, true, -1.0, false)
        {
            return true;
        }
        goals::continue_target(e, m, level, None, false, &mut self.unseen, 60)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.aggro_time = mth::reduced_tick_delay(5);
        self.teleport_time = 0;
        let _ = e;
        st_mut(m).stared_at = true;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.pending = None;
        set_target(e, m, None);
        self.target = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if goals::target(m, level).is_none() {
            self.target = None;
        }
        if let Some(p) = self.pending {
            self.aggro_time -= 1;
            if self.aggro_time <= 0 {
                self.target = Some(p);
                self.pending = None;
                set_target(e, m, Some(p));
                self.unseen = 0;
            }
            return;
        }
        if let Some(t) = self.target.and_then(|id| goals::living(level, id)) {
            if stared_by(e, level, t.id) {
                if t.pos.distance_to_sqr(e.position()) < 16.0 {
                    teleport(e, m, level);
                }
                self.teleport_time = 0;
            } else if t.pos.distance_to_sqr(e.position()) > 256.0 {
                let tt = self.teleport_time;
                self.teleport_time += 1;
                if tt >= mth::reduced_tick_delay(30) && teleport_towards(e, m, level, &t) {
                    self.teleport_time = 0;
                }
            }
        }
    }
}

/// `ResetUniversalAngerTargetGoal`: the `universalAnger` game rule is off.
#[derive(Clone, Debug)]
pub struct ResetUniversalAnger {
    pub last_hurt_by_player_timestamp: i32,
}

impl CustomGoal for ResetUniversalAnger {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ResetUniversalAngerTargetGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
}

/// `EndermanLeaveBlockGoal`.
#[derive(Clone, Debug)]
struct LeaveBlock;

impl CustomGoal for LeaveBlock {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "EndermanLeaveBlockGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if st(m).carried.is_none() || !level.mob_griefing() {
            return false;
        }
        e.random.next_int_bounded(mth::reduced_tick_delay(2000)) == 0
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let x = crate::math::floor(e.x() - 1.0 + e.random.next_double() * 2.0);
        let y = crate::math::floor(e.y() + e.random.next_double() * 2.0);
        let z = crate::math::floor(e.z() - 1.0 + e.random.next_double() * 2.0);
        let pos = BlockPos::new(x, y, z);
        let target = level.block(pos);
        let below = level.block(pos.below());
        let Some(carried) = st(m).carried else { return };
        if can_place(e, level, pos, carried, target, below) {
            level.set_block(pos, carried, 3);
            level.emit(Event::GameEvent { event: "minecraft:block_place", pos: pos.center(), entity: Some(e.id) });
            st_mut(m).carried = None;
        }
    }
}

fn can_place(e: &Entity, level: &dyn EntityLevel, pos: BlockPos, carried: u16, target: u16, below: u16) -> bool {
    if !kiln_data::blocks_types::is_air(target) || kiln_data::blocks_types::is_air(below) || crate::blocks::block_name(below) == "minecraft:bedrock" {
        return false;
    }
    if !path::collision_full_block(below) || !can_survive(level, carried, pos, below) {
        return false;
    }
    let b = Aabb::new(pos.x as f64, pos.y as f64, pos.z as f64, pos.x as f64 + 1.0, pos.y as f64 + 1.0, pos.z as f64 + 1.0);
    level.entities_in(&b, EntityFilter::Any, e.id).is_empty()
        // (a sneaking player's box is 0.3 shorter: the wider query only narrows the candidates)
        && !level.players_in(&Aabb::new(b.min_x, b.min_y - 0.5, b.min_z, b.max_x, b.max_y, b.max_z)).iter().any(|p| !p.spectator && Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + 1.8, p.pos.z + 0.3).intersects(&b))
}

/// `BlockState.canSurvive` for the holdable blocks, on a full block. Approximation: plants
/// want soil (flowers and roots: dirt-like blocks; fungi and nether roots also nylium and soul
/// soil; cactus: sand), everything else stands anywhere.
fn can_survive(_level: &dyn EntityLevel, carried: u16, _pos: BlockPos, below: u16) -> bool {
    let name = crate::blocks::block_name(carried);
    let ground = crate::blocks::block_name(below);
    let dirtlike = matches!(
        ground,
        "minecraft:dirt" | "minecraft:grass_block" | "minecraft:podzol" | "minecraft:coarse_dirt" | "minecraft:mycelium" | "minecraft:rooted_dirt" | "minecraft:moss_block" | "minecraft:mud" | "minecraft:muddy_mangrove_roots" | "minecraft:farmland" | "minecraft:pale_moss_block"
    );
    let nether = matches!(ground, "minecraft:crimson_nylium" | "minecraft:warped_nylium" | "minecraft:soul_soil");
    match name {
        "minecraft:cactus" => matches!(ground, "minecraft:sand" | "minecraft:red_sand" | "minecraft:cactus"),
        "minecraft:crimson_fungus" | "minecraft:warped_fungus" | "minecraft:crimson_roots" | "minecraft:warped_roots" => dirtlike || nether,
        "minecraft:brown_mushroom" | "minecraft:red_mushroom" => true,
        _ if crate::blocks::has_tag(carried, crate::blocks::Tag::EdibleForSheep) || name.ends_with("_tulip") || is_small_flower(name) => dirtlike,
        _ => true,
    }
}

fn is_small_flower(name: &str) -> bool {
    matches!(
        name,
        "minecraft:dandelion" | "minecraft:poppy" | "minecraft:blue_orchid" | "minecraft:allium" | "minecraft:azure_bluet" | "minecraft:oxeye_daisy" | "minecraft:cornflower" | "minecraft:lily_of_the_valley" | "minecraft:wither_rose" | "minecraft:torchflower" | "minecraft:open_eyeblossom" | "minecraft:closed_eyeblossom" | "minecraft:golden_dandelion"
    )
}

/// `EndermanTakeBlockGoal`.
#[derive(Clone, Debug)]
struct TakeBlock;

impl CustomGoal for TakeBlock {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "EndermanTakeBlockGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if st(m).carried.is_some() || !level.mob_griefing() {
            return false;
        }
        e.random.next_int_bounded(mth::reduced_tick_delay(20)) == 0
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let x = crate::math::floor(e.x() - 2.0 + e.random.next_double() * 4.0);
        let y = crate::math::floor(e.y() + e.random.next_double() * 3.0);
        let z = crate::math::floor(e.z() - 2.0 + e.random.next_double() * 4.0);
        let pos = BlockPos::new(x, y, z);
        let state = level.block(pos);
        let bp = e.block_position();
        let from = Vec3::new(bp.x as f64 + 0.5, y as f64 + 0.5, bp.z as f64 + 0.5);
        let to = Vec3::new(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
        // `level.clip(OUTLINE)`: the first block hit, or the end block on a miss.
        // Approximation: collision shapes stand in for outline shapes.
        let hit = crate::clip::traverse_blocks(from, to, |p| {
            let s = level.block(p);
            let (shape, _) = crate::collision::collision_shape(s, p, &crate::collision::CollisionContext::EMPTY);
            crate::clip::shape_clips(&shape, from, to, p).then_some(p)
        })
        .unwrap_or_else(|| BlockPos::containing(to.x, to.y, to.z));
        if enderman_holdable(state) && hit == pos {
            level.set_block(pos, 0, 3);
            level.emit(Event::GameEvent { event: "minecraft:block_destroy", pos: pos.center(), entity: Some(e.id) });
            st_mut(m).carried = Some(kiln_data::blocks_types::block_of(state).default);
        }
    }
}
