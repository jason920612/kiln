//! Shulker: sits attached to a block face, peeks, opens to shoot homing bullets, teleports
//! when hurt or when its block goes away.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::attributes::Op;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, LOOK, MOVE, TARGET};
use crate::mob::{self, mth, Category, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Shulker;

pub static KIND: Shulker = Shulker;

static INFO: Info = Info {
    category: Category::Monster,
    head: (180, 180, 10),
    ambient_interval: 120,
    sound_source: "hostile",
    ..Info::misc("minecraft:shulker", &[(MaxHealth, 30.0)])
};

/// `COVERED_ARMOR_MODIFIER`.
const COVERED: &str = "minecraft:covered";
/// `DATA_COLOR_ID` for no color.
pub const NO_COLOR: i8 = 16;

#[derive(Clone, Debug)]
pub struct ShulkerState {
    /// `DATA_ATTACH_FACE_ID`, `DATA_PEEK_ID` (0 to 100) and `DATA_COLOR_ID`.
    pub attach: Direction,
    pub peek: i32,
    pub color: i8,
    /// `currentPeekAmount` and its previous value.
    pub cur_peek: f32,
    pub cur_peek_o: f32,
}

pub fn st(m: &MobData) -> &ShulkerState {
    ext::state::<ShulkerState>(m).expect("shulker state")
}

fn st_mut(m: &mut MobData) -> &mut ShulkerState {
    ext::state_mut::<ShulkerState>(m).expect("shulker state")
}

impl Kind for Shulker {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(ShulkerState { attach: Direction::Down, peek: 0, color: NO_COLOR, cur_peek: 0.0, cur_peek_o: 0.0 }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(LookAtPlayerHorizontal { look_at: None, look_time: 0 })));
        g.add(4, Goal::Custom(Box::new(AttackGoal { attack_time: 0 })));
        g.add(7, Goal::Custom(Box::new(PeekGoal { peek_time: 0 })));
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(NearestAttackGoal { target: None, unseen: 0 })));
        t.add(3, Goal::Custom(Box::new(DefenseAttackGoal)));
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        // `Shulker.setPos` snaps to the block (vanilla does it when the mob is placed).
        let p = e.position();
        let snapped = snap(p);
        if snapped != p {
            e.set_pos(snapped);
            e.set_old_pos_and_rot();
            update_bb(e, m);
        }
    }

    /// `getDeltaMovement` is always zero and `setDeltaMovement` does nothing: the shulker only
    /// makes the zero move of `travel` (which leaves it not on the ground).
    fn travel(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, _input: Vec3) -> bool {
        e.delta = Vec3::ZERO;
        e.do_move(level, MoverType::SelfMove, Vec3::ZERO);
        e.delta = Vec3::ZERO;
        true
    }

    fn tick_body(&self, _e: &mut Entity, _m: &mut MobData) -> bool {
        true
    }

    fn tick_look(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        e.x_rot = 0.0;
        if m.look.cooldown > 0 {
            m.look.cooldown -= 1;
            if let Some(y) = look_y_rot(e, m) {
                m.y_head_rot = mth::rotate_towards(m.y_head_rot, y, m.look.y_max_rot_speed);
            }
            e.x_rot = mth::rotate_towards(e.x_rot, 0.0, m.look.x_max_rot_angle);
        } else {
            m.y_head_rot = mth::rotate_towards(m.y_head_rot, m.y_body_rot, 10.0);
        }
        true
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        e.delta = Vec3::ZERO;
        if !can_stay_at(e, m, level, e.block_position(), st(m).attach) {
            // `findNewAttachment`.
            match find_attachable_surface(e, m, level, e.block_position()) {
                Some(d) => {
                    st_mut(m).attach = d;
                    update_bb(e, m);
                }
                None => {
                    teleport_somewhere(e, m, level);
                }
            }
        }
        if update_peek_amount(m) {
            on_peek_amount_change(e, m, level);
        }
    }

    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        // The projectile dealing the damage is mid-tick and out of the level's reach: arrows and
        // shulker bullets are told apart by their damage types.
        let direct = source.direct.or(source.attacker);
        let direct_type = direct.and_then(|id| level.entity(id)).map(|d| d.type_name);
        if st(m).peek == 0 && (source.kind == DamageKind::Arrow || direct_type.is_some_and(|t| matches!(t, "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident"))) {
            return Some(false);
        }
        let bullet = direct_type == Some("minecraft:shulker_bullet") || (source.kind == DamageKind::MobProjectile && direct_type.is_none());
        let r = mob::hurt_base(e, m, level, *source, amount);
        e.delta = Vec3::ZERO;
        if r {
            if (m.health as f64) < m.max_health() as f64 * 0.5 && e.random.next_int_bounded(4) == 0 {
                teleport_somewhere(e, m, level);
            } else if source.kind.is_tag("minecraft:is_projectile") && bullet {
                hit_by_shulker_bullet(e, m, level);
            }
        }
        Some(r)
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, _group: &mut GroupData) {
        e.y_rot = 0.0;
        m.y_head_rot = 0.0;
        e.set_old_pos_and_rot();
        ext::mob_finalize(m, r);
    }

    fn ambient_sound(&self, m: &MobData, default: Option<&'static str>) -> Option<&'static str> {
        // Closed shulkers keep quiet.
        default.filter(|_| st(m).peek != 0)
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        // `AbstractGolem.removeWhenFarAway`.
        Some(false)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        let s = st(m);
        if s.attach == Direction::Down && s.cur_peek > 0.0 {
            let h = 1.0 + s.cur_peek;
            return (base.0, base.1 * h, base.2 * h);
        }
        base
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let face = r.num("AttachFace").map(|v| v as i64 as i32).and_then(|i| Direction::ALL.get(i as usize).copied()).unwrap_or(Direction::Down);
        let peek = r.byte_or("Peek", 0) as i32;
        let color = r.byte_or("Color", NO_COLOR);
        let s = st_mut(m);
        s.attach = face;
        s.peek = peek;
        s.color = color;
        update_bb(e, m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("AttachFace", Tag::Byte(s.attach.index() as i8));
        o.put("Peek", Tag::Byte(s.peek as i8));
        o.put("Color", Tag::Byte(s.color));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::shulker as f;
        use kiln_proto::packets::entity::metadata::Direction as D;
        let s = st(m);
        let dir = match s.attach {
            Direction::Down => D::Down,
            Direction::Up => D::Up,
            Direction::North => D::North,
            Direction::South => D::South,
            Direction::West => D::West,
            Direction::East => D::East,
        };
        d.set(f::ATTACH_FACE, &DataValue::Direction(dir));
        d.set(f::PEEK, &DataValue::Byte(s.peek as i8));
        d.set(f::COLOR, &DataValue::Byte(s.color));
    }
}

// ---------------------------------------------------------------------- shape and peeking

fn snap(p: Vec3) -> Vec3 {
    Vec3::new(crate::math::floor(p.x) as f64 + 0.5, crate::math::floor(p.y + 0.5) as f64, crate::math::floor(p.z) as f64 + 0.5)
}

fn scale(m: &MobData) -> f32 {
    (m.attrs.value(Attr::Scale) as f32).min(3.0)
}

/// `getPhysicalPeek`.
fn physical_peek(amount: f32) -> f32 {
    0.5 - mth::sin(((0.5 + amount) * 3.1415927) as f64) * 0.5
}

/// `AABB.contract`.
fn contract(b: &Aabb, x: f64, y: f64, z: f64) -> Aabb {
    let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) = (b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z);
    if x < 0.0 {
        x0 -= x;
    } else if x > 0.0 {
        x1 -= x;
    }
    if y < 0.0 {
        y0 -= y;
    } else if y > 0.0 {
        y1 -= y;
    }
    if z < 0.0 {
        z0 -= z;
    } else if z > 0.0 {
        z1 -= z;
    }
    Aabb::new(x0, y0, z0, x1, y1, z1)
}

