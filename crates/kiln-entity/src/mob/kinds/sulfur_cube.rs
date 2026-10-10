//! Sulfur cube: a cube mob ([`super::slime`] has the shared `AbstractCubeMob` code) that hops about
//! (size 2; the halves it splits into are size 1 babies that grow up with a sulfur-yellow feed),
//! swallows an item and then behaves by what the item is: a ball that bounces, slides, floats,
//! burns, explodes... (the archetypes of [`super::sulfur_archetype`]). A player shears the item
//! out, hits the ball along where he looks, or pushes it by walking against it.
//!
//! The body slot holds the swallowed item; with one the goals are gone (it no longer hops), the
//! move control does nothing and the look control keeps it facing a side of the grid.

use super::slime::{self, Cube, CubeFloat, CubeKeepOnJumping, CubeRandomDirection};
use super::sulfur_archetype::{self as arch, Archetype};
use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::{Attr, Op};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView, state, state_mut};
use crate::mob::goals::{self, Goal, LOOK, Living};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, SpawnContext, mth};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct SulfurCube;

pub static KIND: SulfurCube = SulfurCube;

static INFO: Info = Info {
    ageable: true,
    head: (75, 0, 10),
    extends_monster: false,
    monster_base: false,
    sound_source: "neutral",
    ..Info::monster("minecraft:sulfur_cube", &[(Attr::TemptRange, 8.0)])
};

/// `SulfurCube.PICKUP_TIMER_DURATION`.
const PICKUP_TIMER_DURATION: i32 = 100;

/// What the cube's archetype made of it (`explosionData`, `knockbackModifier`, `soundSettings`,
/// `contactDamages`, `floatsInLiquids`), the swallowed item and the fuse.
#[derive(Clone, Debug)]
pub struct Sulfur {
    pub pickup_timer: i32,
    pub push_sound_cooldown: i32,
    pub floats_in_liquids: bool,
    pub explosion: Option<arch::Explosion>,
    pub knockback: (f32, f32),
    /// (hit sound, push sound, push cooldown in seconds, impulse threshold).
    pub sound: (&'static str, &'static str, f32, f32),
    pub contact_damages: Vec<arch::ContactDamage>,
    /// `fuse`: -1 while the cube is not primed.
    pub fuse: i32,
    pub from_bucket: bool,
    /// The `BODY` slot and its drop chance.
    pub body: ItemStack,
    pub body_drop: f32,
    /// The body as `lastEquipmentItems` has it (changes are noticed a tick later).
    pub last_body: ItemStack,
}

impl Sulfur {
    pub fn new() -> Sulfur {
        Sulfur {
            pickup_timer: 0,
            push_sound_cooldown: 0,
            floats_in_liquids: false,
            explosion: None,
            knockback: arch::DEFAULT_KNOCKBACK,
            sound: (arch::DEFAULT_SOUND.0, arch::DEFAULT_SOUND.1, arch::DEFAULT_SOUND.2, arch::DEFAULT_SOUND.3),
            contact_damages: Vec::new(),
            fuse: -1,
            from_bucket: false,
            body: ItemStack::empty(),
            body_drop: 0.085,
            last_body: ItemStack::empty(),
        }
    }
}

impl Default for Sulfur {
    fn default() -> Self {
        Sulfur::new()
    }
}

fn sul(m: &MobData) -> &Sulfur {
    state::<Cube>(m).and_then(|c| c.sulfur.as_deref()).expect("sulfur cube state")
}

fn sul_mut(m: &mut MobData) -> &mut Sulfur {
    state_mut::<Cube>(m).and_then(|c| c.sulfur.as_deref_mut()).expect("sulfur cube state")
}

/// `hasBodyItem`.
pub fn has_body(m: &MobData) -> bool {
    !sul(m).body.is_empty()
}

/// `isPrimed`.
pub fn is_primed(m: &MobData) -> bool {
    state::<Cube>(m).and_then(|c| c.sulfur.as_deref()).is_some_and(|s| s.fuse >= 0)
}

/// `isTiny`.
fn is_tiny(m: &MobData) -> bool {
    slime::size(m) <= 1
}

/// `canExplode`.
fn can_explode(e: &Entity, m: &MobData) -> bool {
    sul(m).explosion.is_some() && mob::is_alive(e, m) && !is_primed(m)
}

