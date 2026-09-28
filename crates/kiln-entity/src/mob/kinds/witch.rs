//! Witch: throws splash potions at players, drinks potions (water breathing, fire resistance,
//! healing, swiftness), shrugs off most magic damage. Also the splash of thrown potions that
//! carry their item (`ThrownSplashPotion.onHitAsPotion` with 26.x's box-to-box distance).

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::attributes::Op;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, Living, LOOK, MOVE, TARGET};
use crate::mob::{self, mth, path, DamageSource, MobData, MobKind, MAINHAND};
use crate::projectile::{Hit, Throwable};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Witch;

pub static KIND: Witch = Witch;

static INFO: Info = Info::monster("minecraft:witch", &[(MaxHealth, 26.0), (MovementSpeed, 0.25)]);

/// `SPEED_MODIFIER_DRINKING`.
const DRINKING: &str = "minecraft:drinking";

/// An active effect of a witch (Kiln's mobs have no general effect system).
#[derive(Clone, Debug)]
pub struct MobEffect {
    pub effect: &'static str,
    pub duration: i32,
    pub amplifier: i32,
}

#[derive(Clone, Debug)]
pub struct WitchState {
    /// `DATA_USING_ITEM` and `usingTime`.
    pub drinking: bool,
    pub using_time: i32,
    /// `NearestHealableRaiderTargetGoal.cooldown` (decremented every tick) and
    /// `NearestAttackableWitchTargetGoal.canAttack`.
    pub heal_cooldown: i32,
    pub can_attack: bool,
    pub effects: Vec<MobEffect>,
    /// `Raider` and `PatrollingMonster` state (witches join raids).
    pub raider: super::raider::RaiderState,
}

fn st(m: &MobData) -> &WitchState {
    ext::state::<WitchState>(m).expect("witch state")
}

fn st_mut(m: &mut MobData) -> &mut WitchState {
    ext::state_mut::<WitchState>(m).expect("witch state")
}

fn has_effect(m: &MobData, effect: &str) -> bool {
    st(m).effects.iter().any(|e| e.effect == effect)
}

/// The attribute modifier of an effect: (attribute, id, amount per level, operation).
fn effect_modifier(effect: &str) -> Option<(Attr, &'static str, f64, Op)> {
    Some(match effect {
        "minecraft:speed" => (MovementSpeed, "minecraft:effect.speed", 0.20000000298023224, Op::AddMultipliedTotal),
        "minecraft:slowness" => (MovementSpeed, "minecraft:effect.slowness", -0.15000000596046448, Op::AddMultipliedTotal),
        "minecraft:strength" => (AttackDamage, "minecraft:effect.strength", 3.0, Op::AddValue),
        "minecraft:weakness" => (AttackDamage, "minecraft:effect.weakness", -4.0, Op::AddValue),
        _ => return None,
    })
}

/// `LivingEntity.addEffect` on a witch: a new effect, or a stronger or longer one replaces the old.
pub fn add_effect(m: &mut MobData, effect: &'static str, duration: i32, amplifier: i32) {
    let s = st_mut(m);
    match s.effects.iter_mut().find(|e| e.effect == effect) {
        Some(e) => {
            if amplifier > e.amplifier || (amplifier == e.amplifier && duration > e.duration) {
                e.amplifier = amplifier;
                e.duration = duration;
            } else {
                return;
            }
        }
        None => s.effects.push(MobEffect { effect, duration, amplifier }),
    }
    if let Some((attr, id, amount, op)) = effect_modifier(effect) {
        m.attrs.set_modifier(attr, id, amount * (amplifier + 1) as f64, op);
    }
}

/// `tickEffects`: instant health heals, poison and regeneration tick, all run down.
fn tick_effects(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let mut effects = std::mem::take(&mut st_mut(m).effects);
    let mut gone = Vec::new();
    for fx in effects.iter_mut() {
        // `shouldApplyEffectTickThisTick` then `applyEffectTick`.
        let every = |base: i32| {
            let i = base >> fx.amplifier;
            i <= 0 || fx.duration % i == 0
        };
        match fx.effect {
            "minecraft:instant_health" if fx.duration >= 1 && mob::is_alive(e, m) => {
                let h = m.health + (4i32 << fx.amplifier).max(0) as f32;
                m.set_health(h);
            }
            "minecraft:poison" if every(25) && m.health > 1.0 => {
                mob::hurt(e, m, level, DamageSource::of(DamageKind::Magic), 1.0);
            }
            "minecraft:regeneration" if every(50) && m.health < m.max_health() => {
                let h = m.health + 1.0;
                m.set_health(h);
            }
            _ => {}
        }
        fx.duration -= 1;
        if fx.duration <= 0 {
            gone.push(fx.effect);
        }
    }
    effects.retain(|fx| fx.duration > 0);
    st_mut(m).effects = effects;
    for g in gone {
        if let Some((attr, id, _, _)) = effect_modifier(g) {
            m.attrs.remove_modifier(attr, id);
        }
    }
}

