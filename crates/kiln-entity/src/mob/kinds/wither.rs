//! The wither (`WitherBoss`): a flying boss (`FlyingMoveControl`, `FlyingPathNavigation`) with
//! three heads. Built from soul sand and wither skeleton skulls it starts invulnerable for 220
//! ticks, regenerating, and ends that with a power 7 explosion. The middle head fires wither
//! skulls at its target (`RangedAttackGoal`), the side heads pick their own targets around it
//! and fire at them, now and then at random blocks (the dangerous blue skulls). Below half
//! health it is powered: arrows no longer hurt it. When hurt it breaks the blocks around its
//! body the next second; it heals 1 a second, never despawns and drops a nether star.

use crate::entity::Entity;
use crate::ext_entity::{fireball, wither_skull};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::control::{self, Operation};
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, state, state_mut};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE, TARGET};
use crate::mob::{self, DamageSource, MobData, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Wither;

pub static KIND: Wither = Wither;

static INFO: Info = Info {
    fire_immune: true,
    sounds: Some("wither"),
    ..Info::monster(
        "minecraft:wither",
        &[(MaxHealth, 300.0), (MovementSpeed, 0.6000000238418579), (FlyingSpeed, 0.6000000238418579), (FollowRange, 40.0), (Armor, 4.0)],
    )
};

/// `INVULNERABLE_TICKS`.
pub const INVULNERABLE_TICKS: i32 = 220;

#[derive(Clone, Debug, Default)]
pub struct WitherState {
    /// `DATA_ID_INV`.
    pub invul: i32,
    /// `DATA_TARGET_A/B/C`: the entity each head aims at (0: none).
    pub targets: [i32; 3],
    pub x_rot_heads: [f32; 2],
    pub y_rot_heads: [f32; 2],
    pub x_rot_o_heads: [f32; 2],
    pub y_rot_o_heads: [f32; 2],
    pub next_head_update: [i32; 2],
    pub idle_head_updates: [i32; 2],
    pub destroy_blocks_tick: i32,
    /// The boss bar's fill (`bossEvent.getProgress`).
    pub boss_progress: f32,
    /// Built from blocks: `SummonedEntityTrigger` fires for players nearby on the first tick.
    pub summoned: bool,
}

fn st(m: &MobData) -> &WitherState {
    state::<WitherState>(m).expect("wither state")
}

fn st_mut(m: &mut MobData) -> &mut WitherState {
    state_mut::<WitherState>(m).expect("wither state")
}

/// `makeInvulnerable`: 220 ticks, the boss bar empty and a third of the health (a wither built
/// from blocks). `summoned` makes the summon trigger fire on its first tick.
pub fn make_invulnerable(m: &mut MobData, summoned: bool) {
    let max = m.max_health();
    let s = st_mut(m);
    s.invul = INVULNERABLE_TICKS;
    s.boss_progress = 0.0;
    s.summoned = summoned;
    m.set_health(max / 3.0);
}

/// The boss bar's fill for a wither (`None` for other mobs).
pub fn boss_bar(m: &MobData) -> Option<f32> {
    state::<WitherState>(m).map(|s| s.boss_progress)
}

/// `isPowered`: at half health or less.
pub fn is_powered(m: &MobData) -> bool {
    m.health <= m.max_health() / 2.0
}

/// `LivingEntity.heal`.
fn heal(m: &mut MobData, amount: f32) {
    if m.health > 0.0 {
        let h = m.health + amount;
        m.set_health(h);
    }
}

/// `Entity.setXRot`: wrapped to 360 and clamped to +-90.
fn set_x_rot(e: &mut Entity, v: f32) {
    e.x_rot = mth::clamp(v % 360.0, -90.0, 90.0);
}

/// `180.0F / (float)Math.PI`.
const DEG_F: f32 = 180.0 / std::f32::consts::PI;
/// `180.0F / (float)Math.PI` (javac folds it to one float constant) as a double.
const DEG_D: f64 = 57.2957763671875;

/// `WitherBoss.rotlerp`.
fn rotlerp(a: f32, b: f32, max: f32) -> f32 {
    let d = mth::wrap_degrees(b - a).clamp(-max, max);
    a + d
}

fn scale(m: &MobData) -> f32 {
    m.attrs.value(Attr::Scale) as f32
}

/// `getHeadX(index)` (0: the middle head; 1, 2, 3: at the body's right, left, right again).
fn head_x(e: &Entity, m: &MobData, i: i32) -> f64 {
    if i <= 0 {
        return e.x();
    }
    let angle = (m.y_body_rot + (180 * (i - 1)) as f32) * (std::f64::consts::PI / 180.0) as f32;
    e.x() + mth::cos(angle as f64) as f64 * 1.3 * scale(m) as f64
}

fn head_y(e: &Entity, m: &MobData, i: i32) -> f64 {
    let h = if i <= 0 { 3.0f32 } else { 2.2 };
    e.y() + (h * scale(m)) as f64
}

fn head_z(e: &Entity, m: &MobData, i: i32) -> f64 {
    if i <= 0 {
        return e.z();
    }
    let angle = (m.y_body_rot + (180 * (i - 1)) as f32) * (std::f64::consts::PI / 180.0) as f32;
    e.z() + mth::sin(angle as f64) as f64 * 1.3 * scale(m) as f64
}

/// Position and eye height of entity `id` (a player or any entity).
fn entity_pos(level: &dyn EntityLevel, id: i32) -> Option<(Vec3, f64)> {
    if let Some(p) = level.player(id) {
        return Some((p.pos, p.pos.y + p.eye_height as f64));
    }
    level.entity(id).map(|o| (o.position(), o.eye_y()))
}

/// `performRangedAttack(head, x, y, z, dangerous)`: a wither skull from the head toward the point.
#[allow(clippy::too_many_arguments)]
fn shoot(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, head: i32, tx: f64, ty: f64, tz: f64, dangerous: bool) {
    if !e.silent {
        level.emit(Event::LevelEvent { event: 1024, pos: e.block_position(), data: 0 });
    }
    let (hx, hy, hz) = (head_x(e, m, head), head_y(e, m, head), head_z(e, m, head));
    let dir = Vec3::new(tx - hx, ty - hy, tz - hz).normalize();
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut skull = wither_skull::new(id, e, dir, dangerous, seed);
    skull.set_pos(Vec3::new(hx, hy, hz));
    level.add_entity(skull);
}

/// `performRangedAttack(head, target)`: at the middle of the target's eye height; the middle
/// head's skull is dangerous one time in a thousand.
fn shoot_at(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, head: i32, t: &Living) {
    let dangerous = head == 0 && e.random.next_float() < 0.001;
    let eye_height = t.eye_y - t.pos.y;
    shoot(e, m, level, head, t.pos.x, t.pos.y + eye_height * 0.5, t.pos.z, dangerous);
}

/// `LIVING_ENTITY_SELECTOR`: not a wither friend (undead); everything living is attackable.
fn selector(t: &Living) -> bool {
    !mob::entity_type_tag(t.type_name, "minecraft:wither_friends")
}

/// `FlyingMoveControl.tick` (max turn 10, not hovering in place: gravity back when idle).
fn flying_move(e: &mut Entity, m: &mut MobData, max_turn: f32, hovers: bool) {
    if m.mov.operation != Operation::MoveTo {
        if !hovers {
            e.no_gravity = false;
        }
        m.yya = 0.0;
        m.zza = 0.0;
        return;
    }
    m.mov.operation = Operation::Wait;
    e.no_gravity = true;
    let [wx, wy, wz] = m.mov.wanted;
    let (xd, yd, zd) = (wx - e.x(), wy - e.y(), wz - e.z());
    let dd = xd * xd + yd * yd + zd * zd;
    if dd < 2.500000277905201e-7 {
        m.yya = 0.0;
        m.zza = 0.0;
        return;
    }
    let y_rot_d = (mth::atan2(zd, xd) * DEG_D) as f32 - 90.0;
    e.y_rot = control::rotlerp(e.y_rot, y_rot_d, 90.0);
    let attr = if e.on_ground { Attr::MovementSpeed } else { Attr::FlyingSpeed };
    let speed = (m.mov.speed_modifier * m.attrs.value(attr)) as f32;
    control::set_speed(m, speed);
    let sd = (xd * xd + zd * zd).sqrt();
    if yd.abs() > 9.999999747378752e-6 || sd.abs() > 9.999999747378752e-6 {
        let x_rot_d = (-(mth::atan2(yd, sd) * DEG_D)) as f32;
        let x = control::rotlerp(e.x_rot, x_rot_d, max_turn);
        set_x_rot(e, x);
        m.yya = if yd > 0.0 { speed } else { -speed };
    }
}

impl Kind for Wither {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.fly = true;
        m.nav.can_float = true;
        m.nav.can_open_doors = false;
        Some(Box::new(WitherState::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(DoNothing)));
        g.add(2, Goal::Custom(Box::new(RangedAttack { target: None, attack_time: -1, see_time: 0, speed: 1.0, interval_min: 40, interval_max: 40, radius: 20.0 })));
        g.add(5, Goal::Custom(Box::new(RandomFlying { wanted: Vec3::ZERO, speed: 1.0, interval: 120, force: false })));
        g.add(6, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(7, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(NearestLiving { target: None, unseen: 0 })));
    }

    /// `WitherBoss.aiStep` before `super.aiStep()`: rises toward the middle head's target and
    /// closes in on it, facing where it flies.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let mut dm = e.delta.multiply(1.0, 0.6, 1.0);
        let t0 = st(m).targets[0];
        if t0 > 0
            && let Some((tp, _)) = entity_pos(level, t0)
        {
            let mut yd = dm.y;
            if e.y() < tp.y || (!is_powered(m) && e.y() < tp.y + 5.0) {
                yd = yd.max(0.0);
                yd += 0.3 - yd * 0.6000000238418579;
            }
            dm = Vec3::new(dm.x, yd, dm.z);
            let delta = Vec3::new(tp.x - e.x(), 0.0, tp.z - e.z());
            if delta.x * delta.x + delta.z * delta.z > 9.0 {
                let s = delta.normalize();
                dm = dm.add(s.x * 0.3 - dm.x * 0.6, 0.0, s.z * 0.3 - dm.z * 0.6);
            }
        }
        e.delta = dm;
        if dm.x * dm.x + dm.z * dm.z > 0.05 {
            e.y_rot = (mth::atan2(dm.z, dm.x) as f32) * DEG_F - 90.0;
        }
    }

    /// `WitherBoss.aiStep` after `super.aiStep()`: the side heads turn toward their targets;
    /// the smoke (and effect) particles still draw from the random.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        {
            let s = st_mut(m);
            s.y_rot_o_heads = s.y_rot_heads;
            s.x_rot_o_heads = s.x_rot_heads;
        }
        for i in 0..2 {
            let id = st(m).targets[i + 1];
            let target = if id > 0 { entity_pos(level, id) } else { None };
            match target {
                Some((tp, eye)) => {
                    let (hx, hy, hz) = (head_x(e, m, i as i32 + 1), head_y(e, m, i as i32 + 1), head_z(e, m, i as i32 + 1));
                    let (xd, yd, zd) = (tp.x - hx, eye - hy, tp.z - hz);
                    let sd = (xd * xd + zd * zd).sqrt();
                    let y_rot_d = (mth::atan2(zd, xd) * DEG_D) as f32 - 90.0;
                    let x_rot_d = (-(mth::atan2(yd, sd) * DEG_D)) as f32;
                    let s = st_mut(m);
                    s.x_rot_heads[i] = rotlerp(s.x_rot_heads[i], x_rot_d, 40.0);
                    s.y_rot_heads[i] = rotlerp(s.y_rot_heads[i], y_rot_d, 10.0);
                }
                None => {
                    let body = m.y_body_rot;
                    let s = st_mut(m);
                    s.y_rot_heads[i] = rotlerp(s.y_rot_heads[i], body, 10.0);
                }
            }
        }
        let powered = is_powered(m);
        for _ in 0..3 {
            // `addParticle(SMOKE, head + gaussian * radius, ...)`.
            for _ in 0..3 {
                e.random.next_gaussian();
            }
            if powered && level.random().next_int_bounded(4) == 0 {
                for _ in 0..3 {
                    e.random.next_gaussian();
                }
            }
        }
        if st(m).invul > 0 {
            for _ in 0..3 {
                e.random.next_gaussian();
                e.random.next_float();
                e.random.next_gaussian();
            }
        }
    }

    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `SummonedEntityTrigger` for players within 50 blocks of the new wither.
        if std::mem::take(&mut st_mut(m).summoned) {
            let area = e.bounding_box().inflate_all(50.0);
            let seen = crate::level::Seen::of_mob(e, m);
            let near: Vec<i32> = level.players_in(&area).iter().map(|p| p.id).collect();
            for player in near {
                level.emit(Event::Criterion { player, criterion: crate::level::Criterion::SummonedEntity { entity: seen.clone() } });
            }
        }
    }

    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).invul > 0 {
            let n = st(m).invul - 1;
            st_mut(m).boss_progress = 1.0 - n as f32 / 220.0;
            if n <= 0 {
                let griefing = level.mob_griefing();
                let interaction = if griefing { crate::explosion::Interaction::DestroyWithDecay } else { crate::explosion::Interaction::Keep };
                crate::explosion::explode(level, Some(e.id), Vec3::new(e.x(), e.eye_y(), e.z()), 7.0, false, interaction);
                if !e.silent {
                    level.emit(Event::GlobalLevelEvent { event: 1023, pos: e.block_position(), data: 0 });
                }
            }
            st_mut(m).invul = n;
            if e.tick_count % 10 == 0 {
                heal(m, 10.0);
            }
            return;
        }
        for i in 1..3usize {
            if e.tick_count < st(m).next_head_update[i - 1] {
                continue;
            }
            st_mut(m).next_head_update[i - 1] = e.tick_count + 10 + e.random.next_int_bounded(10);
            if level.difficulty() >= 2 {
                let idle = st(m).idle_head_updates[i - 1];
                st_mut(m).idle_head_updates[i - 1] = idle + 1;
                if idle > 15 {
                    let p = e.position();
                    let xt = next_double(&mut e.random, p.x - 10.0, p.x + 10.0);
                    let yt = next_double(&mut e.random, p.y - 5.0, p.y + 5.0);
                    let zt = next_double(&mut e.random, p.z - 10.0, p.z + 10.0);
                    shoot(e, m, level, i as i32 + 1, xt, yt, zt, true);
                    st_mut(m).idle_head_updates[i - 1] = 0;
                }
            }
            let head_target = st(m).targets[i];
            if head_target > 0 {
                let current = goals::living(level, head_target);
                let ok = current.is_some_and(|t| {
                    goals::can_attack(m, level, &t) && e.position().distance_to_sqr(t.pos) <= 900.0 && mob::has_line_of_sight_cached(e, m, level, &t)
                });
                if ok {
                    let t = current.unwrap();
                    shoot_at(e, m, level, i as i32 + 1, &t);
                    st_mut(m).next_head_update[i - 1] = e.tick_count + 40 + e.random.next_int_bounded(20);
                    st_mut(m).idle_head_updates[i - 1] = 0;
                } else {
                    st_mut(m).targets[i] = 0;
                }
            } else {
                // `getNearbyEntities(LivingEntity, TARGETING_CONDITIONS, this, box)`.
                let area = e.bounding_box().inflate(20.0, 8.0, 20.0);
                let mut found = Vec::new();
                for id in level.entities_in(&area, EntityFilter::Living, i32::MIN) {
                    let Some(t) = goals::living(level, id) else { continue };
                    if selector(&t) && goals::targeting_ok(e, m, level, &t, true, 20.0, true) {
                        found.push(id);
                    }
                }
                if !found.is_empty() {
                    let k = e.random.next_int_bounded(found.len() as i32) as usize;
                    st_mut(m).targets[i] = found[k];
                }
            }
        }
        let target = goals::target(m, level).map_or(0, |t| t.id);
        st_mut(m).targets[0] = target;
        if st(m).destroy_blocks_tick > 0 {
            st_mut(m).destroy_blocks_tick -= 1;
            if st(m).destroy_blocks_tick == 0 && level.mob_griefing() {
                let w = crate::math::floor((e.width / 2.0 + 1.0) as f64);
                let h = crate::math::floor(e.height as f64);
                let b = e.block_position();
                let mut destroyed = false;
                // `BlockPos.betweenClosed`: x fastest, then y, then z.
                for z in b.z - w..=b.z + w {
                    for y in b.y..=b.y + h {
                        for x in b.x - w..=b.x + w {
                            let p = BlockPos::new(x, y, z);
                            if wither_skull::wither_can_destroy(level.block(p)) {
                                destroyed = level.destroy_block(p, true) || destroyed;
                            }
                        }
                    }
                }
                if destroyed {
                    level.emit(Event::LevelEvent { event: 1022, pos: e.block_position(), data: 0 });
                }
            }
        }
        if e.tick_count % 20 == 0 {
            heal(m, 1.0);
        }
        let progress = m.health / m.max_health();
        st_mut(m).boss_progress = progress;
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        flying_move(e, m, 10.0, false);
        true
    }

    /// `hurtServer`: immune to drowning, other withers and their skulls, anything while
    /// invulnerable, arrows while powered and wither friends; a hit makes it break blocks
    /// around itself in a second and stirs the side heads.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let type_of = |id: Option<i32>| id.and_then(|a| if level.player(a).is_some() { Some("minecraft:player") } else { level.entity(a).map(|o| o.type_name) });
        let attacker = type_of(source.attacker);
        if source.kind.is_tag("minecraft:wither_immune_to") || matches!(attacker, Some("minecraft:wither" | "minecraft:wither_skull")) {
            return Some(false);
        }
        if st(m).invul > 0 && !source.kind.is_tag("minecraft:bypasses_invulnerability") {
            return Some(false);
        }
        if is_powered(m) {
            let direct = type_of(source.direct);
            let arrow = matches!(direct, Some("minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:trident" | "minecraft:wind_charge"))
                || (direct.is_none() && matches!(source.kind, DamageKind::Arrow | DamageKind::Trident));
            if arrow {
                return Some(false);
            }
        }
        if attacker.is_some_and(|t| mob::entity_type_tag(t, "minecraft:wither_friends")) {
            return Some(false);
        }
        let s = st_mut(m);
        if s.destroy_blocks_tick <= 0 {
            s.destroy_blocks_tick = 20;
        }
        for i in &mut s.idle_head_updates {
            *i += 3;
        }
        Some(mob::hurt_base(e, m, level, *source, amount))
    }

    /// `dropCustomDeathLoot`: a nether star that lasts (`setExtendedLifetime`).
    fn die(&self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel, _source: &DamageSource) {
        if !level.mob_drops() {
            return;
        }
        let Some(star) = kiln_item::ItemStack::of("minecraft:nether_star", 1) else { return };
        // `spawnAtLocation` with the item's own random throw.
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut item = crate::item::new(id, 0, star, seed);
        item.set_pos(e.position());
        let dx = item.random.next_double() * 0.2 - 0.1;
        let dz = item.random.next_double() * 0.2 - 0.1;
        item.delta = Vec3::new(dx, 0.2, dz);
        if let crate::entity::EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
            d.age = -6000;
        }
        item.set_old_pos_and_rot();
        level.add_entity(item);
    }

    /// `WitherBoss.addEffect` always refuses (and `canBeAffected` excludes wither).
    fn can_be_affected(&self, _m: &MobData, _effect: &crate::effect::Effect, _base: bool) -> bool {
        false
    }

    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(50)
    }

    fn check_despawn(&self, e: &mut Entity, level: &dyn EntityLevel) -> bool {
        if level.difficulty() == 0 {
            e.discard();
        } else if let Some(m) = mob::data_mut(e) {
            m.no_action_time = 0;
        }
        true
    }

    fn checks_fall_damage(&self) -> bool {
        // `FlyingPathNavigation` mobs still fall; the wither has no fall damage override.
        true
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let invul = r.int_or("Invul", 0);
        st_mut(m).invul = invul;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("Invul", Tag::Int(st(m).invul));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::wither_boss as w;
        let s = st(m);
        d.set(w::TARGET_A, &DataValue::Int(s.targets[0]));
        d.set(w::TARGET_B, &DataValue::Int(s.targets[1]));
        d.set(w::TARGET_C, &DataValue::Int(s.targets[2]));
        d.set(w::ID_INV, &DataValue::Int(s.invul));
    }
}