/// `getProgressDeltaAabb`.
pub fn progress_delta_aabb(scale: f32, dir: Direction, from: f32, to: f32, pos: Vec3) -> Aabb {
    let s = scale as f64;
    let b = Aabb::new(-s * 0.5, 0.0, -s * 0.5, s * 0.5, s, s * 0.5);
    let max = from.max(to) as f64;
    let min = from.min(to) as f64;
    let (dx, dy, dz) = dir.step();
    let b = b.expand_towards(dx as f64 * max * s, dy as f64 * max * s, dz as f64 * max * s);
    let b = contract(&b, -dx as f64 * (1.0 + min) * s, -dy as f64 * (1.0 + min) * s, -dz as f64 * (1.0 + min) * s);
    b.offset(pos.x, pos.y, pos.z)
}

/// `getProgressAabb`.
pub fn progress_aabb(scale: f32, dir: Direction, progress: f32, pos: Vec3) -> Aabb {
    progress_delta_aabb(scale, dir, -1.0, progress, pos)
}

/// `makeBoundingBox`: the shell opened by the current peek, away from the attached face.
fn update_bb(e: &mut Entity, m: &MobData) {
    let s = st(m);
    let bb = progress_aabb(scale(m), s.attach.opposite(), physical_peek(s.cur_peek), e.position());
    e.set_bounding_box(bb);
}