/// A `minecraft:potion` item stack of `item` (`minecraft:potion`, `minecraft:splash_potion`).
fn potion_stack(item: &str, potion: &str) -> ItemStack {
    let mut s = ItemStack::of(item, 1).unwrap_or_else(ItemStack::empty);
    let contents = kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id(potion), ..Default::default() };
    s.insert(kiln_item::keys::POTION_CONTENTS, contents);
    s
}

fn potion_of(s: &ItemStack) -> Option<&'static str> {
    s.get(kiln_item::keys::POTION_CONTENTS).and_then(|c| c.potion).and_then(|id| kiln_item::registry::POTION.name(id))
}

/// The effects of the potions witches use: (effect, duration, amplifier).
fn potion_effects(potion: &str) -> &'static [(&'static str, i32, i32)] {
    match potion {
        "minecraft:water_breathing" => &[("minecraft:water_breathing", 3600, 0)],
        "minecraft:fire_resistance" => &[("minecraft:fire_resistance", 3600, 0)],
        "minecraft:healing" => &[("minecraft:instant_health", 1, 0)],
        "minecraft:harming" => &[("minecraft:instant_damage", 1, 0)],
        "minecraft:swiftness" => &[("minecraft:speed", 3600, 0)],
        "minecraft:slowness" => &[("minecraft:slowness", 1800, 0)],
        "minecraft:poison" => &[("minecraft:poison", 900, 0)],
        "minecraft:weakness" => &[("minecraft:weakness", 1800, 0)],
        "minecraft:regeneration" => &[("minecraft:regeneration", 900, 0)],
        _ => &[],
    }
}

/// `PotionContents.getColor` (opaque) for the potions witches use.
fn potion_color(potion: &str) -> i32 {
    let rgb = match potion {
        "minecraft:water_breathing" => 10017472,
        "minecraft:fire_resistance" => 16750848,
        "minecraft:healing" => 16262179,
        "minecraft:harming" => 11101546,
        "minecraft:swiftness" => 3402751,
        "minecraft:slowness" => 9154528,
        "minecraft:poison" => 8889187,
        "minecraft:weakness" => 4738376,
        "minecraft:regeneration" => 13458603,
        _ => 0x385DC6,
    };
    (0xFF00_0000u32 as i32) | rgb
}

