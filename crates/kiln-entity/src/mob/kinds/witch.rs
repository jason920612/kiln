//! Witch: throws splash potions at players, drinks potions (water breathing, fire resistance,
//! healing, swiftness), shrugs off most magic damage. Also the splash of thrown potions that
//! carry their item (`ThrownSplashPotion.onHitAsPotion` with 26.x's box-to-box distance).

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::attributes::Op;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, LOOK, MOVE, TARGET};
use crate::mob::{self, mth, path, DamageSource, MobData, MAINHAND};
use crate::projectile::{Hit, Throwable};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Witch;

pub static KIND: Witch = Witch;

static INFO: Info = Info::monster("minecraft:witch", &[(MaxHealth, 26.0), (MovementSpeed, 0.25)]);

/// `SPEED_MODIFIER_DRINKING`.
const DRINKING: &str = "minecraft:drinking";

#[derive(Clone, Debug)]
pub struct WitchState {
    /// `DATA_USING_ITEM` and `usingTime`.
    pub drinking: bool,
    pub using_time: i32,
    /// `NearestHealableRaiderTargetGoal.cooldown` (decremented every tick) and
    /// `NearestAttackableWitchTargetGoal.canAttack`.
    pub heal_cooldown: i32,
    pub can_attack: bool,
}

fn st(m: &MobData) -> &WitchState {
    ext::state::<WitchState>(m).expect("witch state")
}

fn st_mut(m: &mut MobData) -> &mut WitchState {
    ext::state_mut::<WitchState>(m).expect("witch state")
}

fn has_effect(m: &MobData, effect: &str) -> bool {
    mob::effects::has_named(m, effect)
}

/// A `minecraft:potion` item stack of `item` (`minecraft:potion`, `minecraft:splash_potion`).
fn potion_stack(item: &str, potion: &str) -> ItemStack {
    let mut s = ItemStack::of(item, 1).unwrap_or_else(ItemStack::empty);
    let contents = kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id(potion), ..Default::default() };
    s.insert(kiln_item::keys::POTION_CONTENTS, contents);
    s
}

fn potion_color(contents: &kiln_item::component::PotionContents) -> i32 {
    crate::effect::potion_color(contents)
}