/// `updatePeekAmount`.
fn update_peek_amount(m: &mut MobData) -> bool {
    let s = st_mut(m);
    s.cur_peek_o = s.cur_peek;
    let target = s.peek as f32 * 0.01;
    if s.cur_peek == target {
        return false;
    }
    s.cur_peek = if s.cur_peek > target { mth::clamp(s.cur_peek - 0.05, target, 1.0) } else { mth::clamp(s.cur_peek + 0.05, 0.0, target) };
    true
}

/// `onPeekAmountChange`: the new size, and entities in the way pushed out.
fn on_peek_amount_change(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    // `refreshDimensions` (the eyes rise with the lid on a floor), then the shell box.
    if let Some(t) = kiln_data::entities::by_name(e.type_name) {
        let (w, h, eye) = KIND.dimensions(m, (t.width, t.height, t.eye_height));
        let sc = scale(m);
        e.width = w * sc;
        e.height = h * sc;
        e.eye_height = eye * sc;
    }
    update_bb(e, m);
    let s = st(m);
    let cur = physical_peek(s.cur_peek);
    let old = physical_peek(s.cur_peek_o);
    let opp = s.attach.opposite();
    let delta = (cur - old) * scale(m);
    if delta <= 0.0 {
        return;
    }
    let area = progress_delta_aabb(scale(m), opp, old, cur, e.position());
    let (dx, dy, dz) = opp.step();
    let v = Vec3::new((delta * dx as f32) as f64, (delta * dy as f32) as f64, (delta * dz as f32) as f64);
    // Approximation: players are not pushed (their movement is their client's).
    for id in level.entities_in(&area, EntityFilter::Any, e.id) {
        let Some(o) = level.entity_mut(id) else { continue };
        if o.type_name == "minecraft:shulker" || o.no_physics || matches!(o.kind, crate::entity::EntityKind::Other { .. }) {
            continue;
        }
        let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", 0, 0, crate::entity::EntityKind::Other { type_name: "minecraft:marker" }, 0));
        o2.do_move(level, MoverType::Shulker, v);
        if let Some(slot) = level.entity_mut(id) {
            *slot = o2;
        }
    }
}

/// `setRawPeekAmount`.
fn set_raw_peek(e: &Entity, m: &mut MobData, level: &mut dyn EntityLevel, amount: i32) {
    m.attrs.remove_modifier(Attr::Armor, COVERED);
    let (sound, event) = if amount == 0 {
        m.attrs.set_modifier(Attr::Armor, COVERED, 20.0, Op::AddValue);
        ("minecraft:entity.shulker.close", "minecraft:container_close")
    } else {
        ("minecraft:entity.shulker.open", "minecraft:container_open")
    };
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "hostile", volume: 1.0, pitch: 1.0 });
    }
    level.emit(Event::GameEvent { event, pos: e.position(), entity: Some(e.id) });
    st_mut(m).peek = amount;
}

// ---------------------------------------------------------------------- attaching and teleporting

/// `isPositionBlocked`.
fn position_blocked(e: &Entity, level: &dyn EntityLevel, pos: BlockPos) -> bool {
    let s = level.block(pos);
    if kiln_data::blocks_types::is_air(s) {
        return false;
    }
    !(crate::blocks::kind(s) == crate::blocks::Kind::MovingPiston && pos == e.block_position())
}

/// `canStayAt`: the neighbour's face is full and the open shell fits.
fn can_stay_at(e: &Entity, m: &MobData, level: &dyn EntityLevel, pos: BlockPos, face: Direction) -> bool {
    if position_blocked(e, level, pos) {
        return false;
    }
    let opp = face.opposite();
    let n = pos.relative(face);
    if !level.is_loaded(n) || !crate::physics::is_face_sturdy(level.block(n), opp) {
        return false;
    }
    let b = progress_aabb(scale(m), opp, 1.0, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5)).deflate_all(1.0e-6);
    crate::collision::no_collision(level, &e.collision_context(), e.id, &b)
}

/// `findAttachableSurface`.
fn find_attachable_surface(e: &Entity, m: &MobData, level: &dyn EntityLevel, pos: BlockPos) -> Option<Direction> {
    Direction::ALL.into_iter().find(|&d| can_stay_at(e, m, level, pos, d))
}