impl Kind for Witch {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(WitchState {
            drinking: false,
            using_time: 0,
            heal_cooldown: 0,
            can_attack: true,
            effects: Vec::new(),
            raider: Default::default(),
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        // `Raider.registerGoals`: patrols, banners, raids and celebrations.
        super::raider::register_raider_goals(m);
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(2, Goal::Custom(Box::new(RangedAttack { target: None, attack_time: -1, see_time: 0 })));
        g.add(2, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(3, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(3, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, super::raider::hurt_by_ignoring_raiders_alone());
        t.add(2, Goal::Custom(Box::new(HealRaiders { target: None, unseen: 0 })));
        t.add(3, Goal::Custom(Box::new(AttackPlayers { target: None, unseen: 0 })));
    }

    fn tick_effects(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        tick_effects(e, m, level);
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
                    && let Some(p) = potion_of(&held)
                {
                    for &(fx, d, a) in potion_effects(p) {
                        add_effect(m, fx, d, a);
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
        super::raider::ai_step_before(e, m, level);
    }

    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        super::raider::die(e, m, level, source);
    }

    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        Some(super::raider::remove_when_far_away(m, dist_sqr))
    }

    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &crate::mob::SpawnContext, group: &mut crate::mob::GroupData) {
        super::raider::finalize_spawn(m, r, group);
        ext::mob_finalize(m, r);
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut crate::persist::Input) {
        super::raider::load(m, r);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut crate::persist::Output) {
        super::raider::save(m, o);
    }

    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        // `hurtServer`: fire resistance stops fire damage.
        (source.kind.is_tag("minecraft:is_fire") && has_effect(m, "minecraft:fire_resistance")).then_some(false)
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
        d.set(kiln_data::entities::data::raider::IS_CELEBRATING, &DataValue::Boolean(st(m).raider.celebrating));
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
            let effects: Vec<&'static str> = m.and_then(ext::state::<WitchState>).map_or(Vec::new(), |s| s.effects.iter().map(|e| e.effect).collect());
            (o.delta, health, Box::new(move |fx: &str| effects.contains(&fx)))
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
    let raider = level.entity(t.id).and_then(mob::data).is_some_and(|om| super::raider::is_raider(om.kind));
    if raider {
        potion = if health <= 4.0 { "minecraft:healing" } else { "minecraft:regeneration" };
        mob::set_target(e, m, None);
    } else if dist >= 8.0 && !has("minecraft:slowness") {
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

fn inverted_heal_and_harm(type_name: &str) -> bool {
    let Some(id) = kiln_data::builtin_id("minecraft:entity_type", type_name) else { return false };
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:entity_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == "minecraft:inverted_healing_and_harm"))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

/// `AbstractThrownPotion.onHit` for a splash potion that knows its item: players in reach get
/// [`Event::PotionSplash`]; mobs take instant health and harm (undead inverted), witches keep
/// the other effects too (approximation: other mobs have no effects in Kiln). Then the splash
/// particles and sound.
pub fn splash(e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit, item: &ItemStack, owner: Option<i32>) {
    let potion = potion_of(item).unwrap_or("minecraft:water");
    let location = match hit {
        Hit::Block { location, .. } | Hit::Entity { location, .. } => location,
    };
    let hit_box = e.bounding_box().offset_vec(location - e.position());
    let area = hit_box.inflate(4.0, 2.0, 4.0);
    let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0)) as f64;
    let effects = potion_effects(potion);
    if !effects.is_empty() {
        for p in level.players().to_vec() {
            if !p.alive || p.spectator {
                continue;
            }
            let h = if p.sneaking { 1.5 } else { 1.8 };
            let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
            if !pb.intersects(&area) {
                continue;
            }
            let d = box_distance_sqr(&hit_box, &pb.inflate_all(margin));
            if d < 16.0 {
                level.emit(Event::PotionSplash { target: p.id, potion, scale: 1.0 - d.sqrt() / 4.0, owner });
            }
        }
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            let Some(o) = level.entity(id) else { continue };
            let Some(om) = mob::data(o) else { continue };
            if om.is_dead_or_dying() {
                continue;
            }
            let d = box_distance_sqr(&hit_box, &o.bounding_box().inflate_all(margin));
            if d >= 16.0 {
                continue;
            }
            let scale = 1.0 - d.sqrt() / 4.0;
            let inverted = inverted_heal_and_harm(o.type_name);
            for &(fx, dur, amp) in effects {
                match fx {
                    "minecraft:instant_health" | "minecraft:instant_damage" => {
                        let harm = fx == "minecraft:instant_damage";
                        if harm == inverted {
                            let heal = (scale * (4i32 << amp) as f64 + 0.5) as i32;
                            if let Some(om) = level.entity_mut(id).and_then(mob::data_mut)
                                && om.health > 0.0
                            {
                                let h = om.health + heal as f32;
                                om.set_health(h);
                            }
                        } else {
                            let dmg = (scale * (6i32 << amp) as f64 + 0.5) as i32;
                            let source = DamageSource { kind: DamageKind::IndirectMagic, attacker: owner, direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
                            if let Some(o) = level.entity_mut(id) {
                                let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
                                mob::hurt_entity(&mut o2, level, source, dmg as f32);
                                if let Some(slot) = level.entity_mut(id) {
                                    *slot = o2;
                                }
                            }
                        }
                    }
                    _ => {
                        let d = (scale * dur as f64 + 0.5) as i32;
                        if d > 20
                            && let Some(om) = level.entity_mut(id).and_then(mob::data_mut)
                            && om.kind == MobKind::Witch
                        {
                            add_effect(om, fx, d, amp);
                        }
                    }
                }
            }
        }
    }
    let instant = effects.iter().any(|(fx, _, _)| fx.starts_with("minecraft:instant_"));
    let pos = e.block_position();
    level.emit(Event::LevelEvent { event: if instant { 2007 } else { 2002 }, pos, data: potion_color(potion) });
    if !e.silent {
        level.emit(Event::LevelEvent { event: if instant { 1054 } else { 1053 }, pos, data: 0 });
    }
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

/// `NearestHealableRaiderTargetGoal`: in an active raid, now and then (a coin flip, then a
/// 10 second cooldown) the witch targets the nearest raider it sees (not another witch) to throw
/// healing at it.
#[derive(Clone, Debug)]
struct HealRaiders {
    target: Option<i32>,
    unseen: i32,
}

impl CustomGoal for HealRaiders {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "NearestHealableRaiderTargetGoal"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if st(m).heal_cooldown > 0 || !e.random.next_bool() {
            return false;
        }
        if !super::raider::has_active_raid(m, level) {
            return false;
        }
        let range = m.attrs.value(FollowRange);
        let area = e.bounding_box().inflate(range, 4.0, range);
        let eye = Vec3::new(e.x(), e.eye_y(), e.z());
        let mut best: Option<(f64, i32)> = None;
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            let raider = level.entity(id).and_then(mob::data).is_some_and(|om| super::raider::is_raider(om.kind) && om.kind != MobKind::Witch);
            if !raider {
                continue;
            }
            let Some(t) = goals::living(level, id) else { continue };
            if !goals::targeting_ok(e, m, level, &t, true, range, true) {
                continue;
            }
            let d = t.pos.distance_to_sqr(eye);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, id));
            }
        }
        self.target = best.map(|(_, id)| id);
        self.target.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::continue_target(e, m, level, self.target, true, &mut self.unseen, 60)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).heal_cooldown = mth::reduced_tick_delay(200);
        mob::set_target(e, m, self.target);
        self.unseen = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        mob::set_target(e, m, None);
        self.target = None;
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