// ---------------------------------------------------------------------- goals

/// `WitherDoNothingGoal`: holds movement, jumping and looking while invulnerable.
#[derive(Clone, Debug)]
struct DoNothing;

impl CustomGoal for DoNothing {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "WitherDoNothingGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP | LOOK
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        st(m).invul > 0
    }
}

/// `RangedAttackGoal` (the middle head's skulls).
#[derive(Clone, Debug)]
struct RangedAttack {
    target: Option<i32>,
    attack_time: i32,
    see_time: i32,
    speed: f64,
    interval_min: i32,
    interval_max: i32,
    radius: f32,
}

impl CustomGoal for RangedAttack {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RangedAttackGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        match goals::target(m, level) {
            Some(t) if t.alive => {
                self.target = Some(t.id);
                true
            }
            _ => false,
        }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level) || (self.target.and_then(|id| goals::living(level, id)).is_some_and(|t| t.alive) && !m.nav.is_done())
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.target = None;
        self.see_time = 0;
        self.attack_time = -1;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.target.and_then(|id| goals::living(level, id)) else { return };
        let d = e.position().distance_to_sqr(t.pos);
        let sees = mob::has_line_of_sight_cached(e, m, level, &t);
        if sees {
            self.see_time += 1;
        } else {
            self.see_time = 0;
        }
        let r2 = (self.radius * self.radius) as f64;
        if d <= r2 && self.see_time >= 5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), self.speed);
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = (d.sqrt() as f32) / self.radius;
            // `performRangedAttack(target, power)`: the power is unused.
            shoot_at(e, m, level, 0, &t);
            self.attack_time = mth_floor_f(dist * (self.interval_max - self.interval_min) as f32 + self.interval_min as f32);
        } else if self.attack_time < 0 {
            let t = d.sqrt() / self.radius as f64;
            let lerp = self.interval_min as f64 + t * (self.interval_max - self.interval_min) as f64;
            self.attack_time = crate::math::floor(lerp);
        }
    }
}