fn shulker_does_not_teleport_to(state: u16) -> bool {
    static F: std::sync::OnceLock<Vec<bool>> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let mut out = vec![false; kiln_data::blocks::STATE_COUNT as usize];
        let ids = kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:block")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == "minecraft:shulker_does_not_teleport_to"))
            .map_or(&[][..], |(_, ids)| *ids);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for &id in ids {
            if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                out[info.first as usize..=info.last as usize].fill(true);
            }
        }
        out
    })[state as usize]
}

/// `teleportSomewhere`: up to five tries within 8 blocks. Approximation: no world border.
fn teleport_somewhere(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
    if m.no_ai || !mob::is_alive(e, m) {
        return false;
    }
    let old = e.block_position();
    for _ in 0..5 {
        let dx = mth::next_int_between(&mut e.random, -8, 8);
        let dy = mth::next_int_between(&mut e.random, -8, 8);
        let dz = mth::next_int_between(&mut e.random, -8, 8);
        let t = old.offset(dx, dy, dz);
        if t.y > level.min_y()
            && kiln_data::blocks_types::is_air(level.block(t))
            && crate::collision::no_collision(level, &e.collision_context(), e.id, &Aabb::of_block(t).deflate_all(1.0e-6))
            && let Some(face) = find_attachable_surface(e, m, level, t)
            && !shulker_does_not_teleport_to(level.block(t.relative(face)))
        {
            st_mut(m).attach = face;
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.shulker.teleport", source: "hostile", volume: 1.0, pitch: 1.0 });
            }
            e.set_pos(Vec3::new(t.x as f64 + 0.5, t.y as f64, t.z as f64 + 0.5));
            st_mut(m).peek = 0;
            update_bb(e, m);
            e.needs_sync = true;
            level.emit(Event::GameEvent { event: "minecraft:teleport", pos: Vec3::new(old.x as f64, old.y as f64, old.z as f64), entity: Some(e.id) });
            let to = e.block_position();
            let packed = ((to.x - old.x + 8) & 255) << 16 | ((to.y - old.y + 8) & 255) << 8 | ((to.z - old.z + 8) & 255);
            level.emit(Event::LevelEvent { event: 2016, pos: old, data: packed });
            m.target = None;
            return true;
        }
    }
    false
}

/// `hitByShulkerBullet`: an open shulker hit by a bullet may teleport and leave a copy behind,
/// less often the more shulkers are around.
fn hit_by_shulker_bullet(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let old = e.position();
    let old_box = e.bounding_box();
    if st(m).peek == 0 || !teleport_somewhere(e, m, level) {
        return;
    }
    let area = old_box.inflate_all(8.0);
    let count = level.entities_in(&area, EntityFilter::Living, i32::MIN).into_iter().filter(|&id| level.entity(id).is_some_and(|o| o.type_name == "minecraft:shulker" && o.is_alive())).count()
        + usize::from(e.bounding_box().intersects(&area));
    let chance = (count as f32 - 1.0) / 5.0;
    if level.random().next_float() < chance {
        return;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut child = mob::new(MobKind::Shulker, id, 0, seed);
    child.set_pos(old);
    child.set_old_pos_and_rot();
    let color = st(m).color;
    if let Some(cm) = mob::data_mut(&mut child) {
        st_mut(cm).color = color;
    }
    level.add_entity(child);
}

// ---------------------------------------------------------------------- looking

/// The forward and side axes of `ShulkerLookControl` for the face opposite the attachment
/// (`Direction.getRotation` applied to south; rounded to whole components).
fn look_axes(opp: Direction) -> ([f32; 3], [f32; 3]) {
    let fwd: [f32; 3] = match opp {
        Direction::Up => [0.0, 0.0, 1.0],
        Direction::Down => [0.0, 0.0, -1.0],
        _ => [0.0, -1.0, 0.0],
    };
    let (nx, ny, nz) = opp.step();
    let n = [nx as f32, ny as f32, nz as f32];
    let side = [n[1] * fwd[2] - n[2] * fwd[1], n[2] * fwd[0] - n[0] * fwd[2], n[0] * fwd[1] - n[1] * fwd[0]];
    (fwd, side)
}

/// `ShulkerLookControl.getYRotD`: the yaw toward the wanted point in the shulker's own frame.
fn look_y_rot(e: &Entity, m: &MobData) -> Option<f32> {
    let (fwd, side) = look_axes(st(m).attach.opposite());
    let [wx, wy, wz] = m.look.wanted;
    let to = [(wx - e.x()) as f32, (wy - e.eye_y()) as f32, (wz - e.z()) as f32];
    let dot = |a: [f32; 3]| a[0] * to[0] + (a[1] * to[1] + a[2] * to[2]);
    let a = dot(side);
    let b = dot(fwd);
    if a.abs() <= 1.0e-5 && b.abs() <= 1.0e-5 {
        return None;
    }
    Some((mth::atan2(-a as f64, b as f64) * 57.2957763671875) as f32)
}

// ---------------------------------------------------------------------- goals

/// `LookAtPlayerGoal(this, Player.class, 8, 0.02, true)`: looks at the nearest player at its
/// own eye height.
#[derive(Clone, Debug)]
struct LookAtPlayerHorizontal {
    look_at: Option<i32>,
    look_time: i32,
}

impl CustomGoal for LookAtPlayerHorizontal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LookAtPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_float() >= 0.02 {
            return false;
        }
        self.look_at = goals::nearest_player(e, m, level, false, 8.0, true, |_| true).map(|p| p.id);
        self.look_at.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return false };
        t.alive && e.position().distance_to_sqr(t.pos) <= 64.0 && self.look_time > 0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time = mth::reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_at = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.look_at.and_then(|id| goals::living(level, id)) else { return };
        if !t.alive {
            return;
        }
        crate::mob::control::look_at(m, t.pos.x, e.eye_y(), t.pos.z);
        self.look_time -= 1;
    }
}