impl Kind for Witch {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(WitchState { drinking: false, using_time: 0, heal_cooldown: 0, can_attack: true }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        // `Raider.registerGoals`: patrols, banners, raids and celebrations, none of which a
        // witch outside a raid ever starts (and none draws randomness).
        g.add(4, Goal::Never);
        g.add(1, Goal::Never);
        g.add(3, Goal::Never);
        g.add(4, Goal::Never);
        g.add(5, Goal::Never);
        g.add(1, Goal::Float);
        g.add(2, Goal::Custom(Box::new(RangedAttack { target: None, attack_time: -1, see_time: 0 })));
        g.add(2, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(3, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(3, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: false, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::Custom(Box::new(HealRaiders)));
        t.add(3, Goal::Custom(Box::new(AttackPlayers { target: None, unseen: 0 })));
    }

    /// `Raider.updateNoActionTime`: two more every tick, whatever the light.
    fn update_no_action_time(&self, _e: &Entity, m: &mut MobData, _level: &dyn EntityLevel) {
        m.no_action_time += 2;
    }

    /// `Witch.aiStep` before `Raider.aiStep`.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !mob::is_alive(e, m) {
            return;
        }
        {
            let s = st_mut(m);
            s.heal_cooldown -= 1;
            s.can_attack = s.heal_cooldown <= 0;
        }
        if st(m).drinking {
            let t = st(m).using_time;
            st_mut(m).using_time -= 1;
            if t <= 0 {
                st_mut(m).drinking = false;
                let held = std::mem::replace(&mut m.equipment[MAINHAND], ItemStack::empty());
                if mob::item_name(&held) == "minecraft:potion"
                    && let Some(contents) = held.get(kiln_item::keys::POTION_CONTENTS)
                {
                    for fx in crate::effect::potion_effects(contents, 1.0) {
                        mob::effects::add(e, m, level, fx, None);
                    }
                }
                level.emit(Event::GameEvent { event: "minecraft:drink", pos: e.position(), entity: Some(e.id) });
                m.attrs.remove_modifier(MovementSpeed, DRINKING);
            }
        } else {
            let mut potion = None;
            let fire_hurt = m.last_damage_source(level.game_time()).is_some_and(|s| s.kind.is_tag("minecraft:is_fire"));
            if e.random.next_float() < 0.15 && e.fluid.is_eye_in_water() && !has_effect(m, "minecraft:water_breathing") {
                potion = Some("minecraft:water_breathing");
            } else if e.random.next_float() < 0.15 && (e.is_on_fire() || fire_hurt) && !has_effect(m, "minecraft:fire_resistance") {
                potion = Some("minecraft:fire_resistance");
            } else if e.random.next_float() < 0.05 && m.health < m.max_health() {
                potion = Some("minecraft:healing");
            } else if e.random.next_float() < 0.5
                && let Some(t) = goals::target(m, level)
                && !has_effect(m, "minecraft:speed")
                && t.pos.distance_to_sqr(e.position()) > 121.0
            {
                potion = Some("minecraft:swiftness");
            }
            if let Some(p) = potion {
                m.equipment[MAINHAND] = potion_stack("minecraft:potion", p);
                // `getUseDuration` of a potion.
                let s = st_mut(m);
                s.using_time = 32;
                s.drinking = true;
                if !e.silent {
                    let pitch = 0.8 + e.random.next_float() * 0.4;
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.witch.drink", source: "hostile", volume: 1.0, pitch });
                }
                m.attrs.remove_modifier(MovementSpeed, DRINKING);
                m.attrs.set_modifier(MovementSpeed, DRINKING, -0.25, Op::AddValue);
            }
        }
        if e.random.next_float() < 7.5e-4 {
            level.emit(Event::EntityEvent { entity: e.id, event: 15 });
        }
    }

    fn damage_after_magic_absorb(&self, id: i32, _m: &MobData, source: &DamageSource, amount: f32) -> f32 {
        let mut amount = amount;
        if source.attacker == Some(id) {
            amount = 0.0;
        }
        if source.kind.is_tag("minecraft:witch_resistant_to") {
            amount *= 0.15;
        }
        amount
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::witch::USING_ITEM, &DataValue::Boolean(st(m).drinking));
    }
}

// ---------------------------------------------------------------------- throwing

/// What the witch knows of its target: velocity, health and effects.
fn target_facts(level: &dyn EntityLevel, t: &Living) -> (Vec3, f32, Box<dyn Fn(&str) -> bool>) {
    if let Some(p) = level.player(t.id) {
        return (Vec3::ZERO, p.health, Box::new(move |fx: &str| p.has_effect(fx)));
    }
    match level.entity(t.id) {
        Some(o) => {
            let m = mob::data(o);
            let health = m.map_or(20.0, |m| m.health);
            let effects: Vec<i32> = m.map_or(Vec::new(), |m| m.effects.keys().copied().collect());
            (o.delta, health, Box::new(move |fx: &str| crate::effect::effect_id(fx).is_some_and(|id| effects.contains(&id))))
        }
        None => (Vec3::ZERO, 20.0, Box::new(|_: &str| false)),
    }
}

