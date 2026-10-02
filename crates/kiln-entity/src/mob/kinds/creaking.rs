//! Creaking: a pale-garden monster that cannot move while a player looks at it. It activates
//! when first looked at within 12 blocks, then hunts that player, freezing whenever anyone
//! watches. One bound to a creaking heart (`home_pos`, see [`crate::mob::kinds::creaking_heart`]) cannot be hurt
//! (a hit only makes it sway and hurts the heart) and crumbles when its heart is gone.
//!
//! Driven by the brain of `CreakingAi` (core: swim, look sink, move sink; idle: start attacking
//! an active creaking's player, look about, stroll; fight: walk to the target, melee it every 40
//! ticks, stop when the player is out of sight), on [`crate::mob::brain`].
//!
//! Gaps: the home-anchored node evaluator only blocks pathing through nodes beyond 32 blocks
//! of the heart (see [`crate::mob::path`]).

use crate::behavior_boilerplate;
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::*;
use crate::mob::brain::sensors;
use crate::mob::brain::{self, Activity, ActivityData, Behavior, Brain, Cx, Mem, Status, Timed, Val};
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::{self, Living};
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
    /// `CAN_MOVE` (synched).
    pub can_move: bool,
    /// `IS_ACTIVE`.
    pub active: bool,
    /// `IS_TEARING_DOWN`.
    pub tearing_down: bool,
    /// `HOME_POS`: the heart it is bound to.
    pub home: Option<BlockPos>,
    invulnerability_ticks: i32,
    attack_ticks: i32,
    player_stuck_counter: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("creaking state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("creaking state")
}

/// The heart the creaking is bound to (`getHomePos`).
pub fn home(m: &MobData) -> Option<BlockPos> {
    st(m).home
}

/// Whether the creaking `e` is bound to a heart (`isHeartBound`): it cannot use portals.
pub fn is_heart_bound(e: &Entity) -> bool {
    mob::data(e).and_then(|m| ext::state::<State>(m)).is_some_and(|s| s.home.is_some())
}

/// `setTearingDown`.
pub fn set_tearing_down(m: &mut MobData) {
    st_mut(m).tearing_down = true;
}

/// `setTransient(home)`: binds the creaking to a heart; hazards cost it less as it cannot be
/// hurt.
pub fn set_transient(m: &mut MobData, home: BlockPos) {
    st_mut(m).home = Some(home);
    m.maluses.push((path::PathType::Damaging, 8.0));
    m.maluses.push((path::PathType::PowderSnow, 8.0));
    m.maluses.push((path::PathType::Lava, 8.0));
    m.maluses.push((path::PathType::Fire, 0.0));
    m.maluses.push((path::PathType::FireInNeighbor, 0.0));
}

fn can_move(m: &MobData) -> bool {
    st(m).can_move
}

/// `isLookingAtMe(player, 0.5, false, true, eyes, y + 0.5 * scale, middle)`: the player's view
/// within 60 degrees of a point of the creaking with a clear line under the visual shapes.
fn looked_at_by(e: &Entity, level: &dyn EntityLevel, p: &crate::level::PlayerView) -> bool {
    let look = crate::ext_entity::fireball::view_vector(p.pitch, p.yaw).normalize();
    let eye = p.pos.y + p.eye_height as f64;
    // `getY() + 0.5 * getScale()`: the scale attribute is 1 for a creaking.
    for h in [e.eye_y(), e.y() + 0.5, (e.eye_y() + e.y()) / 2.0] {
        let dir = Vec3::new(e.x() - p.pos.x, h - eye, e.z() - p.pos.z).normalize();
        let dot = look.x * dir.x + look.y * dir.y + look.z * dir.z;
        if dot > 1.0 - 0.5 {
            // `player.hasLineOfSight(creaking, VISUAL, NONE, y)`: a clip from the player's eyes
            // with the player's collision context.
            let from = Vec3::new(p.pos.x, eye, p.pos.z);
            let to = Vec3::new(e.x(), h, e.z());
            if to.distance_to_sqr(from).sqrt() <= 128.0 && !clip_visual(level, from, to, &player_context(p)) {
                return true;
            }
        }
    }
    false
}

/// `CollisionContext.of(player)` as `ClipContext` builds it: the context-dependent shapes
/// (scaffolding) read whether the player is above them and sneaking.
fn player_context(p: &crate::level::PlayerView) -> crate::collision::CollisionContext {
    crate::collision::CollisionContext {
        descending: p.sneaking,
        entity_bottom: p.pos.y,
        has_entity: true,
        ..crate::collision::CollisionContext::EMPTY
    }
}