/// The sulfur cube's sounds (`getSquishSound`, `getJumpSound`).
pub fn cube_sound(m: &MobData, what: &str) -> String {
    let tiny = is_tiny(m);
    match (what, tiny) {
        ("squish", true) => "minecraft:entity.small_sulfur_cube.squish".to_owned(),
        ("squish", false) if has_body(m) => "minecraft:entity.sulfur_cube.bounce".to_owned(),
        ("jump", true) => "minecraft:entity.small_sulfur_cube.jump".to_owned(),
        (w, _) => format!("minecraft:entity.sulfur_cube.{w}"),
    }
}

fn sound(e: &Entity, level: &mut dyn EntityLevel, name: &'static str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: name, source: "neutral", volume, pitch });
    }
}

/// `SulfurCube.setSize`: a size 1 cube is a baby.
pub fn set_size(e: &mut Entity, m: &mut MobData, size: i32, update_health: bool) {
    slime::set_size(e, m, size, update_health);
    if update_health && size == 1 && !m.baby() {
        mob::set_age(e, m, mob::breed::BABY_START_AGE);
    }
}

// ---------------------------------------------------------------------- the body item

/// `isSwallowableItem`.
fn swallowable(item: i32) -> bool {
    mob::item_tag(item, "minecraft:sulfur_cube_swallowable")
}

/// `canHoldItem`: nothing in the body slot yet, a swallowable item, not a baby.
fn can_hold(m: &MobData, stack: &ItemStack) -> bool {
    !has_body(m) && !stack.is_empty() && swallowable(stack.item()) && !m.baby()
}

/// The part of `collectEquipmentChanges` for the body: the old item's archetypes give up their
/// modifiers, the new one's apply; goals come and go with the item.
fn apply_body_change(e: &mut Entity, m: &mut MobData, old: &ItemStack, new: &ItemStack) {
    if new.is_empty() {
        // `registerGoals`.
        KIND.register_goals(m);
    } else {
        m.goals.remove_where(|_| true);
        m.speed = 0.0;
        m.zza = 0.0;
    }
    for a in arch::matching(old) {
        for (attr, id, _, _) in a.modifiers() {
            m.attrs.remove_modifier(attr, id);
        }
    }
    {
        let s = sul_mut(m);
        s.floats_in_liquids = false;
        s.explosion = None;
        s.contact_damages.clear();
        s.knockback = arch::DEFAULT_KNOCKBACK;
        s.sound = (arch::DEFAULT_SOUND.0, arch::DEFAULT_SOUND.1, arch::DEFAULT_SOUND.2, arch::DEFAULT_SOUND.3);
    }
    for a in arch::matching(new) {
        apply_archetype(m, a);
    }
    sync_physics(e, m);
}

fn apply_archetype(m: &mut MobData, a: &'static Archetype) {
    let s = sul_mut(m);
    if a.buoyant {
        s.floats_in_liquids = true;
    }
    if a.explosion.is_some() {
        s.explosion = a.explosion;
    }
    if let Some(c) = a.contact_damage {
        s.contact_damages.push(c);
    }
    s.knockback = (a.horizontal_power, a.vertical_power);
    s.sound = (a.hit_sound, a.push_sound, a.push_sound_cooldown, a.push_sound_impulse_threshold);
    for (attr, id, amount, op) in a.modifiers() {
        m.attrs.set_modifier(attr, id, amount, op);
    }
}

/// What the entity carries of the attributes: `getEntityBounciness`, `getAirDrag` and `maxUpStep`.
fn sync_physics(e: &mut Entity, m: &MobData) {
    e.bounciness = m.attrs.value(Attr::Bounciness);
    let base = if has_body(m) { 0.91 } else { 0.98 };
    let modifier = m.attrs.value(Attr::AirDragModifier) as f32;
    e.air_drag_override = Some(mth::clamp(1.0 - (1.0 - base) * modifier, 0.0, 1.0));
    e.max_up_step = if has_body(m) { 0.0 } else { m.attrs.value(Attr::StepHeight) as f32 };
}

/// `Mob.setItemSlotAndDropWhenKilled(BODY, stack)`.
fn set_body(m: &mut MobData, stack: ItemStack) {
    let s = sul_mut(m);
    s.body = stack;
    s.body_drop = 2.0;
}

// ---------------------------------------------------------------------- the fuse