/// `Mth.nextDouble(random, min, max)`.
fn next_double(r: &mut dyn RandomSource, min: f64, max: f64) -> f64 {
    if min >= max { min } else { r.next_double() * (max - min) + min }
}

/// `Mth.floor(float)`.
fn mth_floor_f(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}

/// `WaterAvoidingRandomFlyingGoal`: every so often a spot a few blocks over the ground along
/// the look, else one in the air (`RandomStrollGoal` with `HoverRandomPos` /
/// `AirAndWaterRandomPos`).
#[derive(Clone, Debug)]
struct RandomFlying {
    wanted: Vec3,
    speed: f64,
    interval: i32,
    force: bool,
}

impl CustomGoal for RandomFlying {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "WaterAvoidingRandomFlyingGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !self.force {
            if m.no_action_time >= 100 {
                return false;
            }
            if e.random.next_int_bounded(mth::reduced_tick_delay(self.interval)) != 0 {
                return false;
            }
        }
        // `getViewVector(0)`: the rotations of the tick's start.
        let view = fireball::view_vector(e.x_rot_o, m.y_head_rot_o);
        let angle = std::f32::consts::FRAC_PI_2;
        let p = random_pos::hover_pos(e, m, level, 8, 7, view.x, view.z, angle, 3, 1)
            .or_else(|| random_pos::air_and_water_pos(e, m, level, 8, 4, -2, view.x, view.z, angle as f64));
        match p {
            Some(p) => {
                self.wanted = p;
                self.force = false;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, self.speed);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// `NearestAttackableTargetGoal<LivingEntity>` with no random interval and the wither's
/// selector: the nearest living thing in the follow range it can see.
#[derive(Clone, Debug)]
struct NearestLiving {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for NearestLiving {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NearestAttackableTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let range = m.attrs.value(Attr::FollowRange);
        let area = e.bounding_box().inflate(range, range, range);
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Living, i32::MIN) {
            let Some(t) = goals::living(level, id) else { continue };
            if !selector(&t) || !goals::targeting_ok(e, m, level, &t, true, range, true) {
                continue;
            }
            let (dx, dy, dz) = (t.pos.x - e.x(), t.pos.y - e.eye_y(), t.pos.z - e.z());
            let d = dx * dx + dy * dy + dz * dz;
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.target = best.map(|(_, id)| id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, None, false, &mut self.unseen, 60)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, self.target);
        self.unseen = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, None);
        self.target = None;
    }
}