/// `BlockState.getVisualShape(level, pos, context)`: the collision shape, except for the blocks
/// that override it: glass, panes, iron bars and powder snow have none; mud and soul sand are
/// whole blocks to the eye; fences and snow layers show their outline shape (a fence is lower
/// than it collides, a layer one step higher).
fn visual_shape(s: u16, p: BlockPos, ctx: &crate::collision::CollisionContext) -> std::borrow::Cow<'static, crate::shape::Shape> {
    use kiln_data::block_logic::{BlockClass as C, is_instance};
    use std::borrow::Cow;
    if is_instance(s, C::TransparentBlock) || is_instance(s, C::IronBarsBlock) || is_instance(s, C::PowderSnowBlock) {
        Cow::Borrowed(crate::physics::empty_shape())
    } else if is_instance(s, C::MudBlock) || is_instance(s, C::SoulSandBlock) {
        Cow::Borrowed(crate::physics::block_shape())
    } else if is_instance(s, C::FenceBlock) || is_instance(s, C::SnowLayerBlock) {
        Cow::Borrowed(crate::physics::outline_shape(s))
    } else {
        crate::collision::collision_shape(s, p, ctx).0
    }
}

/// `Level.clip(ClipContext(from, to, VISUAL, NONE, player))`: whether a block's visual shape is in
/// the way.
fn clip_visual(level: &dyn EntityLevel, from: Vec3, to: Vec3, ctx: &crate::collision::CollisionContext) -> bool {
    crate::clip::traverse_blocks(from, to, |p| {
        let shape = visual_shape(level.block(p), p, ctx);
        crate::clip::shape_clips(&shape, from, to, p).then_some(())
    })
    .is_some()
}

/// `checkCanMove`: frozen while an attackable player (not in a carved pumpkin once active) of
/// the nearest players looks at it; the first look within 12 blocks activates it against that
/// player. Reads the `NEAREST_PLAYERS` memory the brain's sensor keeps.
fn check_can_move(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    let ids: Vec<i32> = m.brain.as_ref().map(|b| b.st.mem.entities(Mem::NearestPlayers).to_vec()).unwrap_or_default();
    let active = st(m).active;
    if ids.is_empty() {
        if active {
            deactivate(e, m, level);
        }
        return true;
    }
    let mut potential = false;
    for id in ids {
        let Some(p) = level.player(id) else { continue };
        let t = goals::living_player(&p);
        if !goals::can_attack(m, level, &t) {
            continue;
        }
        potential = true;
        // `isActive && !PLAYER_NOT_WEARING_DISGUISE_ITEM.test(player)`.
        if active && mob::item_tag(p.head, "minecraft:gaze_disguise_equipment") {
            continue;
        }
        if looked_at_by(e, level, &p) {
            if active {
                return false;
            }
            if p.pos.distance_to_sqr(e.position()) < 144.0 {
                activate(e, m, level, p.id);
                return false;
            }
        }
    }
    if !potential && active {
        deactivate(e, m, level);
    }
    true
}

/// `activate(player)`.
fn activate(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: i32) {
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.set(Mem::AttackTarget, Val::Entity(player));
    }
    // `Mob.getTarget` is the brain's attack target.
    m.target = Some(player);
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    mob::make_sound(e, m, level, "minecraft:entity.creaking.activate");
    st_mut(m).active = true;
}

/// `deactivate()`.
fn deactivate(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.erase(Mem::AttackTarget);
    }
    m.target = None;
    level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
    mob::make_sound(e, m, level, "minecraft:entity.creaking.deactivate");
    st_mut(m).active = false;
}

/// `Mob.stopInPlace`.
fn stop_in_place(e: &mut Entity, m: &mut MobData) {
    m.nav.stop();
    m.xxa = 0.0;
    m.yya = 0.0;
    mob::control::set_speed(m, 0.0);
    e.delta = Vec3::ZERO;
}

/// `playerIsStuckInYou`: a nearby player's eyes inside the creaking's box for more than 4 checks.
pub fn player_is_stuck_in_you(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
    let ids: Vec<i32> = m.brain.as_ref().map(|b| b.st.mem.entities(Mem::NearestPlayers).to_vec()).unwrap_or_default();
    if ids.is_empty() {
        st_mut(m).player_stuck_counter = 0;
        return false;
    }
    let bb = e.bounding_box();
    for id in ids {
        let Some(p) = level.player(id) else { continue };
        if bb.contains(Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z)) {
            let s = st_mut(m);
            s.player_stuck_counter += 1;
            return s.player_stuck_counter > 4;
        }
    }
    st_mut(m).player_stuck_counter = 0;
    false
}