/// `PrimedTnt.getRandomShortFuse`.
fn random_short_fuse(fuse: i32, r: &mut dyn RandomSource) -> i32 {
    r.next_int_bounded((fuse / 4).max(1)) + fuse / 8
}

/// `primeTime`: the fuse is lit (a short one when an explosion lit it).
pub fn prime_time(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, short_fuse: bool) -> bool {
    let Some(ex) = sul(m).explosion else { return false };
    if !mob::is_alive(e, m) || !level.tnt_explodes() || is_primed(m) {
        return false;
    }
    let fuse = if short_fuse { random_short_fuse(ex.fuse, &mut e.random) } else { ex.fuse };
    e.invulnerable = true;
    sul_mut(m).fuse = fuse;
    mob::make_sound(e, m, level, "minecraft:entity.tnt.primed");
    level.emit(Event::GameEvent { event: "minecraft:prime_fuse", pos: e.position(), entity: Some(e.id) });
    true
}

/// `tickFuse`, `primeWhenOnPoweredPosition`.
fn tick_fuse(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = sul_mut(m);
    if s.fuse > 0 {
        s.fuse -= 1;
    }
    let (fuse, explosion) = (s.fuse, s.explosion);
    if let Some(ex) = explosion
        && fuse == 0
    {
        crate::leash::drop_leash(e, Some(m), level);
        m.dead = true;
        if level.tnt_explodes() {
            let interaction = if level.mob_griefing() { crate::explosion::Interaction::Tnt } else { crate::explosion::Interaction::Keep };
            let center = Vec3::new(e.x(), e.y() + e.height as f64 * 0.0625, e.z());
            crate::explosion::explode(level, Some(e.id), center, ex.power as f32, ex.causes_fire, interaction);
        }
        e.discard();
    }
}

fn prime_when_powered(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if can_explode(e, m) && level.best_own_or_neighbour_signal(e.block_position()) != 0 {
        prime_time(e, m, level, false);
    }
}

// ---------------------------------------------------------------------- goals

/// `SulfurCubeTemptGoal` (a `TemptGoal.ForNonPathfinders` that steers the cube's move control).
#[derive(Clone, Debug)]
struct SulfurCubeTempt {
    player: Option<i32>,
    calm_down: i32,
}

/// The items `addBehaviourGoals` tempts with: a baby's feed, else what can be swallowed.
fn tempts(m: &MobData, item: i32) -> bool {
    item != 0 && if m.baby() { mob::item_tag(item, "minecraft:sulfur_cube_food") } else { swallowable(item) }
}

impl CustomGoal for SulfurCubeTempt {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SulfurCubeTemptGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let range = m.attrs.value(Attr::TemptRange);
        let baby = m.baby();
        let found = goals::nearest_player(e, m, &*level, false, range, false, |p| {
            let ok = |item: i32| item != 0 && if baby { mob::item_tag(item, "minecraft:sulfur_cube_food") } else { swallowable(item) };
            ok(p.main_hand) || ok(p.off_hand)
        });
        self.player = found.map(|t| t.id);
        self.player.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level)
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.player = None;
        slime::set_wanted_movement(m, 0.0);
        self.calm_down = mth::reduced_tick_delay(100);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return };
        let (max_y, max_x) = (m.kind.max_head_y_rot() + 20, m.max_head_x_rot());
        m.look.set_look_at(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z, max_y as f32, max_x as f32);
        if e.position().distance_to_sqr(p.pos) < 1.0 {
            slime::set_wanted_movement(m, 0.0);
        } else if let Some(t) = goals::living(&*level, p.id) {
            // `navigateTowards`: `lookAt(player, 10, 10)`, then straight ahead.
            mob::mob_look_at(e, &t, 10.0, 10.0);
            slime::set_direction(m, e.y_rot, true);
        }
    }
}

/// `SulfurCubeSearchForItemsGoal`: toward the nearest item it could swallow within 8 blocks.
#[derive(Clone, Debug)]
struct SearchForItems {
    target: Option<i32>,
}

/// `SulfurCube.ALLOWED_ITEMS`.
fn allowed_item(level: &dyn EntityLevel, id: i32) -> Option<Vec3> {
    let it = level.entity(id)?;
    let EntityKind::Item(d) = &it.kind else { return None };
    (d.pickup_delay <= 0 && !it.is_removed() && !d.stack.is_empty() && swallowable(d.stack.item())).then(|| it.position())
}

