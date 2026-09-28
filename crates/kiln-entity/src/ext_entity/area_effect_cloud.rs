//! Area effect clouds (`AreaEffectCloud`): what lingering potions leave behind (and the ender
//! dragon's breath). After its wait time the cloud shrinks by `radius_per_tick` and every five
//! ticks gives its potion's effects to the living entities standing within its radius (players
//! and mobs; instantaneous effects at half strength), then leaves each of them alone for the
//! reapplication delay. Each use can shrink it (`radius_on_use`) or shorten it
//! (`duration_on_use`); it is gone when it ends or gets smaller than half a block.

use crate::effect::{self, Effect};
use crate::entity::{Entity, EntityKind};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, Vec3};
use crate::persist::{Input, Output};
use kiln_item::component::PotionContents;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::metadata::Particle;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const TYPE: &str = "minecraft:area_effect_cloud";

#[derive(Clone, Debug)]
pub struct AreaEffectCloud {
    pub radius: f32,
    pub waiting: bool,
    pub potion: PotionContents,
    pub potion_duration_scale: f32,
    /// `customParticle` (the dragon's breath); `None`: `entity_effect` in the potion's colour.
    pub custom_particle: Option<Particle>,
    /// Entity id -> the tick it may be affected again (`victims`).
    pub victims: Vec<(i32, i32)>,
    /// -1: forever.
    pub duration: i32,
    pub wait_time: i32,
    pub reapplication_delay: i32,
    pub duration_on_use: i32,
    pub radius_on_use: f32,
    pub radius_per_tick: f32,
    /// The living entity that made it (`owner`), and its UUID for saving.
    pub owner: Option<i32>,
    pub owner_uuid: Option<u128>,
    /// A loaded cloud's saved `Age`, taken over on its first tick.
    pub loaded_age: Option<i32>,
}

impl Default for AreaEffectCloud {
    fn default() -> Self {
        AreaEffectCloud {
            radius: 3.0,
            waiting: false,
            potion: PotionContents::default(),
            potion_duration_scale: 1.0,
            custom_particle: None,
            victims: Vec::new(),
            duration: -1,
            wait_time: 20,
            reapplication_delay: 20,
            duration_on_use: 0,
            radius_on_use: 0.0,
            radius_per_tick: 0.0,
            owner: None,
            owner_uuid: None,
            loaded_age: None,
        }
    }
}

/// A new cloud at `pos` with `cloud`'s settings (`new AreaEffectCloud(level, x, y, z)`).
pub fn new(id: i32, uuid: u128, pos: Vec3, cloud: AreaEffectCloud, seed: i64) -> Entity {
    let mut e = Entity::new(TYPE, id, uuid, EntityKind::Other { type_name: TYPE }, seed);
    e.no_physics = true;
    e.no_gravity = true;
    let mut cloud = cloud;
    cloud.radius = cloud.radius.clamp(0.0, 32.0);
    set_size(&mut e, cloud.radius);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e.kind = EntityKind::Ext(Box::new(cloud));
    e
}

/// `getDimensions`: `scalable(radius * 2, 0.5)`, keeping the position (`refreshDimensions`).
fn set_size(e: &mut Entity, radius: f32) {
    let w = radius * 2.0;
    if e.width != w || e.height != 0.5 {
        e.width = w;
        e.height = 0.5;
        e.eye_height = 0.425;
        let p = e.position();
        e.set_pos(p);
    }
}