/// `tearDown`: the creaking crumbles away (pale oak wood and awake-heart crumbles) and goes.
pub fn tear_down(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let b = e.bounding_box();
    let c = b.center();
    let spread = Vec3::new((b.max_x - b.min_x) * 0.3, (b.max_y - b.min_y) * 0.3, (b.max_z - b.min_z) * 0.3);
    level.crumble_particles(c, kiln_data::blocks::default_state::PALE_OAK_WOOD, 100, spread);
    let awake = super::creaking_heart::with_state(kiln_data::blocks::default_state::CREAKING_HEART, "awake");
    level.crumble_particles(c, awake, 10, spread);
    mob::make_sound(e, m, level, "minecraft:entity.creaking.death");
    e.discard();
    // `LivingEntity.remove`: `brain.clearMemories()`.
    if let Some(b) = m.brain.as_mut() {
        b.st.mem.clear_all();
    }
    m.target = None;
}

/// `creakingDeathEffects(source)`: `die` and the twitching sound (a bound creaking whose heart
/// was broken by a player or an explosion).
pub fn death_effects(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
    resolve_blame(e, m, level, source);
    mob::die(e, m, level, *source);
    mob::make_sound(e, m, level, "minecraft:entity.creaking.twitch");
}

/// `resolveMobResponsibleForDamage` and `resolvePlayerResponsibleForDamage`: who the hit blames
/// (a player, for the heart to feel it).
fn resolve_blame(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, source: &DamageSource) -> Option<i32> {
    if let Some(a) = source.attacker
        && !source.kind.is_tag("minecraft:no_anger")
        && goals::living(level, a).is_some()
    {
        m.last_hurt_by_mob = Some(a);
        m.last_hurt_by_mob_timestamp = e.tick_count;
    }
    if source.attacker_is_player {
        m.last_hurt_by_player = source.attacker;
        m.last_hurt_by_player_memory = 100;
    }
    m.last_hurt_by_player
}