impl CustomGoal for SearchForItems {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SulfurCubeSearchForItemsGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.baby() || sul(m).pickup_timer > 0 {
            return false;
        }
        let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
        let here = e.position();
        // `getNearestEntity(list, x, y, z)`: the first of the shortest distances.
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Item, e.id) {
            let Some(p) = allowed_item(&*level, id) else { continue };
            let d = p.distance_to_sqr(here);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.target = best.map(|(_, id)| id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level)
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(it) = self.target.and_then(|id| level.entity(id)) else { return };
        // `Mob.lookAt(entity)` of a non-living entity: the middle of its box.
        let b = it.bounding_box();
        let t = Living {
            id: it.id,
            type_name: it.type_name,
            pos: it.position(),
            eye_y: (b.min_y + b.max_y) / 2.0,
            alive: true,
            player: false,
            creative: false,
            spectator: false,
            invulnerable: false,
            sneaking: false,
            invisible: false,
            armor_cover: 0.0,
            bb: b,
        };
        mob::mob_look_at(e, &t, 10.0, 10.0);
        slime::set_direction(m, e.y_rot, true);
    }
}

// ---------------------------------------------------------------------- pushing and hitting

/// A view of an entity that hits or pushes: its eyes, look direction and feet, from the player list
/// or the entity.
fn actor(level: &dyn EntityLevel, id: i32) -> Option<(Vec3, Vec3, Vec3)> {
    if let Some(p) = level.player(id) {
        let eye = Vec3::new(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z);
        return Some((eye, mob::brain::behaviors::view_vector(p.pitch, p.yaw), p.pos));
    }
    let o = level.entity(id)?;
    Some((Vec3::new(o.x(), o.eye_y(), o.z()), mob::brain::behaviors::view_vector(o.x_rot, o.y_rot), o.position()))
}

/// `Vec2.rotate(radians)` (float math, the sine table).
fn rotate(v: (f32, f32), radians: f64) -> (f32, f32) {
    let (c, s) = (mth::cos(radians), mth::sin(radians));
    (v.0 * c - v.1 * s, v.1 * c + v.0 * s)
}

/// `applyHorizontalHitAngleScale`.
fn horizontal_hit_angle_scale(scale: f32, dir: (f32, f32), eye: Vec3, look: Vec3, center: Vec3) -> (f32, f32) {
    let to = (center - eye).normalize();
    let a = mth::atan2(look.x * to.z - look.z * to.x, look.x * to.x + look.z * to.z) as f32;
    rotate(dir, (a * scale) as f64)
}

/// `applyVerticalHitAnglePowerTransfer`.
fn vertical_hit_angle_power_transfer(scale: f32, h: f32, v: f32, eye: Vec3, look: Vec3, center: Vec3, height: f32) -> (f32, f32) {
    let half = 0.5 * height;
    let top = center.add(0.0, half as f64, 0.0);
    let bottom = center.add(0.0, -half as f64, 0.0);
    let to_top = (top - eye).normalize();
    let to_bottom = (bottom - eye).normalize();
    // `Mth.clampedMap(look.y, top.y, bottom.y, -1, 1)`.
    let t = (look.y - to_top.y) / (to_bottom.y - to_top.y);
    let mapped = if t < 0.0 {
        -1.0
    } else if t > 1.0 {
        1.0
    } else {
        -1.0 + t * (1.0 - -1.0)
    } as f32;
    let mut shift = (mapped * scale).abs();
    if mapped < 0.0 {
        shift = -shift;
    }
    (h * (1.0 - shift), v * (1.0 + shift))
}

/// `applyVerticalPositionAnglePowerRotation`.
fn vertical_position_angle_power_rotation(scale: f32, h: f32, v: f32, max_h: f32, max_v: f32, from: Vec3, to: Vec3) -> (f32, f32) {
    let d = to - from;
    let angle = mth::atan2(-d.y, d.horizontal_distance()) as f32;
    let mut r = rotate((h, v), (-angle * scale) as f64);
    let fx = if max_h > 0.0 { r.0.abs() / max_h } else { 0.0 };
    let fy = if max_v > 0.0 { r.1.abs() / max_v } else { 0.0 };
    let big = fx.max(fy);
    if big > 1.0 {
        let k = 1.0 / big;
        r = (r.0 * k, r.1 * k);
    }
    r
}