/// `ThrownLingeringPotion.onHitAsPotion`: a cloud of the potion item's contents where the
/// potion broke (at the entity it hit, if any): radius 3 shrinking to nothing over 600 ticks,
/// half a block less per use, a 10 tick wait.
pub fn lingering(potion_item: &kiln_item::ItemStack, at: Vec3, owner: Option<i32>, owner_uuid: Option<u128>) -> AreaEffectCloud {
    let mut c = AreaEffectCloud {
        owner,
        owner_uuid,
        radius: 3.0,
        radius_on_use: -0.5,
        duration: 600,
        wait_time: 10,
        ..AreaEffectCloud::default()
    };
    c.radius_per_tick = -c.radius / c.duration as f32;
    // `applyComponentsFromItemStack`.
    if let Some(p) = potion_item.get(kiln_item::keys::POTION_CONTENTS) {
        c.potion = p.clone();
    }
    if let Some(s) = potion_item.get(kiln_item::keys::POTION_DURATION_SCALE) {
        c.potion_duration_scale = *s;
    }
    let _ = at;
    c
}

impl AreaEffectCloud {
    /// `DATA_PARTICLE`.
    pub fn particle(&self) -> Option<Particle> {
        if let Some(p) = &self.custom_particle {
            return Some(p.clone());
        }
        let kind = kiln_data::builtin_id("minecraft:particle_type", "minecraft:entity_effect")?;
        let color = effect::potion_color(&self.potion) | (0xFF00_0000u32 as i32);
        Some(Particle { kind, options: color.to_be_bytes().to_vec() })
    }

    fn has_effects(&self) -> bool {
        !self.potion.custom_effects.is_empty() || !effect::potion_effects(&self.potion, 1.0).is_empty()
    }

    /// `serverTick`.
    fn server_tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if self.duration != -1 && e.tick_count - self.wait_time >= self.duration {
            e.discard();
            return;
        }
        let should_wait = e.tick_count < self.wait_time;
        self.waiting = should_wait;
        if should_wait {
            return;
        }
        let mut radius = self.radius;
        if self.radius_per_tick != 0.0 {
            radius += self.radius_per_tick;
            if radius < 0.5 {
                e.discard();
                return;
            }
            self.set_radius(e, radius);
        }
        if e.tick_count % 5 != 0 {
            return;
        }
        let now = e.tick_count;
        self.victims.retain(|&(_, until)| now < until);
        if !self.has_effects() {
            self.victims.clear();
            return;
        }
        let effects = effect::potion_effects(&self.potion, self.potion_duration_scale);
        for id in targets(e, level) {
            if self.victims.iter().any(|&(v, _)| v == id) || !affected_by_any(level, id, &effects) {
                continue;
            }
            let Some(p) = position_of(level, id) else { continue };
            let (dx, dz) = (p.x - e.x(), p.z - e.z());
            if dx * dx + dz * dz > (radius * radius) as f64 {
                continue;
            }
            self.victims.push((id, now + self.reapplication_delay));
            for fx in &effects {
                if fx.kind().instantaneous() {
                    level.apply_instantaneous_effect(id, fx, Some((e.id, e.position())), self.owner, 0.5);
                } else {
                    level.add_effect_instance(id, fx.clone(), Some(e.id));
                }
            }
            if self.radius_on_use != 0.0 {
                radius += self.radius_on_use;
                if radius < 0.5 {
                    e.discard();
                    return;
                }
                self.set_radius(e, radius);
            }
            if self.duration_on_use != 0 && self.duration != -1 {
                self.duration += self.duration_on_use;
                if self.duration <= 0 {
                    e.discard();
                    return;
                }
            }
        }
    }

    fn set_radius(&mut self, e: &mut Entity, radius: f32) {
        self.radius = radius.clamp(0.0, 32.0);
        set_size(e, self.radius);
    }
}

/// The living entities in the cloud's box (`getEntitiesOfClass(LivingEntity, box)`): mobs and
/// players (by their stand-ins, or their views when they have none), affected by potions
/// (not spectators, not dying).
fn targets(e: &Entity, level: &dyn EntityLevel) -> Vec<i32> {
    let bb = e.bounding_box();
    let mut out = Vec::new();
    for id in level.entities_in(&bb, EntityFilter::Living, e.id) {
        if let Some(p) = level.player(id) {
            if p.alive && !p.spectator {
                out.push(id);
            }
        } else if level.entity(id).and_then(crate::mob::data).is_some_and(|m| !m.is_dead_or_dying()) {
            out.push(id);
        }
    }
    for p in level.players() {
        let h = if p.sneaking { 1.5 } else { 1.8 };
        let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
        if pb.intersects(&bb) && p.alive && !p.spectator && !out.contains(&p.id) {
            out.push(p.id);
        }
    }
    out
}