impl Kind for Creaking {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        Some(Box::new(State { can_move: true, active: false, tearing_down: false, home: None, invulnerability_ticks: 0, attack_ticks: 0, player_stuck_counter: 0 }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    /// `Creaking.tick`: without its heart a bound creaking dies.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(h) = st(m).home
            && !level.heart_protects(h, e.id, e.uuid)
        {
            m.set_health(0.0);
        }
    }

    /// `Creaking.aiStep` before `LivingEntity.aiStep`: the animation timers and freezing.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        if s.invulnerability_ticks > 0 {
            s.invulnerability_ticks -= 1;
        }
        if s.attack_ticks > 0 {
            s.attack_ticks -= 1;
        }
        let was = can_move(m);
        let now = check_can_move(e, m, level);
        if now != was {
            level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
            if now {
                mob::make_sound(e, m, level, "minecraft:entity.creaking.unfreeze");
            } else {
                stop_in_place(e, m);
                mob::make_sound(e, m, level, "minecraft:entity.creaking.freeze");
            }
        }
        st_mut(m).can_move = now;
    }

    /// `customServerAiStep`: the brain, then `CreakingAi.updateActivity`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        brain::tick_brain(e, m, level);
        let movable = can_move(m);
        if let Some(b) = m.brain.as_mut() {
            if movable {
                b.st.set_active_activity_to_first_valid(&[Activity::Fight, Activity::Idle]);
            } else {
                b.st.use_default_activity();
            }
        }
        sync_target(m);
    }

    fn path_home(&self, m: &MobData) -> Option<BlockPos> {
        st(m).home
    }

    /// `CreakingNavigation.tick`, `CreakingMoveControl`, `CreakingLookControl` and
    /// `CreakingJumpControl`: nothing works while frozen.
    fn ticks_navigation(&self, m: &MobData) -> bool {
        can_move(m)
    }

    fn tick_move(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !can_move(m)
    }

    fn tick_look(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !can_move(m)
    }

    fn tick_jump(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if can_move(m) {
            return false;
        }
        m.jumping = false;
        true
    }

    /// `CreakingBodyRotationControl`.
    fn tick_body(&self, _e: &mut Entity, m: &mut MobData) -> bool {
        !can_move(m)
    }

    /// `isPushable`, `push`: a frozen creaking cannot be pushed; nor knocked back.
    fn can_be_pushed(&self, m: &MobData) -> bool {
        can_move(m)
    }

    fn knockback_immune(&self, m: &MobData) -> bool {
        !can_move(m)
    }

    /// `fireImmune` while bound: fire cannot hurt it.
    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        st(m).home.is_some() && kind.is_tag("minecraft:is_fire")
    }

    /// `hurtServer`: a heart-bound creaking sways instead of taking damage (and, when a player
    /// blamed for it, the heart hurts).
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let _ = amount;
        let home = st(m).home?;
        if source.kind.is_tag("minecraft:bypasses_invulnerability") {
            return None;
        }
        if e.is_removed() || e.invulnerable || st(m).invulnerability_ticks > 0 || m.is_dead_or_dying() {
            return Some(false);
        }
        let player = resolve_blame(e, m, &*level, source);
        let direct = source.direct.or(source.attacker);
        let direct_living = direct.is_some_and(|d| goals::living(&*level, d).is_some());
        if !direct_living && !source.kind.is_tag("minecraft:is_projectile") && player.is_none() {
            return Some(false);
        }
        st_mut(m).invulnerability_ticks = 8;
        level.emit(Event::EntityEvent { entity: e.id, event: 66 });
        level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(e.id) });
        if level.heart_protects(home, e.id, e.uuid) {
            if player.is_some() {
                let at = e.bounding_box().center();
                level.heart_creaking_hurt(home, e.id, e.uuid, at);
            }
            mob::make_sound(e, m, level, "minecraft:entity.creaking.sway");
        }
        Some(true)
    }

    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        st_mut(m).attack_ticks = 15;
        level.emit(Event::EntityEvent { entity: e.id, event: 4 });
        let hurt = mob::do_hurt_target_base(e, m, level, t);
        if hurt {
            // `Mob.doHurtTarget`: `playAttackSound`.
            mob::make_sound(e, m, level, "minecraft:entity.creaking.attack");
        }
        Some(hurt)
    }

    /// `tickDeath`: a bound creaking whose heart was broken twitches for 45 ticks, then
    /// crumbles.
    fn tick_death(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        if s.home.is_none() || !s.tearing_down {
            return false;
        }
        m.death_time += 1;
        if m.death_time > 45 && !e.is_removed() {
            tear_down(e, m, level);
        }
        true
    }

    /// `LivingEntity.remove` cleared the brain's memories: no attack target any more.
    fn on_killed_removal(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
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

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        if let Some(Tag::IntArray(a)) = r.get("home_pos")
            && a.len() == 3
        {
            let home = BlockPos::new(a[0], a[1], a[2]);
            set_transient(m, home);
        }
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

/// `getTarget` is `getTargetFromBrain`: the attack target memory.
fn sync_target(m: &mut MobData) {
    m.target = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
}

/// `CreakingAi$1`: `Swim(0.8)` that only starts while the creaking can move.
#[derive(Clone, Debug)]
struct CreakingSwim(Swim);

impl Behavior for CreakingSwim {
    fn name(&self) -> &'static str {
        ""
    }
    fn check_extra_start(&mut self, cx: &mut Cx) -> bool {
        can_move(cx.m) && self.0.check_extra_start(cx)
    }
    fn can_still_use(&mut self, cx: &mut Cx) -> bool {
        self.0.can_still_use(cx)
    }
    fn tick(&mut self, cx: &mut Cx) {
        self.0.tick(cx);
    }
    behavior_boilerplate!();
}

/// `CreakingAi.getActivities` and the sensors of `Creaking.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![Box::new(sensors::NearestLivingEntities), Box::new(sensors::Players)];
    let core = ActivityData::create(Activity::Core, 0, vec![Timed::new(CreakingSwim(Swim { chance: 0.8 })), LookAtTargetSink::new(45, 90), MoveToTargetSink::new()]);
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![
            start_attacking(|cx| st(cx.m).active, |cx| cx.b.mem.entity(Mem::NearestVisibleAttackablePlayer)),
            SetEntityLookTargetSometimes::new(None, 8.0, (30, 60)),
            brain::Gate::run_one(vec![
                (stroll(0.3, StrollKind::Land { avoid_water: true }), 2),
                (set_walk_target_from_look_target(0.3, 3), 2),
                (DoNothing::new(30, 60), 1),
            ]),
        ],
    );
    let fight = ActivityData::with_conditions(
        Activity::Fight,
        vec![
            (10, set_walk_target_from_attack_target_if_out_of_reach(|_| 1.0)),
            (11, melee_attack_when(|cx| can_move(cx.m), 40)),
            (12, stop_attacking_if_target_invalid(|cx, t| !(t.player && cx.b.mem.entities(Mem::NearestVisibleAttackablePlayers).contains(&t.id)), |_, _| {}, true)),
        ],
        &[(Mem::AttackTarget, Status::ValuePresent)],
    );
    Brain::new(&[], sensors, vec![core, idle, fight], random)
}