/// `Witch.performRangedAttack`: harming by default, slowness from afar, poison on a healthy
/// target, now and then weakness up close.
fn perform_ranged_attack(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
    if st(m).drinking {
        return;
    }
    let (tv, health, has) = target_facts(level, t);
    let dx = t.pos.x + tv.x - e.x();
    let dy = t.eye_y - 1.100000023841858 - e.y();
    let dz = t.pos.z + tv.z - e.z();
    let dist = (dx * dx + dz * dz).sqrt();
    let mut potion = "minecraft:harming";
    if dist >= 8.0 && !has("minecraft:slowness") {
        potion = "minecraft:slowness";
    } else if health >= 8.0 && !has("minecraft:poison") {
        potion = "minecraft:poison";
    } else if dist <= 3.0 && !has("minecraft:weakness") && e.random.next_float() < 0.25 {
        potion = "minecraft:weakness";
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let pos = Vec3::new(e.x(), e.eye_y() - 0.10000000149011612, e.z());
    let mut p = crate::projectile::new(id, 0, Throwable::SplashPotion, pos, Vec3::ZERO, Some(e.id), seed);
    if let EntityKind::Throwable(d) = &mut p.kind {
        d.item = Some(potion_stack("minecraft:splash_potion", potion));
    }
    mob::species::shoot(&mut p, dx, dy + dist * 0.2, dz, if dist <= 2.0 { 0.45 } else { 0.75 }, 8.0);
    level.add_entity(p);
    if !e.silent {
        let pitch = 0.8 + e.random.next_float() * 0.4;
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.witch.throw", source: "hostile", volume: 1.0, pitch });
    }
}

/// `AABB.distanceToSqr(AABB)`.
fn box_distance_sqr(a: &Aabb, b: &Aabb) -> f64 {
    let dx = (a.min_x - b.max_x).max(b.min_x - a.max_x).max(0.0);
    let dy = (a.min_y - b.max_y).max(b.min_y - a.max_y).max(0.0);
    let dz = (a.min_z - b.max_z).max(b.min_z - a.max_z).max(0.0);
    dx * dx + dy * dy + dz * dz
}

/// `AbstractThrownPotion.onHit` for a splash potion that knows its item
/// (`ThrownSplashPotion.onHitAsPotion`): every living entity within reach of the hit box
/// (26.x's box-to-box distance with the projectile margin) gets the instantaneous effects at
/// the distance's scale and the others with scaled durations (dropped at 20 ticks or less);
/// then the splash particles and sound.
pub fn splash(e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit, item: &ItemStack, owner: Option<i32>) {
    let contents = item.get(kiln_item::keys::POTION_CONTENTS).cloned().unwrap_or_default();
    let duration_scale = item.get(kiln_item::keys::POTION_DURATION_SCALE).copied().unwrap_or(1.0);
    let location = match hit {
        Hit::Block { location, .. } | Hit::Entity { location, .. } => location,
    };
    let hit_box = e.bounding_box().offset_vec(location - e.position());
    let area = hit_box.inflate(4.0, 2.0, 4.0);
    let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0)) as f64;
    let effects = crate::effect::potion_effects(&contents, 1.0);
    if !effects.is_empty() {
        // Players (their stand-ins join the sections before the mobs around them), then mobs.
        let mut targets: Vec<(i32, f64)> = Vec::new();
        for p in level.players() {
            if !p.alive || p.spectator {
                continue;
            }
            let h = if p.sneaking { 1.5 } else { 1.8 };
            let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
            if pb.intersects(&area) {
                targets.push((p.id, box_distance_sqr(&hit_box, &pb.inflate_all(margin))));
            }
        }
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            if level.player(id).is_some() {
                continue;
            }
            let Some(o) = level.entity(id) else { continue };
            if mob::data(o).is_none_or(|om| om.is_dead_or_dying()) {
                continue;
            }
            targets.push((id, box_distance_sqr(&hit_box, &o.bounding_box().inflate_all(margin))));
        }
        let source = owner.or(Some(e.id));
        for (id, d) in targets {
            if d >= 16.0 {
                continue;
            }
            let scale = 1.0 - d.sqrt() / 4.0;
            for fx in &effects {
                if fx.kind().instantaneous() {
                    level.apply_instantaneous_effect(id, fx, Some((e.id, e.position())), owner, scale);
                } else {
                    let mut nf = crate::effect::Effect::with_flags(fx.id, fx.duration, fx.amplifier, fx.ambient, fx.visible);
                    nf.duration = fx.map_duration(|d| (scale * d as f64 * duration_scale as f64 + 0.5) as i32);
                    if !nf.ends_within(20) {
                        level.add_effect_instance(id, nf, source);
                    }
                }
            }
        }
    }
    break_effects(e, level, &contents);
}