fn position_of(level: &dyn EntityLevel, id: i32) -> Option<Vec3> {
    match level.player(id) {
        Some(p) => Some(p.pos),
        None => level.entity(id).map(Entity::position),
    }
}

/// `!allEffects.stream().noneMatch(entity::canBeAffected)` (players take everything).
fn affected_by_any(level: &dyn EntityLevel, id: i32, effects: &[Effect]) -> bool {
    if level.player(id).is_some() {
        return !effects.is_empty();
    }
    let Some(m) = level.entity(id).and_then(crate::mob::data) else { return false };
    effects.iter().any(|fx| crate::mob::effects::can_be_affected(m, fx))
}

impl EntityExt for AreaEffectCloud {
    entity_ext_boilerplate!();

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if let Some(age) = self.loaded_age.take() {
            e.tick_count = age + 1;
        }
        set_size(e, self.radius);
        e.base_tick(level);
        self.server_tick(e, level);
    }

    fn save(&self, e: &Entity, o: &mut Output) {
        o.put("Age", Tag::Int(e.tick_count));
        o.put("Duration", Tag::Int(self.duration));
        o.put("WaitTime", Tag::Int(self.wait_time));
        o.put("ReapplicationDelay", Tag::Int(self.reapplication_delay));
        o.put("DurationOnUse", Tag::Int(self.duration_on_use));
        o.put("RadiusOnUse", Tag::Float(self.radius_on_use));
        o.put("RadiusPerTick", Tag::Float(self.radius_per_tick));
        o.put("Radius", Tag::Float(self.radius));
        if let Some(u) = self.owner_uuid {
            o.put("Owner", crate::persist::uuid_to_tag(u));
        }
        if self.potion != PotionContents::default() {
            use kiln_item::component::ComponentValue;
            o.put("potion_contents", self.potion.to_value().to_nbt());
        }
        if self.potion_duration_scale != 1.0 {
            o.put("potion_duration_scale", Tag::Float(self.potion_duration_scale));
        }
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::area_effect_cloud as f;
        d.set(f::RADIUS, &DataValue::Float(self.radius));
        if self.waiting {
            d.set(f::WAITING, &DataValue::Boolean(true));
        }
        if let Some(p) = self.particle() {
            d.set(f::PARTICLE, &DataValue::Particle(p));
        }
    }
}

/// Reads a saved cloud (`readAdditionalSaveData`; the custom particle is not kept).
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    use kiln_item::component::ComponentValue;
    let potion = r.get("potion_contents").and_then(|t| PotionContents::from_value(&kiln_item::Value::from_nbt(t)).ok()).unwrap_or_default();
    let age = r.int_or("Age", 0);
    Some(Box::new(AreaEffectCloud {
        loaded_age: Some(age),
        duration: r.int_or("Duration", -1),
        wait_time: r.int_or("WaitTime", 20),
        reapplication_delay: r.int_or("ReapplicationDelay", 20),
        duration_on_use: r.int_or("DurationOnUse", 0),
        radius_on_use: r.float_or("RadiusOnUse", 0.0),
        radius_per_tick: r.float_or("RadiusPerTick", 0.0),
        radius: r.float_or("Radius", 3.0).clamp(0.0, 32.0),
        owner_uuid: r.uuid("Owner"),
        potion,
        potion_duration_scale: r.float_or("potion_duration_scale", 1.0),
        ..AreaEffectCloud::default()
    }))
}