/// `ShulkerAttackGoal`: opens and shoots a bullet every 1 to 5.5 seconds at a target within 20
/// blocks.
#[derive(Clone, Debug)]
struct AttackGoal {
    attack_time: i32,
}

impl CustomGoal for AttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ShulkerAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_some_and(|t| t.alive) && level.difficulty() != 0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.attack_time = 20;
        set_raw_peek(e, m, level, 100);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        set_raw_peek(e, m, level, 0);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if level.difficulty() == 0 {
            return;
        }
        self.attack_time -= 1;
        let Some(t) = goals::target(m, level) else { return };
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 180.0, 180.0);
        if e.position().distance_to_sqr(t.pos) < 400.0 {
            if self.attack_time <= 0 {
                self.attack_time = 20 + e.random.next_int_bounded(10) * 20 / 2;
                let axis = st(m).attach.axis();
                let id = level.next_entity_id();
                let seed = level.fresh_seed();
                let bullet = crate::ext_entity::shulker_bullet::new(id, e, t.id, axis, level, seed);
                level.add_entity(bullet);
                let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.shulker.shoot", source: "hostile", volume: 2.0, pitch });
                }
            }
        } else {
            m.target = None;
        }
    }
}

/// `ShulkerPeekGoal`: without a target, now and then opens a little for 1 to 3 seconds.
#[derive(Clone, Debug)]
struct PeekGoal {
    peek_time: i32,
}

impl CustomGoal for PeekGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ShulkerPeekGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_none()
            && e.random.next_int_bounded(mth::reduced_tick_delay(40)) == 0
            && can_stay_at(e, m, level, e.block_position(), st(m).attach)
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::target(m, level).is_none() && self.peek_time > 0
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.peek_time = mth::reduced_tick_delay(20 * (1 + e.random.next_int_bounded(3)));
        set_raw_peek(e, m, level, 30);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if goals::target(m, level).is_none() {
            set_raw_peek(e, m, level, 0);
        }
    }
    fn tick(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.peek_time -= 1;
    }
}

/// `ShulkerNearestAttackGoal`: a `NearestAttackableTargetGoal<Player>` that must see, off in
/// peaceful.
#[derive(Clone, Debug)]
struct NearestAttackGoal {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for NearestAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ShulkerNearestAttackGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if level.difficulty() == 0 {
            return false;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(10)) != 0 {
            return false;
        }
        let range = m.attrs.value(FollowRange);
        self.target = goals::nearest_player(e, m, level, true, range, true, |_| true).map(|p| p.id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, None, true, &mut self.unseen, 60)
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = self.target;
        self.unseen = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.target = None;
        self.target = None;
    }
}

/// `ShulkerDefenseAttackGoal`: only for shulkers on a team (Kiln has no teams).
#[derive(Clone, Debug)]
struct DefenseAttackGoal;

impl CustomGoal for DefenseAttackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ShulkerDefenseAttackGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
}