/// `SulfurCube.knockback(strength, dx, dz, source, amount, extra)` for a hit by an entity on a cube
/// with a body: the hit's direction, the angle of the hit on the ball and the distance from the
/// hitter decide how far and how high it flies. Returns false when the plain knockback applies.
fn hit_knockback(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, strength: f64, dx: f64, dz: f64, source: &DamageSource, amount: f32, extra: bool) -> bool {
    let Some(attacker) = source.attacker else { return false };
    if !has_body(m) {
        return false;
    }
    let Some((eye, look, feet)) = actor(&*level, attacker) else { return false };
    let s = sul(m);
    let (h_power, v_power) = s.knockback;
    let hit_sound = s.sound.0;
    let center = e.bounding_box().center();
    let look = look.normalize();
    let flat = horizontal_hit_angle_scale(1.6, (dx as f32, dz as f32), eye, look, center);
    let (mut h, mut v) = vertical_hit_angle_power_transfer(0.5, h_power, v_power, eye, look, center, e.height);
    (h, v) = vertical_position_angle_power_rotation(0.8, h, v, h_power, v_power, feet, e.position());
    let (dx, dz) = (flat.0 as f64, flat.1 as f64);
    let k = mth::sqrt_f(amount) * if extra { (strength as f32) * 0.25 } else { 1.0 };
    h *= k;
    v *= k;
    let resist = m.attrs.value(Attr::KnockbackResistance);
    h *= (1.0 - resist) as f32;
    v *= (1.0 - resist) as f32;
    e.needs_sync = true;
    let motion = e.delta;
    h *= 0.4;
    h = mth::clamp(h, -128.0, 128.0);
    v = mth::clamp(v, -128.0, 128.0);
    let push = Vec3::new(dx, 0.0, dz).normalize().scale(h as f64);
    e.delta = Vec3::new(motion.x - push.x, motion.y + v as f64 * 1.2, motion.z - push.z);
    sound(e, level, hit_sound, 1.0, 1.0);
    true
}

/// `applyContactDamage(entity)`: the hot cube burns what it touches.
fn apply_contact_damage(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, other: i32) {
    let damages = sul(m).contact_damages.clone();
    if damages.is_empty() {
        return;
    }
    let is_player = level.player(other).is_some();
    for d in damages {
        let attacker = (d.attribute_to_source || is_player).then_some(e.id);
        let source = DamageSource { kind: DamageKind::Named(d.damage_type), attacker, direct: attacker, pos: Some(e.position()), attacker_is_player: false };
        if is_player {
            level.hurt_player(other, source, d.amount);
        } else {
            slime::hurt_other(level, other, source, d.amount);
        }
    }
}

/// `SulfurCube.playerPush`: walking or riding into a ball pushes it, away from the player and by how
/// fast the player moves.
fn player_push(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, p: &Living) {
    if !has_body(m) {
        return;
    }
    let view = level.player(p.id);
    let passenger = view.as_ref().is_some_and(|v| v.vehicle.is_some());
    // The root vehicle (a boat, a horse) is what bumps the cube.
    let root = if passenger {
        let mut id = p.id;
        let mut guard = 0;
        while let Some(v) = level.player(id).and_then(|pv| pv.vehicle).or_else(|| level.entity(id).and_then(|o| o.vehicle)) {
            id = v;
            guard += 1;
            if guard > 8 {
                break;
            }
        }
        goals::living(&*level, id).map(|l| (l.pos, l.bb.max_y - l.bb.min_y)).or_else(|| level.entity(id).map(|o| (o.position(), o.height as f64)))
    } else {
        Some((p.pos, p.bb.max_y - p.bb.min_y))
    };
    let Some((root_pos, root_height)) = root else { return };
    let away = e.position() - root_pos;
    let (player_y, cube_y) = (root_pos.y, e.y());
    let cube_top = cube_y + e.height as f64;
    let player_top = player_y + (root_height as f32) as f64;
    if !(away.horizontal_distance() < 1.2999999523162842 && player_y <= cube_top && player_top > cube_y) {
        return;
    }
    let power = (1.0 - m.attrs.value(Attr::KnockbackResistance)).max(0.0);
    let dir = away.horizontal().normalize().scale(power);
    let factor: f32 = if passenger { 0.16 } else { 0.3 };
    let speed = mth::clamp_d(level.known_movement(p.id).length() * 2.0 * factor as f64, 0.0, 0.5);
    let up = if e.on_ground { power * 0.30000001192092896 } else { 0.0 };
    let push = Vec3::new(dir.x, up, dir.z).scale(speed);
    e.needs_sync = true;
    let (push_sound, cooldown, threshold) = {
        let s = sul(m);
        (s.sound.1, s.sound.2, s.sound.3)
    };
    if push.length_sqr() > (threshold * threshold) as f64 && sul(m).push_sound_cooldown <= 0 {
        sul_mut(m).push_sound_cooldown = (cooldown * 20.0) as i32;
        sound(e, level, push_sound, 1.0, 1.0);
    }
    e.delta = e.delta + push;
    apply_contact_damage(e, m, level, p.id);
}