/// `AbstractThrownPotion.onHit`'s particles and sound: the instant kind when the base potion
/// has an instantaneous effect.
fn break_effects(e: &Entity, level: &mut dyn EntityLevel, contents: &kiln_item::component::PotionContents) {
    let instant = contents.potion.and_then(|id| kiln_item::registry::POTION.name(id)).is_some_and(|p| {
        crate::effect::named_potion_effects(p, 1.0).iter().any(|fx| fx.kind().instantaneous())
    });
    let color = potion_color(contents);
    let pos = e.block_position();
    level.emit(Event::LevelEvent { event: if instant { 2007 } else { 2002 }, pos, data: color });
    if !e.silent {
        level.emit(Event::LevelEvent { event: if instant { 1054 } else { 1053 }, pos, data: 0 });
    }
}

/// `ThrownLingeringPotion.onHitAsPotion`: an area effect cloud at the entity hit, or where the
/// potion broke; then the particles and sound.
pub fn linger(e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit, item: &ItemStack, owner: Option<i32>) {
    let contents = item.get(kiln_item::keys::POTION_CONTENTS).cloned().unwrap_or_default();
    let has_effects = !crate::effect::potion_effects(&contents, 1.0).is_empty();
    if has_effects {
        let at = match hit {
            Hit::Entity { id, .. } => level.player(id).map(|p| p.pos).or_else(|| level.entity(id).map(Entity::position)).unwrap_or(e.position()),
            Hit::Block { .. } => e.position(),
        };
        // `setOwner` takes living owners only.
        let owner = owner.filter(|&o| level.player(o).is_some() || level.entity(o).is_some_and(|x| mob::data(x).is_some()));
        let owner_uuid = owner.and_then(|o| level.player(o).map(|p| p.uuid).or_else(|| level.entity(o).map(|x| x.uuid)));
        let cloud = crate::ext_entity::area_effect_cloud::lingering(item, at, owner, owner_uuid);
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let c = crate::ext_entity::area_effect_cloud::new(id, 0, at, cloud, seed);
        level.add_entity(c);
    }
    break_effects(e, level, &contents);
}

// ---------------------------------------------------------------------- goals

/// `RangedAttackGoal(this, 1.0, 60, 10)`.
#[derive(Clone, Debug)]
struct RangedAttack {
    target: Option<i32>,
    attack_time: i32,
    see_time: i32,
}

const RADIUS: f32 = 10.0;
const INTERVAL: i32 = 60;

impl CustomGoal for RangedAttack {
    custom_goal_boilerplate!();
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
        if !(d > (RADIUS * RADIUS) as f64) && self.see_time >= 5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(t.pos.x, t.pos.y, t.pos.z), 1.0);
        }
        m.look.set_look_at(t.pos.x, t.eye_y, t.pos.z, 30.0, 30.0);
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = d.sqrt() as f32 / RADIUS;
            perform_ranged_attack(e, m, level, &t);
            self.attack_time = crate::math::floor((dist * (INTERVAL - INTERVAL) as f32 + INTERVAL as f32) as f64);
        } else if self.attack_time < 0 {
            self.attack_time = crate::math::floor(crate::math::lerp(d.sqrt() / RADIUS as f64, INTERVAL as f64, INTERVAL as f64));
        }
    }
}

/// `NearestHealableRaiderTargetGoal`: without a raid it never starts, but a lone witch's
/// cooldown is always run out, so it flips a coin every time it is asked.
#[derive(Clone, Debug)]
struct HealRaiders;

impl CustomGoal for HealRaiders {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NearestHealableRaiderTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if st(m).heal_cooldown > 0 || !e.random.next_bool() {
            return false;
        }
        // `hasActiveRaid`: Kiln has no raids.
        false
    }
}

/// `NearestAttackableWitchTargetGoal<Player>` (random interval 10, must see).
#[derive(Clone, Debug)]
struct AttackPlayers {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for AttackPlayers {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NearestAttackableWitchTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !st(m).can_attack {
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