// ---------------------------------------------------------------------- the kind

impl Kind for SulfurCube {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        slime::new_state_with(m, Some(Box::new(Sulfur::new())))
    }

    /// `AbstractCubeMob.registerGoals` and `addBehaviourGoals`: tempt and the search for items.
    fn register_goals(&self, m: &mut MobData) {
        slime::register_base_goals(m);
        m.goals.add(2, Goal::Custom(Box::new(SulfurCubeTempt { player: None, calm_down: 0 })));
        m.goals.add(3, Goal::Custom(Box::new(SearchForItems { target: None })));
    }

    fn sound_volume(&self, m: &MobData) -> f32 {
        0.4 * slime::size(m) as f32
    }

    fn touches_players(&self) -> bool {
        true
    }

    /// `tick`: the fuse and the powered position, then the cube's squish.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        tick_fuse(e, m, level);
        prime_when_powered(e, m, level);
        slime::pre_tick(m);
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        slime::post_tick(e, m, level);
    }

    /// `LivingEntity.detectEquipmentUpdates` for the body slot.
    fn detect_equipment_updates(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let (old, new) = {
            let s = sul(m);
            (s.last_body.clone(), s.body.clone())
        };
        if old != new {
            apply_body_change(e, m, &old, &new);
            sul_mut(m).last_body = new;
        }
    }

    /// `Mob.aiStep`'s pick up of items (reach 1, 0, 1), `customServerAiStep`'s timers.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !has_body(m) && mob::is_alive(e, m) && !m.dead && level.mob_griefing() {
            let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
            for id in level.entities_in(&area, EntityFilter::Item, e.id) {
                let Some(it) = level.entity(id) else { continue };
                let EntityKind::Item(d) = &it.kind else { continue };
                if it.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 || !can_hold(m, &d.stack) {
                    continue;
                }
                // `pickUpItem`.
                if sul(m).pickup_timer > 0 {
                    continue;
                }
                let mut stack = d.stack.clone();
                let one = stack.split(1);
                if let Some(it) = level.entity_mut(id)
                    && let EntityKind::Item(d) = &mut it.kind
                {
                    d.stack = stack.clone();
                    if stack.is_empty() {
                        it.discard();
                    }
                }
                set_body(m, one);
                sound(e, level, "minecraft:entity.sulfur_cube.absorb", 1.0, 1.0);
            }
        }
        let s = sul_mut(m);
        if s.pickup_timer > 0 {
            s.pickup_timer -= 1;
        }
        if s.push_sound_cooldown > 0 {
            s.push_sound_cooldown -= 1;
        }
    }

    /// `SulfurCubeMobMoveControl`: a ball does not steer.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !has_body(m) {
            slime::tick_move(e, m, level);
        }
        true
    }

    /// `SulfurCubeLookControl`: a ball keeps facing along the grid.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !has_body(m) {
            return false;
        }
        // `Mth.wrapDegrees90`.
        let mut r = e.y_rot % 90.0;
        if r >= 45.0 {
            r -= 90.0;
        }
        if r < -45.0 {
            r += 90.0;
        }
        e.y_rot -= r;
        m.y_head_rot = e.y_rot;
        true
    }

    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        slime::jump_from_ground(e, m, level);
        true
    }

    fn player_touch(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: &Living) {
        player_push(e, m, level, player);
    }

    /// `doPush`: the hot ones burn what they bump.
    fn do_push_mut(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, other: i32) {
        apply_contact_damage(e, m, level, other);
    }

    fn on_killed_removal(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        slime::on_killed_removal(e, m, level);
    }

    /// `hurtServer`: fire and blasts light the fuse; most damage only knocks a ball about.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        if !has_body(m) {
            return None;
        }
        if can_explode(e, m) && !is_primed(m) {
            let burning_arrow = source.direct.and_then(|d| level.entity(d)).is_some_and(|d| matches!(d.kind, EntityKind::Arrow(_)) && d.is_on_fire());
            if source.kind.is_tag("minecraft:is_fire") || burning_arrow {
                prime_time(e, m, level, false);
            } else if source.kind.is_tag("minecraft:is_explosion") {
                prime_time(e, m, level, true);
            }
        }
        if source.kind.is_tag("minecraft:sulfur_cube_with_block_immune_to") {
            if !source.kind.is_tag("minecraft:no_knockback") {
                mob::deal_default_knockback(e, m, level, source, amount);
            }
            return Some(true);
        }
        None
    }

    /// `knockback(..)`: the hit knocks a ball about by where and how hard it came.
    fn hit_knockback(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, strength: f64, dx: f64, dz: f64, source: &DamageSource, amount: f32) -> bool {
        hit_knockback(e, m, level, strength, dx, dz, source, amount, false)
    }

    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(if is_tiny(m) { "minecraft:entity.small_sulfur_cube.hurt" } else { "minecraft:entity.sulfur_cube.hurt" })
    }

    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        Some(if is_tiny(m) { "minecraft:entity.small_sulfur_cube.death" } else { "minecraft:entity.sulfur_cube.death" })
    }

    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        let _ = (m, kind);
        false
    }

    /// `canBreatheUnderwater`: a ball does; the cube itself drowns as slimes do.
    fn breathes_under_water_now(&self, m: &MobData) -> Option<bool> {
        has_body(m).then_some(true)
    }

    /// `getFluidJumpThreshold`: a fifth of its height.
    fn fluid_jump_threshold(&self, e: &Entity) -> Option<f64> {
        Some(e.height as f64 * 0.2)
    }

    /// `omnidirectionalAirMover`.
    fn omnidirectional_air_mover_now(&self, m: &MobData) -> bool {
        has_body(m)
    }

    /// `travelInFluid`: a floating ball bobs up.
    fn after_travel_in_fluid(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if !has_body(m) || !sul(m).floats_in_liquids {
            return;
        }
        let wobble = (0.2f32 * mth::sin(((e.tick_count as f32) * 0.4) as f64)) as f64;
        let height = if e.is_in_water() { e.fluid_height_water() } else { e.fluid_height_lava() };
        let threshold = e.height as f64 * 0.2;
        let up = height - threshold + wobble;
        if up > 0.0 {
            e.delta = e.delta.add(0.0, up.min(1.0) * 0.03999999910593033, 0.0);
        }
    }

    fn can_be_pushed(&self, m: &MobData) -> bool {
        let _ = m;
        true
    }

    /// `AgeableMob.ageBoundaryReached`: a grown cube is size 2.
    fn age_boundary_reached(&self, e: &mut Entity, m: &mut MobData) {
        if !m.baby() {
            set_size(e, m, 2, true);
        }
    }

    fn allowed_in_peaceful(&self) -> Option<bool> {
        Some(true)
    }

    /// `requiresCustomPersistence`: a ball or a bucketed cube stays.
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        (has_body(m) || sul(m).from_bucket).then_some(false)
    }

    fn set_extra_equipment(&self, m: &mut MobData, slot: u8, stack: ItemStack) -> bool {
        if slot != 6 {
            return false;
        }
        set_body(m, stack);
        true
    }

    fn remove_extra_equipment(&self, m: &mut MobData, slot: u8) -> Option<ItemStack> {
        (slot == 6).then(|| std::mem::take(&mut sul_mut(m).body))
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = sul(m);
        if s.body.is_empty() { Vec::new() } else { vec![(6, s.body.clone())] }
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = sul_mut(m);
        vec![(std::mem::take(&mut s.body), s.body_drop)]
    }

    /// `getBaseExperienceReward`: a baby none, else one or two.
    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        Some(if m.baby() { 0 } else { 1 + e.random.next_int_bounded(2) })
    }

    fn spawn_ignores_light(&self) -> bool {
        true
    }

    /// `checkSulfurCubeSpawnRules`: anywhere.
    fn check_spawn_rules(&self, _view: &dyn SpawnView, _pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(true)
    }

    /// `AgeableMob.finalizeSpawn` (5 percent babies after the first of a group), `Mob.finalizeSpawn`
    /// and `setSpawnSize`: a baby is size 1, the others 2.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        ext::ageable_finalize(e, m, r, group, 0.05);
        ext::mob_finalize(m, r);
        let size = if m.baby() { 1 } else { 2 };
        set_size(e, m, size, true);
    }

    /// `mobInteract`.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let name = if stack.is_empty() { "" } else { mob::item_name(stack) };
        if m.baby() {
            if !stack.is_empty() && mob::item_tag(stack.item(), "minecraft:sulfur_cube_food") && m.age < 0 && !m.age_locked {
                let age = m.age;
                mob::age_up(e, m, mob::breed::speed_up_seconds_when_feeding(-age), true);
                mob::make_sound(e, m, level, "minecraft:entity.small_sulfur_cube.eat");
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            return None;
        }
        if is_primed(m) {
            return Some(Outcome::PASS);
        }
        if can_explode(e, m) && matches!(name, "minecraft:flint_and_steel" | "minecraft:fire_charge") {
            if !level.tnt_explodes() {
                return Some(Outcome::PASS);
            }
            prime_time(e, m, level, false);
            let held = if name == "minecraft:flint_and_steel" { HeldChange::Damage(1) } else { HeldChange::Consume(1) };
            return Some(Outcome::success(held));
        }
        if name == "minecraft:shears" && has_body(m) {
            // `shear`: the item pops out above the cube and the cube does not take another for a while.
            let item = std::mem::take(&mut sul_mut(m).body);
            mob::spawn_at_location_offset(e, level, item, e.height);
            sound(e, level, "minecraft:entity.sulfur_cube.eject", 1.0, 1.0);
            sul_mut(m).pickup_timer = PICKUP_TIMER_DURATION;
            level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        if !stack.is_empty() && swallowable(stack.item()) {
            // `equipItem`.
            if has_body(m) {
                if sul(m).body.is_same_item(stack) {
                    return Some(Outcome::PASS);
                }
                let old = std::mem::take(&mut sul_mut(m).body);
                mob::spawn_at_location_offset(e, level, old, e.height);
            }
            let mut one = stack.clone();
            one.set_count(1);
            set_body(m, one);
            sound(e, level, "minecraft:entity.sulfur_cube.absorb", 1.0, 1.0);
            level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
            let _ = MAINHAND;
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        None
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let pickup = r.int_or("pickup_timer", 0);
        let from_bucket = r.bool_or("from_bucket", false);
        let fuse = r.int_or("fuse", -1);
        let body = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "body").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let body_drop = match r.get("drop_chances") {
            Some(Tag::Compound(dc)) => dc.iter().find(|(k, _)| k == "body").and_then(|(_, v)| v.as_f64()).map(|f| f as f32),
            _ => None,
        };
        slime::load(e, m, r);
        let s = sul_mut(m);
        s.pickup_timer = pickup;
        s.from_bucket = from_bucket;
        s.fuse = fuse;
        if let Some(b) = body {
            s.body = b;
        }
        if let Some(d) = body_drop {
            s.body_drop = d;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        slime::save(m, o);
        let s = sul(m);
        o.put("pickup_timer", Tag::Int(s.pickup_timer));
        o.put("from_bucket", Tag::Byte(s.from_bucket as i8));
        o.put("fuse", Tag::Int(s.fuse));
        if !s.body.is_empty() {
            let entry = ("body".to_owned(), s.body.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        if s.body_drop != 0.085 {
            let entry = ("body".to_owned(), Tag::Float(s.body_drop));
            match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                Some((_, Tag::Compound(dc))) => dc.push(entry),
                _ => o.put("drop_chances", Tag::Compound(vec![entry])),
            }
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::sulfur_cube;
        slime::entity_data(m, d);
        let s = sul(m);
        d.set(sulfur_cube::MAX_FUSE, &DataValue::Int(s.fuse));
        d.set(sulfur_cube::FROM_BUCKET, &DataValue::Boolean(s.from_bucket));
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        slime::dimensions(m, base)
    }
}

#[allow(dead_code)]
fn unused(_: Op, _: &CubeFloat, _: &CubeKeepOnJumping, _: &CubeRandomDirection) {}
